//! NeoForge mod loader client for querying available versions,
//! resolving installer URLs, and running the NeoForge install pipeline.

use crate::api::error::ApiError;
use crate::api::installer::{self, InstallParams, LoaderInstallResult};
use crate::api::ApiClient;
use crate::output;
use crate::utils::maven::parse_maven_versions;

const NEOFORGE_MAVEN_URL: &str = "https://maven.neoforged.net/releases";

crate::api::define_api_client!(NeoForgeClient, NEOFORGE_MAVEN_URL);

impl NeoForgeClient {
	/// Returns all NeoForge loader versions available for a given Minecraft game version.
	pub async fn get_available_versions(
		&self,
		game_version: &str,
	) -> Result<Vec<String>, ApiError> {
		let url = format!(
			"{}/net/neoforged/neoforge/maven-metadata.xml",
			self.base_url
		);
		let response = self.send_retried(&url, Vec::new()).await?;
		let response = Self::ensure_success(response)?;
		let xml = response.text().await?;
		parse_neoforge_versions(&xml, game_version)
	}

	/// Returns the best NeoForge version for a given Minecraft game version,
	/// preferring stable releases over alpha/beta.
	pub async fn get_latest_version(
		&self,
		game_version: &str,
	) -> Result<String, ApiError> {
		let versions = self.get_available_versions(game_version).await?;
		pick_best_version(&versions, game_version).ok_or_else(|| {
			ApiError::not_found(format!(
				"No NeoForge versions found for MC {}",
				game_version
			))
		})
	}

	/// Builds the Maven URL for a NeoForge installer JAR.
	pub fn get_installer_url(
		&self,
		loader_version: &str,
	) -> String {
		format!(
			"{}/net/neoforged/neoforge/{}/neoforge-{}-installer.jar",
			self.base_url, loader_version, loader_version
		)
	}

	/// Downloads the NeoForge installer, extracts its install profile, and runs
	/// the full processor-based installation pipeline.
	pub async fn install(
		&self,
		params: &InstallParams<'_>,
	) -> Result<LoaderInstallResult, ApiError> {
		let loader_ver = if params.loader_version.is_empty() {
			output::info("Fetching latest NeoForge loader version...");
			self.get_latest_version(params.game_version).await?
		} else {
			let expected_prefix =
				mc_to_neoforge_version_prefix(params.game_version);
			if !params.loader_version.starts_with(&expected_prefix) {
				output::warning(format!(
					"NeoForge {} may not be compatible with MC {} \
					 (expected version starting with {})",
					params.loader_version, params.game_version, expected_prefix
				));
			}
			params.loader_version.to_string()
		};
		output::bullet(format!("NeoForge loader version: {}", loader_ver));

		let installer_url = self.get_installer_url(&loader_ver);
		let neoforge_cache = params
			.cache_dir
			.join("neoforge")
			.join(params.game_version)
			.join(&loader_ver);
		let installer_filename =
			format!("neoforge-{}-installer.jar", loader_ver);

		installer::download_and_run_installer(
			"NeoForge",
			&installer_url,
			&installer_filename,
			params.side,
			params.mc_jar,
			&neoforge_cache,
			params.root_dir,
			params.java_path,
			&self.client,
		)
		.await
		.map_err(ApiError::install_error)
	}
}

fn parse_neoforge_versions(
	xml: &str,
	game_version: &str,
) -> Result<Vec<String>, ApiError> {
	let prefix = mc_to_neoforge_version_prefix(game_version);
	Ok(parse_maven_versions(xml, Some(&prefix)))
}

fn neoforge_version_stability(v: &str) -> u8 {
	if v.contains("-alpha") {
		0
	} else if v.contains("-beta") {
		1
	} else {
		2
	}
}

fn neoforge_version_sort_key(v: &str) -> (u8, Vec<u32>) {
	let stability = neoforge_version_stability(v);
	let base = v.split('-').next().unwrap_or(v);
	let nums: Vec<u32> =
		base.split('.').filter_map(|p| p.parse().ok()).collect();
	(stability, nums)
}

fn pick_best_version(
	versions: &[String],
	_game_version: &str,
) -> Option<String> {
	versions
		.iter()
		.max_by_key(|v| neoforge_version_sort_key(v))
		.cloned()
}

fn mc_to_neoforge_version_prefix(mc_version: &str) -> String {
	let parts: Vec<u32> = mc_version
		.split('.')
		.filter_map(|p| p.parse().ok())
		.collect();

	if parts.len() >= 2 {
		let major = parts.first().copied().unwrap_or(1);
		let minor = parts.get(1).copied().unwrap_or(0);
		let patch = parts.get(2).copied().unwrap_or(0);
		if major == 1 {
			if patch > 0 {
				format!("{}.{}.", minor, patch)
			} else {
				format!("{}.", minor)
			}
		} else {
			format!("{}.{}.{}.", major, minor, patch)
		}
	} else {
		mc_version.to_string()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_neoforge_versions() {
		let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <versioning>
    <versions>
      <version>21.1.172</version>
      <version>21.1.171</version>
      <version>21.0.167</version>
      <version>20.4.238</version>
    </versions>
  </versioning>
</metadata>"#;
		let versions = parse_neoforge_versions(xml, "1.21.1").unwrap();
		assert_eq!(versions, vec!["21.1.172", "21.1.171"]);
		let versions_20 = parse_neoforge_versions(xml, "1.20.4").unwrap();
		assert_eq!(versions_20, vec!["20.4.238"]);
	}

	#[test]
	fn test_neoforge_installer_url() {
		let client = NeoForgeClient::new();
		let url = client.get_installer_url("21.1.172");
		assert_eq!(
			url,
			"https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.172/neoforge-21.1.172-installer.jar"
		);
	}

	#[test]
	fn test_mc_to_neoforge_prefix() {
		assert_eq!(mc_to_neoforge_version_prefix("1.21.1"), "21.1.");
		assert_eq!(mc_to_neoforge_version_prefix("1.21"), "21.");
		assert_eq!(mc_to_neoforge_version_prefix("1.20.4"), "20.4.");
		assert_eq!(mc_to_neoforge_version_prefix("1.16.5"), "16.5.");
		assert_eq!(mc_to_neoforge_version_prefix("26.1"), "26.1.0.");
		assert_eq!(mc_to_neoforge_version_prefix("26.1.2"), "26.1.2.");
	}

	#[test]
	fn test_pick_best_version_prefers_stable() {
		let versions: Vec<String> = vec![
			"26.1.2.0-beta".to_string(),
			"26.1.2.5-beta".to_string(),
			"26.1.2.10".to_string(),
			"26.1.2.3-alpha".to_string(),
		];
		assert_eq!(
			pick_best_version(&versions, "26.1.2"),
			Some("26.1.2.10".to_string())
		);
	}

	#[test]
	fn test_pick_best_version_falls_back_to_beta() {
		let versions: Vec<String> = vec![
			"26.1.2.0-alpha".to_string(),
			"26.1.2.5-beta".to_string(),
			"26.1.2.29-beta".to_string(),
		];
		assert_eq!(
			pick_best_version(&versions, "26.1.2"),
			Some("26.1.2.29-beta".to_string())
		);
	}

	#[test]
	fn test_pick_best_version_empty() {
		assert_eq!(pick_best_version(&[], "26.1"), None);
	}
}
