//! Initialize a new modpack workspace.
//!
//! Creates the standard directory layout and `modpack.toml`:
//!
//! ```text
//! <output_dir>/
//!   modpack.toml      ← modpack metadata (name, MC version, loader)
//!   mods/             ← each mod gets its own subdirectory with a entry.ron
//!   config/           ← shared config files
//!   resourcepacks/   ← resource pack storage
//!   shaderpacks/     ← shader pack storage
//!   resources/client/ ← client-side files (options.txt, etc.)
//!   resources/server/ ← server-side files (server.properties, etc.)
//! ```
//!
//! When no flags are given, the command becomes interactive (prompts for
//! name, version, loader). When any flag is provided, it runs non-interactively
//! with defaults for missing values.

use anyhow::{Context, Result};
use clap::Parser;
use dialoguer::{Input, Select};
use std::fs;
use std::path::PathBuf;

use crate::app::AppContext;
use crate::config::{LoaderConfig, ModpackManifest};
use crate::output;
use crate::types::LoaderType;

const DEFAULT_MINECRAFT_VERSION: &str = "1.21.5";
const DEFAULT_MODPACK_NAME: &str = "my-modpack";
const DEFAULT_MODPACK_VERSION: &str = "1.0.0";

/// Initialize a new modpack workspace (creates directories and modpack.toml).
#[derive(Parser, Debug)]
pub struct InitCommand {
	#[arg(short = 'n', long)]
	pub name: Option<String>,

	#[arg(short = 'V', long)]
	pub minecraft_version: Option<String>,

	#[arg(short = 'L', long)]
	pub loader: Option<String>,

	#[arg(long)]
	pub loader_version: Option<String>,

	#[arg(long)]
	pub description: Option<String>,

	#[arg(short = 'o', long, default_value = ".")]
	pub output_dir: PathBuf,

	#[arg(long, default_value = "false")]
	pub interactive: bool,
}

