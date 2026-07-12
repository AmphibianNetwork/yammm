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

use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::{
	ModEnv, ModInfo, ModVersion, SourceDependency, VersionFilters,
};

const SOURCE: &str = "mock";

#[derive(Clone, Default, Debug)]
pub struct MockSource {
	mods: Arc<Mutex<HashMap<String, ModInfo>>>,
	versions: Arc<Mutex<HashMap<String, Vec<ModVersion>>>>,
	deps: Arc<Mutex<HashMap<String, Vec<SourceDependency>>>>,
	search_results: Arc<Mutex<Vec<ModInfo>>>,
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

	/// Seed search results returned for every query (the query string is
	/// ignored — tests assert on filter/limit/offset behaviour, not match
	/// scoring).
	pub fn set_search_results(
		&self,
		results: Vec<ModInfo>,
	) {
		*self.search_results.lock().unwrap() = results;
	}
}

impl ModSourceProvider for MockSource {
	fn name(&self) -> &str {
		"mock"
	}

	fn supports_search(&self) -> bool {
		!self.search_results.lock().unwrap().is_empty()
	}

	fn get_mod_env(
		&self,
		_mod_info: &ModInfo,
	) -> ModEnv {
		ModEnv::Both
	}

	async fn search(
		&self,
		_query: &str,
		filters: &SearchFilters,
	) -> ProviderResult<Vec<ModInfo>> {
		let all = self.search_results.lock().unwrap().clone();
		let offset = filters.offset.unwrap_or(0);
		let limit = filters.limit.unwrap_or(usize::MAX);
		Ok(all.into_iter().skip(offset).take(limit).collect())
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		self.mods
			.lock()
			.unwrap()
			.get(mod_id)
			.cloned()
			.ok_or_else(|| ProviderError::NotFound {
				provider: SOURCE,
				what: mod_id.to_string(),
			})
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		_filters: &VersionFilters,
	) -> ProviderResult<Vec<ModVersion>> {
		self.versions
			.lock()
			.unwrap()
			.get(mod_id)
			.cloned()
			.ok_or_else(|| ProviderError::NotFound {
				provider: SOURCE,
				what: mod_id.to_string(),
			})
	}

	async fn get_dependencies(
		&self,
		_mod_id: &str,
		version_id: &str,
	) -> ProviderResult<Vec<SourceDependency>> {
		Ok(self
			.deps
			.lock()
			.unwrap()
			.get(version_id)
			.cloned()
			.unwrap_or_default())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::providers::provider::Provider;
	use crate::types::{ModSource, ProjectType};

	fn mk_info(id: &str) -> ModInfo {
		ModInfo {
			id: id.to_string(),
			name: id.to_string(),
			description: String::new(),
			source: ModSource::modrinth(id),
			minecraft_versions: vec!["1.20.4".to_string()],
			loaders: vec!["fabric".to_string()],
			downloads: 0,
			url: String::new(),
			project_type: Some(ProjectType::Mod),
			client_side: None,
			server_side: None,
		}
	}

	#[tokio::test]
	async fn search_returns_seeded_results_through_provider() {
		let mock = MockSource::new();
		mock.set_search_results(vec![mk_info("a"), mk_info("b")]);
		let provider = Provider::Mock(mock);

		let filters = SearchFilters::default();
		let results = provider.search("anything", &filters).await.unwrap();
		assert_eq!(results.len(), 2);
		assert_eq!(results[0].id, "a");
		assert_eq!(results[1].id, "b");
	}

	#[tokio::test]
	async fn search_applies_limit_filter() {
		let mock = MockSource::new();
		mock.set_search_results(vec![mk_info("a"), mk_info("b"), mk_info("c")]);
		let provider = Provider::Mock(mock);

		let filters = SearchFilters {
			limit: Some(2),
			..Default::default()
		};
		let results = provider.search("q", &filters).await.unwrap();
		assert_eq!(results.len(), 2);
		assert_eq!(results[0].id, "a");
		assert_eq!(results[1].id, "b");
	}

	#[tokio::test]
	async fn search_applies_offset_filter() {
		let mock = MockSource::new();
		mock.set_search_results(vec![mk_info("a"), mk_info("b"), mk_info("c")]);
		let provider = Provider::Mock(mock);

		let filters = SearchFilters::default().with_offset(Some(1));
		let results = provider.search("q", &filters).await.unwrap();
		assert_eq!(results.len(), 2);
		assert_eq!(results[0].id, "b");
		assert_eq!(results[1].id, "c");
	}

	#[tokio::test]
	async fn search_combines_offset_and_limit_for_paging() {
		let mock = MockSource::new();
		mock.set_search_results(
			(0..10).map(|n| mk_info(&format!("m{n}"))).collect(),
		);
		let provider = Provider::Mock(mock);

		// Page 2 with page size 3: skip 3, take 3 → m3, m4, m5
		let filters = SearchFilters {
			limit: Some(3),
			offset: Some(3),
			..Default::default()
		};
		let results = provider.search("q", &filters).await.unwrap();
		assert_eq!(
			results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
			vec!["m3", "m4", "m5"]
		);
	}

	#[tokio::test]
	async fn search_offset_past_end_returns_empty() {
		let mock = MockSource::new();
		mock.set_search_results(vec![mk_info("a"), mk_info("b")]);
		let provider = Provider::Mock(mock);

		let filters = SearchFilters::default().with_offset(Some(100));
		let results = provider.search("q", &filters).await.unwrap();
		assert!(results.is_empty());
	}

	#[tokio::test]
	async fn get_dependencies_returns_empty_for_unknown_version() {
		let mock = MockSource::new();
		let provider = Provider::Mock(mock);
		let deps = provider.get_dependencies("any", "unknown").await.unwrap();
		assert!(deps.is_empty());
	}
}
