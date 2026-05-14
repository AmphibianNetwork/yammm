//! `SourceRegistry` — maps `ModSource` discriminants to `Provider`.
//!
//! Built once in `AppContext::build()` and shared via `Arc`.
//! `SourceKey` exists separately from `ModSource` because `ModSource`
//! carries source-specific data and can't be used as a `HashMap` key.

use std::collections::HashMap;

use crate::config::GlobalConfig;
use crate::providers::curseforge::CurseForgeSource;
use crate::providers::modrinth::ModrinthSource;
use crate::providers::provider::Provider;
use crate::providers::url::UrlSource;
use crate::types::ModSource;
use anyhow::Result;

/// Stable key derived from `ModSource` variant (without inner data).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceKey {
	Modrinth,
	CurseForge,
	Url,
}

impl From<&ModSource> for SourceKey {
	fn from(s: &ModSource) -> Self {
		match s {
			ModSource::Modrinth { .. } => SourceKey::Modrinth,
			ModSource::CurseForge { .. } => SourceKey::CurseForge,
			ModSource::Url { .. } => SourceKey::Url,
		}
	}
}

/// Maps `SourceKey` → `Provider`. Constructed once and shared via `Arc`.
#[derive(Debug)]
pub struct SourceRegistry {
	providers: HashMap<SourceKey, Provider>,
}

impl SourceRegistry {
	/// Build the registry from global config.
	pub fn from_config(
		config: &GlobalConfig,
		http_client: reqwest::Client,
	) -> Self {
		let mut providers: HashMap<SourceKey, Provider> = HashMap::new();

		providers.insert(
			SourceKey::Modrinth,
			Provider::Modrinth(ModrinthSource::new(http_client.clone())),
		);

		let cf_key = config
			.api_keys
			.curseforge
			.clone()
			.or_else(|| std::env::var("CURSEFORGE_API_TOKEN").ok());
		if cf_key.is_some() {
			providers.insert(
				SourceKey::CurseForge,
				Provider::CurseForge(CurseForgeSource::new(
					cf_key,
					http_client.clone(),
				)),
			);
		} else {
			tracing::debug!(
				"No CurseForge API key configured — CurseForge source disabled"
			);
		}

		providers.insert(
			SourceKey::Url,
			Provider::Url(UrlSource::with_http_client(http_client.clone())),
		);

		Self { providers }
	}

	/// Get the provider for a given `ModSource`.
	pub fn get(
		&self,
		source: &ModSource,
	) -> Result<&Provider> {
		let key = SourceKey::from(source);
		match self.providers.get(&key) {
			Some(p) => Ok(p),
			None => Err(crate::errors::YammmError::invalid_args(format!(
				"Unknown mod source: {:?}",
				key
			))
			.into()),
		}
	}

	/// Get the provider by key directly.
	pub fn get_by_key(
		&self,
		key: &SourceKey,
	) -> Option<&Provider> {
		self.providers.get(key)
	}

	#[cfg(test)]
	pub fn new_with_mock(mock: crate::providers::mock::MockSource) -> Self {
		let mut providers: HashMap<SourceKey, Provider> = HashMap::new();
		providers.insert(SourceKey::Modrinth, Provider::Mock(mock));
		Self { providers }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_source_key_from_mod_source() {
		assert_eq!(
			SourceKey::from(&ModSource::modrinth("jei")),
			SourceKey::Modrinth
		);
		assert_eq!(
			SourceKey::from(&ModSource::curseforge("123")),
			SourceKey::CurseForge
		);
		assert_eq!(
			SourceKey::from(&ModSource::url("https://example.com")),
			SourceKey::Url
		);
	}

	#[test]
	fn test_registry_get_found() {
		let mock = crate::providers::mock::MockSource::new();
		let registry = SourceRegistry::new_with_mock(mock);
		let provider = registry.get(&ModSource::modrinth("test"));
		assert!(provider.is_ok());
	}

	#[test]
	fn test_registry_get_missing_key() {
		let mock = crate::providers::mock::MockSource::new();
		let registry = SourceRegistry::new_with_mock(mock);
		let result = registry.get(&ModSource::curseforge("test"));
		assert!(result.is_err());
	}

	#[test]
	fn test_registry_get_by_key() {
		let mock = crate::providers::mock::MockSource::new();
		let registry = SourceRegistry::new_with_mock(mock);
		assert!(registry.get_by_key(&SourceKey::Modrinth).is_some());
		assert!(registry.get_by_key(&SourceKey::CurseForge).is_none());
	}
}
