//! Minecraft library downloading and native extraction. Downloads MC library
//! JARs, verifies their checksums, and extracts native libraries (.so/.dll/.dylib)
//! from classifier JARs into the natives directory.

use anyhow::Result;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::api::minecraft::{Artifact, Library, OsRule};
use crate::output;

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

/// Downloads a single library artifact, verifying its SHA-1 checksum when
/// the manifest provides one. Streams through a tmp file so memory stays
/// bounded regardless of artifact size.
async fn download_artifact(
	client: &reqwest::Client,
	artifact: &Artifact,
	dest: &Path,
) -> Result<()> {
	let policy = crate::api::streaming::HashPolicy::from_optional(
		artifact.sha1.as_deref().map(|hex| {
			crate::api::streaming::ExpectedHash {
				hash_type: crate::types::HashType::Sha1,
				hex,
			}
		}),
		"Minecraft library manifest artifact has no sha1",
	);
	crate::api::streaming::download_to_file(
		client,
		&artifact.url,
		dest,
		policy,
		&artifact.path,
	)
	.await?;
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
	let Some(os) = os_rule else {
		return true;
	};

	if let Some(ref name) = os.name
		&& name != crate::utils::current_os_name()
	{
		return false;
	}

	if let Some(ref arch) = os.arch
		&& arch == "x86"
		&& !cfg!(target_arch = "x86")
	{
		return false;
	}

	true
}

fn rule_matches_features(
	features: &Option<std::collections::BTreeMap<String, serde_json::Value>>
) -> bool {
	let Some(features) = features else {
		return true;
	};
	for value in features.values() {
		if value.as_bool() == Some(true) {
			return false;
		}
	}
	true
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::api::minecraft::Rule;

	fn lib_with_rules(rules: Vec<Rule>) -> Library {
		Library {
			name: "test:lib:1".to_string(),
			downloads: None,
			rules: Some(rules),
		}
	}

	fn allow_for_os(name: &str) -> Rule {
		Rule {
			action: "allow".to_string(),
			os: Some(OsRule {
				name: Some(name.to_string()),
				arch: None,
			}),
			features: None,
		}
	}

	fn disallow_for_os(name: &str) -> Rule {
		Rule {
			action: "disallow".to_string(),
			os: Some(OsRule {
				name: Some(name.to_string()),
				arch: None,
			}),
			features: None,
		}
	}

	#[test]
	fn test_library_without_rules_is_always_included() {
		let lib = Library {
			name: "no-rules".to_string(),
			downloads: None,
			rules: None,
		};
		assert!(should_include_library(&lib));
	}

	#[test]
	fn test_library_with_allow_for_current_os_is_included() {
		let lib =
			lib_with_rules(vec![allow_for_os(crate::utils::current_os_name())]);
		assert!(should_include_library(&lib));
	}

	#[test]
	fn test_library_with_allow_for_other_os_is_excluded() {
		// Pick a fictitious OS name so it never matches the current platform.
		let lib = lib_with_rules(vec![allow_for_os("haiku-os")]);
		assert!(
			!should_include_library(&lib),
			"libraries default to disallowed; an allow that doesn't match shouldn't flip it"
		);
	}

	#[test]
	fn test_library_allow_then_disallow_for_current_os() {
		// Real-world pattern: allow-all (no os rule) followed by disallow-osx.
		// On the current OS this should resolve to the latter rule's outcome.
		let allow_all = Rule {
			action: "allow".to_string(),
			os: None,
			features: None,
		};
		let lib = lib_with_rules(vec![
			allow_all,
			disallow_for_os(crate::utils::current_os_name()),
		]);
		assert!(
			!should_include_library(&lib),
			"later disallow for current OS must override earlier allow-all"
		);
	}

	#[test]
	fn test_rule_matches_features_with_true_feature_disqualifies() {
		let mut features = std::collections::BTreeMap::new();
		features
			.insert("is_demo_user".to_string(), serde_json::Value::Bool(true));
		assert!(
			!rule_matches_features(&Some(features)),
			"any feature flag set to true disqualifies the rule"
		);
	}

	#[test]
	fn test_rule_matches_features_with_no_features_or_all_false() {
		assert!(rule_matches_features(&None));

		let mut features = std::collections::BTreeMap::new();
		features.insert(
			"has_custom_resolution".to_string(),
			serde_json::Value::Bool(false),
		);
		assert!(rule_matches_features(&Some(features)));
	}

	#[test]
	fn test_native_classifier_candidates_includes_arch_specific_first() {
		let candidates = native_classifier_candidates();
		assert_eq!(candidates.len(), 2, "expected arch-specific + generic");
		let os = crate::utils::current_os_name();
		// The first should embed an arch suffix; the second should not.
		assert!(
			candidates[0].starts_with(&format!("natives-{}-", os)),
			"first candidate should be arch-specific: {}",
			candidates[0]
		);
		assert_eq!(candidates[1], format!("natives-{}", os));
	}
}