impl InitCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("InitCommand running");

		let init_info = self
			.collect_init_info(&ctx.http_client)
			.await
			.context("Failed to collect modpack info")?;

		tracing::debug!(
			"Init info collected: name={}, minecraft_version={}, loader={}",
			init_info.name,
			init_info.minecraft_version,
			init_info.loader
		);

		if !self.output_dir.exists() {
			fs::create_dir_all(&self.output_dir).with_context(|| {
				format!(
					"Failed to create directory: {}",
					self.output_dir.display()
				)
			})?;
		}

		ensure_dir(&self.output_dir.join("mods"), "mods")?;
		ensure_dir(&self.output_dir.join("config"), "config")?;
		ensure_dir(&self.output_dir.join("resourcepacks"), "resourcepacks")?;
		ensure_dir(&self.output_dir.join("shaderpacks"), "shaderpacks")?;
		ensure_dir(
			&self.output_dir.join("resources").join("client"),
			"resources/client",
		)?;
		ensure_dir(
			&self.output_dir.join("resources").join("server"),
			"resources/server",
		)?;

		let config = ModpackManifest {
			name: init_info.name,
			description: init_info.description.unwrap_or_default(),
			version: init_info
				.version
				.unwrap_or_else(|| DEFAULT_MODPACK_VERSION.to_string()),
			minecraft_version: init_info.minecraft_version,
			loader: LoaderConfig {
				loader: Some(init_info.loader),
				version: init_info.loader_version.unwrap_or_default(),
			},
			mod_path: None,
			resource_pack_path: None,
			shader_pack_path: None,
		};

		let modpack_path = self.output_dir.join("modpack.toml");
		if modpack_path.exists() {
			output::warning("modpack.toml already exists, skipping");
		} else {
			crate::storage::ManifestStore::new(&modpack_path)
				.save(&config)
				.context("Failed to write modpack.toml")?;
			output::success("Created modpack.toml");
		}

		let gitignore_path = self.output_dir.join(".gitignore");
		if !gitignore_path.exists() {
			let gitignore_content = "\
mods/*.zip

.idea/
.vscode/
*.swp
*.swo

.DS_Store
Thumbs.db

.yammm/

target/
dist/

logs/
*.log
";
			fs::write(&gitignore_path, gitignore_content)
				.context("Failed to write .gitignore")?;
			output::success("Created .gitignore");
		}

		let readme_path = self.output_dir.join("README.md");
		if !readme_path.exists() {
			let mut content = String::new();
			content.push_str(&format!("# {}\n\n", config.name));
			if !config.description.is_empty() {
				content.push_str(&format!("{}\n\n", config.description));
			}
			content.push_str(&format!(
				"- **Minecraft Version:** {}\n",
				config.minecraft_version
			));
			content.push_str(&format!(
				"- **Mod Loader:** {}\n\n",
				config
					.loader
					.loader
					.map(|l| l.to_string())
					.unwrap_or_default()
			));
			content.push_str("```bash\nyammm add modrinth:jei\n```\n");
			content.push_str("```bash\nyammm launch\n```\n");
			content.push_str("```bash\nyammm export\n```");

			fs::write(&readme_path, content)
				.context("Failed to write README.md")?;
			output::success("Created README.md");
		}

		output::success("Modpack workspace initialized!");
		output::dim(format!("   Directory: {}", self.output_dir.display()));
		output::blank_line();
		output::heading("Next steps:");
		output::bullet("Run 'yammm add <mod>' to add mods");
		output::bullet("Run 'yammm launch' to launch Minecraft");

		Ok(())
	}

	async fn collect_init_info(
		&self,
		http_client: &reqwest::Client,
	) -> Result<InitInfo> {
		let interactive = self.should_interact();

		if interactive {
			output::blank_line();
			output::heading("Welcome to YAMMM Modpack Creator!");
			output::dim("Let's create your modpack step by step.");
			output::blank_line();
		}

		let name = self.prompt_name(interactive)?;
		let minecraft_version = self.prompt_minecraft_version(interactive)?;
		let loader = self.prompt_loader(interactive)?;
		let loader_version = self
			.prompt_loader_version(
				interactive,
				&loader,
				&minecraft_version,
				http_client,
			)
			.await;
		let description = self.prompt_description(interactive)?;
		let version = self.prompt_version(interactive)?;

		Ok(InitInfo {
			name,
			version,
			minecraft_version,
			loader,
			loader_version,
			description,
		})
	}

	fn prompt_name(
		&self,
		interactive: bool,
	) -> Result<String> {
		if let Some(ref n) = self.name {
			return Ok(n.clone());
		}
		if interactive {
			Ok(Input::<String>::new()
				.with_prompt(format!(
					"Modpack name (default: {})",
					DEFAULT_MODPACK_NAME
				))
				.default(DEFAULT_MODPACK_NAME.to_string())
				.interact()?)
		} else {
			Ok(DEFAULT_MODPACK_NAME.to_string())
		}
	}

	fn prompt_minecraft_version(
		&self,
		interactive: bool,
	) -> Result<String> {
		if let Some(ref v) = self.minecraft_version {
			return Ok(v.clone());
		}
		if interactive {
			Ok(Input::<String>::new()
				.with_prompt(format!(
					"Minecraft version (default: {})",
					DEFAULT_MINECRAFT_VERSION
				))
				.validate_with(|x: &String| -> Result<(), &str> {
					if x.is_empty() {
						Err("Version cannot be empty")
					} else {
						Ok(())
					}
				})
				.default(DEFAULT_MINECRAFT_VERSION.to_string())
				.interact()?)
		} else {
			Ok(DEFAULT_MINECRAFT_VERSION.to_string())
		}
	}

	fn prompt_loader(
		&self,
		interactive: bool,
	) -> Result<LoaderType> {
		if let Some(ref l) = self.loader {
			return l.parse().map_err(|e: crate::types::LoaderError| {
				crate::errors::YammmError::invalid_args(e.to_string()).into()
			});
		}
		if interactive {
			let choices = LoaderType::all();
			let labels: Vec<String> =
				choices.iter().map(|l| format!("{:?}", l)).collect();
			let index = Select::new()
				.with_prompt("Select mod loader")
				.items(&labels)
				.default(0)
				.interact()?;
			Ok(choices[index])
		} else {
			Ok(LoaderType::default())
		}
	}

	async fn prompt_loader_version(
		&self,
		interactive: bool,
		loader: &LoaderType,
		mc_version: &str,
		http_client: &reqwest::Client,
	) -> Option<String> {
		if let Some(ref v) = self.loader_version {
			return Some(v.clone());
		}
		if interactive {
			fetch_loader_version_for(loader, mc_version, http_client).await
		} else {
			None
		}
	}

	fn prompt_description(
		&self,
		interactive: bool,
	) -> Result<Option<String>> {
		if let Some(ref d) = self.description {
			return Ok(Some(d.clone()));
		}
		if interactive {
			let desc = Input::<String>::new()
				.with_prompt("Description (optional, press Enter to skip)")
				.allow_empty(true)
				.interact()?;
			Ok(if desc.is_empty() { None } else { Some(desc) })
		} else {
			Ok(None)
		}
	}

	fn prompt_version(
		&self,
		interactive: bool,
	) -> Result<Option<String>> {
		if interactive {
			let v = Input::<String>::new()
				.with_prompt(format!(
					"Modpack version (default: {})",
					DEFAULT_MODPACK_VERSION
				))
				.allow_empty(true)
				.default(DEFAULT_MODPACK_VERSION.to_string())
				.interact()?;
			Ok(if v.is_empty() { None } else { Some(v) })
		} else {
			Ok(None)
		}
	}

	/// Decide whether to show interactive prompts.
	///
	/// Interactive mode activates when:
	/// - `--interactive` flag is explicitly set, OR
	/// - No name, version, or loader flags were provided (blank slate)
	///
	/// If the user provides *any* of those flags, we assume they want
	/// non-interactive mode with defaults for the rest.
	fn should_interact(&self) -> bool {
		self.interactive
			|| (self.name.is_none()
				&& self.minecraft_version.is_none()
				&& self.loader.is_none())
	}
}

