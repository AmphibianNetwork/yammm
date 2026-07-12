use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::output;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallerLibrary {
	pub name: String,
	#[serde(default)]
	pub url: Option<String>,
	#[serde(default)]
	pub downloads: InstallerLibraryDownloads,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallerLibraryDownloads {
	#[serde(default)]
	pub artifact: Option<InstallerArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallerArtifact {
	pub url: String,
	#[serde(default)]
	pub sha1: Option<String>,
	#[serde(default)]
	pub size: Option<i64>,
	pub path: String,
}

pub async fn download_profile_libraries(
	profile: &super::profile::InstallProfile,
	cache_dir: &Path,
	http_client: &reqwest::Client,
) -> Result<Vec<PathBuf>> {
	let lib_dir = cache_dir.join("libraries");
	std::fs::create_dir_all(&lib_dir)?;

	let mut paths = Vec::new();
	let mut failures = Vec::new();
	let total = profile.libraries.len();
	let pb = output::download_progress(total as u64);
	pb.set_message("Downloading installer libraries");

	for lib in &profile.libraries {
		let outcome = if let Some(ref artifact) = lib.downloads.artifact {
			let dest = lib_dir.join(&artifact.path);
			let policy = crate::api::streaming::HashPolicy::from_optional(
				artifact.sha1.as_deref().map(|sha1| {
					crate::api::streaming::ExpectedHash {
						hash_type: crate::types::HashType::Sha1,
						hex: sha1,
					}
				}),
				"installer profile artifact has no sha1 in manifest",
			);
			download_one(
				http_client,
				&artifact.url,
				&dest,
				policy,
				&artifact.path,
			)
			.await
		} else if let Some(ref maven_base) = lib.url {
			let relative = crate::utils::maven::coords_to_path(&lib.name);
			let dest = lib_dir.join(&relative);
			let download_url =
				format!("{}/{}", maven_base.trim_end_matches('/'), relative);
			// Forge/NeoForge maven entries rarely include a hash; we still
			// stream + atomically rename, but we can't integrity-check.
			download_one(
				http_client,
				&download_url,
				&dest,
				crate::api::streaming::HashPolicy::AcceptedUnhashed {
					reason: "Forge/NeoForge maven library, no hash in manifest",
				},
				&lib.name,
			)
			.await
		} else {
			Ok(None)
		};

		match outcome {
			Ok(Some(path)) => paths.push(path),
			Ok(None) => {}
			Err((label, err)) => {
				failures.push(format!("{}: {}", label, err));
			}
		}
		pb.inc(1);
	}

	pb.finish_and_clear();

	if failures.is_empty() {
		Ok(paths)
	} else {
		Err(crate::errors::YammmError::download_failed(format!(
			"Failed to download {} installer lib(s): {}",
			failures.len(),
			failures.join(", ")
		))
		.into())
	}
}

/// Download one library to `dest`. Returns `Ok(Some(path))` on success
/// (including cache hits), `Err((label, msg))` on failure so the outer
/// loop can aggregate.
async fn download_one(
	http_client: &reqwest::Client,
	url: &str,
	dest: &Path,
	policy: crate::api::streaming::HashPolicy<'_>,
	label: &str,
) -> std::result::Result<Option<PathBuf>, (String, anyhow::Error)> {
	match crate::api::streaming::download_to_file(
		http_client,
		url,
		dest,
		policy,
		label,
	)
	.await
	{
		Ok(_) => Ok(Some(dest.to_path_buf())),
		Err(e) => Err((label.to_string(), e)),
	}
}

pub(crate) async fn collect_version_libs(
	version_libs: &[serde_json::Value],
	lib_dir: &Path,
	http_client: &reqwest::Client,
) -> Result<Vec<PathBuf>> {
	let mut jars = Vec::new();
	let mut failures = Vec::new();
	for lib in version_libs {
		let name = lib.get("name").and_then(|n| n.as_str());
		if let Some(downloads) = lib.get("downloads") {
			if let Some(artifact) = downloads.get("artifact")
				&& let Some(path_str) =
					artifact.get("path").and_then(|p| p.as_str())
			{
				let lib_path = lib_dir.join(path_str);
				if !lib_path.exists()
					&& let Some(url) =
						artifact.get("url").and_then(|u| u.as_str())
					&& !url.is_empty()
				{
					let sha1 = artifact.get("sha1").and_then(|s| s.as_str());
					let policy =
						crate::api::streaming::HashPolicy::from_optional(
							sha1.map(|hex| {
								crate::api::streaming::ExpectedHash {
									hash_type: crate::types::HashType::Sha1,
									hex,
								}
							}),
							"version manifest library artifact has no sha1",
						);
					if let Err(e) = crate::api::streaming::download_to_file(
						http_client,
						url,
						&lib_path,
						policy,
						path_str,
					)
					.await
					{
						failures.push(format!("{}: {}", path_str, e));
					}
				}
				if lib_path.exists() {
					jars.push(lib_path);
				}
			}
		} else if let Some(maven_url) = lib.get("url").and_then(|u| u.as_str())
			&& let Some(lib_name) = name
		{
			let relative = crate::utils::maven::coords_to_path(lib_name);
			let lib_path = lib_dir.join(&relative);
			if !lib_path.exists() {
				let download_url =
					format!("{}/{}", maven_url.trim_end_matches('/'), relative);
				if let Err(e) = crate::api::streaming::download_to_file(
					http_client,
					&download_url,
					&lib_path,
					crate::api::streaming::HashPolicy::AcceptedUnhashed {
						reason: "version manifest maven library, no hash exposed",
					},
					lib_name,
				)
				.await
				{
					failures.push(format!("{}: {}", lib_name, e));
				}
			}
			if lib_path.exists() {
				jars.push(lib_path);
			}
		}
	}
	if failures.is_empty() {
		Ok(jars)
	} else {
		Err(crate::errors::YammmError::download_failed(format!(
			"Failed to download {} version lib(s): {}",
			failures.len(),
			failures.join(", ")
		))
		.into())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_installer_library_deserialization() {
		let json = r#"{
			"name": "net.neoforged:neoforge:20.4.1",
			"downloads": {
				"artifact": {
					"url": "https://example.com/neoforge.jar",
					"sha1": "abc123",
					"size": 12345,
					"path": "net/neoforged/neoforge/20.4.1/neoforge.jar"
				}
			}
		}"#;
		let lib: InstallerLibrary = serde_json::from_str(json).unwrap();
		assert_eq!(lib.name, "net.neoforged:neoforge:20.4.1");
		let artifact = lib.downloads.artifact.unwrap();
		assert_eq!(artifact.url, "https://example.com/neoforge.jar");
		assert_eq!(artifact.sha1, Some("abc123".to_string()));
		assert_eq!(artifact.size, Some(12345));
		assert_eq!(artifact.path, "net/neoforged/neoforge/20.4.1/neoforge.jar");
	}

	#[test]
	fn test_installer_library_defaults() {
		let json = r#"{ "name": "test:lib:1.0" }"#;
		let lib: InstallerLibrary = serde_json::from_str(json).unwrap();
		assert!(lib.downloads.artifact.is_none());
		assert!(lib.url.is_none());
	}

	#[test]
	fn test_installer_library_with_url() {
		let json = r#"{
			"name": "net.neoforged.installertools:installertools:2.1.2",
			"url": "https://maven.neoforged.net/releases/"
		}"#;
		let lib: InstallerLibrary = serde_json::from_str(json).unwrap();
		assert_eq!(
			lib.url.as_deref(),
			Some("https://maven.neoforged.net/releases/")
		);
		assert!(lib.downloads.artifact.is_none());
	}

	#[test]
	fn test_installer_artifact_deserialization_minimal() {
		let json = r#"{
			"url": "https://example.com/lib.jar",
			"path": "com/example/lib/1.0/lib.jar"
		}"#;
		let artifact: InstallerArtifact = serde_json::from_str(json).unwrap();
		assert!(artifact.sha1.is_none());
		assert!(artifact.size.is_none());
	}

	#[test]
	fn test_installer_library_downloads_default() {
		let downloads: InstallerLibraryDownloads =
			serde_json::from_str("{}").unwrap();
		assert!(downloads.artifact.is_none());
	}
}
