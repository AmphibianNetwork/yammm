//! CurseForge API client
//!
//! This module provides a client for interacting with the CurseForge API.
//! Note: CurseForge API is now part of the Overwolf ecosystem.
//! See: <https://support.curseforge.com/en/support/solutions/articles/9000080163-curseforge-api-documentation->

use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::api::error::ApiError;
use crate::types::{
	DependencyKind, HashType, ModInfo, ModSource, ModVersion, ProjectType,
};

const CURSEFORGE_API_URL: &str = "https://api.curseforge.com";

const CF_MINECRAFT_GAME_ID: &str = "432";
const CF_MOD_CLASS_ID: &str = "6";
const CF_RESOURCEPACK_CLASS_ID: i64 = 12;
const CF_SHADER_CLASS_ID: i64 = 6552;

crate::api::define_api_client!(CurseForgeClient, CURSEFORGE_API_URL, api_key);

impl CurseForgeClient {
	pub async fn search(
		&self,
		query: &str,
		minecraft_version: Option<&str>,
		loader: Option<&str>,
	) -> Result<Vec<CfProject>, ApiError> {
		let url = format!("{}/v1/mods/search", self.base_url);
		let mut req = self.client.get(&url);
		req = req.query(&[
			("gameId", CF_MINECRAFT_GAME_ID),
			("searchFilter", query),
			("classId", CF_MOD_CLASS_ID),
		]);
		if let Some(mc_ver) = minecraft_version {
			req = req.query(&[("gameVersion", mc_ver)]);
		}
		if let Some(ldr) = loader {
			req = req.query(&[("modLoaderType", cf_loader_type(ldr))]);
		}

		// We build the request to compute the full URL (with query params),
		// then pass the URL to `send_retried` which rebuilds the request on each
		// retry attempt. reqwest::RequestBuilder is not cloneable, so we can't
		// retry with the builder directly.
		let full_url = req.build()?.url().to_string();
		let headers = self.auth_headers();

		let response = self.send_retried(&full_url, headers).await?;
		let result: CfSearchResponse = response.json().await?;
		Ok(result.data)
	}

	pub async fn get_mod(
		&self,
		mod_id: i64,
	) -> Result<CfProject, ApiError> {
		let url = format!("{}/v1/mods/{}", self.base_url, mod_id);
		let headers = self.auth_headers();
		let response = self.send_retried(&url, headers).await?;
		let response = Self::ensure_success(response)?;
		let result: CfModResponse = response.json().await?;
		Ok(result.data)
	}

	pub async fn get_file(
		&self,
		file_id: i64,
	) -> Result<CfFile, ApiError> {
		let url = format!("{}/v1/mods/files/{}", self.base_url, file_id);
		let headers = self.auth_headers();
		let response = self.send_retried(&url, headers).await?;
		let response = Self::ensure_success(response)?;
		let result: CfFileListResponse = response.json().await?;
		result
			.data
			.into_iter()
			.next()
			.ok_or_else(|| ApiError::not_found(format!("file {}", file_id)))
	}

	pub async fn get_files(
		&self,
		mod_id: i64,
		minecraft_version: Option<&str>,
		loader: Option<&str>,
	) -> Result<Vec<CfFile>, ApiError> {
		let url = format!("{}/v1/mods/{}/files", self.base_url, mod_id);
		let mut req = self.client.get(&url);
		if let Some(mc_ver) = minecraft_version {
			req = req.query(&[("gameVersion", mc_ver)]);
		}
		if let Some(ldr) = loader {
			req = req.query(&[("modLoaderType", cf_loader_type(ldr))]);
		}

		// We build the request to compute the full URL (with query params),
		// then pass the URL to `send_retried` which rebuilds the request on each
		// retry attempt. reqwest::RequestBuilder is not cloneable, so we can't
		// retry with the builder directly.
		let full_url = req.build()?.url().to_string();
		let headers = self.auth_headers();

		let response = self.send_retried(&full_url, headers).await?;
		let result: CfFileListResponse = response.json().await?;
		Ok(result.data)
	}

	pub async fn get_file_download_url(
		&self,
		mod_id: i64,
		file_id: i64,
	) -> Result<String, ApiError> {
		let url = format!(
			"{}/v1/mods/{}/files/{}/download-url",
			self.base_url, mod_id, file_id
		);
		let headers = self.auth_headers();
		let result: CfDownloadUrlResponse =
			self.fetch_json(&url, headers).await?;
		Ok(result.data)
	}

