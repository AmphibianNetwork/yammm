use crate::app::AppContext;
use crate::errors::YammmError;
use crate::output;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::process::Command;

/// View or modify the global yammm configuration.
#[derive(Parser, Debug)]
pub struct ConfigCommand {
	#[command(subcommand)]
	pub command: ConfigSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigSubcommand {
	Edit,
	Show,
	Get { key: ConfigKey },
	Set { key: ConfigKey, value: String },
	Reset,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ConfigKey {
	#[value(name = "default_modpack_dir")]
	DefaultModpackDir,
	#[value(name = "cache_dir")]
	CacheDir,
	#[value(name = "cache_max_size_mb")]
	CacheMaxSizeMb,
	#[value(name = "max_concurrent_downloads")]
	MaxConcurrentDownloads,
	#[value(name = "api_keys.curseforge")]
	ApiKeysCurseforge,
	#[value(name = "output.format")]
	OutputFormat,
	#[value(name = "output.color")]
	OutputColor,
}

impl ConfigCommand {
	pub async fn run(
		self,
		mut ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("ConfigCommand running");

		match self.command {
			ConfigSubcommand::Edit => {
				let path = crate::config::GlobalConfig::config_path()
					.ok_or_else(|| {
						crate::errors::YammmError::config_error(
							"Failed to find config path",
						)
					})?;
				let editor = std::env::var("EDITOR")
					.unwrap_or_else(|_| "vi".to_string());
				let status = Command::new(&editor)
					.arg(&path)
					.status()
					.with_context(|| {
						format!(
							"Failed to launch editor '{}'. Set $EDITOR to a valid editor.",
							editor
						)
					})?;
				if !status.success() {
					return Err(crate::errors::YammmError::general(
						"Editor exited with error",
					)
					.into());
				}
			}
			ConfigSubcommand::Show => {
				output::heading("Global configuration");
				let toml_string = toml::to_string_pretty(&ctx.global)?;
				println!("{}", toml_string);
			}
			ConfigSubcommand::Get { key } => {
				let val = get_config_value(&ctx.global, &key)?;
				println!("{}", val);
			}
			ConfigSubcommand::Set { key, value } => {
				set_config_value(&mut ctx.global, &key, &value)?;
				ctx.global.save()?;
				output::success("Configuration updated.");
			}
			ConfigSubcommand::Reset => {
				let proceed = dialoguer::Confirm::new()
					.with_prompt("Reset global configuration to defaults?")
					.default(false)
					.interact()?;

				if proceed {
					let default_config = crate::config::GlobalConfig::default();
					default_config.save()?;
					output::success("Configuration reset to defaults.");
				}
			}
		}

		Ok(())
	}
}

fn get_config_value(
	config: &crate::config::GlobalConfig,
	key: &ConfigKey,
) -> Result<String> {
	match key {
		ConfigKey::DefaultModpackDir => Ok(config
			.default_modpack_dir
			.as_ref()
			.map(|p| p.display().to_string())
			.unwrap_or_default()),
		ConfigKey::CacheDir => Ok(config
			.cache_dir
			.as_ref()
			.map(|p| p.display().to_string())
			.unwrap_or_default()),
		ConfigKey::CacheMaxSizeMb => Ok(config
			.cache_max_size_mb
			.map(|v| v.to_string())
			.unwrap_or_default()),
		ConfigKey::MaxConcurrentDownloads => Ok(config
			.max_concurrent_downloads
			.map(|v| v.to_string())
			.unwrap_or_default()),
		ConfigKey::ApiKeysCurseforge => {
			Ok(config.api_keys.curseforge.clone().unwrap_or_default())
		}
		ConfigKey::OutputFormat => Ok(config.output.format.to_string()),
		ConfigKey::OutputColor => Ok(config.output.color.to_string()),
	}
}

fn set_config_value(
	config: &mut crate::config::GlobalConfig,
	key: &ConfigKey,
	value: &str,
) -> Result<()> {
	match key {
		ConfigKey::DefaultModpackDir => {
			config.default_modpack_dir = Some(std::path::PathBuf::from(value));
		}
		ConfigKey::CacheDir => {
			config.cache_dir = Some(std::path::PathBuf::from(value));
		}
		ConfigKey::CacheMaxSizeMb => {
			config.cache_max_size_mb = Some(value.parse().map_err(|_| {
				YammmError::config_error(format!(
					"Invalid number for cache_max_size_mb: {}",
					value
				))
			})?);
		}
		ConfigKey::MaxConcurrentDownloads => {
			config.max_concurrent_downloads =
				Some(value.parse().map_err(|_| {
					YammmError::config_error(format!(
						"Invalid number for max_concurrent_downloads: {}",
						value
					))
				})?);
		}
		ConfigKey::ApiKeysCurseforge => {
			config.api_keys.curseforge = if value.is_empty() {
				None
			} else {
				Some(value.to_string())
			};
		}
		ConfigKey::OutputFormat => {
			config.output.format =
				value.parse().map_err(YammmError::config_error)?;
		}
		ConfigKey::OutputColor => {
			config.output.color = value.parse().map_err(|_| {
				YammmError::config_error(format!(
					"Invalid boolean for output.color: {}",
					value
				))
			})?;
		}
	}
	Ok(())
}
