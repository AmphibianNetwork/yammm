use anyhow::{Context, Result};
use clap::Parser;

use crate::app::AppContext;
use crate::config::ModpackManifest;
use crate::errors::YammmError;
use crate::output;
use crate::providers::SourceRegistry;
use crate::types::VersionFilters;

/// Check all installed mods for available updates and apply them.
#[derive(Parser, Debug)]
pub struct UpdateCommand {
	#[arg(short = 'y', long)]
	pub yes: bool,
}

/// A single mod with an available update.
#[derive(Debug, Clone)]
pub struct ModUpdate {
	/// Slug / identifier of the mod.
	pub id: String,
	/// Human-readable mod name.
	pub name: String,
	/// Currently installed version string.
	pub current_version: String,
	/// Latest available version string.
	pub latest_version: String,
	/// Download URL for the latest version.
	pub download_url: String,
	/// Hash of the latest version file, if known.
	pub hash: Option<String>,
	/// Algorithm used for the hash.
	pub hash_type: crate::types::HashType,
}

/// Result of checking all mods for updates.
pub struct UpdateCheckResult {
	/// Mods with a newer version available.
	pub updates: Vec<ModUpdate>,
	/// Mods where the update check itself failed (name, error).
	pub failed_checks: Vec<(String, String)>,
}

pub async fn check_for_updates(
	storage: &crate::storage::Storage,
	modpack: &ModpackManifest,
	registry: &SourceRegistry,
) -> Result<UpdateCheckResult> {
	let config_filters = modpack.version_filters();
	let all_items = storage.list_all()?;

	let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
	let mut tasks = tokio::task::JoinSet::new();

	for mod_ron in &all_items {
		if !mod_ron.source.requires_api() {
			continue;
		}

		if mod_ron.unresolved {
			continue;
		}

		let provider = match registry.get(&mod_ron.source) {
			Ok(p) => p.clone(),
			Err(_) => continue,
		};

		let filters = VersionFilters {
			minecraft_version: config_filters.minecraft_version.clone(),
			loader: config_filters.loader,
		};
		let source_id = mod_ron.source.source_id().to_string();
		let id = mod_ron.id.clone();
		let name = mod_ron.name.clone();
		let current_version = mod_ron.version.clone();
		let permit = sem
			.clone()
			.acquire_owned()
			.await
			.context("semaphore closed unexpectedly")?;

		tasks.spawn(async move {
			let _permit = permit;
			let result =
				provider.get_latest_version(&source_id, &filters).await;
			(id, name, current_version, result)
		});
	}

	let mut updates = Vec::new();
	let mut failed_checks: Vec<(String, String)> = Vec::new();
	while let Some(result) = tasks.join_next().await {
		let (id, name, current_version, version_result) = result
			.unwrap_or_else(|e| {
				(
					"unknown".to_string(),
					"unknown".to_string(),
					String::new(),
					Err(anyhow::anyhow!("{}", e)),
				)
			});
		match version_result {
			Ok(latest) => {
				if latest.version != current_version {
					updates.push(ModUpdate {
						id,
						name,
						current_version,
						latest_version: latest.version,
						download_url: latest.download_url,
						hash: latest.hash,
						hash_type: latest.hash_type,
					});
				}
			}
			Err(e) => {
				failed_checks.push((name, format!("{}", e)));
			}
		}
	}

	Ok(UpdateCheckResult {
		updates,
		failed_checks,
	})
}

impl UpdateCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let app = ctx.require_modpack()?;
		let modpack = &app.config;
		let storage = &app.storage;

		let config_filters = modpack.version_filters();

		let spin = output::spinner("Checking for updates...");
		let result = check_for_updates(storage, modpack, &ctx.registry).await?;
		spin.finish_and_clear();

		if !result.failed_checks.is_empty() {
			output::blank_line();
			output::warning(format!(
				"{} mod(s) could not be checked for updates:",
				result.failed_checks.len()
			));
			for (name, err) in &result.failed_checks {
				output::bullet(format!("{}: {}", name, err));
			}
		}

		if result.updates.is_empty() {
			if result.failed_checks.is_empty() {
				output::success(
					"All mods, resource packs, and shader packs are up to date.",
				);
			}
			return Ok(());
		}

		output::blank_line();
		output::heading(format!(
			"{} item(s) with available updates:",
			result.updates.len()
		));
		for update in &result.updates {
			output::bullet(output::version_arrow(
				&update.current_version,
				&update.latest_version,
			));
		}

		let mut updated = 0usize;
		let mut failed = 0usize;

		for update in &result.updates {
			let should_update = if self.yes {
				true
			} else {
				dialoguer::Confirm::new()
					.with_prompt(format!(
						"Update {} {}?",
						update.name,
						output::version_arrow(
							&update.current_version,
							&update.latest_version
						)
					))
					.default(true)
					.interact()?
			};

			if !should_update {
				continue;
			}

			match apply_update(
				storage,
				update,
				&ctx.registry,
				config_filters.minecraft_version.as_deref(),
				config_filters.loader,
			)
			.await
			{
				Ok(()) => {
					output::success(format!(
						"{} updated to {}",
						update.name, update.latest_version
					));
					updated += 1;
				}
				Err(e) => {
					output::error(format!(
						"{} update failed: {}",
						update.name, e
					));
					failed += 1;
				}
			}
		}

		output::blank_line();
		let summary = format!("{} updated, {} failed", updated, failed);
		if failed > 0 {
			output::warning(summary);
			return Err(YammmError::download_failed(format!(
				"{} update(s) failed",
				failed
			))
			.into());
		} else {
			output::success(summary);
		}

		Ok(())
	}
}

async fn apply_update(
	storage: &crate::storage::Storage,
	update: &ModUpdate,
	registry: &crate::providers::SourceRegistry,
	mc_version: Option<&str>,
	loader: Option<crate::types::LoaderType>,
) -> Result<()> {
	let (project_type, mut mod_ron) = storage.find_any(&update.id)?;

	mod_ron.version = update.latest_version.clone();
	mod_ron.download_url = update.download_url.clone();
	mod_ron.hash = update.hash.clone();

	if let Some(deps) = fetch_updated_deps(
		registry,
		&mod_ron.source,
		mc_version,
		loader,
		&mod_ron.id,
	)
	.await
	{
		mod_ron.dependencies = deps;
	}

	storage.save(project_type, &update.id, &mod_ron)?;

	Ok(())
}

async fn fetch_updated_deps(
	registry: &crate::providers::SourceRegistry,
	source: &crate::types::ModSource,
	mc_version: Option<&str>,
	loader: Option<crate::types::LoaderType>,
	mod_id: &str,
) -> Option<Vec<crate::types::Dependency>> {
	if !source.requires_api() {
		return None;
	}

	let provider = registry.get(source).ok()?;
	let filters = VersionFilters {
		minecraft_version: mc_version.map(String::from),
		loader,
	};
	let version = provider
		.get_latest_version(source.source_id(), &filters)
		.await
		.ok()?;
	let version_id = version.version_id?;
	let raw_deps = provider
		.get_dependencies(source.source_id(), &version_id)
		.await
		.ok()?;

	Some(
		raw_deps
			.into_iter()
			.filter_map(|d| {
				if d.mod_id == mod_id {
					return None;
				}
				let kind = match d.dep_type {
					crate::types::DependencyKind::Embedded => {
						return None;
					}
					k => k,
				};
				Some(crate::types::Dependency::new(
					crate::utils::slugify(&d.mod_id),
					d.source.unwrap_or_else(|| source.clone()),
					kind,
				))
			})
			.collect(),
	)
}
