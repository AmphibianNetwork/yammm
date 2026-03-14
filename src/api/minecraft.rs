//! Minecraft version manifest client for fetching version metadata,
//! downloading game JARs, and syncing asset indexes.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::api::error::ApiError;
use crate::api::ApiClient;

const MINECRAFT_META_URL: &str = "https://piston-meta.mojang.com";
const MAX_CONCURRENT_ASSET_DOWNLOADS: usize = 16;

crate::api::define_api_client!(MinecraftClient, MINECRAFT_META_URL);

impl MinecraftClient {
	/// Fetches the full Minecraft version manifest listing all releases and snapshots.
	pub async fn get_version_manifest(
		&self
	) -> Result<VersionManifest, ApiError> {
		let url = format!("{}/mc/game/version_manifest_v2.json", self.base_url);
		self.fetch_json(&url, Vec::new()).await
	}

	/// Fetches detailed version info (downloads, libraries, asset index) for a specific Minecraft version.
	pub async fn get_version_info(
		&self,
		version_id: &str,
	) -> Result<VersionInfo, ApiError> {
		let manifest = self.get_version_manifest().await?;
		let entry = manifest
			.versions
			.iter()
			.find(|v| v.id == version_id)
			.ok_or_else(|| {
				ApiError::not_found(format!(
					"Version '{}' not found in manifest",
					version_id
				))
			})?;

		let response = self.send_retried(&entry.url, Vec::new()).await?;
		let response = Self::ensure_success(response)?;
		let info: VersionInfo = response.json().await?;
		Ok(info)
	}

	/// Downloads a Minecraft JAR (client or server) if not already cached.
	pub async fn download_jar(
		&self,
		download_info: &DownloadInfo,
		version: &str,
		side: &str,
		cache_dir: &Path,
	) -> Result<std::path::PathBuf, ApiError> {
		let version_dir = cache_dir.join(version);
		let filename = format!("{}.jar", side);
		let dest = version_dir.join(&filename);

		if dest.exists() {
			tracing::debug!("MC JAR already cached: {}", dest.display());
			return Ok(dest);
		}

		std::fs::create_dir_all(&version_dir)?;

		tracing::info!("Downloading {}...", filename);
		let response =
			self.send_retried(&download_info.url, Vec::new()).await?;
		let response = Self::ensure_success(response)?;
		let bytes = response.bytes().await?;

		let computed_sha1 =
			crate::types::HashType::Sha1.compute_for_bytes(&bytes);
		if computed_sha1 != download_info.sha1 {
			return Err(ApiError::HashMismatch {
				name: filename,
				expected: download_info.sha1.clone(),
				actual: computed_sha1,
			});
		}

		std::fs::write(&dest, &bytes)?;

		Ok(dest)
	}

	/// Downloads all game assets referenced by the asset index, skipping already-cached objects.
	///
	/// Downloads are concurrent (up to 16 at a time) with SHA-1 verification.
	/// Returns an error if any assets fail to download or fail hash verification.
	pub async fn download_assets(
		&self,
		asset_index: &AssetIndex,
		assets_dir: &Path,
	) -> Result<(), ApiError> {
		let index_dir = assets_dir.join("indexes");
		let objects_dir = assets_dir.join("objects");

		std::fs::create_dir_all(&index_dir)?;
		std::fs::create_dir_all(&objects_dir)?;

		let index_file = index_dir.join(format!("{}.json", asset_index.id));
		if !index_file.exists() {
			let response =
				self.send_retried(&asset_index.url, Vec::new()).await?;
			let response = Self::ensure_success(response)?;
			let bytes = response.bytes().await?;
			std::fs::write(&index_file, &bytes)?;
		}

		let index_data = std::fs::read_to_string(&index_file)?;
		let index: AssetIndexFile = serde_json::from_str(&index_data)?;

		let to_download: Vec<(String, AssetObject)> = index
			.objects
			.iter()
			.filter(|(_, object)| {
				let prefix = &object.hash[..2];
				!objects_dir.join(prefix).join(&object.hash).exists()
			})
			.map(|(name, object)| (name.clone(), object.clone()))
			.collect();

		let total = index.objects.len();

		if to_download.is_empty() {
			return Ok(());
		}

		let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(
			MAX_CONCURRENT_ASSET_DOWNLOADS,
		));
		let mut tasks = tokio::task::JoinSet::new();
		let mut downloaded = 0u64;
		let mut failures: Vec<String> = Vec::new();

