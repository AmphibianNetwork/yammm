mod app;
mod tui;

use crate::app::AppContext;
use crate::output;
use crate::utils::list_files;
use anyhow::Result;
use clap::Parser;
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Parser, Debug)]
pub struct OrganizeCommand {
	#[command(subcommand)]
	pub command: Option<OrganizeSubcommand>,
}

#[derive(Parser, Debug)]
pub enum OrganizeSubcommand {
	Client(ClientOrganize),
	Server(ServerOrganize),
}

#[derive(Parser, Debug)]
pub struct ClientOrganize;

#[derive(Parser, Debug)]
pub struct ServerOrganize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
	Client,
	Server,
}

impl Side {
	pub fn as_str(self) -> &'static str {
		match self {
			Side::Client => "client",
			Side::Server => "server",
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IgnoredConfigs {
	#[serde(default)]
	pub client: Vec<String>,
	#[serde(default)]
	pub server: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OrphanConfig {
	pub path: PathBuf,
	pub file_name: String,
	pub rel_path: String,
	pub content: String,
}

#[derive(Debug, Clone)]
pub struct OrganizeResult {
	pub assigned: std::collections::HashMap<String, usize>,
	pub ignored_count: usize,
	pub skipped_count: usize,
	pub ignored_new: Vec<String>,
}

impl OrganizeCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let app = ctx.require_modpack()?;
		let storage = &app.storage;

		let subcommand = self
			.command
			.unwrap_or(OrganizeSubcommand::Client(ClientOrganize));

		let (side, config_dir) = match &subcommand {
			OrganizeSubcommand::Client(_) => {
				(Side::Client, app.root_dir.join("client").join("config"))
			}
			OrganizeSubcommand::Server(_) => {
				(Side::Server, app.root_dir.join("server").join("config"))
			}
		};

		if !config_dir.exists() {
			return Err(crate::errors::YammmError::invalid_args(format!(
				"Source directory not found: {}\nLaunch {} first with `yammm launch {}`",
				config_dir.display(),
				side.as_str(),
				side.as_str()
			))
			.into());
		}

		let ignore_path =
			app.root_dir.join(".yammm").join("ignored_configs.ron");
		let mut ignored = load_ignored_configs(&ignore_path);

		let ignore_list = match side {
			Side::Client => &ignored.client,
			Side::Server => &ignored.server,
		};

		let mods = storage.list(crate::types::ProjectType::Mod)?;
		let mod_names: Vec<String> =
			mods.iter().map(|m| m.id.clone()).collect();

		if mod_names.is_empty() {
			output::warning(
				"No mods in modpack. Add mods first with `yammm add`.",
			);
			return Ok(());
		}

		output::info(format!(
			"Scanning {} for orphan configs...",
			config_dir.display()
		));

		let orphan_configs = find_orphan_configs(&config_dir, ignore_list)?;

		if orphan_configs.is_empty() {
			output::success("No orphan configs to organize");
			return Ok(());
		}

		output::success(format!(
			"Found {} orphan configs",
			orphan_configs.len()
		));

		let result =
			tui::run_tui(&orphan_configs, &mod_names, side, &app.root_dir)?;

		if !result.ignored_new.is_empty() {
			let ignore_entry_list = match side {
				Side::Client => &mut ignored.client,
				Side::Server => &mut ignored.server,
			};
			for entry in &result.ignored_new {
				if !ignore_entry_list.contains(entry) {
					ignore_entry_list.push(entry.clone());
				}
			}
		}

		save_ignored_configs(&ignore_path, &ignored)?;

		output::blank_line();
		output::heading("Summary");
		let total_assigned: usize = result.assigned.values().sum();
		output::bullet(format!("Assigned: {} files", total_assigned));
		output::bullet(format!("Ignored: {} files", result.ignored_count));
		output::bullet(format!("Skipped: {} files", result.skipped_count));
		if !result.assigned.is_empty() {
			output::blank_line();
			output::dim("Files assigned to:");
			for (mod_id, count) in &result.assigned {
				output::bullet(format!("{}: {} configs", mod_id, count));
			}
		}

		Ok(())
	}
}

pub fn find_orphan_configs(
	config_dir: &Path,
	ignore_list: &[String],
) -> Result<Vec<OrphanConfig>> {
	let mut orphans = Vec::new();

	if !config_dir.is_dir() {
		return Ok(orphans);
	}

	for entry in list_files(config_dir, false) {
		let path = entry.as_path();

		if !path.is_file() || path.is_symlink() {
			continue;
		}

		if path
			.metadata()
			.map(|m| m.file_type().is_symlink())
			.unwrap_or(false)
		{
			continue;
		}

		let file_name = path
			.file_name()
			.unwrap_or_default()
			.to_string_lossy()
			.to_string();

		let relative = path
			.strip_prefix(config_dir)
			.unwrap_or(path)
			.to_string_lossy()
			.to_string();

		let is_ignored = ignore_list.iter().any(|pattern| {
			if pattern.contains('*') {
				glob_match(pattern, &relative)
			} else {
				pattern == &file_name || pattern == &relative
			}
		});

		if is_ignored {
			continue;
		}

		let content = match std::fs::read_to_string(path) {
			Ok(c) => c,
			Err(e) => {
				tracing::warn!("Failed to read {}: {}", path.display(), e);
				continue;
			}
		};

		orphans.push(OrphanConfig {
			path: path.to_path_buf(),
			file_name,
			rel_path: relative,
			content,
		});
	}

	orphans.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
	Ok(orphans)
}

fn glob_match(
	pattern: &str,
	path: &str,
) -> bool {
	let parts: Vec<&str> = pattern.split('*').collect();
	if parts.len() == 2 {
		let prefix = parts[0];
		let suffix = parts[1];
		path.starts_with(prefix) && path.ends_with(suffix)
	} else {
		pattern == path
	}
}

fn load_ignored_configs(path: &Path) -> IgnoredConfigs {
	if !path.exists() {
		return IgnoredConfigs::default();
	}
	let contents = match std::fs::read_to_string(path) {
		Ok(c) => c,
		Err(e) => {
			tracing::warn!("Failed to read ignored configs: {}", e);
			return IgnoredConfigs::default();
		}
	};
	match ron::from_str(&contents) {
		Ok(c) => c,
		Err(e) => {
			tracing::warn!("Failed to parse ignored configs: {}", e);
			IgnoredConfigs::default()
		}
	}
}

pub fn save_ignored_configs(
	path: &Path,
	ignored: &IgnoredConfigs,
) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let contents =
		ron::ser::to_string_pretty(ignored, ron::ser::PrettyConfig::default())?;
	std::fs::write(path, contents)?;
	Ok(())
}

