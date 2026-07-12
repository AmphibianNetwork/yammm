use super::ImportCommand;
use super::helpers::{ExtractDecision, classify_archive_entry};
use crate::app::AppContext;
use crate::output;
use anyhow::{Context, Result};
use ron::de::from_bytes;

use crate::types::{ProjectType, TrackedMod};
use std::io::Read;
use std::path::{Path, PathBuf};

impl ImportCommand {
	pub(super) async fn import_ympk(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let output_dir = if let Some(ref dir) = self.output {
			dir.clone()
		} else if let Some(app) = ctx.modpack() {
			app.root_dir.clone()
		} else {
			let name =
				self.file.file_stem().unwrap_or_default().to_string_lossy();
			PathBuf::from(name.to_string())
		};

		output::info(format!("Importing: {}", self.file.display()));
		output::bullet("Format: YMPK (native yammm modpack)");
		output::bullet(format!("Extracting to: {}", output_dir.display()));

		// JSON mode is non-interactive — treat as if --yes were passed.
		if output_dir.exists() && !self.yes && !output::is_json_mode() {
			let proceed = dialoguer::Confirm::new()
				.with_prompt(
					"Destination directory already exists. Import anyway?",
				)
				.default(false)
				.interact()?;

			if !proceed {
				output::cancelled("Import");
				return Ok(());
			}
		}

		let file = std::fs::File::open(&self.file)?;
		let mut archive =
			zip::ZipArchive::new(file).context("Not a valid ZIP archive")?;

		let mut modpack_toml_bytes = Vec::new();
		archive
			.by_name("modpack.toml")
			.context("No modpack.toml found in YMPK")?
			.read_to_end(&mut modpack_toml_bytes)?;
		let mut modpack_config: crate::config::ModpackManifest =
			toml::from_slice(&modpack_toml_bytes)
				.context("Failed to parse modpack.toml")?;

		let config_data = extract_config_data(&mut archive, &output_dir)?;

		std::fs::create_dir_all(&output_dir)?;

		let modpack_path = output_dir.join("modpack.toml");
		let jar_cache = ctx.jar_cache().clone();

		let app = if modpack_path.exists() {
			let mut existing =
				crate::app::App::load(output_dir.clone(), jar_cache)?;
			let existing_config = existing.config.clone();
			if existing_config.name.is_empty()
				&& !modpack_config.name.is_empty()
			{
				existing.config.name = std::mem::take(&mut modpack_config.name);
			}
			if existing_config.version.is_empty()
				&& !modpack_config.version.is_empty()
			{
				existing.config.version =
					std::mem::take(&mut modpack_config.version);
			}
			if existing_config.description.is_empty()
				&& !modpack_config.description.is_empty()
			{
				existing.config.description =
					std::mem::take(&mut modpack_config.description);
			}
			existing
		} else {
			crate::app::App::from_parts(
				output_dir.clone(),
				modpack_config,
				jar_cache,
			)
		};

		let storage = &app.storage;
		let mut added = 0usize;
		let mut skipped = 0usize;

		for pt in ProjectType::VARIANTS {
			let prefix = match pt {
				ProjectType::Mod => "mods",
				ProjectType::ResourcePack => "resourcepacks",
				ProjectType::Shader => "shaderpacks",
			};

			let mut entries_to_process: Vec<(String, Vec<u8>)> = Vec::new();

			for i in 0..archive.len() {
				let mut zip_file = archive.by_index(i)?;
				let name = zip_file.name().to_string();
				let name_normalized = name.replace('\\', "/");

				if name_normalized.starts_with(&format!("{}/", prefix))
					&& name_normalized.ends_with("/entry.ron")
				{
					let mut data = Vec::new();
					Read::read_to_end(&mut zip_file, &mut data)?;
					entries_to_process.push((name_normalized, data));
				}
			}

			for (name, data) in &entries_to_process {
				let tracked_mod: TrackedMod = match from_bytes(data) {
					Ok(m) => m,
					Err(e) => {
						tracing::warn!("Failed to parse {}: {}", name, e);
						continue;
					}
				};

				let slug = &tracked_mod.id;

				if storage.exists(*pt, slug) {
					output::bullet(format!("{} (already installed)", slug));
					skipped += 1;
					continue;
				}

				storage.save(*pt, slug, &tracked_mod)?;
				output::bullet(format!("{} (new)", slug));
				added += 1;
			}
		}

		if !modpack_path.exists() {
			storage.save_modpack(&app.config)?;
			output::success("Created modpack.toml");
		} else {
			storage.save_modpack(&app.config)?;
			output::success("Updated modpack.toml");
		}

		write_config_data(&config_data)?;

		if output::is_json_mode() {
			output::emit_json(&serde_json::json!({
				"command": "import",
				"format": "ympk",
				"output_dir": output_dir.display().to_string(),
				"added": added,
				"skipped": skipped,
			}))?;
			return Ok(());
		}

		output::success(format!(
			"Done! Added {} mods, {} skipped. Modpack ready at: {}",
			added,
			skipped,
			output_dir.display()
		));

		Ok(())
	}
}

fn extract_config_data(
	archive: &mut zip::ZipArchive<std::fs::File>,
	root_dir: &Path,
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
	let mut files = Vec::new();
	let config_root = root_dir.join("config");

	for i in 0..archive.len() {
		let mut zip_file = archive.by_index(i)?;
		let name = zip_file.name().to_string();

		let outpath =
			match classify_archive_entry(&name, "config", &config_root) {
				ExtractDecision::Extract(p) => p,
				ExtractDecision::Skip => continue,
				ExtractDecision::Unsafe => {
					tracing::warn!(
						"Skipping config with path escape: {}",
						name
					);
					continue;
				}
			};

		if outpath.exists() {
			continue;
		}

		let mut data = Vec::new();
		Read::read_to_end(&mut zip_file, &mut data)?;
		files.push((outpath, data));
	}

	Ok(files)
}

fn write_config_data(files: &[(PathBuf, Vec<u8>)]) -> Result<()> {
	let mut extracted = 0usize;

	for (outpath, data) in files {
		if let Some(p) = outpath.parent()
			&& !p.exists()
		{
			std::fs::create_dir_all(p)?;
		}

		std::fs::write(outpath, data)?;
		extracted += 1;
	}

	if extracted > 0 {
		output::success(format!("Extracted {} config file(s)", extracted));
	}

	Ok(())
}
