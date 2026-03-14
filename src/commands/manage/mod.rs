mod app;
mod deps;
mod render;
mod tui;

use crate::app::AppContext;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct ManageCommand;

impl ManageCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let app = ctx.require_modpack()?;

		let all_items = app.storage.list_all()?;

		if all_items.is_empty() {
			crate::output::warning(
				"No mods in modpack. Add mods first with `yammm add`.",
			);
			return Ok(());
		}

		let modpack = &app.config;
		let modpack_name = if modpack.name.is_empty() {
			"Unnamed modpack".to_string()
		} else {
			modpack.name.clone()
		};
		let modpack_version = modpack.minecraft_version.clone();
		let modpack_loader = format!(
			"{} {}",
			modpack
				.loader
				.loader
				.map(|l| l.to_string())
				.unwrap_or_default(),
			modpack.loader.version
		);

		let filters = modpack.version_filters();

		tui::run_tui(
			&app.storage,
			&ctx.registry,
			all_items,
			modpack_name,
			modpack_version,
			modpack_loader,
			filters,
		)?;

		Ok(())
	}
}
