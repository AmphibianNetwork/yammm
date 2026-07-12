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

	/// Report what would update without applying anything.
	///
	/// Pairs naturally with `--json` for CI drift detection: a script
	/// runs `yammm --json update --check-only` on a schedule, parses
	/// the `updates_available` array, and alerts / files PRs on change.
	#[arg(long)]
	pub check_only: bool,
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
	max_concurrent: usize,
) -> Result<UpdateCheckResult> {
	let config_filters = modpack.version_filters();
	let all_items = storage.list_all()?;

	let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
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
		let (id, name, current_version, version_result) = match result {
			Ok(tuple) => tuple,
			Err(e) => {
				tracing::warn!("update check task failed: {e}");
				failed_checks
					.push(("unknown".to_string(), format!("task failed: {e}")));
				continue;
			}
		};
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
		let result = check_for_updates(
			storage,
			modpack,
			ctx.registry(),
			ctx.global().max_concurrent_downloads(),
		)
		.await?;
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

		// --check-only short-circuits before any prompt or write. The
		// caller just wants to know whether there is drift.
		if self.check_only {
			if output::is_json_mode() {
				let updates_available: Vec<_> = result
					.updates
					.iter()
					.map(|u| {
						serde_json::json!({
							"id": u.id,
							"name": u.name,
							"current_version": u.current_version,
							"latest_version": u.latest_version,
						})
					})
					.collect();
				output::emit_json(&serde_json::json!({
					"command": "update",
					"check_only": true,
					"updates_available": updates_available,
					"checks_failed": result.failed_checks.iter().map(|(n, e)|
						serde_json::json!({"name": n, "error": e})
					).collect::<Vec<_>>(),
				}))?;
				return Ok(());
			}
			if result.updates.is_empty() {
				if result.failed_checks.is_empty() {
					output::success("All up to date.");
				}
				return Ok(());
			}
			output::blank_line();
			output::heading(format!(
				"{} update(s) available (not applied):",
				result.updates.len()
			));
			for update in &result.updates {
				output::bullet(format!(
					"{} {}",
					update.name,
					output::version_arrow(
						&update.current_version,
						&update.latest_version
					)
				));
			}
			return Ok(());
		}

		if result.updates.is_empty() {
			if output::is_json_mode() {
				output::emit_json(&serde_json::json!({
					"command": "update",
					"updated": [],
					"failed": [],
					"checks_failed": result.failed_checks.iter().map(|(n, e)|
						serde_json::json!({"name": n, "error": e})
					).collect::<Vec<_>>(),
				}))?;
				return Ok(());
			}
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

		// JSON mode is non-interactive — never prompt.
		let auto_yes = self.yes || output::is_json_mode();

		let mut updated = 0usize;
		let mut failed = 0usize;
		let mut applied: Vec<serde_json::Value> = Vec::new();
		let mut failed_updates: Vec<serde_json::Value> = Vec::new();

		for update in &result.updates {
			let should_update = if auto_yes {
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
				ctx.registry(),
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
					applied.push(serde_json::json!({
						"id": update.id,
						"name": update.name,
						"from": update.current_version,
						"to": update.latest_version,
					}));
				}
				Err(e) => {
					output::error(format!(
						"{} update failed: {}",
						update.name, e
					));
					failed += 1;
					failed_updates.push(serde_json::json!({
						"id": update.id,
						"name": update.name,
						"error": format!("{}", e),
					}));
				}
			}
		}

		if output::is_json_mode() {
			output::emit_json(&serde_json::json!({
				"command": "update",
				"updated": applied,
				"failed": failed_updates,
				"checks_failed": result.failed_checks.iter().map(|(n, e)|
					serde_json::json!({"name": n, "error": e})
				).collect::<Vec<_>>(),
			}))?;
			// Keep the non-zero exit code on failure so scripts can branch
			// on success without parsing JSON if they don't want to.
			if failed > 0 {
				return Err(YammmError::download_failed(format!(
					"{} update(s) failed",
					failed
				))
				.into());
			}
			return Ok(());
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

	let provider = registry
		.get(source)
		.map_err(|e| {
			tracing::warn!("No provider for {source}: {e}");
			e
		})
		.ok()?;
	let filters = VersionFilters {
		minecraft_version: mc_version.map(String::from),
		loader,
	};
	let version = provider
		.get_latest_version(source.source_id(), &filters)
		.await
		.map_err(|e| {
			tracing::warn!("Failed to get latest version for {mod_id}: {e}");
			e
		})
		.ok()?;
	let version_id = version.version_id?;
	let raw_deps = provider
		.get_dependencies(source.source_id(), &version_id)
		.await
		.map_err(|e| {
			tracing::warn!("Failed to get dependencies for {mod_id}: {e}");
			e
		})
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
