//! CurseForge source implementation.
//!
//! Unlike Modrinth, CurseForge **requires an API key** for all operations.
//! Without it, the API returns 403. The key can be set via:
//! - `api_keys.curseforge` in the global config
//! - `CURSEFORGE_API_TOKEN` environment variable
//!
//! CurseForge uses numeric IDs for projects and files (not slugs), so
//! `parse_project_id` and `parse_file_id` validate that the identifiers
//! are numeric before making API calls.
//!
//! Download URLs from CurseForge can be empty (server-side policy). When
//! that happens, we fall back to `get_file_download_url` which resolves
//! the URL through the API's redirect mechanism.

use std::sync::Arc;

use crate::api::CurseForgeClient;
use crate::api::error::ApiError;
use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::{
	ModEnv, ModInfo, ModVersion, ProjectType, SourceDependency, VersionFilters,
};
use anyhow::Result;

const SOURCE: &str = "curseforge";

/// Log a warning when rate-limited, suggesting the user add an API key
/// for higher rate limits (unauthenticated access is heavily throttled).
fn log_rate_limit_warning(err: &ApiError) {
	if err.is_rate_limited() {
		tracing::warn!(
			"CurseForge API rate limited. Consider setting api_keys.curseforge in your config for higher rate limits."
		);
	}
}

fn map_cf_error(err: ApiError) -> ProviderError {
	log_rate_limit_warning(&err);
	ProviderError::from_api_error(err, SOURCE)
}

fn map_cf_error_for_mod(
	err: ApiError,
	mod_id: i64,
) -> ProviderError {
	log_rate_limit_warning(&err);
	match err {
		ApiError::NotFound(_) | ApiError::Http { status: 404, .. } => {
			ProviderError::NotFound {
				provider: SOURCE,
				what: mod_id.to_string(),
			}
		}
		other => ProviderError::from_api_error(other, SOURCE),
	}
}

#[derive(Clone, Debug)]
pub struct CurseForgeSource {
	client: Arc<CurseForgeClient>,
}

impl CurseForgeSource {
	pub fn new(
		api_key: Option<String>,
		http_client: reqwest::Client,
	) -> Self {
		let client = CurseForgeClient::new().with_client(http_client);
		let client = match api_key {
			Some(key) => client.with_api_key(key),
			None => client,
		};
		Self {
			client: Arc::new(client),
		}
	}

	fn parse_numeric_id(
		id: &str,
		label: &str,
	) -> Result<i64> {
		id.parse().map_err(|_| {
			crate::errors::YammmError::invalid_args(format!(
				"Invalid CurseForge {} (must be numeric): {}",
				label, id
			))
			.into()
		})
	}

	/// Ensure an API key is configured before making CurseForge API calls.
	/// Returns an error with setup instructions if no key is present.
	fn require_api_key(
		&self,
		operation: &str,
	) -> anyhow::Result<()> {
		if !self.client.has_api_key() {
			return Err(crate::errors::YammmError::config_error(
				format!(
					"CurseForge API key required for {}. Set api_keys.curseforge in your config or the CURSEFORGE_API_TOKEN environment variable.",
					operation
				),
			).into());
		}
		Ok(())
	}
}

impl ModSourceProvider for CurseForgeSource {
	fn name(&self) -> &str {
		"curseforge"
	}

	fn supports_search(&self) -> bool {
		true
	}

	fn get_mod_env(
		&self,
		mod_info: &ModInfo,
	) -> ModEnv {
		// The CurseForge API doesn't expose per-side flags the way Modrinth's
		// `client_side` / `server_side` fields do, so we can't tell whether an
		// arbitrary mod is client-only or server-only. Resource packs and
		// shaders, however, are always client-side by definition — infer those
		// from the project type. Everything else falls back to Both.
		match mod_info.project_type {
			Some(ProjectType::ResourcePack) | Some(ProjectType::Shader) => {
				ModEnv::Client
			}
			_ => ModEnv::Both,
		}
	}

