//! Modpack configuration (modpack.toml).
//! Individual mods are stored in `.ron` files under the mods/ directory.

use crate::errors::YammmError;
use crate::types::{LoaderType, VersionFilters};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Modpack configuration loaded from modpack.toml.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModpackManifest {
	#[serde(default)]
	pub name: String,

	#[serde(default)]
	pub description: String,

	#[serde(default)]
	pub version: String,

	#[serde(default)]
	pub minecraft_version: String,

	#[serde(default)]
	pub loader: LoaderConfig,

	#[serde(default)]
	pub mod_path: Option<PathBuf>,

	#[serde(default)]
	pub resource_pack_path: Option<PathBuf>,

	#[serde(default)]
	pub shader_pack_path: Option<PathBuf>,
}

/// Mod loader configuration (e.g., Fabric 0.16.5).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoaderConfig {
	#[serde(default)]
	pub loader: Option<LoaderType>,

	#[serde(default)]
	pub version: String,
}

impl LoaderConfig {
	/// Returns the configured loader type, defaulting to Fabric.
	pub fn loader_or_default(&self) -> LoaderType {
		self.loader.unwrap_or(LoaderType::Fabric)
	}
}

impl ModpackManifest {
	pub fn new() -> Self {
		Self::default()
	}

	fn resolve_dir(
		&self,
		custom: &Option<PathBuf>,
		base_path: &std::path::Path,
		default: &str,
	) -> PathBuf {
		custom
			.as_ref()
			.map(|p| base_path.join(p))
			.unwrap_or_else(|| base_path.join(default))
	}

	pub fn mods_dir(
		&self,
		base_path: &std::path::Path,
	) -> PathBuf {
		self.resolve_dir(&self.mod_path, base_path, "mods")
	}

	pub fn resourcepacks_dir(
		&self,
		base_path: &std::path::Path,
	) -> PathBuf {
		self.resolve_dir(&self.resource_pack_path, base_path, "resourcepacks")
	}

	pub fn shaderpacks_dir(
		&self,
		base_path: &std::path::Path,
	) -> PathBuf {
		self.resolve_dir(&self.shader_pack_path, base_path, "shaderpacks")
	}

	/// Checks if the modpack has a non-empty minecraft_version.
	pub fn is_initialized(&self) -> bool {
		!self.minecraft_version.is_empty()
	}

	/// True when *no* identifying field is set — the default-empty manifest
	/// produced by `new()`. Importers and `init` deliberately round-trip this
	/// state through disk, so validation has to let it pass.
	fn is_fully_empty(&self) -> bool {
		self.name.is_empty()
			&& self.description.is_empty()
			&& self.version.is_empty()
			&& self.minecraft_version.is_empty()
			&& self.loader.loader.is_none()
			&& self.loader.version.is_empty()
			&& self.mod_path.is_none()
			&& self.resource_pack_path.is_none()
			&& self.shader_pack_path.is_none()
	}

	/// Reject manifests that look hand-broken: identifying fields present but
	/// `minecraft_version` missing, or `loader.version` set without a loader
	/// type. Fully-empty manifests (fresh from `init` before prompts run, or
	/// freshly-imported MRPACKs awaiting their `depends` map) pass through.
	pub fn validate(&self) -> Result<(), YammmError> {
		if self.is_fully_empty() {
			return Ok(());
		}
		if self.minecraft_version.is_empty() {
			return Err(YammmError::config_error(
				"modpack.toml: minecraft_version is required when other \
				 fields are set",
			));
		}
		if !self.loader.version.is_empty() && self.loader.loader.is_none() {
			return Err(YammmError::config_error(
				"modpack.toml: loader.version is set but loader type is \
				 missing — add a `loader.loader` field (fabric, forge, quilt, \
				 or neoforge)",
			));
		}
		Ok(())
	}

	/// Extract version filters. Empty strings/None become None.
	pub fn version_filters(&self) -> VersionFilters {
		let minecraft_version = if self.minecraft_version.is_empty() {
			None
		} else {
			Some(self.minecraft_version.clone())
		};
		VersionFilters {
			minecraft_version,
			loader: self.loader.loader,
		}
	}