#[derive(Debug)]
struct InitInfo {
	name: String,
	version: Option<String>,
	minecraft_version: String,
	loader: LoaderType,
	loader_version: Option<String>,
	description: Option<String>,
}

async fn fetch_loader_version(
	loader_name: &str,
	fetch_fn: impl std::future::Future<Output = Result<String, anyhow::Error>>,
) -> Option<String> {
	output::info(format!("Fetching latest {} loader version...", loader_name));
	match fetch_fn.await {
		Ok(version) => {
			output::bullet(format!(
				"{} loader version: {}",
				loader_name, version
			));
			Some(version)
		}
		Err(e) => {
			output::warning(format!(
				"Could not fetch {} loader version: {}",
				loader_name, e
			));
			None
		}
	}
}

async fn fetch_loader_version_for(
	loader: &crate::types::LoaderType,
	mc_version: &str,
	http_client: &reqwest::Client,
) -> Option<String> {
	match loader {
		LoaderType::Fabric => {
			let client = crate::api::FabricClient::new()
				.with_client(http_client.clone());
			fetch_loader_version("Fabric", async {
				client
					.get_latest_loader_version(mc_version)
					.await
					.map_err(Into::into)
			})
			.await
		}
		LoaderType::Quilt => {
			let client =
				crate::api::QuiltClient::new().with_client(http_client.clone());
			fetch_loader_version("Quilt", async {
				client
					.get_latest_loader_version(mc_version)
					.await
					.map_err(Into::into)
			})
			.await
		}
		LoaderType::Forge => {
			let client =
				crate::api::ForgeClient::new().with_client(http_client.clone());
			fetch_loader_version("Forge", async {
				client
					.get_latest_version(mc_version)
					.await
					.map_err(Into::into)
			})
			.await
		}
		LoaderType::NeoForge => {
			let client = crate::api::NeoForgeClient::new()
				.with_client(http_client.clone());
			fetch_loader_version("NeoForge", async {
				client
					.get_latest_version(mc_version)
					.await
					.map_err(Into::into)
			})
			.await
		}
	}
}

fn ensure_dir(
	path: &std::path::Path,
	label: &str,
) -> anyhow::Result<()> {
	if !path.exists() {
		fs::create_dir_all(path)
			.with_context(|| format!("Failed to create {} directory", label))?;
		output::success(format!("Created {} directory", label));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_init_command_parse() {
		let cmd = InitCommand::parse_from([
			"init",
			"--name",
			"test-modpack",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		]);

		assert_eq!(cmd.name, Some("test-modpack".to_string()));
		assert_eq!(cmd.minecraft_version, Some("1.20.4".to_string()));
		assert_eq!(cmd.loader, Some("fabric".to_string()));
	}

	#[test]
	fn test_init_command_defaults() {
		let cmd = InitCommand::parse_from(["init"]);

		assert_eq!(cmd.name, None);
		assert_eq!(cmd.minecraft_version, None);
		assert_eq!(cmd.loader, None);
		assert!(!cmd.interactive);
	}

	#[test]
	fn test_should_interact() {
		let cmd = InitCommand {
			name: None,
			minecraft_version: None,
			loader: None,
			loader_version: None,
			description: None,
			output_dir: PathBuf::from("."),
			interactive: false,
		};
		assert!(cmd.should_interact());

		let cmd = InitCommand {
			name: Some("test".to_string()),
			minecraft_version: None,
			loader: None,
			loader_version: None,
			description: None,
			output_dir: PathBuf::from("."),
			interactive: false,
		};
		assert!(!cmd.should_interact());

		let cmd = InitCommand {
			name: Some("test".to_string()),
			minecraft_version: None,
			loader: None,
			loader_version: None,
			description: None,
			output_dir: PathBuf::from("."),
			interactive: true,
		};
		assert!(cmd.should_interact());
	}
}
