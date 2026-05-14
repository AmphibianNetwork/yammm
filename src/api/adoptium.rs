//! Adoptium/Temurin JDK client for downloading and extracting JDK runtimes.

use crate::api::ApiClient;
use crate::api::error::ApiError;
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

use crate::output;

const ADOPTIUM_API: &str = "https://api.adoptium.net/v3";

crate::api::define_api_client!(AdoptiumClient, ADOPTIUM_API);

impl AdoptiumClient {
	/// Fetches the latest JDK download info for the given major version,
	/// auto-detecting the current OS and architecture.
	/// On macOS ARM, falls back to x64 (Rosetta 2) if no ARM build exists.
	pub async fn get_latest_jdk(
		&self,
		major_version: i32,
	) -> Result<AdoptiumAsset, ApiError> {
		let arch = detect_arch();
		let os = detect_os();

		if let Some(asset) = self.try_fetch_jdk(major_version, os, arch).await?
		{
			return Ok(asset);
		}

		// On macOS ARM, fall back to x64 via Rosetta 2
		if cfg!(target_os = "macos")
			&& arch == "aarch64"
			&& let Some(asset) =
				self.try_fetch_jdk(major_version, os, "x64").await?
		{
			output::warning(format!(
				"No JDK {} for ARM, using x64 via Rosetta 2",
				major_version
			));
			return Ok(asset);
		}

		Err(ApiError::not_found(format!(
			"No JDK {} release found for {}/{}",
			major_version, os, arch
		)))
	}

	async fn try_fetch_jdk(
		&self,
		major_version: i32,
		os: &str,
		arch: &str,
	) -> Result<Option<AdoptiumAsset>, ApiError> {
		let url = format!(
			"{}/assets/latest/{}/hotspot?architecture={}&os={}&image_type=jdk&vendor=eclipse",
			self.base_url, major_version, arch, os
		);

		let response = self.send_retried(&url, Vec::new()).await;
		let response = match response {
			Ok(r) => r,
			Err(ApiError::Http { status, .. })
				if status == 0 || (400..=499).contains(&status) =>
			{
				return Ok(None);
			}
			Err(e) => return Err(e),
		};
		if !response.status().is_success() {
			return Ok(None);
		}

		let assets: Vec<AdoptiumRelease> = response.json().await?;
		let release = match assets.into_iter().next() {
			Some(r) => r,
			None => return Ok(None),
		};

		let pkg = match release.binary.package {
			Some(p) => p,
			None => return Ok(None),
		};

		Ok(Some(AdoptiumAsset {
			major_version,
			download_url: pkg.link,
			checksum: pkg.checksum,
			checksum_type: pkg
				.checksum_type
				.unwrap_or_else(|| "sha256".to_string()),
			file_name: pkg.name,
			arch: arch.to_string(),
		}))
	}
}

pub struct AdoptiumAsset {
	pub major_version: i32,
	pub download_url: String,
	pub checksum: String,
	pub checksum_type: String,
	pub file_name: String,
	pub arch: String,
}

#[derive(Debug, Deserialize)]
struct AdoptiumRelease {
	binary: AdoptiumBinary,
}

#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
	package: Option<AdoptiumPackage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdoptiumPackage {
	link: String,
	name: String,
	checksum: String,
	#[serde(default)]
	checksum_type: Option<String>,
}

/// Returns the Adoptium OS identifier for the current platform.
pub fn detect_os() -> &'static str {
	if cfg!(target_os = "macos") {
		"mac"
	} else if cfg!(target_os = "windows") {
		"windows"
	} else {
		"linux"
	}
}

/// Returns the Adoptium architecture identifier for the current CPU.
pub fn detect_arch() -> &'static str {
	if cfg!(target_arch = "aarch64") {
		"aarch64"
	} else if cfg!(target_arch = "x86_64") {
		"x64"
	} else if cfg!(target_arch = "x86") {
		"x86"
	} else {
		"x64"
	}
}

/// Builds the expected directory name for a Temurin JDK installation.
/// If `arch_override` is given, uses that arch instead of auto-detecting
/// (needed for x64-via-Rosetta JDKs on ARM macOS).
pub fn java_dir_name(
	major_version: i32,
	arch_override: Option<&str>,
) -> String {
	let os = detect_os();
	let arch = arch_override.unwrap_or(detect_arch());
	format!("temurin-{}-{}-{}", major_version, os, arch)
}