	async fn search(
		&self,
		query: &str,
		filters: &SearchFilters,
	) -> ProviderResult<Vec<ModInfo>> {
		self.require_api_key("search")?;
		let loader_str = filters.version.loader.map(|l| l.as_str());
		let projects = self
			.client
			.search(
				query,
				filters.version.minecraft_version.as_deref(),
				loader_str,
				filters.limit,
				filters.offset,
			)
			.await
			.map_err(map_cf_error)?;
		Ok(projects
			.into_iter()
			.map(CurseForgeClient::to_mod_info)
			.collect())
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		self.require_api_key("fetching mod info")?;
		let mod_id = Self::parse_numeric_id(mod_id, "project ID")?;
		let project = self
			.client
			.get_mod(mod_id)
			.await
			.map_err(|e| map_cf_error_for_mod(e, mod_id))?;
		Ok(CurseForgeClient::to_mod_info(project))
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> ProviderResult<Vec<ModVersion>> {
		self.require_api_key("fetching versions")?;
		let mod_id = Self::parse_numeric_id(mod_id, "project ID")?;
		let loader_str = filters.loader.map(|l| l.as_str());
		let files = self
			.client
			.get_files(mod_id, filters.minecraft_version.as_deref(), loader_str)
			.await
			.map_err(|e| map_cf_error_for_mod(e, mod_id))?;

		let mut versions = Vec::new();
		for file in files {
			let download_url = match file.download_url {
				Some(ref url) if !url.is_empty() => url.clone(),
				_ => match self
					.client
					.get_file_download_url(mod_id, file.id)
					.await
				{
					Ok(url) => url,
					Err(e) => {
						tracing::warn!(
							"Failed to resolve download URL for file {}: {}",
							file.id,
							e
						);
						continue;
					}
				},
			};
			versions.push(CurseForgeClient::to_mod_version(file, download_url));
		}
		Ok(versions)
	}

	async fn get_dependencies(
		&self,
		mod_id: &str,
		version_id: &str,
	) -> ProviderResult<Vec<SourceDependency>> {
		self.require_api_key("fetching dependencies")?;
		let mod_id = Self::parse_numeric_id(mod_id, "project ID")?;
		let file_id = Self::parse_numeric_id(version_id, "file ID")?;

		let file = self.client.get_file(file_id).await.map_err(map_cf_error)?;

		if file.mod_id != mod_id {
			return Err(ProviderError::BadResponse {
				provider: SOURCE,
				message: format!(
					"file {} does not belong to project {}",
					version_id, mod_id
				),
			});
		}

		Ok(CurseForgeClient::to_source_dependencies(&file))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::ModSource;

	fn make_mod_info(project_type: Option<ProjectType>) -> ModInfo {
		ModInfo {
			id: "1".to_string(),
			name: String::new(),
			description: String::new(),
			source: ModSource::curseforge("1".to_string()),
			minecraft_versions: vec![],
			loaders: vec![],
			downloads: 0,
			url: String::new(),
			project_type,
			client_side: None,
			server_side: None,
		}
	}

	#[test]
	fn test_get_mod_env_resourcepack_is_client_only() {
		let source = CurseForgeSource::new(None, reqwest::Client::new());
		let info = make_mod_info(Some(ProjectType::ResourcePack));
		assert_eq!(source.get_mod_env(&info), ModEnv::Client);
	}

	#[test]
	fn test_get_mod_env_shader_is_client_only() {
		let source = CurseForgeSource::new(None, reqwest::Client::new());
		let info = make_mod_info(Some(ProjectType::Shader));
		assert_eq!(source.get_mod_env(&info), ModEnv::Client);
	}

	#[test]
	fn test_get_mod_env_mod_defaults_to_both() {
		let source = CurseForgeSource::new(None, reqwest::Client::new());
		let info = make_mod_info(Some(ProjectType::Mod));
		assert_eq!(source.get_mod_env(&info), ModEnv::Both);
	}

	#[test]
	fn test_get_mod_env_unknown_defaults_to_both() {
		let source = CurseForgeSource::new(None, reqwest::Client::new());
		let info = make_mod_info(None);
		assert_eq!(source.get_mod_env(&info), ModEnv::Both);
	}

	#[test]
	fn test_parse_numeric_id_valid() {
		assert_eq!(
			CurseForgeSource::parse_numeric_id("231093", "project").unwrap(),
			231093
		);
	}

	#[test]
	fn test_parse_numeric_id_invalid() {
		assert!(
			CurseForgeSource::parse_numeric_id("sodium", "project").is_err()
		);
	}

	#[test]
	fn test_require_api_key_present() {
		let source = CurseForgeSource::new(
			Some("test-key".to_string()),
			reqwest::Client::new(),
		);
		assert!(source.require_api_key("test").is_ok());
	}

	#[test]
	fn test_require_api_key_absent() {
		let source = CurseForgeSource::new(None, reqwest::Client::new());
		assert!(source.require_api_key("test").is_err());
	}
}
