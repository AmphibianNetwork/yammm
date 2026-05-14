//! Modrinth API client
//!
//! This module provides a client for interacting with the Modrinth API.
//! See: <https://docs.modrinth.com/docs/api/>

use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::api::error::ApiError;
use crate::types::{HashType, ModInfo, ModSource, ModVersion};

const MODRINTH_API_URL: &str = "https://api.modrinth.com/v2";

crate::api::define_api_client!(ModrinthClient, MODRINTH_API_URL);

impl ModrinthClient {
	/// Search for mods using the v2/search endpoint
	pub async fn search(
		&self,
		query: &str,
		limit: Option<usize>,
	) -> Result<Vec<ModrinthSearchHit>, ApiError> {
		let url = format!("{}/search", self.base_url);
		let mut request = self.client.get(&url).query(&[("query", query)]);

		if let Some(lim) = limit {
			request = request.query(&[("limit", lim.to_string())]);
		}

		let full_url = request.build()?.url().to_string();

		let response = self.send_retried(&full_url, Vec::new()).await?;

		let result: ModrinthSearchResult = response.json().await?;
		Ok(result.hits)
	}

	/// Resolve a mod slug or ID to its actual project ID
	/// This handles both slugs (like "emi") and actual IDs (like "fRiHVvU7")
	pub async fn resolve_mod_id(
		&self,
		mod_identifier: &str,
	) -> Result<String, ApiError> {
		let mod_data = self.get_mod(mod_identifier).await?;
		Ok(mod_data.project_id)
	}

	/// Get mod details by searching for it
	/// The Modrinth v2/mod endpoint has inconsistent behavior, so we use search
	pub async fn get_mod(
		&self,
		mod_identifier: &str,
	) -> Result<ModrinthSearchHit, ApiError> {
		match self.get_project_direct(mod_identifier).await {
			Ok(hit) => return Ok(hit),
			Err(ApiError::Http { status: 404, .. }) => {}
			Err(e) => return Err(e),
		}

		let hits = self.search(mod_identifier, Some(10)).await?;

		for hit in &hits {
			if hit.slug == mod_identifier || hit.project_id == mod_identifier {
				return Ok(hit.clone());
			}
		}

		for hit in &hits {
			if hit
				.slug
				.to_lowercase()
				.contains(&mod_identifier.to_lowercase())
				|| hit
					.title
					.to_lowercase()
					.contains(&mod_identifier.to_lowercase())
			{
				return Ok(hit.clone());
			}
		}

		Err(ApiError::not_found(format!(
			"Mod '{}' not found",
			mod_identifier
		)))
	}

	pub async fn get_project_direct(
		&self,
		mod_identifier: &str,
	) -> Result<ModrinthSearchHit, ApiError> {
		let url = format!("{}/project/{}", self.base_url, mod_identifier);
		let response = self.send_retried(&url, Vec::new()).await?;
		let response = Self::ensure_success(response)?;
		let project: ModrinthProject = response.json().await?;
		Ok(project.into_search_hit())
	}

	/// Get versions for a mod with optional server-side filtering
	/// Uses `loaders[]` and `game_versions[]` query params for server-side filtering
	pub async fn get_versions(
		&self,
		mod_identifier: &str,
		minecraft_version: Option<&str>,
		loader: Option<&str>,
	) -> Result<Vec<ModrinthVersion>, ApiError> {
		let base =
			format!("{}/project/{}/version", self.base_url, mod_identifier);
		let url = self.build_versions_url(&base, minecraft_version, loader)?;
		self.fetch_json(&url, Vec::new()).await
	}

	/// Get a specific version by ID
	pub async fn get_version(
		&self,
		version_id: &str,
	) -> Result<ModrinthVersion, ApiError> {
		let url = format!("{}/version/{}", self.base_url, version_id);
		self.fetch_json(&url, Vec::new()).await
	}

	/// Get the latest version for a mod with server-side filtering
	pub async fn get_latest_version(
		&self,
		mod_identifier: &str,
		minecraft_version: Option<&str>,
		loader: Option<&str>,
	) -> Result<ModrinthVersion, ApiError> {
		let mod_hit = self.get_mod(mod_identifier).await?;
		let project_id = mod_hit.project_id.clone();

		let base = format!("{}/project/{}/version", self.base_url, project_id);
		let url = self.build_versions_url(&base, minecraft_version, loader)?;

		let versions: Vec<ModrinthVersion> =
			self.fetch_json(&url, Vec::new()).await?;

		versions.into_iter().next().ok_or_else(|| {
			ApiError::not_found(format!(
				"No versions found for mod '{}' with filters (minecraft: {:?}, loader: {:?})",
				mod_identifier, minecraft_version, loader
			))
		})
	}

