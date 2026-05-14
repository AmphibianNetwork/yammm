//! Forge mod loader client for querying available versions,
//! resolving installer URLs, and running the Forge install pipeline.

use crate::api::ApiClient;
use crate::api::error::ApiError;
use crate::api::installer::{self, InstallParams, LoaderInstallResult};
use crate::output;
use crate::utils::maven::parse_maven_versions;

const FORGE_MAVEN_URL: &str = "https://maven.minecraftforge.net";

crate::api::define_api_client!(ForgeClient, FORGE_MAVEN_URL);

impl ForgeClient {
	/// Returns all Forge loader versions available for a given Minecraft game version.
	pub async fn get_available_versions(
		&self,
		game_version: &str,
	) -> Result<Vec<String>, ApiError> {
		let url = format!(
			"{}/net/minecraftforge/forge/maven-metadata.xml",
			self.base_url
		);
		let response = self.send_retried(&url, Vec::new()).await?;
		let response = Self::ensure_success(response)?;
		let xml = response.text().await?;
		parse_forge_versions(&xml, game_version)
	}

	/// Returns the latest Forge version for a given Minecraft game version.
	pub async fn get_latest_version(
		&self,
		game_version: &str,
	) -> Result<String, ApiError> {
		let versions = self.get_available_versions(game_version).await?;
		versions.last().cloned().ok_or_else(|| {
			ApiError::not_found(format!(
				"No Forge versions found for MC {}",
				game_version
			))
		})
	}

	/// Builds the Maven URL for a Forge installer JAR.
	pub fn get_installer_url(
		&self,
		game_version: &str,
		loader_version: &str,
	) -> String {
		let version_tag = format!("{}-{}", game_version, loader_version);
		format!(
			"{}/net/minecraftforge/forge/{}/forge-{}-installer.jar",
			self.base_url, version_tag, version_tag
		)
	}

	/// Downloads the Forge installer, extracts its install profile, and runs
	/// the full processor-based installation pipeline.
	pub async fn install(
		&self,
		params: &InstallParams<'_>,
	) -> Result<LoaderInstallResult, ApiError> {
		let loader_ver = if params.loader_version.is_empty() {
			output::info("Fetching latest Forge loader version...");
			self.get_latest_version(params.game_version).await?
		} else {
			params.loader_version.to_string()
		};
		output::bullet(format!("Forge loader version: {}", loader_ver));

		let installer_url =
			self.get_installer_url(params.game_version, &loader_ver);
		let forge_cache = params
			.cache_dir
			.join("forge")
			.join(params.game_version)
			.join(&loader_ver);
		let installer_filename = format!(
			"forge-{}-{}-installer.jar",
			params.game_version, loader_ver
		);

		installer::download_and_run_installer(
			"Forge",
			&installer_url,
			&installer_filename,
			params.side,
			params.mc_jar,
			&forge_cache,
			params.root_dir,
			params.java_path,
			&self.client,
		)
		.await
		.map_err(ApiError::install_error)
	}
}

fn parse_forge_versions(
	xml: &str,
	game_version: &str,
) -> Result<Vec<String>, ApiError> {
	let prefix = format!("{}-", game_version);
	Ok(parse_maven_versions(xml, Some(&prefix))
		.iter()
		.filter_map(|v| v.strip_prefix(&prefix))
		.map(|s| s.to_string())
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_forge_versions() {
		let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <versioning>
    <versions>
      <version>1.21.1-52.0.40</version>
      <version>1.21.1-52.0.39</version>
      <version>1.20.4-49.1.12</version>
      <version>1.20.4-49.1.11</version>
    </versions>
  </versioning>
</metadata>"#;
		let versions = parse_forge_versions(xml, "1.21.1").unwrap();
		assert_eq!(versions, vec!["52.0.40", "52.0.39"]);
	}

	#[test]
	fn test_forge_installer_url() {
		let client = ForgeClient::new();
		let url = client.get_installer_url("1.21.1", "52.0.40");
		assert_eq!(
			url,
			"https://maven.minecraftforge.net/net/minecraftforge/forge/1.21.1-52.0.40/forge-1.21.1-52.0.40-installer.jar"
		);
	}
}
