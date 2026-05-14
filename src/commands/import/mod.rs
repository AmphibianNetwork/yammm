mod helpers;
mod mrpack;
mod resolve;
mod ympk;

use crate::app::AppContext;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct ImportCommand {
	pub file: PathBuf,

	#[arg(short = 'o', long)]
	pub output: Option<PathBuf>,

	#[arg(short = 'y', long)]
	pub yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFormat {
	Mrpack,
	Ympk,
}

impl ImportCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("ImportCommand running");

		if !self.file.exists() {
			return Err(crate::errors::YammmError::invalid_args(format!(
				"File not found: {}",
				self.file.display()
			))
			.into());
		}

		let format = detect_format(&self.file)?;

		match format {
			ImportFormat::Mrpack => self.import_mrpack(ctx).await,
			ImportFormat::Ympk => self.import_ympk(ctx).await,
		}
	}
}

fn detect_format(path: &std::path::Path) -> Result<ImportFormat> {
	let ext = path
		.extension()
		.map(|e| e.to_string_lossy().to_lowercase())
		.unwrap_or_default();

	match ext.as_str() {
		"mrpack" => Ok(ImportFormat::Mrpack),
		"ympk" => Ok(ImportFormat::Ympk),
		"zip" => detect_zip_format(path),
		_ => Err(crate::errors::YammmError::invalid_args(format!(
			"Unsupported format: .{}",
			ext
		))
		.into()),
	}
}

fn detect_zip_format(path: &std::path::Path) -> Result<ImportFormat> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file).with_context(|| {
		format!("{} is not a valid ZIP archive", path.display())
	})?;

	if archive.by_name("modrinth.index.json").is_ok() {
		return Ok(ImportFormat::Mrpack);
	}

	if archive.by_name("modpack.toml").is_ok() {
		return Ok(ImportFormat::Ympk);
	}

	Err(crate::errors::YammmError::invalid_args(
		format!(
			"Cannot determine format of {}. Neither modrinth.index.json nor modpack.toml found inside.",
			path.display()
		),
	).into())
}

#[cfg(test)]
mod tests {
	use super::helpers::{extract_slug_from_path, extract_version_from_path};
	use super::mrpack::{determine_source, extract_project_id_from_cdn_url};
	use crate::types::ModSource;

	#[test]
	fn test_extract_slug_from_path() {
		assert_eq!(extract_slug_from_path("mods/sodium.jar"), "sodium");
		assert_eq!(extract_slug_from_path("mods/lithium-2.0.jar"), "lithium");
		assert_eq!(
			extract_slug_from_path("mods\\windows-mod.jar"),
			"windows-mod"
		);
		assert_eq!(
			extract_slug_from_path("resourcepacks/some-pack.zip"),
			"some-pack"
		);
		assert_eq!(
			extract_slug_from_path("shaderpacks/bsl-shaders.zip"),
			"bsl-shaders"
		);
	}

	#[test]
	fn test_extract_version_from_path() {
		assert_eq!(extract_version_from_path("mods/sodium-0.5.8.jar"), "0.5.8");
		assert_eq!(
			extract_version_from_path("mods/fabric-api-0.92.0+1.20.4.jar"),
			"0.92.0+1.20.4"
		);
		assert_eq!(extract_version_from_path("mods/simplemod.jar"), "0.0.0");
		assert_eq!(
			extract_version_from_path("mods/iris-mc1.20.4-1.6.17.jar"),
			"1.6.17"
		);
	}

	#[test]
	fn test_determine_source_modrinth_cdn() {
		let source = determine_source(
			&[
				"https://cdn.modrinth.com/data/fRiHVvU7/versions/1.0/mod.jar"
					.to_string(),
			],
			"abc",
		);
		match source {
			ModSource::Modrinth { id } => assert_eq!(id, "fRiHVvU7"),
			_ => panic!("Expected Modrinth source with project ID"),
		}
	}

	#[test]
	fn test_determine_source_modrinth_no_cdn() {
		let source = determine_source(
			&["https://api.modrinth.com/some/path.jar".to_string()],
			"abc",
		);
		match source {
			ModSource::Modrinth { id } => assert_eq!(id, "abc"),
			_ => panic!("Expected Modrinth source with slug fallback"),
		}
	}

	#[test]
	fn test_determine_source_curseforge() {
		let source = determine_source(
			&["https://edge.forge.curseforge.com/files/123/mod.jar"
				.to_string()],
			"abc",
		);
		assert!(matches!(source, ModSource::CurseForge { .. }));
	}

	#[test]
	fn test_determine_source_url() {
		let source = determine_source(
			&["https://example.com/mod.jar".to_string()],
			"abc",
		);
		assert!(matches!(source, ModSource::Url { .. }));
	}

	#[test]
	fn test_extract_project_id_from_cdn_url() {
		assert_eq!(
			extract_project_id_from_cdn_url(
				"https://cdn.modrinth.com/data/fRiHVvU7/versions/abc123/mod.jar"
			),
			Some("fRiHVvU7".to_string())
		);
		assert_eq!(
			extract_project_id_from_cdn_url(
				"https://api.modrinth.com/v2/project/emi"
			),
			None
		);
		assert_eq!(
			extract_project_id_from_cdn_url("https://example.com/mod.jar"),
			None
		);
	}
}
