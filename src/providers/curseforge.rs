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

use crate::api::error::ApiError;
use crate::api::CurseForgeClient;
use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::{ModInfo, ModVersion, SourceDependency, VersionFilters};
use anyhow::Result;

/// Log a warning when rate-limited, suggesting the user add an API key
/// for higher rate limits (unauthenticated access is heavily throttled).
fn log_rate_limit_warning(err: &ApiError) {
	if err.is_rate_limited() {
		tracing::warn!("CurseForge API rate limited. Consider setting api_keys.curseforge in your config for higher rate limits.");
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

	async fn search(
		&self,
		query: &str,
		filters: &SearchFilters,
	) -> Result<Vec<ModInfo>> {
		self.require_api_key("search")?;
		let loader_str = filters.version.loader.as_ref().map(|l| l.to_string());
		let projects = self
			.client
			.search(
				query,
				filters.version.minecraft_version.as_deref(),
				loader_str.as_deref(),
			)
			.await
			.map_err(|e| {
				log_rate_limit_warning(&e);
				crate::errors::YammmError::network_error(format!(
					"Search failed: {}",
					e
				))
			})?;
		Ok(projects
			.into_iter()
			.map(CurseForgeClient::to_mod_info)
			.collect())
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> Result<ModInfo> {
		self.require_api_key("fetching mod info")?;
		let mod_id = Self::parse_numeric_id(mod_id, "project ID")?;
		let project = self.client.get_mod(mod_id).await.map_err(|e| {
			log_rate_limit_warning(&e);
			crate::errors::YammmError::network_error(format!(
				"Failed to fetch CurseForge project {}: {}",
				mod_id, e
			))
		})?;
		Ok(CurseForgeClient::to_mod_info(project))
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> Result<Vec<ModVersion>> {
		self.require_api_key("fetching versions")?;
		let mod_id = Self::parse_numeric_id(mod_id, "project ID")?;
		let loader_str = filters.loader.as_ref().map(|l| l.to_string());
		let files = self
			.client
			.get_files(
				mod_id,
				filters.minecraft_version.as_deref(),
				loader_str.as_deref(),
			)
			.await
			.map_err(|e| {
				log_rate_limit_warning(&e);
				crate::errors::YammmError::network_error(format!(
					"Failed to fetch versions for CurseForge project {}: {}",
					mod_id, e
				))
			})?;

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
	) -> Result<Vec<SourceDependency>> {
		self.require_api_key("fetching dependencies")?;
		let mod_id = Self::parse_numeric_id(mod_id, "project ID")?;
		let file_id = Self::parse_numeric_id(version_id, "file ID")?;

		let file = self.client.get_file(file_id).await.map_err(|e| {
			log_rate_limit_warning(&e);
			crate::errors::YammmError::network_error(format!(
				"Failed to fetch CurseForge file {}: {}",
				version_id, e
			))
		})?;

		if file.mod_id != mod_id {
			return Err(crate::errors::YammmError::invalid_args(format!(
				"CurseForge file {} does not belong to project {}",
				version_id, mod_id
			))
			.into());
		}

		Ok(CurseForgeClient::to_source_dependencies(&file))
	}
}
