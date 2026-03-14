//! Mod loader types (Fabric, Forge, Quilt, NeoForge).
//!
//! Used to filter versions when querying providers and to determine
//! which installer to use when launching Minecraft.
//!
//! `is_fabric_like()` groups Fabric and Quilt together because they
//! share the same mod loading infrastructure (Fabric Loader API).

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Error for invalid loader names
#[derive(Debug, thiserror::Error)]
#[error("Unknown loader: {0}")]
pub struct LoaderError(pub String);

/// Supported mod loaders
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LoaderType {
	/// Default loader type; Fabric is the most widely used mod loader
	#[default]
	Fabric,
	Forge,
	#[serde(alias = "neoforge")]
	NeoForge,
	Quilt,
}

impl FromStr for LoaderType {
	type Err = LoaderError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"fabric" => Ok(LoaderType::Fabric),
			"forge" => Ok(LoaderType::Forge),
			"neoforge" => Ok(LoaderType::NeoForge),
			"quilt" => Ok(LoaderType::Quilt),
			_ => Err(LoaderError(s.to_string())),
		}
	}
}

impl LoaderType {
	pub fn as_str(&self) -> &'static str {
		match self {
			LoaderType::Fabric => "fabric",
			LoaderType::Forge => "forge",
			LoaderType::NeoForge => "neoforge",
			LoaderType::Quilt => "quilt",
		}
	}

	pub fn display_name(&self) -> &'static str {
		match self {
			LoaderType::Fabric => "Fabric",
			LoaderType::Forge => "Forge",
			LoaderType::NeoForge => "NeoForge",
			LoaderType::Quilt => "Quilt",
		}
	}

	/// Returns all supported loaders
	pub fn all() -> &'static [LoaderType] {
		&[
			LoaderType::Fabric,
			LoaderType::Forge,
			LoaderType::NeoForge,
			LoaderType::Quilt,
		]
	}

	pub fn is_fabric_like(&self) -> bool {
		matches!(self, LoaderType::Fabric | LoaderType::Quilt)
	}
}

impl std::fmt::Display for LoaderType {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_loader_type_from_str() {
		assert_eq!("fabric".parse::<LoaderType>().unwrap(), LoaderType::Fabric);
		assert_eq!("FORGE".parse::<LoaderType>().unwrap(), LoaderType::Forge);
		assert_eq!("Quilt".parse::<LoaderType>().unwrap(), LoaderType::Quilt);
		assert!("unknown".parse::<LoaderType>().is_err());
	}

	#[test]
	fn test_loader_type_display() {
		assert_eq!(LoaderType::Fabric.to_string(), "fabric");
		assert_eq!(LoaderType::Forge.to_string(), "forge");
		assert_eq!(LoaderType::NeoForge.to_string(), "neoforge");
		assert_eq!(LoaderType::Quilt.to_string(), "quilt");
	}

	#[test]
	fn test_default() {
		assert_eq!(LoaderType::default(), LoaderType::Fabric);
	}

	#[test]
	fn test_all() {
		let all = LoaderType::all();
		assert_eq!(all.len(), 4);
		assert!(all.contains(&LoaderType::Fabric));
		assert!(all.contains(&LoaderType::Forge));
		assert!(all.contains(&LoaderType::NeoForge));
		assert!(all.contains(&LoaderType::Quilt));
	}
}
