//! Fabric and Quilt loader clients for fetching loader versions,
//! profiles, and downloading runtime libraries.

use crate::api::error::ApiError;
use serde::{Deserialize, Serialize};
use std::path::Path;

const FABRIC_META_URL: &str = "https://meta.fabricmc.net/v2";

crate::api::define_api_client!(FabricClient, FABRIC_META_URL);

impl FabricClient {
	/// Returns all Fabric loader versions available for a given Minecraft version.
	pub async fn get_loader_versions(
		&self,
		game_version: &str,
	) -> Result<Vec<FabricLoaderVersion>, ApiError> {
		let url = format!("{}/versions/loader/{}", self.base_url, game_version);
		let versions =
			fetch_and_deserialize(&self.client, &url, "Fabric loader versions")
				.await?;
		Ok(versions)
	}

	/// Returns the latest stable Fabric loader version, falling back to the first available.
	pub async fn get_latest_loader_version(
		&self,
		game_version: &str,
	) -> Result<String, ApiError> {
		let versions = self.get_loader_versions(game_version).await?;
		versions
			.iter()
			.find(|v| v.loader.stable)
			.or_else(|| versions.first())
			.map(|v| v.loader.version.clone())
			.ok_or_else(|| {
				ApiError::not_found(format!(
					"No Fabric loader versions found for MC {}",
					game_version
				))
			})
	}

	/// Fetches the Fabric launch profile (main class + libraries) for a specific loader version.
	pub async fn get_profile(
		&self,
		game_version: &str,
		loader_version: &str,
	) -> Result<FabricProfile, ApiError> {
		let url = format!(
			"{}/versions/loader/{}/{}/profile/json",
			self.base_url, game_version, loader_version
		);
		let profile =
			fetch_and_deserialize(&self.client, &url, "Fabric profile").await?;
		Ok(profile)
	}

