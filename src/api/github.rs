//! GitHub API client for resolving mod releases from GitHub repositories.

use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::api::error::ApiError;
use crate::types::{HashType, ModInfo, ModSource, ModVersion};
use crate::utils::slugify;

const GITHUB_API_URL: &str = "https://api.github.com";

crate::api::define_api_client!(GitHubClient, GITHUB_API_URL);

impl GitHubClient {
	pub fn from_shared_client(client: reqwest::Client) -> Self {
		Self {
			client,
			base_url: GITHUB_API_URL.to_string(),
		}
	}

	/// Fetches all releases for a GitHub repository.
	pub async fn get_releases(
		&self,
		owner: &str,
		repo: &str,
	) -> Result<Vec<GitHubRelease>, ApiError> {
		let url =
			format!("{}/repos/{}/{}/releases", self.base_url, owner, repo);
		self.fetch_json(&url, Vec::new()).await
	}

	/// Fetches a single release by its tag name.
	pub async fn get_release_by_tag(
		&self,
		owner: &str,
		repo: &str,
		tag: &str,
	) -> Result<GitHubRelease, ApiError> {
		let url = format!(
			"{}/repos/{}/{}/releases/tags/{}",
			self.base_url, owner, repo, tag
		);
		self.fetch_json(&url, Vec::new()).await
	}

	/// Converts a GitHub release into a generic `ModInfo`.
	pub fn to_mod_info(
		release: GitHubRelease,
		owner_repo: &str,
	) -> ModInfo {
		let repo_name = owner_repo.split('/').nth(1).unwrap_or(owner_repo);
		let github_url = format!("https://github.com/{}", owner_repo);
		ModInfo {
			id: slugify(repo_name),
			name: release
				.name
				.clone()
				.unwrap_or_else(|| repo_name.to_string()),
			description: release.body.clone().unwrap_or_default(),
			source: ModSource::url(&github_url),
			minecraft_versions: vec![release.tag_name.clone()],
			loaders: vec![],
			downloads: 0,
			url: github_url,
			project_type: None,
			client_side: None,
			server_side: None,
		}
	}

	/// Converts a GitHub release into a generic `ModVersion`, selecting the
	/// primary JAR asset from the release.
	pub fn to_mod_version(
		release: GitHubRelease,
		owner_repo: &str,
	) -> Option<ModVersion> {
		let jar_asset = find_primary_jar(&release.assets, owner_repo)?;

		Some(ModVersion {
			version_id: None,
			version: release.tag_name,
			minecraft_versions: vec![],
			loaders: vec![],
			download_url: jar_asset.browser_download_url.clone(),
			hash: None,
			hash_type: HashType::default(),
			file_size: jar_asset.size.max(0) as u64,
			release_date: release.published_at.unwrap_or_default(),
		})
	}
}

