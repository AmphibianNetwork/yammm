//! Remove a mod, resource pack, or shader pack from the modpack.
//!
//! Before removing, checks for **reverse dependencies** — other installed
//! mods that depend on the mod being removed. If dependents exist:
//! - With `--force`: removes only the target, ignoring dependents
//! - Without `--force`: offers to remove dependents too, or abort
//!
//! After removal, runs `cleanup_stale_deps` to remove dangling dependency
//! entries from all remaining mods' `.ron` files.

use crate::app::AppContext;
use crate::output;
use crate::utils::slugify;
use clap::Parser;

/// Remove a mod, resource pack, or shader pack from the modpack.
#[derive(Parser, Debug)]
pub struct RemoveCommand {
	pub identifier: String,

	/// Skip confirmation prompts
	#[arg(short = 'y', long)]
	pub yes: bool,

	/// Remove even if other mods depend on this one (don't remove dependents)
	#[arg(long)]
	pub force: bool,
}

impl RemoveCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> anyhow::Result<()> {
		tracing::debug!("RemoveCommand running");
		tracing::debug!("Identifier: {}", self.identifier);

		let app = ctx.require_modpack()?;
		let storage = &app.storage;

		let slug = slugify(&self.identifier);

		let (_project_type, mod_ron) = storage.find_any(&slug)?;

		let dependents =
			check_reverse_dependencies(storage, &mod_ron.id, &mod_ron.source)?;

		let mut to_remove = vec![(slug.clone(), mod_ron.name.clone())];

		if !dependents.is_empty() {
			output::warning(format!(
				"{} mods depend on {}:",
				dependents.len(),
				console::Style::new().bold().apply_to(&mod_ron.name)
			));
			for dep in &dependents {
				output::bullet(&dep.1);
			}

			if self.force {
				output::dim("--force used. Will remove only this mod.");
			} else if !self.yes {
				let remove_dependents = dialoguer::Confirm::new()
					.with_prompt("Remove dependent mods as well?")
					.default(false)
					.interact()?;

				if remove_dependents {
					to_remove.extend(dependents);
				} else {
					let proceed = dialoguer::Confirm::new()
						.with_prompt("Proceed with removing ONLY this mod? (may break dependent mods)")
						.default(false)
						.interact()?;
					if !proceed {
						output::cancelled("Removal");
						return Ok(());
					}
				}
			}
		} else if !self.yes {
			let proceed = dialoguer::Confirm::new()
				.with_prompt(format!(
					"Are you sure you want to remove {}?",
					mod_ron.name
				))
				.default(true)
				.interact()?;
			if !proceed {
				output::cancelled("Removal");
				return Ok(());
			}
		}

		for (mod_slug, mod_name) in to_remove {
			let (pt, _) = storage.find_any(&mod_slug)?;
			storage.remove(pt, &mod_slug)?;
			cleanup_stale_deps(storage, &mod_slug, &mod_ron.source)?;
			output::success(format!("Removed {} from modpack", mod_name));
		}

		Ok(())
	}
}

/// Find all installed mods that declare a dependency on `target_slug`.
///
/// This is the **reverse** of the normal dependency direction: instead of
/// "what does mod X depend on?", we ask "what mods depend on mod X?".
/// This prevents the user from accidentally breaking their modpack.
fn check_reverse_dependencies(
	storage: &crate::storage::Storage,
	target_slug: &str,
	removed_source: &crate::types::ModSource,
) -> anyhow::Result<Vec<(String, String)>> {
	let mut dependents = Vec::new();

	let all_items = storage.list_all().unwrap_or_default();

	for other_mod in all_items {
		if other_mod.id == target_slug {
			continue;
		}

		for dep in &other_mod.dependencies {
			let dep_slug = slugify(&dep.mod_id);
			let matches_slug =
				dep_slug == target_slug || dep.mod_id == target_slug;
			let matches_source =
				dep.source.source_id() == removed_source.source_id();
			if matches_slug || matches_source {
				dependents.push((other_mod.id.clone(), other_mod.name.clone()));
				break;
			}
		}
	}

	Ok(dependents)
}

/// Remove dangling dependency references from all remaining mods.
///
/// When mod A is removed, any other mod that listed A as a dependency still
/// has that entry in its `.ron` file. This function scans all mods and removes
/// references to the deleted mod, keeping the dependency list consistent.
fn cleanup_stale_deps(
	storage: &crate::storage::Storage,
	removed_slug: &str,
	removed_source: &crate::types::ModSource,
) -> anyhow::Result<()> {
	for project_type in crate::types::ProjectType::VARIANTS {
		for mut mod_ron in
			storage.store_for(*project_type).list().unwrap_or_default()
		{
			let before_len = mod_ron.dependencies.len();
			// Retain only deps that don't reference the removed mod
			mod_ron.dependencies.retain(|d| {
				let slug_match = slugify(&d.mod_id) != removed_slug
					&& d.mod_id != removed_slug;
				let source_match =
					d.source.source_id() != removed_source.source_id();
				slug_match && source_match
			});
			// Only re-save if we actually changed the dependency list
			if mod_ron.dependencies.len() < before_len {
				storage.save(*project_type, &mod_ron.id, &mod_ron)?;
			}
		}
	}

	Ok(())
}
