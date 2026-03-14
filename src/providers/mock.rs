//! Mock source provider for testing.
//!
//! Returns canned `ModInfo`, `ModVersion`, and `SourceDependency` data
//! that is configured before each test. Uses `Arc<Mutex<HashMap>>` so
//! data can be inserted from the test thread and read from async tasks.
//!
//! Usage: create a `MockSource`, call `add_mod`/`add_versions`/`add_deps`,
//! then wrap it in `Provider::Mock(mock)` or `SourceRegistry::new_with_mock(mock)`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::{ModInfo, ModVersion, SourceDependency, VersionFilters};
use anyhow::Result;

#[derive(Clone, Default, Debug)]
pub struct MockSource {
	mods: Arc<Mutex<HashMap<String, ModInfo>>>,
	versions: Arc<Mutex<HashMap<String, Vec<ModVersion>>>>,
	deps: Arc<Mutex<HashMap<String, Vec<SourceDependency>>>>,
}

impl MockSource {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn add_mod(
		&self,
		mod_id: impl Into<String>,
		info: ModInfo,
	) {
		self.mods.lock().unwrap().insert(mod_id.into(), info);
	}

	pub fn add_versions(
		&self,
		mod_id: impl Into<String>,
		versions: Vec<ModVersion>,
	) {
		self.versions
			.lock()
			.unwrap()
			.insert(mod_id.into(), versions);
	}

	pub fn add_deps(
		&self,
		version_id: impl Into<String>,
		deps: Vec<SourceDependency>,
	) {
		self.deps.lock().unwrap().insert(version_id.into(), deps);
	}
}

impl ModSourceProvider for MockSource {
	fn name(&self) -> &str {
		"mock"
	}

	fn supports_search(&self) -> bool {
		false
	}

	async fn search(
		&self,
		_query: &str,
		_filters: &SearchFilters,
	) -> Result<Vec<ModInfo>> {
		Ok(vec![])
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> Result<ModInfo> {
		self.mods
			.lock()
			.unwrap()
			.get(mod_id)
			.cloned()
			.ok_or_else(|| {
				crate::errors::YammmError::mod_not_found(mod_id).into()
			})
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		_filters: &VersionFilters,
	) -> Result<Vec<ModVersion>> {
		self.versions
			.lock()
			.unwrap()
			.get(mod_id)
			.cloned()
			.ok_or_else(|| {
				crate::errors::YammmError::mod_not_found(mod_id).into()
			})
	}

	async fn get_dependencies(
		&self,
		_mod_id: &str,
		version_id: &str,
	) -> Result<Vec<SourceDependency>> {
		Ok(self
			.deps
			.lock()
			.unwrap()
			.get(version_id)
			.cloned()
			.unwrap_or_default())
	}
}
