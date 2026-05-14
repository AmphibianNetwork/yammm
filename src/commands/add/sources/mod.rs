//! Mod source implementations for the `add` command.
//!
//! `AddContext` encapsulates all the state needed to add a single mod:
//! the source to fetch from, version constraints, storage, and filters.
//!
//! The `add()` method handles the full lifecycle:
//! fetch metadata → handle existing → resolve version → save .ron

use std::sync::Arc;

use crate::output;
use crate::providers::{Provider, SearchFilters, SourceRegistry};
use crate::services::connector::is_connector_installed;
use crate::types::VersionFilters;
use crate::types::{
	LoaderType, ModEnv, ModInfo, ModSource, ModVersion, ProjectType,
	TrackedMod, VersionReq,
};
use crate::utils::slugify;

/// Context for adding a single mod to the modpack.
///
/// Bundles all the parameters needed for the add flow so they can be
/// passed around without long parameter lists. Created once per `add`
/// invocation and reused for dependency installation.
pub struct AddContext<'a> {
	pub source: &'a ModSource,
	pub version_req: Option<VersionReq>,
	pub force: bool,
	pub storage: &'a crate::storage::Storage,
	pub mc_version: Option<&'a str>,
	pub loader: Option<LoaderType>,
	pub registry: Arc<SourceRegistry>,
	pub env_override: Option<ModEnv>,
	pub project_type_override: Option<&'a ProjectType>,
	pub categories: Vec<String>,
}

impl<'a> AddContext<'a> {
	pub fn parse_version(
		version: Option<&str>
	) -> anyhow::Result<Option<VersionReq>> {
		version
			.map(VersionReq::parse)
			.transpose()
			.map_err(Into::into)
	}

	/// Add a mod to the modpack.
	///
	/// Full flow: resolve the source → fetch metadata from the provider → handle
	/// the already-installed case (update prompt or overwrite) → resolve the
	/// best matching version → save the `.ron` metadata file.
	///
	/// Returns `(slug, project_type)` so the caller can resolve dependencies.
	pub async fn add(
		&self,
		identifier: &str,
	) -> anyhow::Result<(String, ProjectType)> {
		output::info(format!("Adding mod: {}", identifier));

		let provider = self.registry.get(self.source)?;
		let mut mod_id = self.source.source_id().to_string();

		let mod_info = self.fetch_mod_info(provider, &mut mod_id).await?;

		let slug = if provider.supports_search() {
			slugify(&mod_id)
		} else {
			slugify(&mod_info.id)
		};

		let project_type = self
			.project_type_override
			.cloned()
			.or(mod_info.project_type)
			.unwrap_or(ProjectType::Mod);

		if self.storage.exists(project_type, &slug) {
			let action = self.handle_existing_mod(&slug, &project_type).await?;
			if action == ExistingModAction::ReturnEarly {
				return Ok((slug, project_type));
			}
		}

		let resolved = self.resolve_version(provider, &mod_id).await?;

		output::bullet(format!("Name: {}", mod_info.name));
		output::bullet(format!("Version: {}", resolved.version_data.version));

		let env = self
			.env_override
			.unwrap_or_else(|| provider.get_mod_env(&mod_info));

		let mut mod_ron = TrackedMod::from_mod_info(
			&mod_info,
			&resolved.version_data,
			slug.clone(),
			project_type,
			env,
		);

		if !self.categories.is_empty() {
			mod_ron.categories = self.categories.clone();
		}

		if resolved.connector_compat {
			mod_ron.connector_compat = true;
		}

		self.storage.save(project_type, &slug, &mod_ron)?;

		output::success(format!("Metadata saved for {}", mod_info.name));

		let type_label = match &project_type {
			ProjectType::Mod => "Mod",
			ProjectType::ResourcePack => "Resource pack",
			ProjectType::Shader => "Shader pack",
		};
		output::success(format!("{} added successfully", type_label));
		output::dim("(JAR will be downloaded when launching or exporting)");

		Ok((slug, project_type))
	}

	fn version_filters(&self) -> VersionFilters {
		VersionFilters {
			minecraft_version: self.mc_version.map(String::from),
			loader: self.loader,
		}
	}

