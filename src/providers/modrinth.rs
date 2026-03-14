//! Modrinth source implementation.
//!
//! Delegates to `ModrinthClient` for HTTP calls and converts the API-specific
//! response types into the generic `ModInfo`/`ModVersion`/`SourceDependency`
//! types that the rest of the app uses.
//!
//! All dependencies returned by Modrinth are scoped to the Modrinth source —
//! they reference other Modrinth projects by slug/ID. Cross-source deps
//! would require the user to add them manually.

use std::sync::Arc;

use crate::api::ModrinthClient;
use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::SourceDependency;
use crate::types::{
	DependencyKind, ModEnv, ModInfo, ModVersion, VersionFilters,
};
use anyhow::Result;

#[derive(Clone, Debug)]
pub struct ModrinthSource {
	client: Arc<ModrinthClient>,
}

impl ModrinthSource {
	pub fn new(http_client: reqwest::Client) -> Self {
		Self {
			client: Arc::new(ModrinthClient::new().with_client(http_client)),
		}
	}
}

impl ModSourceProvider for ModrinthSource {
	fn name(&self) -> &str {
		"modrinth"
	}

	fn supports_search(&self) -> bool {
		true
	}

	async fn search(
		&self,
		query: &str,
		filters: &SearchFilters,
	) -> Result<Vec<ModInfo>> {
		let hits =
			self.client
				.search(query, filters.limit)
				.await
				.map_err(|e| {
					crate::errors::YammmError::network_error(format!(
						"Search failed: {}",
						e
					))
				})?;

		let loader = filters.version.loader.map(|l| l.to_string());
		let mc_ver = filters.version.minecraft_version.as_deref();

		let results: Vec<ModInfo> = hits
			.into_iter()
			.filter(|h| matches_minecraft_version(h, mc_ver))
			.filter(|h| matches_loader(h, loader.as_deref()))
			.map(ModrinthClient::to_mod_info_from_hit)
			.collect();

		Ok(results)
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> Result<ModInfo> {
		let hit = self.client.get_mod(mod_id).await.map_err(|e| {
			crate::errors::YammmError::mod_not_found(format!(
				"Mod not found: {} ({})",
				mod_id, e
			))
		})?;
		Ok(ModrinthClient::to_mod_info_from_hit(hit))
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> Result<Vec<ModVersion>> {
		let loader_str = filters.loader.as_ref().map(|l| l.to_string());
		let versions = self
			.client
			.get_versions(
				mod_id,
				filters.minecraft_version.as_deref(),
				loader_str.as_deref(),
			)
			.await
			.map_err(|e| {
				crate::errors::YammmError::mod_not_found(format!(
					"Versions not found for {}: {}",
					mod_id, e
				))
			})?;

		Ok(versions
			.into_iter()
			.map(ModrinthClient::to_mod_version)
			.collect())
	}

	async fn get_dependencies(
		&self,
		_mod_id: &str,
		version_id: &str,
	) -> Result<Vec<SourceDependency>> {
		let deps =
			self.client
				.get_dependencies(version_id)
				.await
				.map_err(|e| {
					crate::errors::YammmError::network_error(format!(
						"Failed to fetch dependencies: {}",
						e
					))
				})?;

		Ok(deps
			.into_iter()
			.filter_map(|d| {
				let mod_id = d.mod_id;
				let dep_type = match d.dependency_type.parse::<DependencyKind>()
				{
					Ok(kind) => kind,
					Err(e) => {
						tracing::warn!("Skipping dependency {}: {}", mod_id, e);
						return None;
					}
				};
				Some(SourceDependency {
					mod_id: mod_id.clone(),
					version_id: d.version_id,
					dep_type,
					source: Some(crate::types::ModSource::modrinth(mod_id)),
				})
			})
			.collect())
	}
}

fn matches_minecraft_version(
	hit: &crate::api::ModrinthSearchHit,
	mc_ver: Option<&str>,
) -> bool {
	let Some(mc) = mc_ver else {
		return true;
	};
	hit.versions
		.iter()
		.any(|v| v.split(&['+', '-', '.']).any(|seg| seg == mc) || v == mc)
}

fn matches_loader(
	hit: &crate::api::ModrinthSearchHit,
	loader: Option<&str>,
) -> bool {
	let Some(ldr) = loader else {
		return true;
	};
	hit.categories
		.iter()
		.any(|c| c.to_lowercase() == ldr.to_lowercase())
}

/// Derive a `ModEnv` from Modrinth's `client_side` / `server_side` values.
///
/// Modrinth uses `"required"`, `"optional"`, `"unsupported"` for each side.
/// This maps those semantics to our `ModEnv` enum.
pub fn mod_env_from_modrinth_sides(
	client_side: Option<&str>,
	server_side: Option<&str>,
) -> ModEnv {
	let client = client_side.unwrap_or("required");
	let server = server_side.unwrap_or("required");
	match (client, server) {
		("required", "required") => ModEnv::Both,
		("required", "unsupported") | ("required", "optional") => {
			ModEnv::Client
		}
		("unsupported", "required") | ("optional", "required") => {
			ModEnv::Server
		}
		_ => ModEnv::Both,
	}
}