	fn build_versions_url(
		&self,
		base: &str,
		minecraft_version: Option<&str>,
		loader: Option<&str>,
	) -> Result<String, ApiError> {
		if minecraft_version.is_none() && loader.is_none() {
			return Ok(base.to_string());
		}
		let mut url = reqwest::Url::parse(base)
			.map_err(|e| ApiError::url_error(e.to_string()))?;
		{
			let mut pairs = url.query_pairs_mut();
			if let Some(mc_ver) = minecraft_version {
				pairs
					.append_pair("game_versions", &format!("[\"{}\"]", mc_ver));
			}
			if let Some(ldr) = loader {
				pairs.append_pair(
					"loaders",
					&format!("[\"{}\"]", ldr.to_lowercase()),
				);
			}
		}
		Ok(url.to_string())
	}

	/// Get dependencies for a version
	pub async fn get_dependencies(
		&self,
		version_id: &str,
	) -> Result<Vec<ModrinthDependency>, ApiError> {
		let version = self.get_version(version_id).await?;
		Ok(version.dependencies.clone())
	}

	/// Look up a version by file hash
	/// Uses the /v2/version_file/{hash} endpoint with optional algorithm param
	pub async fn get_version_by_hash(
		&self,
		hash: &str,
		algorithm: &str,
	) -> Result<ModrinthVersion, ApiError> {
		let url = format!(
			"{}/version_file/{}?algorithm={}",
			self.base_url, hash, algorithm
		);
		self.fetch_json(&url, Vec::new()).await
	}

	/// Convert a search hit to our internal ModInfo
	pub fn to_mod_info_from_hit(hit: ModrinthSearchHit) -> ModInfo {
		let slug = if hit.slug.is_empty() {
			hit.project_id.clone()
		} else {
			hit.slug.clone()
		};
		let loaders = hit
			.categories
			.iter()
			.filter(|c| {
				matches!(c.as_str(), "fabric" | "forge" | "neoforge" | "quilt")
			})
			.cloned()
			.collect();
		ModInfo {
			id: slug.clone(),
			name: hit.title,
			description: hit.description,
			source: ModSource::modrinth(&slug),
			minecraft_versions: hit
				.versions
				.into_iter()
				.filter(|v| !v.is_empty())
				.collect(),
			loaders,
			downloads: hit.downloads.max(0) as u64,
			url: format!("https://modrinth.com/mod/{}", slug),
			project_type: hit.project_type.parse().ok(),
			client_side: hit.client_side,
			server_side: hit.server_side,
		}
	}

	/// Convert to our internal ModVersion
	pub fn to_mod_version(version_data: ModrinthVersion) -> ModVersion {
		let hash = version_data
			.files
			.iter()
			.find_map(|f| f.hashes.as_ref().and_then(|h| h.sha512.clone()));

		let hash_type = if hash.is_some() {
			HashType::Sha512
		} else {
			HashType::default()
		};

		let primary_file = version_data.files.iter().find(|f| f.primary);
		let download_url =
			primary_file.map(|f| f.url.clone()).unwrap_or_default();

		let file_size = primary_file.map(|f| f.size.max(0) as u64).unwrap_or(0);

		ModVersion {
			version_id: Some(version_data.id),
			version: version_data.version_number,
			minecraft_versions: version_data.game_versions,
			loaders: version_data.loaders,
			download_url,
			hash,
			hash_type,
			file_size,
			release_date: version_data.date_published,
		}
	}
}

/// Version data from Modrinth API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthVersion {
	pub id: String,
	#[serde(rename = "project_id")]
	pub mod_id: String,
	pub author_id: Option<String>,
	pub featured: bool,
	pub name: String,
	pub version_number: String,
	pub changelog: Option<String>,
	pub changelog_url: Option<String>,
	pub date_published: String,
	pub downloads: i64,
	pub version_type: String,
	pub loaders: Vec<String>,
	pub game_versions: Vec<String>,
	pub dependencies: Vec<ModrinthDependency>,
	pub files: Vec<ModrinthFile>,
}

