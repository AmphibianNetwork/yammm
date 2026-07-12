//! CLI subcommand definitions and dispatch.

pub(crate) mod add;
mod auth;
mod cache;
mod completions;
mod config;
mod export;
mod import;
mod info;
mod init;
mod launch;
#[cfg(feature = "tui")]
mod manage;
#[cfg(feature = "tui")]
mod organize;
mod remove;
mod search;
mod self_update;
mod update;

pub use add::AddCommand;
pub use auth::AuthCommand;
pub use cache::CacheCommand;
pub use completions::CompletionsCommand;
pub use config::ConfigCommand;
pub use export::ExportCommand;
pub use import::ImportCommand;
pub use info::InfoCommand;
pub use init::InitCommand;
pub use launch::LaunchCommand;
#[cfg(feature = "tui")]
pub use manage::ManageCommand;
#[cfg(feature = "tui")]
pub use organize::OrganizeCommand;
pub use remove::RemoveCommand;
pub use search::SearchCommand;
pub use self_update::SelfUpdateCommand;
pub use update::UpdateCommand;

use crate::app::AppContext;
use clap::Parser;

/// CLI representation of a mod source (`--source` flag).
///
/// Exists separately from `types::ModSource` because it's limited to
/// CLI-selectable sources and handles URL/file:// detection.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq)]
pub enum CliSource {
	#[value(name = "modrinth")]
	Modrinth,
	#[value(name = "curseforge")]
	CurseForge,
}

impl CliSource {
	/// Convert a CLI source + raw identifier into a `ModSource`.
	/// Detects URLs and file paths first (regardless of `--source`).
	#[allow(clippy::wrong_self_convention)] // call sites already have an owned CliSource; &self is fine here
	pub fn to_mod_source(
		&self,
		identifier: &str,
	) -> crate::types::ModSource {
		if crate::types::ModSource::is_url_like(identifier) {
			crate::types::ModSource::url(identifier)
		} else {
			// Try parsing as a prefixed string first (e.g. "mr:sodium").
			let prefixed = match self {
				CliSource::CurseForge => format!("cf:{}", identifier),
				CliSource::Modrinth => format!("mr:{}", identifier),
			};
			prefixed.parse().unwrap_or_else(|_| match self {
				CliSource::CurseForge => {
					crate::types::ModSource::curseforge(identifier)
				}
				CliSource::Modrinth => {
					crate::types::ModSource::modrinth(identifier)
				}
			})
		}
	}
}

#[derive(Parser, Debug)]
pub enum Command {
	/// Initialize a new modpack workspace
	Init(InitCommand),

	/// Add a mod to the current modpack
	Add(AddCommand),

	/// Export the current modpack
	Export(ExportCommand),

	/// Import a modpack from MRPACK or YMPK format
	Import(ImportCommand),

	/// Launch Minecraft with the current modpack
	Launch(LaunchCommand),

	/// Remove a mod from the current modpack
	Remove(RemoveCommand),

	/// Search for mods on CurseForge or Modrinth
	Search(SearchCommand),

	/// Display information about the current modpack
	Info(InfoCommand),

	/// Manage Microsoft/Mojang authentication
	Auth(AuthCommand),

	/// Manage global cache
	Cache(CacheCommand),

	/// Manage global configuration
	Config(ConfigCommand),

	/// Update mods to their latest versions
	Update(UpdateCommand),

	/// Update yammm to the latest version
	SelfUpdate(SelfUpdateCommand),

	/// Generate shell completions
	Completions(CompletionsCommand),

	/// Organize orphan config files to their mod directories
	#[cfg(feature = "tui")]
	Organize(OrganizeCommand),

	/// Interactive modpack management TUI
	#[cfg(feature = "tui")]
	Manage(ManageCommand),
}

impl Command {
	/// Whether this command supports machine-readable JSON output via the
	/// global `--json` flag. Commands that return `false` here will reject
	/// `--json` up front rather than emit nothing or mix text into a JSON
	/// stream — that lets scripts pin to a stable schema and fail loud
	/// when they ask for something the binary cannot deliver yet.
	fn supports_json(&self) -> bool {
		match self {
			Command::Info(_)
			| Command::Search(_)
			| Command::Cache(_)
			| Command::Add(_)
			| Command::Remove(_)
			| Command::Update(_)
			| Command::Export(_)
			| Command::Import(_)
			| Command::Init(_)
			| Command::Auth(_)
			| Command::Config(_) => true,
			Command::Launch(_)
			| Command::SelfUpdate(_)
			| Command::Completions(_) => false,
			#[cfg(feature = "tui")]
			Command::Organize(_) | Command::Manage(_) => false,
		}
	}

	fn name(&self) -> &'static str {
		match self {
			Command::Init(_) => "init",
			Command::Add(_) => "add",
			Command::Auth(_) => "auth",
			Command::Export(_) => "export",
			Command::Import(_) => "import",
			Command::Launch(_) => "launch",
			Command::Remove(_) => "remove",
			Command::Search(_) => "search",
			Command::Info(_) => "info",
			Command::Cache(_) => "cache",
			Command::Config(_) => "config",
			Command::Update(_) => "update",
			Command::SelfUpdate(_) => "self-update",
			Command::Completions(_) => "completions",
			#[cfg(feature = "tui")]
			Command::Organize(_) => "organize",
			#[cfg(feature = "tui")]
			Command::Manage(_) => "manage",
		}
	}

	/// Dispatch to the appropriate command's `run` method.
	pub async fn run(
		self,
		ctx: AppContext,
	) -> anyhow::Result<()> {
		if crate::output::is_json_mode() && !self.supports_json() {
			return Err(crate::errors::YammmError::invalid_args(format!(
				"command '{}' does not yet support --json output. \
				 Run without --json, or open an issue requesting JSON for this command.",
				self.name()
			))
			.into());
		}

		match self {
			Command::Init(cmd) => cmd.run(ctx).await,
			Command::Add(cmd) => cmd.run(ctx).await,
			Command::Auth(cmd) => cmd.run(ctx).await,
			Command::Export(cmd) => cmd.run(ctx).await,
			Command::Import(cmd) => cmd.run(ctx).await,
			Command::Launch(cmd) => cmd.run(ctx).await,
			Command::Remove(cmd) => cmd.run(ctx).await,
			Command::Search(cmd) => cmd.run(ctx).await,
			Command::Info(cmd) => cmd.run(ctx).await,
			Command::Cache(cmd) => cmd.run(ctx).await,
			Command::Config(cmd) => cmd.run(ctx).await,
			Command::Update(cmd) => cmd.run(ctx).await,
			Command::SelfUpdate(cmd) => cmd.run(ctx).await,
			Command::Completions(cmd) => cmd.run(ctx).await,
			#[cfg(feature = "tui")]
			Command::Organize(cmd) => cmd.run(ctx).await,
			#[cfg(feature = "tui")]
			Command::Manage(cmd) => cmd.run(ctx).await,
		}
	}
}
