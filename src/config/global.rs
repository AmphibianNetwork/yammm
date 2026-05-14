//! Global user configuration (`~/.config/yammm/config.toml`).
//! File is created with mode 0o600 on Unix to protect API keys.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global user configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
	#[serde(default)]
	pub api_keys: ApiKeys,

	#[serde(default)]
	pub output: OutputConfig,

	#[serde(
		default,
		deserialize_with = "deserialize_optional_path",
		skip_serializing_if = "Option::is_none"
	)]
	pub cache_dir: Option<PathBuf>,

	#[serde(default)]
	pub cache_max_size_mb: Option<u64>,

	#[serde(default)]
	pub max_concurrent_downloads: Option<usize>,

	#[serde(default)]
	pub default_modpack_dir: Option<PathBuf>,
}

/// Treats empty strings as `None` in TOML deserialization.
fn deserialize_optional_path<'de, D>(
	deserializer: D
) -> std::result::Result<Option<PathBuf>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let opt: Option<PathBuf> = Option::deserialize(deserializer)?;
	Ok(opt.filter(|p| !p.as_os_str().is_empty()))
}

/// API keys for external services.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiKeys {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub curseforge: Option<String>,
}

#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
	#[default]
	Table,
	Compact,
	Json,
}

impl OutputFormat {
	pub fn as_str(self) -> &'static str {
		match self {
			OutputFormat::Table => "table",
			OutputFormat::Compact => "compact",
			OutputFormat::Json => "json",
		}
	}
}

impl std::fmt::Display for OutputFormat {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

impl std::str::FromStr for OutputFormat {
	type Err = OutputFormatParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"table" => Ok(OutputFormat::Table),
			"compact" => Ok(OutputFormat::Compact),
			"json" => Ok(OutputFormat::Json),
			other => Err(OutputFormatParseError {
				input: other.to_string(),
			}),
		}
	}
}

#[derive(Debug, thiserror::Error)]
#[error("Unknown output format: '{input}'. Valid: table, compact, json")]
pub struct OutputFormatParseError {
	pub input: String,
}

const fn default_true() -> bool {
	true
}

/// Output formatting preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
	#[serde(default)]
	pub format: OutputFormat,

	#[serde(default = "default_true")]
	pub color: bool,
}

impl Default for OutputConfig {
	fn default() -> Self {
		Self {
			format: OutputFormat::Table,
			color: true,
		}
	}
}

impl GlobalConfig {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn config_path() -> Option<PathBuf> {
		dirs::config_dir().map(|dir| dir.join("yammm").join("config.toml"))
	}

	pub fn default_cache_dir() -> PathBuf {
		dirs::cache_dir()
			.map(|dir| dir.join("yammm"))
			.unwrap_or_else(|| PathBuf::from("./.yammm-cache"))
	}

	pub fn max_concurrent_downloads(&self) -> usize {
		self.max_concurrent_downloads.unwrap_or(8)
	}

	pub fn cache_max_size_mb(&self) -> u64 {
		self.cache_max_size_mb.unwrap_or(5000)
	}

	/// Loads global config, returning defaults if the file doesn't exist.
	pub fn load() -> Result<Self> {
		let path =
			Self::config_path().with_context(|| "Config path not found")?;

		if !path.exists() {
			return Ok(Self::new());
		}

		let contents = std::fs::read_to_string(&path).with_context(|| {
			format!("Cannot read config: {}", path.display())
		})?;

		toml::from_str(&contents).context("Failed to parse global config")
	}

	/// Saves global config. Creates parent directories if needed.
	pub fn save(&self) -> Result<()> {
		let path =
			Self::config_path().with_context(|| "Config path not found")?;

		let contents =
			toml::to_string_pretty(self).context("Serialization error")?;

		crate::utils::write_secret_file(&path, &contents)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;
	use tempfile::TempDir;

	#[test]
	fn test_new_config() {
		let config = GlobalConfig::new();
		assert_eq!(config.output.format, OutputFormat::Table);
		assert!(config.output.color);
	}

	#[test]
	fn test_default_cache_dir() {
		let cache_dir = GlobalConfig::default_cache_dir();
		assert!(!cache_dir.as_os_str().is_empty());
	}

	#[test]
	fn test_roundtrip() {
		let temp_dir = TempDir::new().unwrap();
		let config_path = temp_dir.path().join("config.toml");

		let mut config = GlobalConfig::new();
		config.output.format = OutputFormat::Json;
		fs::write(&config_path, toml::to_string_pretty(&config).unwrap())
			.unwrap();

		let contents = fs::read_to_string(&config_path).unwrap();
		let loaded: GlobalConfig = toml::from_str(&contents).unwrap();
		assert_eq!(loaded.output.format, OutputFormat::Json);
	}

	#[test]
	fn test_default_values() {
		let config = GlobalConfig::new();
		assert!(config.api_keys.curseforge.is_none());
		assert!(config.cache_dir.is_none());
		assert!(config.default_modpack_dir.is_none());
	}

	#[test]
	fn test_api_keys_roundtrip() {
		let temp_dir = TempDir::new().unwrap();
		let config_path = temp_dir.path().join("config.toml");

		let mut config = GlobalConfig::new();
		config.api_keys.curseforge = Some("test-key".to_string());
		fs::write(&config_path, toml::to_string_pretty(&config).unwrap())
			.unwrap();

		let contents = fs::read_to_string(&config_path).unwrap();
		let loaded: GlobalConfig = toml::from_str(&contents).unwrap();
		assert_eq!(loaded.api_keys.curseforge, Some("test-key".to_string()));
	}

	#[test]
	fn test_config_path() {
		if let Some(path) = GlobalConfig::config_path() {
			assert!(path.to_string_lossy().contains("yammm"));
		}
	}
}