		for (name, object) in to_download {
			let permit = sem.clone().acquire_owned().await.map_err(|_| {
				ApiError::Network("semaphore closed unexpectedly".into())
			})?;
			let client = self.client.clone();
			let objects_dir = objects_dir.clone();

			tasks.spawn(async move {
				let _permit = permit;
				let prefix = &object.hash[..2];
				let obj_dir = objects_dir.join(prefix);
				std::fs::create_dir_all(&obj_dir)?;
				let url = format!(
					"https://resources.download.minecraft.net/{}/{}",
					prefix, object.hash
				);
				let response = client.get(&url).send().await?;
				if !response.status().is_success() {
					return Err(ApiError::http(
						response.status().as_u16(),
						format!("Failed to download asset {}", name),
					));
				}
				let bytes = response.bytes().await?;
				let computed_sha1 =
					crate::types::HashType::Sha1.compute_for_bytes(&bytes);
				if computed_sha1 != object.hash {
					return Err(ApiError::HashMismatch {
						name: name.clone(),
						expected: object.hash,
						actual: computed_sha1,
					});
				}
				let obj_path = obj_dir.join(&object.hash);
				std::fs::write(&obj_path, &bytes)?;
				Ok(name)
			});

			while tasks.len() > MAX_CONCURRENT_ASSET_DOWNLOADS {
				if let Some(result) = tasks.join_next().await {
					match result {
						Ok(Ok(_)) => {
							downloaded += 1;
						}
						Ok(Err(e)) => {
							tracing::warn!("{}", e);
							failures.push(format!("{}", e));
						}
						Err(e) => {
							failures.push(format!("task panicked: {}", e));
						}
					}
				}
			}
		}

		while let Some(result) = tasks.join_next().await {
			match result {
				Ok(Ok(_)) => {
					downloaded += 1;
				}
				Ok(Err(e)) => {
					tracing::warn!("{}", e);
					failures.push(format!("{}", e));
				}
				Err(e) => {
					failures.push(format!("task panicked: {}", e));
				}
			}
		}

		if !failures.is_empty() {
			return Err(ApiError::Io(std::io::Error::other(format!(
				"{} asset(s) failed: {}",
				failures.len(),
				failures.join(", ")
			))));
		}

		if downloaded > 0 {
			tracing::info!(
				"Downloaded {} new assets ({} total)",
				downloaded,
				total
			);
		}