	fn search_filters(
		&self,
		limit: usize,
	) -> SearchFilters {
		SearchFilters::new(self.version_filters(), Some(limit))
	}

	/// Try to get mod info by ID; if that fails and the provider supports
	/// search, fall back to a search + interactive selection.
	async fn fetch_mod_info(
		&self,
		provider: &Provider,
		mod_id: &mut String,
	) -> anyhow::Result<ModInfo> {
		let spin = output::spinner("Fetching metadata...");

		match provider.get_mod(mod_id).await {
			Ok(info) => {
				spin.finish_and_clear();
				Ok(info)
			}
			Err(e) => {
				spin.finish_and_clear();
				if !provider.supports_search() {
					return Err(e);
				}
				output::info(format!(
					"Exact match not found, searching for '{}'...",
					mod_id
				));
				let filters = self.search_filters(10);
				let results = provider.search(mod_id, &filters).await?;
				if results.is_empty() {
					return Err(crate::errors::YammmError::mod_not_found(
						format!("No mods found matching '{}'", mod_id),
					)
					.into());
				}

				let choices: Vec<String> = results
					.iter()
					.map(|m| format!("{} ({})", m.name, m.id))
					.collect();

				let selection = dialoguer::Select::new()
					.with_prompt("Select a mod to add")
					.items(&choices)
					.default(0)
					.interact()?;

				let selected_mod = &results[selection];
				*mod_id = selected_mod.id.clone();

				Ok(selected_mod.clone())
			}
		}
	}

	/// Handle the case where a mod with this slug is already installed.
	///
	/// Three possible outcomes:
	/// - `--force`: remove the old entry and continue (overwrite)
	/// - Different source: ask whether to overwrite (source conflict)
	/// - Same source, newer version available: offer to update
	async fn handle_existing_mod(
		&self,
		slug: &str,
		project_type: &ProjectType,
	) -> anyhow::Result<ExistingModAction> {
		if self.force {
			output::info("Updating existing entry");
			self.storage.remove(*project_type, slug)?;
			return Ok(ExistingModAction::Continue);
		}

		if let Ok(existing) = self.storage.load(*project_type, slug)
			&& existing.source.source_id() != self.source.source_id()
		{
			return self.handle_source_conflict(slug, project_type).await;
		}

		self.check_for_update(slug, project_type).await
	}

	async fn handle_source_conflict(
		&self,
		slug: &str,
		project_type: &ProjectType,
	) -> anyhow::Result<ExistingModAction> {
		crate::output::warning(format!(
			"Slug '{}' already used by a different source. The new mod may conflict.",
			slug
		));
		let overwrite = dialoguer::Confirm::new()
			.with_prompt("Overwrite existing entry?")
			.default(false)
			.interact()
			.unwrap_or(false);
		if !overwrite {
			crate::output::dim("Skipped — not overwriting existing entry");
			return Ok(ExistingModAction::ReturnEarly);
		}
		self.storage.remove(*project_type, slug)?;
		Ok(ExistingModAction::Continue)
	}

	async fn check_for_update(
		&self,
		slug: &str,
		project_type: &ProjectType,
	) -> anyhow::Result<ExistingModAction> {
		let existing = self.storage.load(*project_type, slug);

		if let Ok(mut mod_ron) = existing {
			let filters = self.version_filters();
			if let Ok(provider) = self.registry.get(&mod_ron.source) {
				if let Ok(latest) = provider
					.get_latest_version(mod_ron.source.source_id(), &filters)
					.await
				{
					if latest.version != mod_ron.version {
						output::bullet(output::version_arrow(
							&mod_ron.version,
							&latest.version,
						));
						output::dim(format!(
							"  {} is present (v{})",
							mod_ron.name, mod_ron.version
						));
						let should_update = dialoguer::Confirm::new()
							.with_prompt("Update to latest version?")
							.default(true)
							.interact()
							.unwrap_or(false);
						if should_update {
							mod_ron.version = latest.version.clone();
							mod_ron.download_url = latest.download_url;
							mod_ron.hash = latest.hash;
							self.storage.save(*project_type, slug, &mod_ron)?;
							output::success(format!(
								"Updated {} to v{}",
								mod_ron.name, mod_ron.version
							));
						}
					} else {
						output::success(format!(
							"{} is already up to date (v{})",
							mod_ron.name, mod_ron.version
						));
					}
				} else {
					output::dim(format!(
						"Already present (v{}) — could not check for updates",
						mod_ron.version
					));
				}
			} else {
				output::dim(format!("Already present (v{})", mod_ron.version));
			}
		}

		Ok(ExistingModAction::ReturnEarly)
	}

