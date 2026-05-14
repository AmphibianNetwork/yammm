//! Persistence layer for `modpack.toml`.

use crate::config::ModpackManifest;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Read/write access to a `modpack.toml` file.
#[derive(Debug, Clone)]
pub struct ManifestStore {
	path: PathBuf,
}

impl ManifestStore {
	pub fn new(path: impl Into<PathBuf>) -> Self {
		Self { path: path.into() }
	}

	pub fn from_root(root: &Path) -> Self {
		Self::new(root.join("modpack.toml"))
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn exists(&self) -> bool {
		self.path.exists()
	}

	pub fn load(&self) -> Result<ModpackManifest> {
		let contents =
			std::fs::read_to_string(&self.path).with_context(|| {
				format!("Cannot read config: {}", self.path.display())
			})?;
		toml::from_str(&contents).context("Failed to parse config")
	}

	pub fn save(
		&self,
		config: &ModpackManifest,
	) -> Result<()> {
		if let Some(parent) = self.path.parent() {
			std::fs::create_dir_all(parent)
				.context("Failed to create modpack config directory")?;
		}
		let contents = toml::to_string_pretty(config)
			.context("Failed to serialize modpack config")?;
		atomic_write(&self.path, &contents)
			.context("Failed to write modpack.toml")
	}
}

fn atomic_write(
	path: &Path,
	contents: &str,
) -> std::io::Result<()> {
	let tmp_path = path.with_extension("tmp");
	std::fs::write(&tmp_path, contents)?;
	std::fs::rename(&tmp_path, path).inspect_err(|_| {
		let _ = std::fs::remove_file(&tmp_path);
	})
}