		Ok(())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
	pub latest: LatestVersion,
	pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersion {
	pub release: String,
	pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
	pub id: String,
	#[serde(rename = "type")]
	pub version_type: String,
	pub url: String,
	pub time: String,
	#[serde(rename = "releaseTime")]
	pub release_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
	pub id: String,
	#[serde(rename = "type")]
	pub version_type: String,
	#[serde(rename = "mainClass")]
	pub main_class: String,
	#[serde(default)]
	pub downloads: VersionDownloads,
	#[serde(default)]
	pub libraries: Vec<Library>,
	#[serde(default, rename = "javaVersion")]
	pub java_version: Option<JavaVersion>,
	#[serde(default, rename = "assetIndex")]
	pub asset_index: Option<AssetIndex>,
	#[serde(default)]
	pub assets: Option<String>,
	#[serde(default)]
	pub arguments: Option<VersionArguments>,
	#[serde(default, rename = "minecraftArguments")]
	pub minecraft_arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndex {
	pub id: String,
	pub sha1: String,
	pub size: i64,
	#[serde(default)]
	pub total_size: Option<i64>,
	pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexFile {
	pub objects: std::collections::BTreeMap<String, AssetObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetObject {
	pub hash: String,
	pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionDownloads {
	#[serde(default)]
	pub client: Option<DownloadInfo>,
	#[serde(default)]
	pub server: Option<DownloadInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
	pub url: String,
	pub sha1: String,
	pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaVersion {
	pub component: String,
	#[serde(rename = "majorVersion")]
	pub major_version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
	pub name: String,
	#[serde(default)]
	pub downloads: Option<LibraryDownloads>,
	#[serde(default)]
	pub rules: Option<Vec<Rule>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDownloads {
	#[serde(default)]
	pub artifact: Option<Artifact>,
	#[serde(default)]
	pub classifiers: Option<std::collections::BTreeMap<String, Artifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
	pub url: String,
	#[serde(default)]
	pub sha1: Option<String>,
	#[serde(default)]
	pub size: Option<i64>,
	pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
	pub action: String,
	#[serde(default)]
	pub os: Option<OsRule>,
	#[serde(default)]
	pub features: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsRule {
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub arch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionArguments {
	#[serde(default)]
	pub jvm: Vec<serde_json::Value>,
	#[serde(default)]
	pub game: Vec<serde_json::Value>,
}

/// Resolves JVM arguments from the MC version info for the current platform.
/// Evaluates conditional rules, substitutes variables, and filters out
/// `-cp`/`${classpath}`/`${main_class}` entries (handled separately by the launcher).
pub fn resolve_mc_jvm_args(
	version_info: &VersionInfo,
	natives_dir: &Path,
	library_dir: &Path,
	version_name: &str,
) -> Vec<String> {
	let mut args = Vec::new();

	if let Some(ref arguments) = version_info.arguments {
		for entry in &arguments.jvm {
			let values = match entry {
				serde_json::Value::String(s) => vec![s.clone()],
				serde_json::Value::Object(obj) => {
					let Some(rules) = obj.get("rules") else {
						continue;
					};
					if !evaluate_rules(rules) {
						continue;
					}
					match obj.get("value") {
						Some(serde_json::Value::String(s)) => vec![s.clone()],
						Some(serde_json::Value::Array(arr)) => arr
							.iter()
							.filter_map(|v| v.as_str().map(String::from))
							.collect(),
						_ => continue,
					}
				}
				_ => continue,
			};

			for value in values {
				if value == "-cp"
					|| value == "${classpath}"
					|| value == "${main_class}"
				{
					continue;
				}
				let resolved = substitute_jvm_vars(
					&value,
					natives_dir,
					library_dir,
					version_name,
				);
				args.push(resolved);
			}
		}
	} else if let Some(ref mc_args) = version_info.minecraft_arguments {
		for arg in mc_args.split_whitespace() {
			if arg == "-cp" || arg == "${classpath}" || arg == "${main_class}" {
				continue;
			}
			let resolved = substitute_jvm_vars(
				arg,
				natives_dir,
				library_dir,
				version_name,
			);
			if !resolved.starts_with("--")
				&& !resolved.starts_with("-D")
				&& !resolved.starts_with("-X")
			{
				continue;
			}
			args.push(resolved);
		}
	}

	args
}

/// Resolves game arguments from the MC version info for the current platform.
pub fn resolve_mc_game_args(version_info: &VersionInfo) -> Vec<String> {
	let mut args = Vec::new();

	if let Some(ref arguments) = version_info.arguments {
		for entry in &arguments.game {
			let values = match entry {
				serde_json::Value::String(s) => vec![s.clone()],
				serde_json::Value::Object(obj) => {
					let Some(rules) = obj.get("rules") else {
						continue;
					};
					if !evaluate_rules(rules) {
						continue;
					}
					match obj.get("value") {
						Some(serde_json::Value::String(s)) => vec![s.clone()],
						Some(serde_json::Value::Array(arr)) => arr
							.iter()
							.filter_map(|v| v.as_str().map(String::from))
							.collect(),
						_ => continue,
					}
				}
				_ => continue,
			};
			args.extend(values);
		}
	}

	args
}

pub fn evaluate_rules(rules: &serde_json::Value) -> bool {
	let Some(arr) = rules.as_array() else {
		return false;
	};
	let mut allowed = false;
	for rule in arr {
		let action = rule.get("action").and_then(|a| a.as_str()).unwrap_or("");
		let os_rule = rule.get("os");
		let features = rule.get("features");

		let rule_matches = match_os_rule(os_rule) && match_features(features);

		if action == "allow" && rule_matches {
			allowed = true;
		} else if action == "disallow" && rule_matches {
			allowed = false;
		}
	}
	allowed
}

fn match_os_rule(os_rule: Option<&serde_json::Value>) -> bool {
	let Some(os) = os_rule else {
		return true;
	};
	if let Some(name) = os.get("name").and_then(|n| n.as_str()) {
		let current = crate::utils::current_os_name();
		if name != current {
			return false;
		}
	}
	if let Some(arch) = os.get("arch").and_then(|a| a.as_str()) {
		if arch == "x86" && !cfg!(target_arch = "x86") {
			return false;
		}
	}
	true
}

fn match_features(features: Option<&serde_json::Value>) -> bool {
	let Some(features) = features else {
		return true;
	};
	if let Some(obj) = features.as_object() {
		for value in obj.values() {
			if value.as_bool() == Some(true) {
				return false;
			}
		}
	}
	true
}

fn substitute_jvm_vars(
	s: &str,
	natives_dir: &Path,
	library_dir: &Path,
	version_name: &str,
) -> String {
	s.replace("${natives_directory}", &natives_dir.to_string_lossy())
		.replace("${library_directory}", &library_dir.to_string_lossy())
		.replace("${classpath_separator}", crate::utils::CLASSPATH_SEPARATOR)
		.replace("${version_name}", version_name)
		.replace("${launcher_name}", "yammm")
		.replace("${launcher_version}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_get_version_manifest() {
		let mut server = mockito::Server::new_async().await;
		let body = serde_json::json!({
			"latest": {
				"release": "1.20.4",
				"snapshot": "24w02a"
			},
			"versions": [{
				"id": "1.20.4",
				"type": "release",
				"url": format!("{}/v1/1.20.4.json", server.url()),
				"time": "2023-12-07T12:00:00+00:00",
				"releaseTime": "2023-12-07T12:00:00+00:00"
			}]
		});
		let _mock = server
			.mock("GET", "/mc/game/version_manifest_v2.json")
			.with_status(200)
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.create_async()
			.await;

		let client = MinecraftClient::new().with_base_url(server.url());
		let manifest = client.get_version_manifest().await.unwrap();
		assert_eq!(manifest.latest.release, "1.20.4");
		assert_eq!(manifest.versions.len(), 1);
		assert_eq!(manifest.versions[0].id, "1.20.4");
	}

	#[tokio::test]
	async fn test_get_version_info_not_found() {
		let mut server = mockito::Server::new_async().await;
		let body = serde_json::json!({
			"latest": {
				"release": "1.20.4",
				"snapshot": "24w02a"
			},
			"versions": [{
				"id": "1.20.4",
				"type": "release",
				"url": "https://example.com/1.20.4.json",
				"time": "2023-12-07T12:00:00+00:00",
				"releaseTime": "2023-12-07T12:00:00+00:00"
			}]
		});
		let _mock = server
			.mock("GET", "/mc/game/version_manifest_v2.json")
			.with_status(200)
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.create_async()
			.await;

		let client = MinecraftClient::new().with_base_url(server.url());
		let result = client.get_version_info("nonexistent").await;
		match result {
			Err(ApiError::NotFound(msg)) => {
				assert!(msg.contains("nonexistent"));
			}
			Err(e) => panic!("Expected NotFound, got: {:?}", e),
			Ok(_) => panic!("Expected error"),
		}
	}

	#[test]
	fn test_resolve_mc_jvm_args_with_arguments() {
		let vi = VersionInfo {
			id: "1.21.1".to_string(),
			version_type: "release".to_string(),
			main_class: "net.minecraft.client.main.Main".to_string(),
			downloads: VersionDownloads::default(),
			libraries: vec![],
			java_version: None,
			asset_index: None,
			assets: None,
			arguments: Some(VersionArguments {
				jvm: vec![
					serde_json::json!(
						"-Djava.library.path=${natives_directory}"
					),
					serde_json::json!("-cp"),
					serde_json::json!("${classpath}"),
					serde_json::json!("${main_class}"),
					serde_json::json!(
						"--add-opens=java.base/java.lang=ALL-UNNAMED"
					),
					serde_json::json!(
						"--add-opens=java.base/java.io=ALL-UNNAMED"
					),
				],
				game: vec![],
			}),
			minecraft_arguments: None,
		};
		let args = resolve_mc_jvm_args(
			&vi,
			Path::new("/natives"),
			Path::new("/libs"),
			"1.21.1",
		);
		assert!(args.contains(&"-Djava.library.path=/natives".to_string()));
		assert!(args.contains(
			&"--add-opens=java.base/java.lang=ALL-UNNAMED".to_string()
		));
		assert!(args.contains(
			&"--add-opens=java.base/java.io=ALL-UNNAMED".to_string()
		));
		assert!(!args.iter().any(|a| a == "-cp" || a == "${classpath}"));
	}

	#[test]
	fn test_resolve_mc_jvm_args_legacy() {
		let vi = VersionInfo {
			id: "1.12.2".to_string(),
			version_type: "release".to_string(),
			main_class: "net.minecraft.client.main.Main".to_string(),
			downloads: VersionDownloads::default(),
			libraries: vec![],
			java_version: None,
			asset_index: None,
			assets: None,
			arguments: None,
			minecraft_arguments: Some(
				"--username ${auth_player_name} --version ${version_name}"
					.to_string(),
			),
		};
		let args = resolve_mc_jvm_args(
			&vi,
			Path::new("/natives"),
			Path::new("/libs"),
			"1.12.2",
		);
		assert!(args.is_empty() || args.iter().all(|a| a.starts_with("-")));
	}

	#[test]
	fn test_version_info_deserialize_with_arguments() {
		let json = serde_json::json!({
			"id": "1.21.1",
			"type": "release",
			"mainClass": "net.minecraft.client.main.Main",
			"arguments": {
				"jvm": ["-Djava.library.path=${natives_directory}"],
				"game": ["--username", "${auth_player_name}"]
			}
		});
		let vi: VersionInfo = serde_json::from_value(json).unwrap();
		assert!(vi.arguments.is_some());
		assert_eq!(vi.arguments.as_ref().unwrap().jvm.len(), 1);
		assert_eq!(vi.arguments.as_ref().unwrap().game.len(), 2);
	}

	#[test]
	fn test_version_info_deserialize_legacy() {
		let json = serde_json::json!({
			"id": "1.12.2",
			"type": "release",
			"mainClass": "net.minecraft.client.main.Main",
			"minecraftArguments": "--username ${auth_player_name}"
		});
		let vi: VersionInfo = serde_json::from_value(json).unwrap();
		assert!(vi.arguments.is_none());
		assert_eq!(
			vi.minecraft_arguments.as_deref(),
			Some("--username ${auth_player_name}")
		);
	}
}
