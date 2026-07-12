//! The `Provider` enum — dispatches mod source operations to concrete types.
//!
//! Uses manual enum dispatch instead of `dyn ModSourceProvider` to avoid
//! boxing async futures. The `dispatch!` macro eliminates repetitive
//! match boilerplate; exhaustiveness errors remind you to update it
//! when adding variants.

use crate::providers::curseforge::CurseForgeSource;
use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::modrinth::ModrinthSource;
use crate::providers::url::UrlSource;
use crate::types::{
	ModEnv, ModInfo, ModVersion, SourceDependency, VersionFilters,
};

/// Filters used when searching for mods.
///
/// `limit` caps the number of hits returned in a single request; `offset`
/// skips that many hits from the start of the result set. Both are 0-based
/// and provider-bounded — Modrinth caps `limit` at 100, CurseForge at 50.
#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
	pub version: VersionFilters,
	pub limit: Option<usize>,
	pub offset: Option<usize>,
}

impl SearchFilters {
	pub fn new(
		version: VersionFilters,
		limit: Option<usize>,
	) -> Self {
		Self {
			version,
			limit,
			offset: None,
		}
	}

	pub fn with_offset(
		mut self,
		offset: Option<usize>,
	) -> Self {
		self.offset = offset;
		self
	}
}