fn find_primary_jar<'a>(
	assets: &'a [GitHubAsset],
	owner_repo: &str,
) -> Option<&'a GitHubAsset> {
	let jar_assets: Vec<&GitHubAsset> = assets
		.iter()
		.filter(|a| a.name.to_lowercase().ends_with(".jar"))
		.collect();

	if jar_assets.is_empty() {
		return None;
	}

	let repo_name = owner_repo.split('/').nth(1).unwrap_or("");

	if !repo_name.is_empty() {
		if let Some(exact) = jar_assets.iter().find(|a| {
			a.name.to_lowercase() == format!("{}.jar", repo_name.to_lowercase())
		}) {
			return Some(exact);
		}

		if let Some(contains) = jar_assets
			.iter()
			.find(|a| a.name.to_lowercase().contains("primary"))
		{
			return Some(contains);
		}
	}

	jar_assets.into_iter().next()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRelease {
	pub id: i64,
	pub tag_name: String,
	pub name: Option<String>,
	pub body: Option<String>,
	pub draft: bool,
	pub prerelease: bool,
	pub published_at: Option<String>,
	pub assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAsset {
	pub id: i64,
	pub name: String,
	pub size: i64,
	pub browser_download_url: String,
	pub content_type: Option<String>,
	pub updated_at: Option<String>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_github_client_new() {
		let client = GitHubClient::new();
		assert_eq!(client.base_url, GITHUB_API_URL);
	}

	#[test]
	fn test_github_client_with_base_url() {
		let client = GitHubClient::new().with_base_url("http://localhost:3000");
		assert_eq!(client.base_url, "http://localhost:3000");
	}

	#[test]
	fn test_github_to_mod_info() {
		let release = GitHubRelease {
			id: 1,
			tag_name: "v1.0.0".to_string(),
			name: Some("Release 1.0.0".to_string()),
			body: Some("A cool mod".to_string()),
			draft: false,
			prerelease: false,
			published_at: Some("2024-01-01T00:00:00Z".to_string()),
			assets: vec![],
		};
		let info = GitHubClient::to_mod_info(release, "FabricMC/fabric");
		assert_eq!(info.id, "fabric");
		assert_eq!(info.name, "Release 1.0.0");
		assert_eq!(info.description, "A cool mod");
		assert_eq!(info.url, "https://github.com/FabricMC/fabric");
	}

	#[test]
	fn test_to_mod_info_fallback_name() {
		let release = GitHubRelease {
			id: 1,
			tag_name: "v1.0.0".to_string(),
			name: None,
			body: None,
			draft: false,
			prerelease: false,
			published_at: None,
			assets: vec![],
		};
		let info = GitHubClient::to_mod_info(release, "FabricMC/fabric");
		assert_eq!(info.name, "fabric");
		assert_eq!(info.description, "");
	}

	#[test]
	fn test_to_mod_version_with_jar() {
		let release = GitHubRelease {
			id: 1,
			tag_name: "v1.0.0".to_string(),
			name: Some("Release 1.0.0".to_string()),
			body: None,
			draft: false,
			prerelease: false,
			published_at: Some("2024-01-01T00:00:00Z".to_string()),
			assets: vec![GitHubAsset {
				id: 1,
				name: "fabric.jar".to_string(),
				size: 1024,
				browser_download_url: "https://example.com/fabric.jar"
					.to_string(),
				content_type: None,
				updated_at: None,
			}],
		};
		let version =
			GitHubClient::to_mod_version(release, "FabricMC/fabric").unwrap();
		assert_eq!(version.version, "v1.0.0");
		assert_eq!(version.download_url, "https://example.com/fabric.jar");
		assert_eq!(version.file_size, 1024);
		assert!(version.hash.is_none());
	}

	#[test]
	fn test_to_mod_version_no_jar() {
		let release = GitHubRelease {
			id: 1,
			tag_name: "v1.0.0".to_string(),
			name: None,
			body: None,
			draft: false,
			prerelease: false,
			published_at: None,
			assets: vec![GitHubAsset {
				id: 1,
				name: "source.zip".to_string(),
				size: 512,
				browser_download_url: "https://example.com/source.zip"
					.to_string(),
				content_type: None,
				updated_at: None,
			}],
		};
		assert!(
			GitHubClient::to_mod_version(release, "FabricMC/fabric").is_none()
		);
	}

	#[test]
	fn test_find_primary_jar_exact_name() {
		let assets = vec![
			GitHubAsset {
				id: 1,
				name: "fabric.jar".to_string(),
				size: 1024,
				browser_download_url: "https://example.com/fabric.jar"
					.to_string(),
				content_type: None,
				updated_at: None,
			},
			GitHubAsset {
				id: 2,
				name: "fabric-sources.jar".to_string(),
				size: 512,
				browser_download_url: "https://example.com/fabric-sources.jar"
					.to_string(),
				content_type: None,
				updated_at: None,
			},
		];
		let result = find_primary_jar(&assets, "FabricMC/fabric").unwrap();
		assert_eq!(result.name, "fabric.jar");
	}

	#[test]
	fn test_find_primary_jar_primary_keyword() {
		let assets = vec![
			GitHubAsset {
				id: 1,
				name: "mod-v1.0.jar".to_string(),
				size: 1024,
				browser_download_url: "https://example.com/mod-v1.0.jar"
					.to_string(),
				content_type: None,
				updated_at: None,
			},
			GitHubAsset {
				id: 2,
				name: "mod-primary.jar".to_string(),
				size: 2048,
				browser_download_url: "https://example.com/mod-primary.jar"
					.to_string(),
				content_type: None,
				updated_at: None,
			},
		];
		let result = find_primary_jar(&assets, "SomeOrg/mymod").unwrap();
		assert_eq!(result.name, "mod-primary.jar");
	}

	#[test]
	fn test_find_primary_jar_first_jar() {
		let assets = vec![
			GitHubAsset {
				id: 1,
				name: "mod-v1.0.jar".to_string(),
				size: 1024,
				browser_download_url: "https://example.com/mod-v1.0.jar"
					.to_string(),
				content_type: None,
				updated_at: None,
			},
			GitHubAsset {
				id: 2,
				name: "mod-v1.0-sources.jar".to_string(),
				size: 512,
				browser_download_url:
					"https://example.com/mod-v1.0-sources.jar".to_string(),
				content_type: None,
				updated_at: None,
			},
		];
		let result = find_primary_jar(&assets, "SomeOrg/mymod").unwrap();
		assert_eq!(result.name, "mod-v1.0.jar");
	}

	#[test]
	fn test_find_primary_jar_no_jars() {
		let assets = vec![GitHubAsset {
			id: 1,
			name: "source.zip".to_string(),
			size: 512,
			browser_download_url: "https://example.com/source.zip".to_string(),
			content_type: None,
			updated_at: None,
		}];
		assert!(find_primary_jar(&assets, "FabricMC/fabric").is_none());
	}
}
