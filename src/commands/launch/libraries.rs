//! Minecraft library downloading and native extraction. Downloads MC library
//! JARs, verifies their checksums, and extracts native libraries (.so/.dll/.dylib)
//! from classifier JARs into the natives directory.

use anyhow::Result;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::api::minecraft::{Artifact, Library, OsRule};
use crate::output;
use crate::types::HashType;

/// Downloads all MC libraries for a version, skipping OS-incompatible ones.
/// Returns the classpath JAR paths and the natives directory path.
pub async fn download_mc_libraries(
	libraries: &[Library],
	mc_version: &str,
	cache_dir: &Path,
	http_client: &reqwest::Client,
) -> Result<(Vec<PathBuf>, PathBuf)> {
	std::fs::create_dir_all(cache_dir)?;
	let mut classpath_paths = Vec::new();

	let natives_dir = cache_dir.join(mc_version).join("natives");
	std::fs::create_dir_all(&natives_dir)?;

	let total = libraries.len();
	let pb = output::download_progress(total as u64);
	pb.set_message("Downloading MC libraries");

	for lib in libraries {
		if !should_include_library(lib) {
			pb.inc(1);
			continue;
		}

		if let Some(ref downloads) = lib.downloads {
			if let Some(ref artifact) = downloads.artifact {
				let dest = cache_dir
					.join(mc_version)
					.join("libraries")
					.join(&artifact.path);
				if !dest.exists() {
					download_artifact(http_client, artifact, &dest).await?;
				}
				classpath_paths.push(dest);
			}

			if let Some(ref classifiers) = downloads.classifiers {
				// Try each native classifier candidate (arch-specific first,
				// then generic) until we find one that exists in this library.
				for native_key in native_classifier_candidates() {
					if let Some(native_artifact) = classifiers.get(&native_key)
					{
						let native_dest =
							cache_dir.join(mc_version).join("natives-lib");
						let jar_dest = native_dest.join(&native_artifact.path);
						if !jar_dest.exists() {
							download_artifact(
								http_client,
								native_artifact,
								&jar_dest,
							)
							.await?;
						}
						extract_natives(&jar_dest, &natives_dir)?;
						break;
					}
				}
			}
		}

		pb.inc(1);
	}

	pb.finish_and_clear();
	Ok((classpath_paths, natives_dir))
}

/// Downloads a single library artifact, verifying its SHA-1 checksum.
async fn download_artifact(
	client: &reqwest::Client,
	artifact: &Artifact,
	dest: &Path,
) -> Result<()> {
	if dest.exists() {
		return Ok(());
	}

	if let Some(parent) = dest.parent() {
		std::fs::create_dir_all(parent)?;
	}

	let response = client.get(&artifact.url).send().await?;
	if !response.status().is_success() {
		return Err(crate::errors::YammmError::download_failed(format!(
			"Failed to download library {}: HTTP {}",
			artifact.path,
			response.status()
		))
		.into());
	}
	let bytes = response.bytes().await?;

	if let Some(ref sha1) = artifact.sha1 {
		let computed = HashType::Sha1.compute_for_bytes(&bytes);
		if computed != *sha1 {
			return Err(crate::errors::YammmError::download_failed(format!(
				"SHA-1 mismatch for {} (expected {}, got {})",
				artifact.path, sha1, computed
			))
			.into());
		}
	}

	std::fs::write(dest, &bytes)?;
	Ok(())
}

/// Build a list of native classifier keys to try, in order of preference.
/// The arch-specific key (e.g. `natives-linux-arm64`) is preferred over the
/// generic one (e.g. `natives-linux`), but the caller must check which one
/// actually exists in the library's classifiers map.
fn native_classifier_candidates() -> Vec<String> {
	let os = crate::utils::current_os_name();

	let arch = if cfg!(target_arch = "aarch64") {
		"arm64"
	} else {
		"x86_64"
	};

	vec![
		format!("natives-{}-{}", os, arch),
		format!("natives-{}", os),
	]
}

/// Extracts native library files (.so, .dll, .dylib, .jnilib) from a JAR
/// into the natives directory, skipping non-native entries and existing files.
fn extract_natives(
	jar_path: &Path,
	natives_dir: &Path,
) -> Result<()> {
	let file = std::fs::File::open(jar_path)?;
	let mut archive = zip::ZipArchive::new(file)?;

	for i in 0..archive.len() {
		let mut zip_file = archive.by_index(i)?;
		let name = zip_file.name().to_string();

		if !name.ends_with(".so")
			&& !name.ends_with(".dll")
			&& !name.ends_with(".dylib")
			&& !name.ends_with(".jnilib")
		{
			continue;
		}

		let filename = name.rsplit('/').next().unwrap_or(&name);
		let out_path = natives_dir.join(filename);

		if out_path.exists() {
			continue;
		}

		let mut buf = Vec::new();
		zip_file.read_to_end(&mut buf)?;
		std::fs::write(&out_path, &buf)?;
	}

	Ok(())
}

/// Evaluates a library's rules to decide if it should be included for the
/// current OS and architecture. Libraries without rules are always included.
/// Rules are processed in order: `allow` sets the flag, `disallow` clears it.
fn should_include_library(lib: &Library) -> bool {
	if let Some(ref rules) = lib.rules {
		// Start disallowed; each rule can allow or disallow based on OS/features
		let mut allowed = false;
		for rule in rules {
			let rule_matches = rule_matches_os(&rule.os)
				&& rule_matches_features(&rule.features);
			if rule.action == "allow" && rule_matches {
				allowed = true;
			} else if rule.action == "disallow" && rule_matches {
				allowed = false;
			}
		}
		allowed
	} else {
		// No rules means the library is always included
		true
	}
}

fn rule_matches_os(os_rule: &Option<OsRule>) -> bool {
	let Some(ref os) = os_rule else {
		return true;
	};

	if let Some(ref name) = os.name {
		if name != crate::utils::current_os_name() {
			return false;
		}
	}

	if let Some(ref arch) = os.arch {
		if arch == "x86" && !cfg!(target_arch = "x86") {
			return false;
		}
	}

	true
}

fn rule_matches_features(
	features: &Option<std::collections::BTreeMap<String, serde_json::Value>>
) -> bool {
	let Some(ref features) = features else {
		return true;
	};
	for value in features.values() {
		if value.as_bool() == Some(true) {
			return false;
		}
	}
	true
}