	/// Resolve the version to install.
	///
	/// If a version constraint was specified (`--version`), finds the first
	/// matching version. Otherwise, picks the latest compatible version.
	///
	/// When running on Forge/NeoForge and no version is found, falls back to
	/// searching for a Fabric version if Sinytra Connector is installed.
	async fn resolve_version(
		&self,
		provider: &Provider,
		mod_id: &str,
	) -> anyhow::Result<ResolvedVersion> {
		let filters = self.version_filters();

		let result = if let Some(req) = &self.version_req {
			let versions = provider.get_versions(mod_id, &filters).await?;
			versions
				.into_iter()
				.find(|v| req.matches(&v.version))
				.map(|v| ResolvedVersion {
					version_data: v,
					connector_compat: false,
				})
				.ok_or_else(|| {
					crate::errors::YammmError::version_conflict(format!(
						"Version matching {} not found",
						req
					))
					.into()
				})
		} else {
			provider
				.get_latest_version(mod_id, &filters)
				.await
				.map(|v| ResolvedVersion {
					version_data: v,
					connector_compat: false,
				})
		};

		if result.is_ok() || !self.is_connector_eligible() {
			return result;
		}

		self.try_connector_fallback(provider, mod_id).await
	}

	fn is_connector_eligible(&self) -> bool {
		matches!(self.loader, Some(LoaderType::Forge | LoaderType::NeoForge))
			&& is_connector_installed(self.storage)
	}

	async fn try_connector_fallback(
		&self,
		provider: &Provider,
		mod_id: &str,
	) -> anyhow::Result<ResolvedVersion> {
		let fabric_filters = VersionFilters {
			minecraft_version: self.mc_version.map(String::from),
			loader: Some(LoaderType::Fabric),
		};

		let fabric_result = if let Some(req) = &self.version_req {
			let versions =
				provider.get_versions(mod_id, &fabric_filters).await?;
			versions
				.into_iter()
				.find(|v| req.matches(&v.version))
				.ok_or_else(|| {
					crate::errors::YammmError::version_conflict(format!(
						"Version matching {} not found (also checked Fabric via Connector)",
						req
					))
					.into()
				})
		} else {
			provider
				.get_latest_version(mod_id, &fabric_filters)
				.await
				.map_err(|_| {
					crate::errors::YammmError::mod_not_found(format!(
						"No versions found for {} (also checked Fabric via Connector)",
						mod_id
					))
					.into()
				})
		};

		if fabric_result.is_err() {
			return fabric_result.map(|v| ResolvedVersion {
				version_data: v,
				connector_compat: true,
			});
		}

		output::blank_line();
		output::info(
			"Sinytra Connector is installed — a Fabric version of this mod is available.",
		);
		let proceed = dialoguer::Confirm::new()
			.with_prompt("Install the Fabric version via Connector?")
			.default(true)
			.interact()
			.unwrap_or(false);

		if !proceed {
			return Err(crate::errors::YammmError::mod_not_found(format!(
				"No {} version found for {}",
				self.loader
					.map(|l| l.display_name())
					.unwrap_or("compatible"),
				mod_id
			))
			.into());
		}

		fabric_result.map(|v| ResolvedVersion {
			version_data: v,
			connector_compat: true,
		})
	}
}

/// Result of version resolution, carrying whether the version was found
/// via Sinytra Connector compatibility (Fabric version on Forge/NeoForge).
struct ResolvedVersion {
	version_data: ModVersion,
	connector_compat: bool,
}

/// Control flow indicator for `handle_existing_mod`.
#[derive(PartialEq)]
enum ExistingModAction {
	/// Don't proceed with adding — the mod is already handled
	ReturnEarly,
	/// Continue with adding (old entry was removed by --force)
	Continue,
}