/// File information from Modrinth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthFile {
	#[serde(default)]
	pub id: String,
	pub filename: String,
	pub primary: bool,
	pub url: String,
	pub size: i64,
	pub hashes: Option<ModrinthHashes>,
	pub file_type: Option<String>,
}

/// Hashes for a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthHashes {
	pub sha512: Option<String>,
	pub sha256: Option<String>,
	pub sha1: Option<String>,
}

/// Dependency from Modrinth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthDependency {
	#[serde(rename = "project_id")]
	pub mod_id: String,
	pub version_id: Option<String>,
	pub file_name: Option<String>,
	pub dependency_type: String,
}

impl ModrinthDependency {
	/// Check if this is a required dependency
	pub fn is_required(&self) -> bool {
		matches!(self.dependency_type.as_str(), "required")
	}
}

/// Project data from the Modrinth /project/{id} endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthProject {
	pub id: String,
	pub slug: String,
	pub project_type: String,
	pub title: String,
	pub description: String,
	pub categories: Vec<String>,
	pub versions: Vec<String>,
	pub downloads: i64,
	pub followers: i64,
	pub icon_url: Option<String>,
	pub published: String,
	pub updated: String,
	pub license: ModrinthLicense,
	pub client_side: Option<String>,
	pub server_side: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthLicense {
	pub id: String,
}

impl ModrinthProject {
	fn into_search_hit(self) -> ModrinthSearchHit {
		ModrinthSearchHit {
			project_id: self.id,
			project_type: self.project_type,
			slug: self.slug,
			author: String::new(),
			title: self.title,
			description: self.description,
			categories: self.categories,
			display_categories: Vec::new(),
			versions: self.versions,
			downloads: self.downloads,
			follows: self.followers,
			icon_url: self.icon_url,
			date_created: self.published,
			date_modified: self.updated,
			latest_version: None,
			license: self.license.id,
			client_side: self.client_side,
			server_side: self.server_side,
		}
	}
}

/// Search result wrapper from Modrinth API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthSearchResult {
	pub hits: Vec<ModrinthSearchHit>,
	pub offset: usize,
	pub limit: usize,
	pub total_hits: usize,
}

