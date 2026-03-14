//! CLI argument parsing via clap.

use crate::app::AppContext;
use crate::commands::Command;
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

/// Yet Another Minecraft Modpack Maker
#[derive(Parser, Debug)]
#[command(name = "yammm")]
#[command(author, version, about = "Yet Another Minecraft Modpack Maker", long_about = None)]
pub struct Cli {
	/// Enable debug logging mode
	#[arg(short = 'd', long, global = true)]
	pub debug: bool,

	/// Path to modpack.toml or modpack directory
	#[arg(short = 'C', long, global = true)]
	pub config: Option<PathBuf>,

	/// Disable SSL verification for HTTPS requests
	#[arg(long, global = true)]
	pub insecure: bool,

	#[command(subcommand)]
	pub command: Command,
}

impl Cli {
	/// Entry point: parse CLI args → build AppContext → dispatch to command.
	pub async fn exec(self) -> Result<()> {
		// YAMMM_DEBUG env var is an alternative to --debug flag
		crate::init_logging(self.debug || std::env::var("YAMMM_DEBUG").is_ok());

		let ctx = AppContext::builder()
			.config_path(self.config)
			.insecure(self.insecure)
			.build()?;

		if !ctx.global.output.color {
			crate::output::set_colors_enabled(false);
		}

		tracing::debug!("AppContext initialized");
		tracing::debug!(
			"Global config: {:?}",
			crate::config::GlobalConfig::config_path()
		);
		tracing::debug!("In modpack directory: {}", ctx.in_modpack());
		tracing::debug!("Cache directory: {:?}", ctx.cache_dir());

		self.command.run(ctx).await
	}
}