	fn auth_headers(&self) -> Vec<(String, String)> {
		match &self.api_key {
			Some(key) => vec![("x-api-key".to_string(), key.clone())],
			None => Vec::new(),
		}
	}

	pub fn to_mod_info(project: CfProject) -> ModInfo {
		let url = project.links.website_url.clone().unwrap_or_else(|| {
			format!(
				"https://www.curseforge.com/minecraft/mc-mods/{}",
				project.slug
			)
		});
		let project_type = match project.class_id {
			Some(CF_RESOURCEPACK_CLASS_ID) => Some(ProjectType::ResourcePack),
			Some(CF_SHADER_CLASS_ID) => Some(ProjectType::Shader),
			_ => Some(ProjectType::Mod),
		};
		let loaders = project
			.categories
			.iter()
			.filter_map(|c| {
				let slug = c.slug.to_lowercase();
				if matches!(
					slug.as_str(),
					"fabric" | "forge" | "neoforge" | "quilt"
				) {
					Some(slug)
				} else {
					None
				}
			})
			.collect();
		ModInfo {
			id: project.id.to_string(),
			name: project.name,
			description: project.summary,
			source: ModSource::curseforge(project.id.to_string()),
			minecraft_versions: project.game_versions.unwrap_or_default(),
			loaders,
			downloads: project.download_count.max(0) as u64,
			url,
			project_type,
			client_side: None,
			server_side: None,
		}
	}

	pub fn to_mod_version(
		file: CfFile,
		download_url: String,
	) -> ModVersion {
		let (hash, hash_type) = extract_hash(file.hashes.as_ref());
		let loaders = extract_loaders(&file);

		ModVersion {
			version_id: Some(file.id.to_string()),
			version: file.display_name,
			minecraft_versions: file.game_versions,
			loaders,
			download_url,
			hash,
			hash_type: hash_type.unwrap_or_default(),
			file_size: file.file_size.max(0) as u64,
			release_date: file.file_date,
		}
	}

	pub fn to_source_dependencies(
		file: &CfFile
	) -> Vec<crate::types::SourceDependency> {
		use crate::types::SourceDependency;
		file.dependencies
			.iter()
			.filter(|dep| dep.mod_id != 0)
			.map(|dep| {
				let dep_type = match dep.relation_type {
					1 => DependencyKind::Embedded,
					2 => DependencyKind::Optional,
					3 => DependencyKind::Required,
					4 => DependencyKind::Optional,
					5 => DependencyKind::Incompatible,
					other => {
						tracing::debug!(
							"Unknown CurseForge relation type {}, treating as optional",
							other
						);
						DependencyKind::Optional
					}
				};
				SourceDependency {
					mod_id: dep.mod_id.to_string(),
					version_id: if dep.file_id != 0 {
						Some(dep.file_id.to_string())
					} else {
						None
					},
					dep_type,
					source: Some(crate::types::ModSource::curseforge(
						dep.mod_id.to_string(),
					)),
				}
			})
			.collect()
	}
}

fn extract_hash(
	hashes: Option<&Vec<CfHash>>
) -> (Option<String>, Option<HashType>) {
	match hashes.and_then(|h| {
		h.iter()
			.find_map(|entry| {
				if entry.algo == 1 {
					HashType::from_curseforge_algo(entry.algo)
						.map(|ht| (entry.value.clone(), ht))
				} else {
					None
				}
			})
			.or_else(|| {
				h.iter().find_map(|entry| {
					HashType::from_curseforge_algo(entry.algo)
						.map(|ht| (entry.value.clone(), ht))
				})
			})
	}) {
		Some((hash, ht)) => (Some(hash), Some(ht)),
		None => (None, None),
	}
}
fn extract_loaders(file: &CfFile) -> Vec<String> {
	let mut loaders = Vec::new();
	if let Some(ref sgvs) = file.sortable_game_versions {
		for sgv in sgvs {
			if let Some(ml) = sgv.mod_loader {
				let name = match ml {
					1 => "forge",
					4 => "fabric",
					5 => "quilt",
					6 => "neoforge",
					_ => continue,
				};
				if !loaders.contains(&name.to_string()) {
					loaders.push(name.to_string());
				}
			}
		}
	}
	loaders
}

