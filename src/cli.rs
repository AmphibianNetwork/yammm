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

	/// Suppress status output (errors and warnings still print to stderr)
	#[arg(short = 'q', long, global = true)]
	pub quiet: bool,

	/// Emit machine-readable JSON output instead of formatted tables.
	///
	/// Supported by `info`, `info mod <id>`, `search`, and `cache status`.
	/// Commands without JSON support return an error rather than emit
	/// partial output, so scripts can pin to a stable schema.
	#[arg(long, global = true)]
	pub json: bool,

	/// Bypass the HTTP metadata cache for this invocation.
	///
	/// Every Modrinth/CurseForge metadata fetch goes straight to the
	/// network — no `If-None-Match` validators sent, no responses
	/// stored. Useful when debugging upstream cache misbehaviour or
	/// when you suspect the local cache is stale.
	#[arg(long, global = true)]
	pub no_http_cache: bool,

	#[command(subcommand)]
	pub command: Command,
}

impl Cli {
	/// Entry point: parse CLI args → build AppContext → dispatch to command.
	pub async fn exec(self) -> Result<()> {
		// YAMMM_DEBUG env var is an alternative to --debug flag
		crate::init_logging(self.debug || std::env::var("YAMMM_DEBUG").is_ok());

		if self.quiet {
			crate::output::set_quiet(true);
		}
		if self.json {
			crate::output::set_json_mode(true);
		}
		if self.no_http_cache {
			crate::api::http_cache::set_bypass(true);
		}

		let ctx = AppContext::builder()
			.config_path(self.config)
			.insecure(self.insecure)
			.build()?;

		if !ctx.global().output.color {
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
