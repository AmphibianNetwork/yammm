//! URL source implementation — handles three URL schemes.
//!
//! | Scheme | Behavior |
//! |--------|----------|
//! | `https://github.com/{owner}/{repo}` | Resolves GitHub releases via `GitHubClient` |
//! | `file:///path/to/mod.jar` | Reads a local JAR, computes hash for caching |
//! | Other HTTP(S) URLs | HEAD request for metadata, direct download URL |
//!
//! URL sources never have dependencies (no way to query them), so
//! `get_dependencies` always returns an empty vec.

use std::path::Path;
use std::sync::Arc;

use crate::api::GitHubClient;
use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::{
	HashType, ModEnv, ModInfo, ModSource, ModVersion, SourceDependency,
	VersionFilters,
};
use crate::utils::{slugify, system_time_to_date, today_iso8601};

const SOURCE: &str = "url";

/// Classifies a URL identifier into its handling strategy.
///
/// This determines which code path handles the URL:
/// - GitHub URLs are resolved via the GitHub Releases API
/// - `file://` URLs read from the local filesystem
/// - Everything else is treated as a direct download link
enum UrlKind<'a> {
	GitHub { owner: &'a str, repo: &'a str },
	File,
	Http,
}

fn classify_url(id: &str) -> UrlKind<'_> {
	if let Some((owner, repo)) = parse_github_owner_repo(id) {
		UrlKind::GitHub { owner, repo }
	} else if is_file_url(id) {
		UrlKind::File
	} else {
		UrlKind::Http
	}
}

#[derive(Clone, Debug)]
pub struct UrlSource {
	http_client: reqwest::Client,
	github_client: Arc<GitHubClient>,
}

impl UrlSource {
	pub fn with_http_client(client: reqwest::Client) -> Self {
		let github_client =
			Arc::new(GitHubClient::from_shared_client(client.clone()));
		Self {
			http_client: client,
			github_client,
		}
	}
}

impl ModSourceProvider for UrlSource {
	fn name(&self) -> &str {
		"url"
	}

	fn supports_search(&self) -> bool {
		false
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
		_filters: &SearchFilters,
	) -> ProviderResult<Vec<ModInfo>> {
		Ok(Vec::new())
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		match classify_url(mod_id) {
			UrlKind::GitHub { owner, repo } => {
				self.get_mod_github(owner, repo, mod_id).await
			}
			UrlKind::File => self.get_mod_file(mod_id),
			UrlKind::Http => self.get_mod_url(mod_id),
		}
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		_filters: &VersionFilters,
	) -> ProviderResult<Vec<ModVersion>> {
		match classify_url(mod_id) {
			UrlKind::GitHub { owner, repo } => {
				self.get_versions_github(owner, repo).await
			}
			UrlKind::File => self.get_versions_file(mod_id),
			UrlKind::Http => self.get_versions_url(mod_id).await,
		}
	}

	async fn get_dependencies(
		&self,
		_mod_id: &str,
		_version_id: &str,
	) -> ProviderResult<Vec<SourceDependency>> {
		Ok(vec![])
	}
}

impl UrlSource {
	async fn get_mod_github(
		&self,
		owner: &str,
		repo: &str,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		let releases =
			self.github_client.get_releases(owner, repo).await.map_err(
				|e| ProviderError::NotFound {
					provider: SOURCE,
					what: format!("GitHub repo {}/{}: {}", owner, repo, e),
				},
			)?;

		let latest = releases.into_iter().next().ok_or_else(|| {
			ProviderError::NotFound {
				provider: SOURCE,
				what: format!("no releases for {}", mod_id),
			}
		})?;

		Ok(GitHubClient::to_mod_info(
			latest,
			&format!("{}/{}", owner, repo),
		))
	}

	fn get_mod_file(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		let path_str = file_path_from_url(mod_id).unwrap_or(mod_id);
		let path = Path::new(path_str);
		let stem = path
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or(path_str);

		Ok(mod_info_from_name(stem, mod_id))
	}

	fn get_mod_url(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		let filename = extract_filename(mod_id);
		let name = strip_extension(filename);

		Ok(mod_info_from_name(name, mod_id))
	}

	async fn get_versions_github(
		&self,
		owner: &str,
		repo: &str,
	) -> ProviderResult<Vec<ModVersion>> {
		let owner_repo = format!("{}/{}", owner, repo);
		let releases =
			self.github_client.get_releases(owner, repo).await.map_err(
				|e| ProviderError::NotFound {
					provider: SOURCE,
					what: format!("GitHub repo {}: {}", owner_repo, e),
				},
			)?;

		Ok(releases
			.into_iter()
			.filter_map(|r| GitHubClient::to_mod_version(r, &owner_repo))
			.collect())
	}

	fn get_versions_file(
		&self,
		mod_id: &str,
	) -> ProviderResult<Vec<ModVersion>> {
		let path_str = file_path_from_url(mod_id).unwrap_or(mod_id);
		let path = Path::new(path_str);
		let metadata = std::fs::metadata(path).ok();
		let file_size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
		let release_date = metadata
			.as_ref()
			.and_then(|m| m.modified().ok())
			.map(system_time_to_date)
			.unwrap_or_else(|| {
				system_time_to_date(std::time::SystemTime::now())
			});
		let hash_type = HashType::Sha512;
		let hash = hash_type.compute_for_file(path).ok();

		Ok(vec![ModVersion {
			version_id: None,
			version: "unknown".to_string(),
			minecraft_versions: vec![],
			loaders: vec![],
			download_url: mod_id.to_string(),
			hash,
			hash_type,
			file_size,
			release_date,
		}])
	}

