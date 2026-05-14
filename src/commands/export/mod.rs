use crate::app::AppContext;
use crate::commands::export::mrpack::export_to_mrpack;
use crate::config::ModpackManifest;
use crate::output;
use crate::services::download_missing_mods;
use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::io::Write;
use std::path::PathBuf;

pub mod mrpack;

/// Export the modpack to a distributable archive format.
#[derive(Parser, Debug)]
pub struct ExportCommand {
	#[arg(short = 'f', long, default_value = "mrpack", value_enum)]
	pub format: ExportFormat,

	#[arg(short = 'o', long)]
	pub output: Option<String>,

	#[arg(short = 'y', long)]
	pub yes: bool,
}

#[derive(Parser, Debug, Clone, ValueEnum)]
pub enum ExportFormat {
	Mrpack,
	Ympk,
}

impl ExportCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("ExportCommand running");
		tracing::debug!("Format: {:?}", self.format);
		tracing::debug!("Output: {:?}", self.output);

		let app = ctx.require_modpack()?;
		let modpack = &app.config;
		let _root_dir = &app.root_dir;

		output::heading(format!("Exporting modpack: {}", modpack.name));
		output::bullet(format!("Format: {:?}", self.format));

		let summary = download_missing_mods(
			&app.storage,
			&app.cache,
			&ctx.http_client,
			ctx.global.max_concurrent_downloads(),
		)
		.await?;
		output::present_download_summary(&summary);
		if !summary.failed.is_empty() {
			return Err(crate::errors::YammmError::download_failed(format!(
				"{} file(s) could not be downloaded",
				summary.failed.len()
			))
			.into());
		}

		let output_path = self.generate_output_path(modpack);
		output::bullet(format!("Output: {}", output_path.display()));

		if !self.yes {
			let proceed = dialoguer::Confirm::new()
				.with_prompt("Create modpack archive?")
				.default(true)
				.interact()?;
			if !proceed {
				output::cancelled("Export");
				return Ok(());
			}
		}

		match self.format {
			ExportFormat::Mrpack => export_to_mrpack(
				modpack,
				&app.storage,
				&app.cache,
				&app.root_dir,
				&output_path,
			)?,
			ExportFormat::Ympk => export_ympk(
				modpack,
				&app.storage,
				&app.cache,
				&app.root_dir,
				&output_path,
			)?,
		}

		output::blank_line();
		output::success(format!("Export complete: {}", output_path.display()));

		Ok(())
	}

	fn generate_output_path(
		&self,
		modpack: &ModpackManifest,
	) -> PathBuf {
		if let Some(output) = &self.output {
			return PathBuf::from(output);
		}

		let name = crate::utils::slugify(&modpack.name);
		let ext = match self.format {
			ExportFormat::Mrpack => "mrpack",
			ExportFormat::Ympk => "ympk",
		};

		let timestamp = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.as_secs().to_string())
			.unwrap_or_default();

		PathBuf::from(format!("{}-{}.{}", name, timestamp, ext))
	}
}

fn export_project_type_to_ympk(
	zip: &mut zip::ZipWriter<std::fs::File>,
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
	project_type: crate::types::ProjectType,
	dir_name: &str,
	ext: &str,
) -> anyhow::Result<()> {
	use zip::write::SimpleFileOptions;

	let items = storage.list(project_type)?;
	let store = storage.store_for(project_type);
	for item in &items {
		let ron_path = store.entry_path(&item.id);
		if ron_path.exists() {
			let content = std::fs::read_to_string(&ron_path)?;
			let entry_path = format!("{}/{}/entry.ron", dir_name, item.id);
			zip.start_file::<_, ()>(entry_path, SimpleFileOptions::default())?;
			zip.write_all(content.as_bytes())?;
		}
		if let Some(jar_path) = item
			.hash
			.as_ref()
			.and_then(|h| cache.get(item.hash_type, h))
		{
			let slug = crate::utils::slugify(&item.name);
			let entry_path = format!("{}/{}{}", dir_name, slug, ext);
			let content = std::fs::read(&jar_path)?;
			zip.start_file::<_, ()>(entry_path, SimpleFileOptions::default())?;
			zip.write_all(&content)?;
		}
	}
	Ok(())
}

fn add_dir_to_zip(
	zip: &mut zip::ZipWriter<std::fs::File>,
	dir: &std::path::Path,
	prefix: &str,
) -> anyhow::Result<()> {
	use zip::write::SimpleFileOptions;

	for entry in std::fs::read_dir(dir)? {
		let entry = entry?;
		let path = entry.path();
		let name = entry.file_name().to_string_lossy().to_string();
		let entry_path = format!("{}/{}", prefix, name);

		if path.is_dir() {
			add_dir_to_zip(zip, &path, &entry_path)?;
		} else {
			let content = std::fs::read(&path)?;
			zip.start_file::<&str, ()>(
				&entry_path,
				SimpleFileOptions::default(),
			)?;
			zip.write_all(&content)?;
		}
	}

	Ok(())
}

fn export_ympk(
	modpack: &ModpackManifest,
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
	root_dir: &std::path::Path,
	output_path: &PathBuf,
) -> anyhow::Result<()> {
	use zip::write::SimpleFileOptions;

	let file = std::fs::File::create(output_path)?;
	let mut zip = zip::ZipWriter::new(file);

	let toml_content = toml::to_string_pretty(modpack)?;
	zip.start_file::<_, ()>("modpack.toml", SimpleFileOptions::default())?;
	zip.write_all(toml_content.as_bytes())?;

	for (project_type, dir_name, ext) in
		crate::types::ProjectType::EXPORT_ENTRIES
	{
		export_project_type_to_ympk(
			&mut zip,
			storage,
			cache,
			*project_type,
			dir_name,
			ext,
		)?;
	}

	let config_dir = root_dir.join("config");
	if config_dir.exists() {
		add_dir_to_zip(&mut zip, &config_dir, "config")?;
	}

	zip.finish()?;
	Ok(())
}
