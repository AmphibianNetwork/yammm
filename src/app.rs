//! Application state and context management.
//!
//! `AppContext` is the single entry point for every command, built via the
//! builder pattern so CLI flags can be applied before heavyweight init.
//! When no `--config` path is given, walks up from cwd looking for
//! `modpack.toml` (like `git` discovers `.git`).

use crate::config::{GlobalConfig, ModpackManifest};
use crate::providers::SourceRegistry;
use crate::storage::{JarCache, Storage};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Application state for a loaded modpack.
#[derive(Debug)]
pub struct App {
	pub root_dir: PathBuf,
	pub config: ModpackManifest,
	pub storage: Storage,
	pub cache: JarCache,
}

impl App {
	pub fn load(
		root_dir: PathBuf,
		cache: JarCache,
	) -> Result<Self> {
		let config_path = root_dir.join("modpack.toml");
		tracing::debug!("Loading modpack from: {}", config_path.display());
		let config = crate::storage::ManifestStore::new(&config_path).load()?;
		tracing::debug!("Loaded modpack: {}", config.name);
		Ok(Self::from_parts(root_dir, config, cache))
	}

	pub fn create(
		root_dir: PathBuf,
		cache: JarCache,
	) -> Self {
		tracing::debug!("Creating new modpack at: {}", root_dir.display());
		let config = ModpackManifest::new();
		Self::from_parts(root_dir, config, cache)
	}

	pub fn from_parts(
		root_dir: PathBuf,
		config: ModpackManifest,
		cache: JarCache,
	) -> Self {
		let storage = Storage::new(&root_dir, &config);
		Self {
			root_dir,
			config,
			storage,
			cache,
		}
	}
}

/// Global application context shared across all CLI commands.
pub struct AppContext {
	pub global: GlobalConfig,
	pub modpack: Option<App>,
	pub cwd: PathBuf,
	pub registry: Arc<SourceRegistry>,
	pub insecure: bool,
	pub http_client: reqwest::Client,
	cache_dir: PathBuf,
	jar_cache: JarCache,
}

impl std::fmt::Debug for AppContext {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		f.debug_struct("AppContext")
			.field("global", &self.global)
			.field("modpack", &self.modpack)
			.field("cwd", &self.cwd)
			.field("insecure", &self.insecure)
			.field("cache_dir", &self.cache_dir)
			.finish_non_exhaustive()
	}
}

/// Builder for constructing `AppContext` with optional parameters.
#[must_use = "call .build() to construct the AppContext"]
pub struct AppContextBuilder {
	config_path: Option<PathBuf>,
	insecure: bool,
}

impl AppContextBuilder {
	pub fn config_path(
		mut self,
		path: impl Into<Option<PathBuf>>,
	) -> Self {
		self.config_path = path.into();
		self
	}

	pub fn insecure(
		mut self,
		insecure: bool,
	) -> Self {
		self.insecure = insecure;
		self
	}

	pub fn build(self) -> Result<AppContext> {
		AppContext::build(self.config_path.as_deref(), self.insecure)
	}
}

impl AppContext {
	pub fn builder() -> AppContextBuilder {
		AppContextBuilder {
			config_path: None,
			insecure: false,
		}
	}

	fn build(
		config_path: Option<&Path>,
		insecure: bool,
	) -> Result<Self> {
		// Load global config first — API keys, cache dir, output prefs.
		// Falls back to defaults if the file doesn't exist yet.
		let global = GlobalConfig::load()?;
		tracing::debug!(
			"Loaded global config from: {:?}",
			GlobalConfig::config_path()
		);

		let cwd = std::env::current_dir()
			.context("Failed to get current directory")?;
		tracing::debug!("Current working directory: {}", cwd.display());

		// Build the HTTP client once — it connection-pools internally,
		// so sharing a single clone across all providers is efficient.
		let http_client = Self::build_http_client(insecure);
		let cache_dir = resolve_cache_dir(&global);
		let jar_cache = Self::build_cache(&cache_dir);

		// Try to discover a modpack by walking up from cwd.
		// This is `None` if we're not inside a modpack directory.
		let modpack = Self::resolve_modpack(&cwd, config_path, &jar_cache)?;

		// The registry owns one Provider per source (Modrinth, CurseForge, URL).
		// Each provider gets its own clone of the HTTP client.
		let registry =
			Arc::new(SourceRegistry::from_config(&global, http_client.clone()));

		Ok(Self {
			global,
			modpack,
			cwd,
			registry,
			insecure,
			http_client,
			cache_dir,
			jar_cache,
		})
	}

