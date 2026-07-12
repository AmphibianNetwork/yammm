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
use crate::services::{cleanup_stale_deps, find_reverse_deps};
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
			find_reverse_deps(storage, &mod_ron.id, &mod_ron.source);

		let mut to_remove = vec![(slug.clone(), mod_ron.name.clone())];

		// JSON mode is intentionally non-interactive: prompts would
		// either hang or fail when stdin is piped. Treat it as
		// `--yes --force` (remove only the target, leave dependents),
		// matching how scripts already expect non-interactive tools to
		// behave.
		let non_interactive = self.yes || output::is_json_mode();
		let force = self.force || output::is_json_mode();

		if !dependents.is_empty() {
			output::warning(format!(
				"{} mods depend on {}:",
				dependents.len(),
				console::Style::new().bold().apply_to(&mod_ron.name)
			));
			for dep in &dependents {
				output::bullet(&dep.1);
			}

			if force {
				output::dim("--force used. Will remove only this mod.");
			} else if !non_interactive {
				let remove_dependents = dialoguer::Confirm::new()
					.with_prompt("Remove dependent mods as well?")
					.default(false)
					.interact()?;

				if remove_dependents {
					to_remove.extend(dependents);
				} else {
					let proceed = dialoguer::Confirm::new()
						.with_prompt(
							"Proceed with removing ONLY this mod? (may break dependent mods)",
						)
						.default(false)
						.interact()?;
					if !proceed {
						output::cancelled("Removal");
						return Ok(());
					}
				}
			}
		} else if !non_interactive {
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

		let mut removed = Vec::with_capacity(to_remove.len());
		for (mod_slug, mod_name) in to_remove {
			let (pt, _) = storage.find_any(&mod_slug)?;
			storage.remove(pt, &mod_slug)?;
			cleanup_stale_deps(storage, &mod_slug, &mod_ron.source)?;
			output::success(format!("Removed {} from modpack", mod_name));
			removed.push(serde_json::json!({
				"id": mod_slug,
				"name": mod_name,
				"project_type": pt.as_str(),
			}));
		}

		if output::is_json_mode() {
			output::emit_json(&serde_json::json!({
				"command": "remove",
				"removed": removed,
			}))?;
		}

		Ok(())
	}
}