	async fn get_versions_url(
		&self,
		mod_id: &str,
	) -> ProviderResult<Vec<ModVersion>> {
		let file_size = match self.http_client.head(mod_id).send().await {
			Ok(response) if response.status().is_success() => response
				.headers()
				.get("content-length")
				.and_then(|v| v.to_str().ok())
				.and_then(|v| v.parse::<u64>().ok())
				.unwrap_or(0),
			_ => {
				tracing::debug!("Could not HEAD URL for metadata: {}", mod_id);
				0u64
			}
		};

		Ok(vec![ModVersion {
			version_id: None,
			version: "unknown".to_string(),
			minecraft_versions: vec![],
			loaders: vec![],
			download_url: mod_id.to_string(),
			hash: None,
			hash_type: HashType::Sha512,
			file_size,
			release_date: today_iso8601(),
		}])
	}
}

fn extract_filename(url: &str) -> &str {
	let path = url.split('?').next().unwrap_or(url);
	let path = path.split('#').next().unwrap_or(path);
	path.rsplit('/').next().unwrap_or(path)
}

fn strip_extension(filename: &str) -> &str {
	filename
		.rsplit_once('.')
		.map(|(name, _)| name)
		.unwrap_or(filename)
}

fn parse_github_owner_repo(url: &str) -> Option<(&str, &str)> {
	let after_scheme = url
		.strip_prefix("https://github.com/")
		.or_else(|| url.strip_prefix("http://github.com/"))?;
	let after_scheme = after_scheme.strip_suffix('/').unwrap_or(after_scheme);
	let owner = after_scheme.split('/').next()?;
	let rest = after_scheme.strip_prefix(owner)?.strip_prefix('/')?;
	let repo = rest.split('/').next()?;
	if owner.is_empty() || repo.is_empty() {
		None
	} else {
		Some((owner, repo))
	}
}

fn is_file_url(url: &str) -> bool {
	url.starts_with("file://")
}

fn file_path_from_url(url: &str) -> Option<&str> {
	url.strip_prefix("file://")
}

fn mod_info_from_name(
	name: &str,
	mod_id: &str,
) -> ModInfo {
	let id = slugify(name);
	ModInfo {
		id,
		name: name.to_string(),
		description: String::new(),
		source: ModSource::url(mod_id),
		minecraft_versions: vec![],
		loaders: vec![],
		downloads: 0,
		url: mod_id.to_string(),
		project_type: None,
		client_side: None,
		server_side: None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_github_owner_repo() {
		let (owner, repo) =
			parse_github_owner_repo("https://github.com/FabricMC/fabric")
				.unwrap();
		assert_eq!(owner, "FabricMC");
		assert_eq!(repo, "fabric");

		let (owner, repo) =
			parse_github_owner_repo("https://github.com/IrisShaders/Iris")
				.unwrap();
		assert_eq!(owner, "IrisShaders");
		assert_eq!(repo, "Iris");
	}

	#[test]
	fn test_parse_github_owner_repo_trailing_slash() {
		let (owner, repo) =
			parse_github_owner_repo("https://github.com/FabricMC/fabric/")
				.unwrap();
		assert_eq!(owner, "FabricMC");
		assert_eq!(repo, "fabric");
	}

	#[test]
	fn test_parse_github_owner_repo_extra_path() {
		let (owner, repo) = parse_github_owner_repo(
			"https://github.com/FabricMC/fabric/releases",
		)
		.unwrap();
		assert_eq!(owner, "FabricMC");
		assert_eq!(repo, "fabric");
	}

	#[test]
	fn test_parse_github_owner_repo_not_github() {
		assert!(
			parse_github_owner_repo("https://example.com/mod.jar").is_none()
		)
	}

	#[test]
	fn test_parse_github_owner_repo_incomplete() {
		assert!(
			parse_github_owner_repo("https://github.com/FabricMC").is_none()
		)
	}

	#[test]
	fn test_is_file_url() {
		assert!(is_file_url("file:///home/user/mod.jar"));
		assert!(is_file_url("file://C:/mods/mod.jar"));
		assert!(!is_file_url("https://example.com/mod.jar"));
		assert!(!is_file_url("http://example.com/mod.jar"));
	}

	#[test]
	fn test_file_path_from_url() {
		assert_eq!(
			file_path_from_url("file:///home/user/mod.jar"),
			Some("/home/user/mod.jar")
		);
	}

	#[test]
	fn test_extract_filename() {
		assert_eq!(extract_filename("https://example.com/mod.jar"), "mod.jar");
		assert_eq!(
			extract_filename("https://example.com/mod.jar?v=1"),
			"mod.jar"
		);
	}

	#[test]
	fn test_strip_extension() {
		assert_eq!(strip_extension("mod.jar"), "mod");
		assert_eq!(strip_extension("mod"), "mod");
	}

	#[test]
	fn test_url_source_with_http_client() {
		let source = UrlSource::with_http_client(reqwest::Client::new());
		assert_eq!(source.name(), "url");
		assert!(!source.supports_search());
	}

	#[test]
	fn test_classify_url_github() {
		match classify_url("https://github.com/user/repo") {
			UrlKind::GitHub { owner, repo } => {
				assert_eq!(owner, "user");
				assert_eq!(repo, "repo");
			}
			_ => panic!("Expected GitHub"),
		}
	}

	#[test]
	fn test_classify_url_file() {
		assert!(matches!(
			classify_url("file:///path/to/mod.jar"),
			UrlKind::File
		));
	}

	#[test]
	fn test_classify_url_http() {
		assert!(matches!(
			classify_url("https://example.com/mod.jar"),
			UrlKind::Http
		));
	}
}
