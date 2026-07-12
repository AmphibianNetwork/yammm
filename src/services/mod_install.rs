//! Core mod installation logic — non-interactive service-layer function.
//!
//! This module provides the fundamental "install a mod" operation without
//! any interactive prompts or UI output, so it can be used by both the
//! `add` command (which adds interactive behaviour on top) and the
//! `deps_install` service (which installs resolved dependencies).

use std::sync::Arc;

use crate::errors::YammmError;
use crate::output;
use crate::providers::SourceRegistry;
use crate::services::connector::is_connector_installed;
use crate::storage::Storage;
use crate::types::{
	LoaderType, ModSource, ProjectType, TrackedMod, VersionFilters, VersionReq,
};
use crate::utils::slugify;
use anyhow::{Context, Result};

pub async fn install_mod(
	source: &ModSource,
	version_req: Option<&VersionReq>,
	force: bool,
	storage: &Storage,
	mc_version: Option<&str>,
	loader: Option<LoaderType>,
	registry: Arc<SourceRegistry>,
) -> Result<(String, ProjectType)> {
	let provider = registry.get(source)?;
	let mod_id = source.source_id().to_string();

	let mod_info = provider.get_mod(&mod_id).await.with_context(|| {
		format!("Failed to fetch metadata for '{}'", mod_id)
	})?;

	let slug = if provider.supports_search() {
		slugify(&mod_id)
	} else {
		slugify(&mod_info.id)
	};

	let project_type = mod_info.project_type.unwrap_or(ProjectType::Mod);

	if storage.exists(project_type, &slug) {
		if force {
			storage.remove(project_type, &slug)?;
		} else {
			output::success(format!("{} is already installed", mod_info.name));
			return Ok((slug, project_type));
		}
	}

	let filters = VersionFilters {
		minecraft_version: mc_version.map(String::from),
		loader,
	};

	let version_data =
		match resolve_version(provider, &mod_id, version_req, &filters).await {
			Ok(v) => v,
			Err(_) if is_connector_eligible(loader, storage) => {
				let fabric_filters = VersionFilters {
					minecraft_version: mc_version.map(String::from),
					loader: Some(LoaderType::Fabric),
				};
				match resolve_version(
					provider,
					&mod_id,
					version_req,
					&fabric_filters,
				)
				.await
				{
					Ok(v) => {
						output::info(
							"Sinytra Connector is installed — using Fabric version.",
						);
						let env = provider.get_mod_env(&mod_info);
						let mut mod_ron = TrackedMod::from_mod_info(
							&mod_info,
							&v,
							slug.clone(),
							project_type,
							env,
						);
						mod_ron.connector_compat = true;
						storage.save(project_type, &slug, &mod_ron)?;
						output::success(format!(
							"Installed {} v{} (via Connector)",
							mod_info.name, v.version
						));
						return Ok((slug, project_type));
					}
					Err(e) => {
						return Err(e.context(format!(
							"No {} or Fabric version found for {}",
							loader
								.map(|l| l.display_name())
								.unwrap_or("compatible"),
							mod_id
						)));
					}
				}
			}
			Err(e) => return Err(e),
		};

	let env = provider.get_mod_env(&mod_info);

	let mod_ron = TrackedMod::from_mod_info(
		&mod_info,
		&version_data,
		slug.clone(),
		project_type,
		env,
	);

	storage.save(project_type, &slug, &mod_ron)?;

	output::success(format!(
		"Installed {} v{}",
		mod_info.name, version_data.version
	));

	Ok((slug, project_type))
}

async fn resolve_version(
	provider: &crate::providers::Provider,
	mod_id: &str,
	version_req: Option<&VersionReq>,
	filters: &VersionFilters,
) -> Result<crate::types::ModVersion> {
	if let Some(req) = version_req {
		let versions = provider.get_versions(mod_id, filters).await?;
		versions
			.into_iter()
			.find(|v| req.matches(&v.version))
			.ok_or_else(|| {
				YammmError::version_conflict(format!(
					"Version matching {} not found",
					req
				))
				.into()
			})
	} else {
		provider
			.get_latest_version(mod_id, filters)
			.await
			.map_err(Into::into)
	}
}

fn is_connector_eligible(
	loader: Option<LoaderType>,
	storage: &Storage,
) -> bool {
	matches!(loader, Some(LoaderType::Forge | LoaderType::NeoForge))
		&& is_connector_installed(storage)
}