/// Returns the path to the `java` binary inside a JDK directory.
pub fn java_binary_path(java_dir: &Path) -> std::path::PathBuf {
	let bin = java_dir.join("bin");
	if cfg!(target_os = "windows") {
		bin.join("java.exe")
	} else {
		bin.join("java")
	}
}

/// Extracts a JDK archive (.tar.gz or .zip) into the destination directory.
pub fn extract_archive(
	archive_path: &Path,
	dest_dir: &Path,
) -> Result<()> {
	std::fs::create_dir_all(dest_dir)?;

	let file_name = archive_path
		.file_name()
		.and_then(|n| n.to_str())
		.unwrap_or("");

	if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
		extract_tar_gz(archive_path, dest_dir)
	} else if file_name.ends_with(".zip") {
		extract_zip(archive_path, dest_dir)
	} else {
		Err(crate::errors::YammmError::invalid_args(format!(
			"Unsupported archive format: {}",
			file_name
		))
		.into())
	}
}

fn extract_tar_gz(
	archive_path: &Path,
	dest_dir: &Path,
) -> Result<()> {
	let file = std::fs::File::open(archive_path)?;
	let gz = flate2::read::GzDecoder::new(file);
	let mut archive = tar::Archive::new(gz);
	archive.unpack(dest_dir)?;
	Ok(())
}

fn extract_zip(
	archive_path: &Path,
	dest_dir: &Path,
) -> Result<()> {
	let file = std::fs::File::open(archive_path)?;
	let mut archive = zip::ZipArchive::new(file)?;
	let dest_dir_canonical = dest_dir
		.canonicalize()
		.unwrap_or_else(|_| dest_dir.to_path_buf());

	for i in 0..archive.len() {
		let mut entry = archive.by_index(i)?;
		let enclosed = match entry.enclosed_name() {
			Some(p) => p,
			None => continue,
		};

		if enclosed.as_os_str().is_empty() {
			continue;
		}

		let out_path = dest_dir.join(&enclosed);

		let out_path_canonical = out_path
			.parent()
			.and_then(|p| p.canonicalize().ok())
			.and_then(|p| out_path.file_name().map(|n| p.join(n)))
			.unwrap_or_else(|| out_path.clone());

		if !out_path_canonical.starts_with(&dest_dir_canonical) {
			tracing::warn!(
				"Skipping zip entry outside destination: {}",
				enclosed.display()
			);
			continue;
		}

		if entry.is_dir() {
			std::fs::create_dir_all(&out_path)?;
		} else {
			if let Some(parent) = out_path.parent() {
				std::fs::create_dir_all(parent)?;
			}
			let mut out_file = std::fs::File::create(&out_path)?;
			std::io::copy(&mut entry, &mut out_file)?;

			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt;
				if let Some(mode) = entry.unix_mode() {
					let _ = std::fs::set_permissions(
						&out_path,
						std::fs::Permissions::from_mode(mode),
					);
				}
			}
		}
	}

	Ok(())
}

// Checks if a directory name contains the major version as a distinct
// numeric segment (e.g. "jdk-8" or "jdk-17.0.1" match 17, but
// "jdk-18.0.1" does NOT match 8).
fn version_dir_matches(
	name: &str,
	major_version: i32,
) -> bool {
	let prefix = format!("-{}.", major_version);
	let suffix = format!("-{}", major_version);
	name.contains(&prefix) || name.ends_with(&suffix)
}

/// Locates the extracted JDK directory inside `dest_dir` using a two-pass
/// heuristic: first look for directories whose name includes the expected
/// version and contains a java binary, then fall back to any directory
/// that contains a java binary.
pub fn find_extracted_jdk_dir(
	dest_dir: &Path,
	major_version: i32,
) -> Option<std::path::PathBuf> {
	if let Ok(entries) = std::fs::read_dir(dest_dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				let name =
					path.file_name().and_then(|n| n.to_str()).unwrap_or("");
				if name.starts_with("jdk-")
					|| name.starts_with("temurin-")
					|| version_dir_matches(name, major_version)
				{
					if java_binary_path(&path).exists() {
						return Some(path);
					}
					let contents_home = path.join("Contents").join("Home");
					if java_binary_path(&contents_home).exists() {
						return Some(contents_home);
					}
				}
			}
		}
	}

	if let Ok(entries) = std::fs::read_dir(dest_dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				if java_binary_path(&path).exists() {
					return Some(path);
				}
				let contents_home = path.join("Contents").join("Home");
				if java_binary_path(&contents_home).exists() {
					return Some(contents_home);
				}
			}
		}
	}

	None
}
