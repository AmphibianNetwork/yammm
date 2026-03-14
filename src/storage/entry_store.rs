//! Persistence layer for individual entry files (mods, resource packs, shaders).
//!
//! Each tracked item is stored in `mods/<slug>/mod.ron`.

use crate::config::ModpackManifest;
use crate::types::TrackedMod;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Read/write access to a directory of tracked entries.
///
/// Each entry has its own subdirectory containing a `mod.ron` file.
/// Used for mods, resource packs, and shader packs alike.
pub struct EntryStore {
	dir: PathBuf,
}

impl EntryStore {
	pub fn new(
		root: &Path,
		config: &ModpackManifest,
	) -> Self {
		Self {
			dir: config.mods_dir(root),
		}
	}

	/// Create an EntryStore from an explicit directory path
	pub fn from_dir(dir: PathBuf) -> Self {
		Self { dir }
	}

	/// Get the base directory path for this store
	pub fn base_dir(&self) -> &Path {
		&self.dir
	}

	/// Get the directory path for a specific entry
	/// Returns: `<dir>/<id>`
	pub fn entry_dir(
		&self,
		id: &str,
	) -> PathBuf {
		self.dir.join(id)
	}

	/// Path to an entry's RON file: `<dir>/<id>/mod.ron`
	pub fn entry_path(
		&self,
		id: &str,
	) -> PathBuf {
		self.dir.join(id).join("mod.ron")
	}

	/// Check if an entry exists.
	pub fn exists(
		&self,
		id: &str,
	) -> bool {
		self.entry_path(id).exists()
	}

	/// Load an entry's RON data.
	pub fn load(
		&self,
		id: &str,
	) -> Result<TrackedMod> {
		let path = self.entry_path(id);
		let contents = std::fs::read_to_string(&path)
			.context(format!("Failed to read entry metadata for '{}'", id))?;
		let data: TrackedMod = ron::from_str(&contents)?;
		Ok(data)
	}

	/// Save an entry's RON data. Creates parent directory if needed.
	pub fn save(
		&self,
		id: &str,
		tracked_mod: &TrackedMod,
	) -> Result<()> {
		let path = self.entry_path(id);
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.context(format!("Failed to create directory for '{}'", id))?;
		}
		let contents = ron::ser::to_string_pretty(
			tracked_mod,
			ron::ser::PrettyConfig::default(),
		)?;
		std::fs::write(&path, contents)
			.context(format!("Failed to write entry metadata for '{}'", id))
	}

	/// Remove an entry's directory. No-op if it doesn't exist.
	pub fn remove(
		&self,
		id: &str,
	) -> Result<()> {
		let dir = self.entry_dir(id);
		if dir.exists() {
			std::fs::remove_dir_all(&dir).context(format!(
				"Failed to remove entry directory for '{}'",
				id
			))?;
		}
		Ok(())
	}

	/// List all entries by reading every `<slug>/mod.ron`.
	pub fn list(&self) -> Result<Vec<TrackedMod>> {
		let mut entries = Vec::new();
		if !self.dir.exists() {
			return Ok(entries);
		}
		for entry in std::fs::read_dir(&self.dir)
			.context("Failed to read entries directory")?
		{
			let entry = entry.context("Failed to read directory entry")?;
			let path = entry.path();
			if path.is_dir() {
				let ron_path = path.join("mod.ron");
				if ron_path.exists() {
					let id = match path.file_name().and_then(|n| n.to_str()) {
						Some(name) if !name.is_empty() => name,
						_ => continue,
					};
					match self.load(id) {
						Ok(mod_ron) => entries.push(mod_ron),
						Err(e) => tracing::warn!(
							"Failed to load entry metadata: {}: {}",
							ron_path.display(),
							e
						),
					}
				}
			}
		}
		Ok(entries)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::TempDir;

	#[test]
	fn test_list_mods_empty() {
		let temp_dir = TempDir::new().unwrap();
		let store = EntryStore::from_dir(temp_dir.path().join("mods"));
		let mods = store.list().unwrap();
		assert!(mods.is_empty());
	}

	#[test]
	fn test_save_load_mod() {
		let temp_dir = TempDir::new().unwrap();
		let mod_ron =
			crate::test_util::make_test_mod("jei", "Just Enough Items");

		let store = EntryStore::from_dir(temp_dir.path().join("mods"));
		store.save("jei", &mod_ron).unwrap();
		let loaded = store.load("jei").unwrap();
		assert_eq!(loaded.id, "jei");
		assert_eq!(loaded.name, "Just Enough Items");
		assert_eq!(loaded.version, "1.0.0");
	}

	#[test]
	fn test_list_mods() {
		let temp_dir = TempDir::new().unwrap();
		let mod1 = crate::test_util::make_test_mod("jei", "JEI");
		let mod2 = crate::test_util::make_test_mod("fabric-api", "Fabric API");

		let store = EntryStore::from_dir(temp_dir.path().join("mods"));
		store.save("jei", &mod1).unwrap();
		store.save("fabric-api", &mod2).unwrap();

		let mods = store.list().unwrap();
		assert_eq!(mods.len(), 2);
	}

	#[test]
	fn test_remove_mod() {
		let temp_dir = TempDir::new().unwrap();
		let mod_ron = crate::test_util::make_test_mod("jei", "JEI");

		let store = EntryStore::from_dir(temp_dir.path().join("mods"));
		store.save("jei", &mod_ron).unwrap();
		assert!(store.exists("jei"));

		store.remove("jei").unwrap();
		assert!(!store.exists("jei"));
	}

	#[test]
	fn test_load_mod_not_found() {
		let temp_dir = TempDir::new().unwrap();
		let store = EntryStore::from_dir(temp_dir.path().join("mods"));
		assert!(store.load("nonexistent").is_err());
	}

	#[test]
	fn test_remove_mod_nonexistent_ok() {
		let temp_dir = TempDir::new().unwrap();
		let store = EntryStore::from_dir(temp_dir.path().join("mods"));
		assert!(store.remove("nonexistent").is_ok());
	}
}