/// Trait that all mod source providers must implement.
///
/// `async fn` in trait + closed-set enum dispatch (see `Provider` below)
/// — no `Box<dyn>`, so we don't pay for boxed futures. The
/// `#[allow(async_fn_in_trait)]` is intentional and load-bearing.
#[allow(async_fn_in_trait)]
pub trait ModSourceProvider {
	fn name(&self) -> &str;
	fn supports_search(&self) -> bool;
	fn get_mod_env(
		&self,
		mod_info: &ModInfo,
	) -> ModEnv;
	async fn search(
		&self,
		query: &str,
		filters: &SearchFilters,
	) -> ProviderResult<Vec<ModInfo>>;
	async fn get_mod(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo>;
	async fn get_versions(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> ProviderResult<Vec<ModVersion>>;
	async fn get_dependencies(
		&self,
		mod_id: &str,
		version_id: &str,
	) -> ProviderResult<Vec<SourceDependency>>;
}

/// Closed-set mod source provider with manual dispatch.
#[derive(Clone, Debug)]
pub enum Provider {
	Modrinth(ModrinthSource),
	CurseForge(CurseForgeSource),
	Url(UrlSource),
	#[cfg(test)]
	Mock(crate::providers::mock::MockSource),
}

/// Dispatch a method call on the inner source type.
///
/// Two variants: `dispatch!(self, method(args))` and
/// `dispatch!(self, method(args).await)` for async methods.
macro_rules! dispatch {
    ($self:expr, $method:ident($($arg:expr),*)) => {
        match $self {
            Self::Modrinth(s) => s.$method($($arg),*),
            Self::CurseForge(s) => s.$method($($arg),*),
            Self::Url(s) => s.$method($($arg),*),
            #[cfg(test)]
            Self::Mock(s) => s.$method($($arg),*),
        }
    };
    ($self:expr, $method:ident($($arg:expr),*).await) => {
        match $self {
            Self::Modrinth(s) => s.$method($($arg),*).await,
            Self::CurseForge(s) => s.$method($($arg),*).await,
            Self::Url(s) => s.$method($($arg),*).await,
            #[cfg(test)]
            Self::Mock(s) => s.$method($($arg),*).await,
        }
    };
}

impl Provider {
	pub fn name(&self) -> &str {
		dispatch!(self, name())
	}

	pub fn supports_search(&self) -> bool {
		dispatch!(self, supports_search())
	}

	pub fn get_mod_env(
		&self,
		mod_info: &ModInfo,
	) -> ModEnv {
		dispatch!(self, get_mod_env(mod_info))
	}

	pub async fn search(
		&self,
		query: &str,
		filters: &SearchFilters,
	) -> ProviderResult<Vec<ModInfo>> {
		dispatch!(self, search(query, filters).await)
	}

	pub async fn get_mod(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		dispatch!(self, get_mod(mod_id).await)
	}

	pub async fn get_versions(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> ProviderResult<Vec<ModVersion>> {
		dispatch!(self, get_versions(mod_id, filters).await)
	}

	pub async fn get_dependencies(
		&self,
		mod_id: &str,
		version_id: &str,
	) -> ProviderResult<Vec<SourceDependency>> {
		dispatch!(self, get_dependencies(mod_id, version_id).await)
	}

	/// Pick the latest version by comparing release dates (ISO 8601 is lexicographically sortable).
	pub async fn get_latest_version(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> ProviderResult<ModVersion> {
		let versions = self.get_versions(mod_id, filters).await?;
		versions
			.into_iter()
			.max_by(|a, b| a.release_date.cmp(&b.release_date))
			.ok_or_else(|| ProviderError::NotFound {
				provider: self.name_static(),
				what: format!("no versions found for {}", mod_id),
			})
	}

	/// Returns the provider name as a `'static` string for use in error
	/// payloads where we'd otherwise have to clone.
	fn name_static(&self) -> &'static str {
		match self {
			Self::Modrinth(_) => "modrinth",
			Self::CurseForge(_) => "curseforge",
			Self::Url(_) => "url",
			#[cfg(test)]
			Self::Mock(_) => "mock",
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::providers::mock::MockSource;
	use crate::types::HashType;
	fn make_version(version: &str) -> ModVersion {
		make_version_with_date(version, "2024-01-01")
	}

	fn make_version_with_date(
		version: &str,
		date: &str,
	) -> ModVersion {
		ModVersion {
			version_id: Some(format!("vid-{}", version)),
			version: version.to_string(),
			minecraft_versions: vec!["1.20.4".to_string()],
			loaders: vec!["fabric".to_string()],
			download_url: format!("https://example.com/{}.jar", version),
			hash: None,
			hash_type: HashType::Sha512,
			file_size: 1000,
			release_date: date.to_string(),
		}
	}

	#[tokio::test]
	async fn test_get_latest_version_picks_newest() {
		let mock = MockSource::new();
		mock.add_versions(
			"test-mod",
			vec![
				make_version_with_date("1.0.0", "2024-01-01"),
				make_version_with_date("2.0.0", "2024-06-15"),
				make_version_with_date("1.5.0", "2024-03-10"),
			],
		);
		let provider = Provider::Mock(mock);
		let filters = VersionFilters {
			minecraft_version: None,
			loader: None,
		};
		let latest = provider
			.get_latest_version("test-mod", &filters)
			.await
			.unwrap();
		assert_eq!(latest.version, "2.0.0");
	}

	#[tokio::test]
	async fn test_get_latest_version_single() {
		let mock = MockSource::new();
		mock.add_versions("test-mod", vec![make_version("1.0.0")]);
		let provider = Provider::Mock(mock);
		let filters = VersionFilters {
			minecraft_version: None,
			loader: None,
		};
		let latest = provider
			.get_latest_version("test-mod", &filters)
			.await
			.unwrap();
		assert_eq!(latest.version, "1.0.0");
	}

	#[tokio::test]
	async fn test_get_latest_version_empty_errors() {
		let mock = MockSource::new();
		mock.add_versions("test-mod", vec![]);
		let provider = Provider::Mock(mock);
		let filters = VersionFilters {
			minecraft_version: None,
			loader: None,
		};
		let result = provider.get_latest_version("test-mod", &filters).await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_get_latest_version_by_date_not_semver() {
		let mock = MockSource::new();
		mock.add_versions(
			"test-mod",
			vec![
				make_version_with_date("1.20.4", "2024-03-01"),
				make_version_with_date("1.20.10", "2024-06-01"),
				make_version_with_date("1.9.0", "2024-09-01"),
			],
		);
		let provider = Provider::Mock(mock);
		let filters = VersionFilters {
			minecraft_version: None,
			loader: None,
		};
		let latest = provider
			.get_latest_version("test-mod", &filters)
			.await
			.unwrap();
		assert_eq!(latest.version, "1.9.0");
	}

	#[test]
	fn test_provider_name() {
		let mock = MockSource::new();
		let provider = Provider::Mock(mock);
		assert_eq!(provider.name(), "mock");
	}

	#[test]
	fn test_provider_supports_search() {
		let mock = MockSource::new();
		let provider = Provider::Mock(mock);
		assert!(!provider.supports_search());
	}
}
