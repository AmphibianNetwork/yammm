use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::templates::DataEntry;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallProfile {
	#[serde(default)]
	pub spec: i32,
	#[serde(default)]
	pub profile: String,
	pub version: String,
	pub minecraft: String,
	pub json: String,
	#[serde(default)]
	pub libraries: Vec<super::libraries::InstallerLibrary>,
	#[serde(default)]
	pub processors: Vec<super::processors::Processor>,
	#[serde(default)]
	pub data: std::collections::BTreeMap<String, DataEntry>,
}

pub fn extract_install_profile(
	installer_jar: &Path
) -> Result<(InstallProfile, serde_json::Value)> {
	let file = std::fs::File::open(installer_jar).with_context(|| {
		format!("Failed to open installer JAR: {}", installer_jar.display())
	})?;
	let mut archive = zip::ZipArchive::new(file)
		.with_context(|| "Failed to read installer JAR as ZIP")?;

	let mut profile_bytes = Vec::new();
	archive
		.by_name("install_profile.json")
		.with_context(|| "No install_profile.json found in installer JAR")?
		.read_to_end(&mut profile_bytes)?;
	let profile: InstallProfile = serde_json::from_slice(&profile_bytes)
		.with_context(|| "Failed to parse install_profile.json")?;

	let json_path = profile.json.trim_start_matches('/');
	let mut version_bytes = Vec::new();
	archive
		.by_name(json_path)
		.with_context(|| format!("No {} found in installer JAR", json_path))?
		.read_to_end(&mut version_bytes)?;
	let version_json: serde_json::Value =
		serde_json::from_slice(&version_bytes)
			.with_context(|| "Failed to parse version JSON from installer")?;

	Ok((profile, version_json))
}

pub fn extract_file_from_installer(
	installer_path: &str,
	installer_jar: &Path,
	temp_dir: &Path,
) -> Result<String> {
	let clean_path = installer_path.trim_start_matches('/');
	let file = std::fs::File::open(installer_jar)?;
	let mut archive = zip::ZipArchive::new(file)?;

	let mut zip_file = archive.by_name(clean_path).with_context(|| {
		format!("File {} not found in installer JAR", clean_path)
	})?;

	std::fs::create_dir_all(temp_dir)?;

	let filename = clean_path.replace('/', "_");
	let tmp_path = temp_dir.join(&filename);
	let mut out_file = std::fs::File::create(&tmp_path)?;
	std::io::copy(&mut zip_file, &mut out_file)?;

	Ok(tmp_path.to_string_lossy().to_string())
}

pub fn extract_launch_args_from_installer(
	installer_jar: &Path,
	lib_dir: &Path,
) -> Result<()> {
	let has_unix_args = find_file_in_dir_tree(lib_dir, "unix_args.txt");
	if has_unix_args.is_some() {
		return Ok(());
	}

	let version_dir = find_neoforge_or_forge_version_dir(lib_dir);
	let dest_dir = match version_dir {
		Some(d) => d,
		None => return Ok(()),
	};

	let file = std::fs::File::open(installer_jar)?;
	let mut archive = zip::ZipArchive::new(file)?;

	for name in &["data/unix_args.txt", "data/win_args.txt"] {
		if let Ok(mut zip_file) = archive.by_name(name) {
			let filename = name.replace("data/", "");
			let dest = dest_dir.join(&filename);
			if !dest.exists() {
				if let Some(parent) = dest.parent() {
					std::fs::create_dir_all(parent)?;
				}
				let mut out = std::fs::File::create(&dest)?;
				std::io::copy(&mut zip_file, &mut out)?;
			}
		}
	}
	Ok(())
}

pub(crate) fn find_neoforge_or_forge_version_dir(
	lib_dir: &Path
) -> Option<PathBuf> {
	let neoforge_dir = lib_dir.join("net").join("neoforged").join("neoforge");
	if neoforge_dir.exists()
		&& let Some(version_dir) = find_dir_with_jar(&neoforge_dir)
	{
		return Some(version_dir);
	}
	let forge_dir = lib_dir.join("net").join("minecraftforge").join("forge");
	if forge_dir.exists()
		&& let Some(version_dir) = find_dir_with_jar(&forge_dir)
	{
		return Some(version_dir);
	}
	None
}

fn find_dir_with_jar(version_parent: &Path) -> Option<PathBuf> {
	for entry in std::fs::read_dir(version_parent).ok()? {
		let entry = entry.ok()?;
		let path = entry.path();
		if path.is_dir() {
			for child in std::fs::read_dir(&path).ok()? {
				let child = child.ok()?;
				let child_path = child.path();
				if child_path.is_file()
					&& child_path.extension().is_some_and(|e| e == "jar")
				{
					return Some(path);
				}
			}
		}
	}
	None
}

pub(crate) fn find_file_in_dir_tree(
	dir: &Path,
	filename: &str,
) -> Option<PathBuf> {
	crate::utils::find_file_recursive(dir, filename)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_install_profile_deserialization() {
		let json = r#"{
			"spec": 1,
			"profile": "PROJECT",
			"version": "20.4.1",
			"minecraft": "1.20.4",
			"json": "/net/neoforged/neoforge/20.4.1/version.json",
			"libraries": [],
			"processors": [],
			"data": {}
		}"#;
		let profile: InstallProfile = serde_json::from_str(json).unwrap();
		assert_eq!(profile.spec, 1);
		assert_eq!(profile.version, "20.4.1");
		assert_eq!(profile.minecraft, "1.20.4");
		assert!(profile.libraries.is_empty());
		assert!(profile.processors.is_empty());
	}

	#[test]
	fn test_install_profile_defaults() {
		let json = r#"{
			"version": "20.4.1",
			"minecraft": "1.20.4",
			"json": "version.json"
		}"#;
		let profile: InstallProfile = serde_json::from_str(json).unwrap();
		assert_eq!(profile.spec, 0);
		assert!(profile.profile.is_empty());
		assert!(profile.libraries.is_empty());
		assert!(profile.processors.is_empty());
		assert!(profile.data.is_empty());
	}

	#[test]
	fn test_find_neoforge_or_forge_version_dir_neither() {
		let temp_dir = tempfile::TempDir::new().unwrap();
		let result = find_neoforge_or_forge_version_dir(temp_dir.path());
		assert!(result.is_none());
	}

	#[test]
	fn test_find_neoforge_or_forge_version_dir_with_neoforge() {
		let temp_dir = tempfile::TempDir::new().unwrap();
		let nf_dir = temp_dir
			.path()
			.join("net")
			.join("neoforged")
			.join("neoforge")
			.join("20.4.1");
		std::fs::create_dir_all(&nf_dir).unwrap();
		std::fs::write(nf_dir.join("neoforge-20.4.1.jar"), "").unwrap();

		let result = find_neoforge_or_forge_version_dir(temp_dir.path());
		assert!(result.is_some());
		let dir = result.unwrap();
		assert!(dir.to_string_lossy().contains("20.4.1"));
	}

	#[test]
	fn test_find_neoforge_or_forge_version_dir_with_forge() {
		let temp_dir = tempfile::TempDir::new().unwrap();
		let forge_dir = temp_dir
			.path()
			.join("net")
			.join("minecraftforge")
			.join("forge")
			.join("47.1.0");
		std::fs::create_dir_all(&forge_dir).unwrap();
		std::fs::write(forge_dir.join("forge-47.1.0.jar"), "").unwrap();

		let result = find_neoforge_or_forge_version_dir(temp_dir.path());
		assert!(result.is_some());
		let dir = result.unwrap();
		assert!(dir.to_string_lossy().contains("47.1.0"));
	}
}