	fn build_http_client(insecure: bool) -> reqwest::Client {
		let mut builder = reqwest::Client::builder()
			.user_agent(format!(
				"AmphibianNetwork/yammm/{} (contact@amphibian.network)",
				env!("CARGO_PKG_VERSION")
			))
			.connect_timeout(std::time::Duration::from_secs(10))
			.timeout(std::time::Duration::from_secs(30));
		if insecure {
			builder = builder.danger_accept_invalid_certs(true);
		}
		builder.build().unwrap_or_else(|e| {
			tracing::warn!("Failed to build HTTP client, using default: {}", e);
			reqwest::Client::new()
		})
	}

	fn build_cache(cache_dir: &Path) -> JarCache {
		let jar_cache = JarCache::new(cache_dir.join("jars"));
		if let Err(e) = jar_cache.init() {
			tracing::warn!("Failed to init jar cache: {}", e);
		}
		jar_cache
	}

	fn resolve_modpack(
		cwd: &Path,
		config_path: Option<&Path>,
		jar_cache: &JarCache,
	) -> Result<Option<App>> {
		let candidate = match find_modpack_dir(cwd, config_path) {
			Some(dir) => dir,
			None => {
				if let Some(path) = config_path {
					return Err(crate::errors::YammmError::invalid_args(
						format!(
							"Specified config path does not exist: {}",
							path.display()
						),
					)
					.into());
				}
				cwd.to_path_buf()
			}
		};
		let modpack_path = candidate.join("modpack.toml");

		if !modpack_path.exists() {
			return Ok(None);
		}

		Ok(Some(App::load(candidate, jar_cache.clone())?))
	}

	/// Get the global cache directory path
	pub fn cache_dir(&self) -> &Path {
		&self.cache_dir
	}

	/// Get the shared JAR cache instance.
	pub fn jar_cache(&self) -> &JarCache {
		&self.jar_cache
	}

	/// Check if we're currently in a modpack directory.
	pub fn in_modpack(&self) -> bool {
		self.modpack.is_some()
	}

	/// Get the modpack reference; errors if no modpack.toml is found.
	pub fn require_modpack(&self) -> Result<&App> {
		self.modpack.as_ref().ok_or_else(|| {
			crate::errors::YammmError::invalid_args(
				"No modpack.toml found in current directory",
			)
			.into()
		})
	}
}

/// Resolve the cache directory: `GlobalConfig.cache_dir` > `YAMMM_CACHE_DIR` env > default.
fn resolve_cache_dir(global: &GlobalConfig) -> PathBuf {
	global
		.cache_dir
		.clone()
		.or_else(|| std::env::var("YAMMM_CACHE_DIR").ok().map(PathBuf::from))
		.unwrap_or_else(GlobalConfig::default_cache_dir)
}

/// Walk up from `cwd` looking for `modpack.toml`.
/// Resolution: `--config` path > `YAMMM_MODPACK` env > walk upward from cwd.
fn find_modpack_dir(
	cwd: &Path,
	config_path: Option<&Path>,
) -> Option<PathBuf> {
	if let Some(p) = config_path {
		let candidate = if p.is_dir() {
			p.to_path_buf()
		} else if p.exists() {
			p.parent().unwrap_or(cwd).to_path_buf()
		} else {
			return None;
		};
		return Some(candidate);
	}

	if let Ok(dir) = std::env::var("YAMMM_MODPACK") {
		return Some(PathBuf::from(dir));
	}

	let mut dir = cwd;
	loop {
		if dir.join("modpack.toml").exists() {
			return Some(dir.to_path_buf());
		}
		dir = dir.parent()?;
	}
}