pub fn assign_config(
	config: &OrphanConfig,
	mod_id: &str,
	dest_idx: usize,
	side: Side,
	root_dir: &Path,
) -> Result<String> {
	let config_dir = match side {
		Side::Client => root_dir.join("client").join("config"),
		Side::Server => root_dir.join("server").join("config"),
	};

	let rel_path_to_side =
		diff_paths(&config.path, &config_dir).ok_or_else(|| {
			crate::errors::YammmError::config_error(format!(
				"Config file outside of config dir: {}",
				config.path.display()
			))
		})?;

	let dest_dir = if dest_idx == 0 {
		root_dir.join("mods").join(mod_id).join("config")
	} else if dest_idx == 1 {
		root_dir
			.join("mods")
			.join(mod_id)
			.join(side.as_str())
			.join("config")
	} else {
		root_dir.join("config")
	};

	std::fs::create_dir_all(&dest_dir)?;
	let dest_path = dest_dir.join(&rel_path_to_side);

	std::fs::create_dir_all(dest_path.parent().ok_or_else(|| {
		crate::errors::YammmError::config_error(format!(
			"Config file has no parent directory: {}",
			dest_path.display()
		))
	})?)?;
	std::fs::copy(&config.path, &dest_path)?;
	std::fs::remove_file(&config.path)?;

	let key = if dest_idx == 2 {
		"fallback (config/)".to_string()
	} else {
		mod_id.to_string()
	};
	Ok(key)
}
