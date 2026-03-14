use crate::app::AppContext;
use crate::output;
use crate::storage::CacheManager;
use anyhow::Result;
use clap::{Parser, Subcommand};

/// Inspect or manage the local file cache.
#[derive(Parser, Debug)]
pub struct CacheCommand {
	#[command(subcommand)]
	pub command: CacheSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum CacheSubcommand {
	Status,
	Clean,
	Obliterate,
}

impl CacheCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("CacheCommand running");

		let cache_mgr = CacheManager::new(ctx.cache_dir().to_path_buf());
		cache_mgr.init()?;

		match self.command {
			CacheSubcommand::Status => {
				let status = cache_mgr.status()?;

				output::heading("Cache Status");
				output::bullet(format!(
					"Root: {}",
					cache_mgr.cache_root().display()
				));
				output::blank_line();

				output::bullet(format!(
					"jars/: {} files, {}",
					status.jars.file_count,
					crate::utils::format_size(status.jars.total_size)
				));
				output::bullet(format!(
					"minecraft/: {} files, {}",
					status.minecraft.file_count,
					crate::utils::format_size(status.minecraft.total_size)
				));
				output::bullet(format!(
					"loaders/: {} files, {}",
					status.loaders.file_count,
					crate::utils::format_size(status.loaders.total_size)
				));

				output::blank_line();
				output::bullet(format!(
					"Total: {} files, {}",
					status.total_files(),
					crate::utils::format_size(status.total_size())
				));
			}
			CacheSubcommand::Clean => {
				output::info("Cleaning oldest files from cache...");
				let max_size_bytes =
					ctx.global.cache_max_size_mb.unwrap_or(5000) * 1024 * 1024;
				let removed = cache_mgr.clean(max_size_bytes)?;
				if removed > 0 {
					output::success(format!(
						"Cleaned. Freed {}.",
						crate::utils::format_size(removed)
					));
				} else {
					output::success(
						"Cache is within threshold. Nothing to clean.",
					);
				}
			}
			CacheSubcommand::Obliterate => {
				output::warning(
					"This will remove ALL cached files (JARs, Minecraft, loaders)!",
				);
				let proceed = dialoguer::Confirm::new()
					.with_prompt(
						"Are you sure you want to completely obliterate the cache?",
					)
					.default(false)
					.interact()?;

				if proceed {
					cache_mgr.obliterate()?;
					output::success("Cache completely obliterated.");
				} else {
					output::cancelled("Cache obliteration");
				}
			}
		}

		Ok(())
	}
}