/// Search hit from Modrinth API (simplified mod representation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthSearchHit {
	pub project_id: String,
	pub project_type: String,
	pub slug: String,
	pub author: String,
	pub title: String,
	pub description: String,
	pub categories: Vec<String>,
	pub display_categories: Vec<String>,
	pub versions: Vec<String>,
	pub downloads: i64,
	pub follows: i64,
	pub icon_url: Option<String>,
	pub date_created: String,
	pub date_modified: String,
	pub latest_version: Option<String>,
	pub license: String,
	pub client_side: Option<String>,
	pub server_side: Option<String>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_modrinth_client_new() {
		let client = ModrinthClient::new();
		assert_eq!(client.base_url, MODRINTH_API_URL);
	}

	#[test]
	fn test_modrinth_client_with_base_url() {
		let client =
			ModrinthClient::new().with_base_url("http://localhost:3000");
		assert_eq!(client.base_url, "http://localhost:3000");
	}

	#[test]
	fn test_dependency_is_required() {
		let required = ModrinthDependency {
			mod_id: "test".to_string(),
			version_id: None,
			file_name: None,
			dependency_type: "required".to_string(),
		};
		assert!(required.is_required());

		let optional = ModrinthDependency {
			mod_id: "test".to_string(),
			version_id: None,
			file_name: None,
			dependency_type: "optional".to_string(),
		};
		assert!(!optional.is_required());
	}

	#[test]
	fn test_to_mod_info_from_hit() {
		let hit = ModrinthSearchHit {
			project_id: "abc123".to_string(),
			project_type: "mod".to_string(),
			slug: "jei".to_string(),
			author: String::new(),
			title: "Just Enough Items".to_string(),
			description: "View items".to_string(),
			categories: vec!["fabric".to_string()],
			display_categories: vec![],
			versions: vec!["1.0.0".to_string()],
			downloads: 1000,
			follows: 50,
			icon_url: None,
			date_created: String::new(),
			date_modified: String::new(),
			latest_version: None,
			license: String::new(),
			client_side: None,
			server_side: None,
		};
		let info = ModrinthClient::to_mod_info_from_hit(hit);
		assert_eq!(info.id, "jei");
		assert_eq!(info.name, "Just Enough Items");
		assert!(info.loaders.contains(&"fabric".to_string()));
	}

	#[test]
	fn test_modrinth_to_mod_version() {
		let version = ModrinthVersion {
			id: "ver1".to_string(),
			mod_id: "mod1".to_string(),
			author_id: None,
			featured: false,
			name: "v1".to_string(),
			version_number: "1.0.0".to_string(),
			changelog: None,
			changelog_url: None,
			date_published: "2024-01-01".to_string(),
			downloads: 0,
			version_type: "release".to_string(),
			loaders: vec!["fabric".to_string()],
			game_versions: vec!["1.20.4".to_string()],
			dependencies: vec![],
			files: vec![ModrinthFile {
				id: String::new(),
				filename: "mod.jar".to_string(),
				primary: true,
				url: "https://cdn.example.com/mod.jar".to_string(),
				size: 1024,
				hashes: Some(ModrinthHashes {
					sha512: Some("a".repeat(128)),
					sha256: None,
					sha1: None,
				}),
				file_type: None,
			}],
		};
		let mv = ModrinthClient::to_mod_version(version);
		assert_eq!(mv.version, "1.0.0");
		assert_eq!(mv.download_url, "https://cdn.example.com/mod.jar");
		assert!(mv.hash.is_some());
	}

	#[tokio::test]
	async fn test_search_returns_hits() {
		let mut server = mockito::Server::new_async().await;
		let body = serde_json::json!({
			"hits": [{
				"project_id": "u6dRKQwU",
				"project_type": "mod",
				"slug": "jei",
				"author": "mezz",
				"title": "Just Enough Items",
				"description": "View items and recipes",
				"categories": ["fabric"],
				"display_categories": [],
				"versions": ["1.20.4"],
				"downloads": 1000000,
				"follows": 50000,
				"icon_url": null,
				"date_created": "2020-01-01T00:00:00Z",
				"date_modified": "2024-01-01T00:00:00Z",
				"latest_version": null,
				"license": "MIT",
				"client_side": "required",
				"server_side": "optional"
			}],
			"offset": 0,
			"limit": 10,
			"total_hits": 1
		});
		let _mock = server
			.mock("GET", "/v2/search")
			.match_query(mockito::Matcher::Any)
			.with_status(200)
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.create_async()
			.await;

		let client =
			ModrinthClient::new().with_base_url(format!("{}/v2", server.url()));
		let hits = client.search("jei", None).await.unwrap();
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].slug, "jei");
		assert_eq!(hits[0].title, "Just Enough Items");
	}

	#[tokio::test]
	async fn test_get_mod_not_found() {
		let mut server = mockito::Server::new_async().await;
		let base = format!("{}/v2", server.url());

		let _mock1 = server
			.mock("GET", "/v2/project/unknown")
			.with_status(404)
			.create_async()
			.await;

		let empty_search = serde_json::json!({
			"hits": [],
			"offset": 0,
			"limit": 10,
			"total_hits": 0
		});
		let _mock2 = server
			.mock("GET", "/v2/search")
			.match_query(mockito::Matcher::Any)
			.with_status(200)
			.with_header("content-type", "application/json")
			.with_body(empty_search.to_string())
			.create_async()
			.await;

		let client = ModrinthClient::new().with_base_url(&base);
		let result = client.get_mod("unknown").await;
		match result {
			Err(ApiError::NotFound(msg)) => {
				assert!(msg.contains("unknown"));
			}
			Err(e) => panic!("Expected NotFound, got: {:?}", e),
			Ok(_) => panic!("Expected error"),
		}
	}

	#[tokio::test]
	async fn test_get_versions() {
		let mut server = mockito::Server::new_async().await;
		let body = serde_json::json!([{
			"id": "ver1",
			"project_id": "jei",
			"author_id": null,
			"featured": false,
			"name": "JEI 1.0.0",
			"version_number": "1.0.0",
			"changelog": null,
			"changelog_url": null,
			"date_published": "2024-01-01T00:00:00Z",
			"downloads": 1000,
			"version_type": "release",
			"loaders": ["fabric"],
			"game_versions": ["1.20.4"],
			"dependencies": [],
			"files": []
		}]);
		let _mock = server
			.mock("GET", "/v2/project/jei/version")
			.with_status(200)
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.create_async()
			.await;

		let client =
			ModrinthClient::new().with_base_url(format!("{}/v2", server.url()));
		let versions = client.get_versions("jei", None, None).await.unwrap();
		assert_eq!(versions.len(), 1);
		assert_eq!(versions[0].id, "ver1");
		assert_eq!(versions[0].version_number, "1.0.0");
	}
}