	/// Fill in empty fields from MRPACK `depends` map. Never overwrites existing values.
	pub fn apply_index_dependencies(
		&mut self,
		deps: &std::collections::HashMap<String, String>,
	) -> bool {
		let mut updated = false;
		if let Some(minecraft_ver) = deps.get("minecraft")
			&& self.minecraft_version.is_empty()
		{
			self.minecraft_version = minecraft_ver.clone();
			updated = true;
		}
		for (key, value) in deps {
			if key == "minecraft" {
				continue;
			}
			let loader_name =
				key.strip_suffix("-loader").unwrap_or(key.as_str());
			if let Ok(loader_type) = loader_name.parse::<LoaderType>()
				&& self.loader.version.is_empty()
			{
				self.loader.loader = Some(loader_type);
				self.loader.version = value.clone();
				updated = true;
			}
		}
		updated
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_new_modpack() {
		let config = ModpackManifest::new();
		assert_eq!(config.name, "");
		assert_eq!(config.description, "");
	}

	#[test]
	fn test_is_initialized() {
		let empty = ModpackManifest::new();
		assert!(!empty.is_initialized());

		let mut initialized = ModpackManifest::new();
		initialized.minecraft_version = "1.20.4".to_string();
		assert!(initialized.is_initialized());
	}

	#[test]
	fn test_save_load() {
		use crate::storage::ManifestStore;
		use tempfile::TempDir;

		let temp_dir = TempDir::new().unwrap();
		let config_path = temp_dir.path().join("modpack.toml");

		let mut config = ModpackManifest::new();
		config.name = "Test Modpack".to_string();
		config.description = "Test description".to_string();
		config.minecraft_version = "1.20.4".to_string();
		config.loader.loader = Some(LoaderType::Fabric);
		ManifestStore::new(&config_path).save(&config).unwrap();

		let loaded = ManifestStore::new(&config_path).load().unwrap();
		assert_eq!(loaded.name, "Test Modpack");
		assert_eq!(loaded.description, "Test description");
		assert_eq!(loaded.minecraft_version, "1.20.4");
		assert_eq!(loaded.loader.loader, Some(LoaderType::Fabric));
	}

	#[test]
	fn test_mods_dir_default() {
		use std::path::Path;
		let config = ModpackManifest::new();
		let base = Path::new("/tmp/modpack");
		assert_eq!(
			config.mods_dir(base),
			std::path::PathBuf::from("/tmp/modpack/mods")
		);
	}

	#[test]
	fn test_mods_dir_custom() {
		use std::path::Path;
		let mut config = ModpackManifest::new();
		config.mod_path = Some(std::path::PathBuf::from("custom-mods"));
		let base = Path::new("/tmp/modpack");
		assert_eq!(
			config.mods_dir(base),
			std::path::PathBuf::from("/tmp/modpack/custom-mods")
		);
	}

	#[test]
	fn test_version_filters_empty() {
		let config = ModpackManifest::new();
		let filters = config.version_filters();
		assert_eq!(filters.minecraft_version, None);
		assert_eq!(filters.loader, None);
	}

	#[test]
	fn test_version_filters_populated() {
		let mut config = ModpackManifest::new();
		config.minecraft_version = "1.20.4".to_string();
		config.loader.loader = Some(LoaderType::Fabric);
		config.loader.version = "0.16.0".to_string();
		let filters = config.version_filters();
		assert_eq!(filters.minecraft_version, Some("1.20.4".to_string()));
		assert_eq!(filters.loader, Some(LoaderType::Fabric));
	}

	#[test]
	fn test_version_filters_non_default_loader() {
		let mut config = ModpackManifest::new();
		config.minecraft_version = "1.20.4".to_string();
		config.loader.loader = Some(LoaderType::Forge);
		let filters = config.version_filters();
		assert_eq!(filters.minecraft_version, Some("1.20.4".to_string()));
		assert_eq!(filters.loader, Some(LoaderType::Forge));
	}

	#[test]
	fn test_apply_index_dependencies_minecraft() {
		let mut config = ModpackManifest::new();
		let mut deps = std::collections::HashMap::new();
		deps.insert("minecraft".to_string(), "1.21.1".to_string());
		let updated = config.apply_index_dependencies(&deps);
		assert!(updated);
		assert_eq!(config.minecraft_version, "1.21.1");
	}

	#[test]
	fn test_apply_index_dependencies_does_not_overwrite() {
		let mut config = ModpackManifest::new();
		config.minecraft_version = "1.20.4".to_string();
		let mut deps = std::collections::HashMap::new();
		deps.insert("minecraft".to_string(), "1.21.1".to_string());
		let updated = config.apply_index_dependencies(&deps);
		assert!(!updated);
		assert_eq!(config.minecraft_version, "1.20.4");
	}

	#[test]
	fn test_apply_index_dependencies_loader() {
		let mut config = ModpackManifest::new();
		let mut deps = std::collections::HashMap::new();
		deps.insert("fabric-loader".to_string(), "0.16.5".to_string());
		let updated = config.apply_index_dependencies(&deps);
		assert!(updated);
		assert_eq!(config.loader.loader, Some(LoaderType::Fabric));
		assert_eq!(config.loader.version, "0.16.5");
	}

	#[test]
	fn test_apply_index_dependencies_loader_bare_key() {
		let mut config = ModpackManifest::new();
		let mut deps = std::collections::HashMap::new();
		deps.insert("forge".to_string(), "49.0.30".to_string());
		let updated = config.apply_index_dependencies(&deps);
		assert!(updated);
		assert_eq!(config.loader.loader, Some(LoaderType::Forge));
		assert_eq!(config.loader.version, "49.0.30");
	}

	#[test]
	fn test_apply_index_dependencies_neoforge_loader_key() {
		let mut config = ModpackManifest::new();
		let mut deps = std::collections::HashMap::new();
		deps.insert("neoforge-loader".to_string(), "20.1.0".to_string());
		let updated = config.apply_index_dependencies(&deps);
		assert!(updated);
		assert_eq!(config.loader.loader, Some(LoaderType::NeoForge));
		assert_eq!(config.loader.version, "20.1.0");
	}

	#[test]
	fn test_apply_index_dependencies_empty_map() {
		let mut config = ModpackManifest::new();
		let deps = std::collections::HashMap::new();
		let updated = config.apply_index_dependencies(&deps);
		assert!(!updated);
	}

	#[test]
	fn test_resourcepacks_dir_default() {
		use std::path::Path;
		let config = ModpackManifest::new();
		let base = Path::new("/tmp/modpack");
		assert_eq!(
			config.resourcepacks_dir(base),
			std::path::PathBuf::from("/tmp/modpack/resourcepacks")
		);
	}

	#[test]
	fn test_validate_empty_manifest_passes() {
		let manifest = ModpackManifest::new();
		assert!(manifest.validate().is_ok());
	}

	#[test]
	fn test_validate_fully_populated_manifest_passes() {
		let mut manifest = ModpackManifest::new();
		manifest.name = "Test".to_string();
		manifest.minecraft_version = "1.20.4".to_string();
		manifest.loader.loader = Some(LoaderType::Fabric);
		manifest.loader.version = "0.16.5".to_string();
		assert!(manifest.validate().is_ok());
	}

	#[test]
	fn test_validate_partial_manifest_rejects_missing_minecraft_version() {
		let mut manifest = ModpackManifest::new();
		manifest.name = "Test".to_string();
		let err = manifest.validate().unwrap_err();
		assert!(err.to_string().contains("minecraft_version"));
	}

	#[test]
	fn test_validate_loader_version_without_loader_type_rejected() {
		let mut manifest = ModpackManifest::new();
		manifest.minecraft_version = "1.20.4".to_string();
		manifest.loader.version = "0.16.5".to_string();
		let err = manifest.validate().unwrap_err();
		assert!(err.to_string().contains("loader type"));
	}

	#[test]
	fn test_validate_loader_type_without_version_passes() {
		// `loader.version = ""` means "use latest stable" — legitimate.
		let mut manifest = ModpackManifest::new();
		manifest.minecraft_version = "1.20.4".to_string();
		manifest.loader.loader = Some(LoaderType::Fabric);
		assert!(manifest.validate().is_ok());
	}

	#[test]
	fn test_shaderpacks_dir_default() {
		use std::path::Path;
		let config = ModpackManifest::new();
		let base = Path::new("/tmp/modpack");
		assert_eq!(
			config.shaderpacks_dir(base),
			std::path::PathBuf::from("/tmp/modpack/shaderpacks")
		);
	}
}