	/// Downloads all Fabric loader libraries, skipping already-cached JARs.
	pub async fn download_libraries(
		&self,
		profile: &FabricProfile,
		game_version: &str,
		loader_version: &str,
		cache_dir: &Path,
	) -> Result<Vec<std::path::PathBuf>, ApiError> {
		download_loader_libraries(
			&self.client,
			&profile.libraries,
			"Fabric",
			cache_dir,
			game_version,
			loader_version,
		)
		.await
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricLoaderVersion {
	pub loader: FabricLoaderInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricLoaderInfo {
	pub version: String,
	#[serde(default)]
	pub stable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricProfile {
	pub main_class: String,
	#[serde(default)]
	pub libraries: Vec<FabricLibrary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricLibrary {
	pub name: String,
	pub url: String,
}

async fn fetch_and_deserialize<T: serde::de::DeserializeOwned>(
	client: &reqwest::Client,
	url: &str,
	label: &str,
) -> Result<T, ApiError> {
	let response = crate::api::retry::send_retried(client, url, Vec::new())
		.await
		.map_err(|e| {
			ApiError::network_error(format!("Failed to fetch {}: {}", label, e))
		})?;
	if !response.status().is_success() {
		return Err(ApiError::http(
			response.status().as_u16(),
			format!("Failed to fetch {}", label),
		));
	}
	let data: T = response.json().await?;
	Ok(data)
}

fn is_valid_jar(path: &std::path::Path) -> bool {
	let Ok(mut file) = std::fs::File::open(path) else {
		return false;
	};
	let meta = match file.metadata() {
		Ok(m) => m,
		Err(_) => return false,
	};
	if meta.len() < 1024 {
		return false;
	}
	let mut buf = [0u8; 2];
	use std::io::Read;
	file.read_exact(&mut buf).is_ok() && buf == *b"PK"
}

async fn download_loader_libraries(
	client: &reqwest::Client,
	libraries: &[FabricLibrary],
	loader_name: &str,
	cache_dir: &Path,
	game_version: &str,
	loader_version: &str,
) -> Result<Vec<std::path::PathBuf>, ApiError> {
	let lib_dir = cache_dir.join(game_version).join(loader_version);
	std::fs::create_dir_all(&lib_dir)?;
	let mut paths = Vec::new();

	for lib in libraries {
		let safe_filename = crate::utils::maven::filename(&lib.name);
		let dest = lib_dir.join(&safe_filename);

		if dest.exists() && is_valid_jar(&dest) {
			tracing::debug!(
				"{} lib already cached: {}",
				loader_name,
				dest.display()
			);
			paths.push(dest);
			continue;
		}

		let download_url = crate::utils::maven::maven_url(&lib.url, &lib.name);
		tracing::debug!("Downloading {} library: {}", loader_name, lib.name);
		// Fabric/Quilt manifests don't carry hashes for individual library
		// downloads, so we can't integrity-check here. Streaming still bounds
		// memory and atomic rename prevents partially-written jars.
		crate::api::streaming::download_to_file(
			client,
			&download_url,
			&dest,
			crate::api::streaming::HashPolicy::AcceptedUnhashed {
				reason: "Fabric/Quilt manifest does not carry per-library hashes",
			},
			&lib.name,
		)
		.await
		.map_err(|e| {
			ApiError::network_error(format!(
				"Failed to download {} library {}: {}",
				loader_name, lib.name, e
			))
		})?;
		paths.push(dest);
	}

	Ok(paths)
}

const QUILT_META_URL: &str = "https://meta.quiltmc.org/v3";

crate::api::define_api_client!(QuiltClient, QUILT_META_URL);

impl QuiltClient {
	/// Returns all Quilt loader versions available for a given Minecraft version.
	pub async fn get_loader_versions(
		&self,
		game_version: &str,
	) -> Result<Vec<QuiltLoaderVersion>, ApiError> {
		let url = format!("{}/versions/loader/{}", self.base_url, game_version);
		let versions =
			fetch_and_deserialize(&self.client, &url, "Quilt loader versions")
				.await?;
		Ok(versions)
	}

	/// Returns the latest stable Quilt loader version (build 0), falling back to the first available.
	pub async fn get_latest_loader_version(
		&self,
		game_version: &str,
	) -> Result<String, ApiError> {
		let versions = self.get_loader_versions(game_version).await?;
		versions
			.iter()
			.find(|v| v.loader.build == 0)
			.or_else(|| versions.first())
			.map(|v| v.loader.version.clone())
			.ok_or_else(|| {
				ApiError::not_found(format!(
					"No Quilt loader versions found for MC {}",
					game_version
				))
			})
	}

	/// Fetches the Quilt launch profile (main class + libraries) for a specific loader version.
	pub async fn get_profile(
		&self,
		game_version: &str,
		loader_version: &str,
	) -> Result<QuiltProfile, ApiError> {
		let url = format!(
			"{}/versions/loader/{}/{}/profile/json",
			self.base_url, game_version, loader_version
		);
		let profile =
			fetch_and_deserialize(&self.client, &url, "Quilt profile").await?;
		Ok(profile)
	}

	/// Downloads all Quilt loader libraries for the given side, skipping already-cached JARs.
	pub async fn download_libraries(
		&self,
		profile: &QuiltProfile,
		side: &str,
		game_version: &str,
		loader_version: &str,
		cache_dir: &Path,
	) -> Result<Vec<std::path::PathBuf>, ApiError> {
		let libs: Vec<FabricLibrary> = profile
			.libraries
			.for_side(side)
			.into_iter()
			.cloned()
			.collect();
		download_loader_libraries(
			&self.client,
			&libs,
			"Quilt",
			cache_dir,
			game_version,
			loader_version,
		)
		.await
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuiltLoaderVersion {
	pub loader: QuiltLoaderInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuiltLoaderInfo {
	pub version: String,
	#[serde(default)]
	pub build: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuiltProfile {
	pub main_class: QuiltMainClass,
	pub libraries: QuiltLibraries,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum QuiltMainClass {
	Structured { client: String, server: String },
	Single(String),
}

impl QuiltMainClass {
	pub fn for_side(
		&self,
		side: &str,
	) -> String {
		match self {
			QuiltMainClass::Structured { client, server } => {
				if side == "client" {
					client.clone()
				} else {
					server.clone()
				}
			}
			QuiltMainClass::Single(s) => {
				if side == "server" {
					"org.quiltmc.loader.impl.launch.knot.KnotServer".to_string()
				} else {
					s.clone()
				}
			}
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum QuiltLibraries {
	Split {
		#[serde(default)]
		client: Vec<FabricLibrary>,
		#[serde(default)]
		common: Vec<FabricLibrary>,
		#[serde(default)]
		server: Vec<FabricLibrary>,
	},
	Flat(Vec<FabricLibrary>),
}

impl QuiltLibraries {
	pub fn for_side(
		&self,
		side: &str,
	) -> Vec<&FabricLibrary> {
		match self {
			QuiltLibraries::Split {
				client,
				common,
				server,
			} => {
				let mut libs: Vec<&FabricLibrary> = common.iter().collect();
				if side == "client" {
					libs.extend(client.iter());
				} else {
					libs.extend(server.iter());
				}
				libs
			}
			QuiltLibraries::Flat(libs) => libs.iter().collect(),
		}
	}
}
