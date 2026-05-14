//! Storage layer: all filesystem I/O for config and mod data.
//!
//! - `ManifestStore` — reads/writes `modpack.toml`
//! - `EntryStore` — reads/writes per-mod `entry.ron` files under `<type_dir>/<slug>/`
//! - `JarCache` — content-addressed JAR file cache (hash-based filenames)
//! - `CacheManager` — unified management across jars/minecraft/loaders subdirs
//!
//! `Storage` is the facade that dispatches to the correct `EntryStore`
//! based on `ProjectType`.

pub mod cache;
pub mod cache_manager;
pub mod entry_store;
pub mod modpack;

pub use cache::JarCache;
pub use cache_manager::CacheManager;
pub use entry_store::EntryStore;
pub use modpack::ManifestStore;

use crate::config::ModpackManifest;
use crate::types::{ProjectType, TrackedMod};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Unified storage facade for modpack interactions.
///
/// Owns the directory layout and delegates to `EntryStore` instances
/// for each project type.
#[derive(Debug, Clone)]
pub struct Storage {
	pub modpack_path: PathBuf,
	pub mods_dir: PathBuf,
	pub resourcepacks_dir: PathBuf,
	pub shaderpacks_dir: PathBuf,
	modpack_store: ManifestStore,
}

impl Storage {
	/// Build storage from a modpack root directory and its manifest.
	pub fn new(
		root: &Path,
		config: &ModpackManifest,
	) -> Self {
		let modpack_path = root.join("modpack.toml");
		let modpack_store = ManifestStore::new(&modpack_path);
		Self {
			modpack_path,
			mods_dir: config.mods_dir(root),
			resourcepacks_dir: config.resourcepacks_dir(root),
			shaderpacks_dir: config.shaderpacks_dir(root),
			modpack_store,
		}
	}

	pub fn load_modpack(&self) -> Result<ModpackManifest> {
		self.modpack_store.load()
	}

	pub fn save_modpack(
		&self,
		modpack: &ModpackManifest,
	) -> Result<()> {
		self.modpack_store.save(modpack)
	}

	/// Dispatch to the correct `EntryStore` based on `ProjectType`.
	pub fn store_for(
		&self,
		project_type: ProjectType,
	) -> EntryStore {
		match project_type {
			ProjectType::Mod => self.mod_store(),
			ProjectType::ResourcePack => self.resource_pack_store(),
			ProjectType::Shader => self.shader_pack_store(),
		}
	}

	pub fn exists(
		&self,
		project_type: ProjectType,
		id: &str,
	) -> bool {
		self.store_for(project_type).exists(id)
	}

	pub fn load(
		&self,
		project_type: ProjectType,
		id: &str,
	) -> Result<TrackedMod> {
		self.store_for(project_type).load(id)
	}

	pub fn save(
		&self,
		project_type: ProjectType,
		id: &str,
		tracked_mod: &TrackedMod,
	) -> Result<()> {
		self.store_for(project_type).save(id, tracked_mod)
	}

	pub fn remove(
		&self,
		project_type: ProjectType,
		id: &str,
	) -> Result<()> {
		self.store_for(project_type).remove(id)
	}

	pub fn list(
		&self,
		project_type: ProjectType,
	) -> Result<Vec<TrackedMod>> {
		self.store_for(project_type).list()
	}

	/// List all tracked items across all project types (mods + resource packs + shaders).
	pub fn list_all(&self) -> Result<Vec<TrackedMod>> {
		let mut all = Vec::new();
		for pt in ProjectType::VARIANTS {
			all.extend(self.store_for(*pt).list()?);
		}
		Ok(all)
	}

	pub fn mod_store(&self) -> EntryStore {
		EntryStore::from_dir(self.mods_dir.clone())
	}

	pub fn resource_pack_store(&self) -> EntryStore {
		EntryStore::from_dir(self.resourcepacks_dir.clone())
	}

	pub fn shader_pack_store(&self) -> EntryStore {
		EntryStore::from_dir(self.shaderpacks_dir.clone())
	}

	/// Search all project types for an entry by slug.
	pub fn find_any(
		&self,
		id: &str,
	) -> Result<(ProjectType, TrackedMod)> {
		for pt in ProjectType::VARIANTS {
			if self.store_for(*pt).exists(id) {
				return Ok((*pt, self.store_for(*pt).load(id)?));
			}
		}
		Err(crate::errors::YammmError::mod_not_found(format!(
			"'{}' not found in mods, resourcepacks, or shaderpacks",
			id
		))
		.into())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::ModpackManifest;
	use crate::types::ProjectType;
	use tempfile::TempDir;

	#[test]
	fn test_storage_dispatch_by_project_type() {
		let temp_dir = TempDir::new().unwrap();
		let config = ModpackManifest::new();
		let storage = Storage::new(temp_dir.path(), &config);

		let mod_ron = crate::test_util::make_test_mod("sodium", "Sodium");

		storage.save(ProjectType::Mod, "sodium", &mod_ron).unwrap();
		assert!(storage.exists(ProjectType::Mod, "sodium"));
		let loaded = storage.load(ProjectType::Mod, "sodium").unwrap();
		assert_eq!(loaded.name, "Sodium");

		storage.remove(ProjectType::Mod, "sodium").unwrap();
		assert!(!storage.exists(ProjectType::Mod, "sodium"));
	}
}