fn cf_loader_type(loader: &str) -> String {
	match loader.to_lowercase().as_str() {
		"forge" => "1".to_string(),
		"fabric" => "4".to_string(),
		"quilt" => "5".to_string(),
		"neoforge" => "6".to_string(),
		other => other.to_string(),
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfSearchResponse {
	pub data: Vec<CfProject>,
	#[serde(default)]
	pub pagination: Option<CfPagination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfModResponse {
	pub data: CfProject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfFileListResponse {
	pub data: Vec<CfFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfDownloadUrlResponse {
	pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfPagination {
	pub index: i32,
	#[serde(rename = "pageSize")]
	pub page_size: i32,
	#[serde(rename = "resultCount")]
	pub result_count: i32,
	#[serde(rename = "totalCount")]
	pub total_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfProject {
	pub id: i64,
	#[serde(default)]
	pub uuid: Option<String>,
	pub slug: String,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub summary: String,
	#[serde(default)]
	pub download_count: i64,
	#[serde(default)]
	pub links: CfLinks,
	#[serde(default)]
	pub game_versions: Option<Vec<String>>,
	#[serde(default)]
	pub categories: Vec<CfCategory>,
	#[serde(default)]
	pub class_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CfLinks {
	#[serde(default)]
	pub website_url: Option<String>,
	#[serde(default)]
	pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfCategory {
	pub id: i64,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfFile {
	pub id: i64,
	#[serde(rename = "modId")]
	pub mod_id: i64,
	#[serde(rename = "displayName", default)]
	pub display_name: String,
	#[serde(rename = "fileName", default)]
	pub file_name: String,
	#[serde(rename = "fileSize", default)]
	pub file_size: i64,
	#[serde(rename = "downloadUrl", default)]
	pub download_url: Option<String>,
	#[serde(default)]
	pub game_versions: Vec<String>,
	#[serde(rename = "sortableGameVersions", default)]
	pub sortable_game_versions: Option<Vec<CfSortableGameVersion>>,
	#[serde(default)]
	pub dependencies: Vec<CfDependency>,
	#[serde(rename = "releaseType", default)]
	pub release_type: i32,
	#[serde(rename = "fileDate", default)]
	pub file_date: String,
	#[serde(default)]
	pub hashes: Option<Vec<CfHash>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfSortableGameVersion {
	#[serde(rename = "gameVersionName", default)]
	pub game_version_name: Option<String>,
	#[serde(rename = "gameVersionPadded", default)]
	pub game_version_padded: Option<String>,
	#[serde(rename = "gameVersion", default)]
	pub game_version: Option<String>,
	#[serde(default)]
	pub mod_loader: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfDependency {
	#[serde(rename = "modId")]
	pub mod_id: i64,
	#[serde(rename = "fileId", default)]
	pub file_id: i64,
	#[serde(rename = "relationType", default)]
	pub relation_type: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfHash {
	#[serde(default)]
	pub value: String,
	#[serde(default)]
	pub algo: i32,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::DependencyKind;

	#[test]
	fn test_curseforge_client_new() {
		let client = CurseForgeClient::new();
		assert_eq!(client.base_url, CURSEFORGE_API_URL);
		assert!(client.api_key.is_none());
	}

	#[test]
	fn test_curseforge_client_with_base_url() {
		let client =
			CurseForgeClient::new().with_base_url("http://localhost:3000");
		assert_eq!(client.base_url, "http://localhost:3000");
	}

	#[test]
	fn test_curseforge_client_with_api_key() {
		let client = CurseForgeClient::new().with_api_key("test-key");
		assert_eq!(client.api_key, Some("test-key".to_string()));
	}

	#[test]
	fn test_extract_loaders() {
		let file = CfFile {
			id: 1,
			mod_id: 1,
			display_name: String::new(),
			file_name: String::new(),
			file_size: 0,
			download_url: None,
			game_versions: vec![],
			sortable_game_versions: Some(vec![
				CfSortableGameVersion {
					game_version_name: Some("1.20.4".to_string()),
					game_version_padded: None,
					game_version: None,
					mod_loader: Some(4),
				},
				CfSortableGameVersion {
					game_version_name: None,
					game_version_padded: None,
					game_version: None,
					mod_loader: Some(1),
				},
			]),
			dependencies: vec![],
			release_type: 1,
			file_date: String::new(),
			hashes: None,
		};
		let loaders = extract_loaders(&file);
		assert_eq!(loaders, vec!["fabric", "forge"]);
	}

	#[test]
	fn test_cf_hash_extraction() {
		let file = CfFile {
			id: 1,
			mod_id: 1,
			display_name: "test".to_string(),
			file_name: "test.jar".to_string(),
			file_size: 1024,
			download_url: Some("http://example.com/test.jar".to_string()),
			game_versions: vec!["1.20.4".to_string()],
			sortable_game_versions: None,
			dependencies: vec![],
			release_type: 1,
			file_date: "2024-01-01T00:00:00Z".to_string(),
			hashes: Some(vec![
				CfHash {
					value: "abc123sha1".to_string(),
					algo: 1,
				},
				CfHash {
					value: "def456md5".to_string(),
					algo: 2,
				},
			]),
		};
		let sha1 = file
			.hashes
			.as_ref()
			.and_then(|h| h.iter().find(|h| h.algo == 1))
			.map(|h| h.value.clone());
		assert_eq!(sha1, Some("abc123sha1".to_string()));
	}

	#[test]
	fn test_to_source_dependencies() {
		let file = CfFile {
			id: 1,
			mod_id: 100,
			display_name: String::new(),
			file_name: String::new(),
			file_size: 0,
			download_url: None,
			game_versions: vec![],
			sortable_game_versions: None,
			dependencies: vec![
				CfDependency {
					mod_id: 200,
					file_id: 0,
					relation_type: 3,
				},
				CfDependency {
					mod_id: 300,
					file_id: 0,
					relation_type: 2,
				},
				CfDependency {
					mod_id: 400,
					file_id: 0,
					relation_type: 1,
				},
				CfDependency {
					mod_id: 0,
					file_id: 0,
					relation_type: 3,
				},
			],
			release_type: 1,
			file_date: String::new(),
			hashes: None,
		};
		let deps = CurseForgeClient::to_source_dependencies(&file);
		assert_eq!(deps.len(), 3);
		assert_eq!(deps[0].mod_id, "200");
		assert_eq!(deps[0].dep_type, DependencyKind::Required);
		assert_eq!(deps[1].mod_id, "300");
		assert_eq!(deps[1].dep_type, DependencyKind::Optional);
		assert_eq!(deps[2].mod_id, "400");
		assert_eq!(deps[2].dep_type, DependencyKind::Embedded);
	}

	#[test]
	fn test_curseforge_to_mod_info() {
		let project = CfProject {
			id: 12345,
			uuid: None,
			slug: "test-mod".to_string(),
			name: "Test Mod".to_string(),
			summary: "A test mod".to_string(),
			download_count: 500,
			links: CfLinks::default(),
			game_versions: Some(vec!["1.20.4".to_string()]),
			categories: vec![CfCategory {
				id: 1,
				name: "Fabric".to_string(),
				slug: "fabric".to_string(),
			}],
			class_id: None,
		};
		let info = CurseForgeClient::to_mod_info(project);
		assert_eq!(info.id, "12345");
		assert_eq!(info.name, "Test Mod");
		assert!(info.loaders.contains(&"fabric".to_string()));
	}

	#[test]
	fn test_curseforge_to_mod_version() {
		let file = CfFile {
			id: 1,
			mod_id: 100,
			display_name: "test-1.0.jar".to_string(),
			file_name: "test-1.0.jar".to_string(),
			file_size: 2048,
			download_url: Some("http://example.com/test.jar".to_string()),
			game_versions: vec!["1.20.4".to_string()],
			sortable_game_versions: None,
			dependencies: vec![],
			release_type: 1,
			file_date: "2024-01-01".to_string(),
			hashes: Some(vec![CfHash {
				value: "abc".to_string(),
				algo: 1,
			}]),
		};
		let mv = CurseForgeClient::to_mod_version(
			file,
			"http://example.com/test.jar".to_string(),
		);
		assert_eq!(mv.version, "test-1.0.jar");
		assert_eq!(mv.file_size, 2048);
	}
}
