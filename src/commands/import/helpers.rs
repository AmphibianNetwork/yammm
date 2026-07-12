use path_clean::PathClean;
use std::path::{Component, Path, PathBuf};

/// Decision returned by [`classify_archive_entry`] for a single zip entry.
pub enum ExtractDecision {
	/// Entry is a regular file inside the requested prefix; extract to this path.
	Extract(PathBuf),
	/// Entry is outside the prefix, a directory marker, or otherwise not a
	/// candidate. Skip silently.
	Skip,
	/// Entry is inside the prefix but contains traversal, absolute, or other
	/// escaping components. Skip and warn.
	Unsafe,
}

/// Classifies a zip-archive entry for safe extraction under `dest_root`.
///
/// `zip_name` is interpreted as a forward-slash separated path after
/// normalizing any backslashes. Matching against `archive_prefix` is
/// component-based via [`Path::strip_prefix`], so a prefix-substring
/// such as `overrides_evil/` will not collide with `overrides/`.
///
/// The resolved path is rejected (returned as [`ExtractDecision::Unsafe`])
/// if the relative portion contains `..`, a root component, or a Windows
/// drive prefix — or if, after lexical normalization, the joined path no
/// longer lives under `dest_root`.
pub fn classify_archive_entry(
	zip_name: &str,
	archive_prefix: &str,
	dest_root: &Path,
) -> ExtractDecision {
	if zip_name.ends_with('/') || zip_name.ends_with('\\') {
		return ExtractDecision::Skip;
	}

	let normalized = zip_name.replace('\\', "/");
	let entry_path = Path::new(&normalized);
	let prefix_path = Path::new(archive_prefix);

	let Ok(relative) = entry_path.strip_prefix(prefix_path) else {
		return ExtractDecision::Skip;
	};

	if relative.as_os_str().is_empty() {
		return ExtractDecision::Skip;
	}

	if relative.components().any(|c| {
		matches!(
			c,
			Component::ParentDir | Component::RootDir | Component::Prefix(_)
		)
	}) {
		return ExtractDecision::Unsafe;
	}

	let dest_clean = dest_root.clean();
	let outpath = dest_clean.join(relative).clean();

	if !outpath.starts_with(&dest_clean) {
		return ExtractDecision::Unsafe;
	}

	ExtractDecision::Extract(outpath)
}

pub fn extract_slug_from_path(path: &str) -> String {
	let path = path.replace('\\', "/");
	let filename = path.rsplit('/').next().unwrap_or(path.as_str());
	let stem = filename
		.strip_suffix(".jar")
		.or_else(|| filename.strip_suffix(".zip"))
		.unwrap_or(filename);
	let parts: Vec<&str> = stem.split('-').collect();
	for i in 1..parts.len() {
		if parts[i].chars().next().is_some_and(|c| c.is_ascii_digit()) {
			return parts[..i].join("-");
		}
	}
	stem.to_string()
}

pub fn extract_version_from_path(path: &str) -> String {
	let path = path.replace('\\', "/");
	let filename = path.rsplit('/').next().unwrap_or(path.as_str());
	let filename = filename
		.strip_suffix(".jar")
		.or_else(|| filename.strip_suffix(".zip"))
		.unwrap_or(filename);
	let parts: Vec<&str> = filename.split('-').collect();
	for i in 1..parts.len() {
		let candidate = parts[i..].join("-");
		if candidate.chars().next().is_some_and(|c| c.is_ascii_digit()) {
			return candidate;
		}
	}
	"0.0.0".to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn classify(
		name: &str,
		prefix: &str,
		root: &str,
	) -> ExtractDecision {
		classify_archive_entry(name, prefix, Path::new(root))
	}

	fn extracted(decision: ExtractDecision) -> Option<PathBuf> {
		match decision {
			ExtractDecision::Extract(p) => Some(p),
			_ => None,
		}
	}

	#[test]
	fn extracts_regular_file_under_prefix() {
		let out = extracted(classify(
			"overrides/config/foo.json",
			"overrides",
			"/pack",
		))
		.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("/pack/config/foo.json"));
	}

	#[test]
	fn extracts_nested_path() {
		let out = extracted(classify(
			"overrides/config/sodium/options.json",
			"overrides",
			"/pack",
		))
		.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("/pack/config/sodium/options.json"));
	}

	#[test]
	fn normalizes_backslash_separators() {
		let out = extracted(classify(
			"overrides\\config\\foo.json",
			"overrides",
			"/pack",
		))
		.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("/pack/config/foo.json"));
	}

	#[test]
	fn skips_entry_outside_prefix() {
		assert!(matches!(
			classify("mods/sodium.jar", "overrides", "/pack"),
			ExtractDecision::Skip
		));
	}

	#[test]
	fn skips_prefix_substring_attack() {
		// "overrides_evil/" must not match "overrides" — the historic
		// string-prefix check would have accepted this.
		assert!(matches!(
			classify("overrides_evil/foo.txt", "overrides", "/pack"),
			ExtractDecision::Skip
		));
	}

	#[test]
	fn skips_sibling_override_dir_with_shared_prefix() {
		// Same component-boundary rule rejects "client-overrides/" when the
		// caller asked specifically for "overrides".
		assert!(matches!(
			classify("client-overrides/foo.txt", "overrides", "/pack"),
			ExtractDecision::Skip
		));
	}

	#[test]
	fn skips_directory_marker_entries() {
		assert!(matches!(
			classify("overrides/", "overrides", "/pack"),
			ExtractDecision::Skip
		));
		assert!(matches!(
			classify("overrides/subdir/", "overrides", "/pack"),
			ExtractDecision::Skip
		));
	}

	#[test]
	fn skips_bare_prefix_entry() {
		assert!(matches!(
			classify("overrides", "overrides", "/pack"),
			ExtractDecision::Skip
		));
	}

	#[test]
	fn rejects_parent_dir_traversal() {
		assert!(matches!(
			classify("overrides/../etc/passwd", "overrides", "/pack"),
			ExtractDecision::Unsafe
		));
	}

	#[test]
	fn rejects_deep_traversal_that_resolves_outside() {
		assert!(matches!(
			classify("overrides/sub/../../etc/passwd", "overrides", "/pack"),
			ExtractDecision::Unsafe
		));
	}

	#[test]
	fn rejects_traversal_that_resolves_inside() {
		// Even if `sub/../foo.json` happens to clean to `foo.json` inside
		// the destination, the `..` component is still a hostile signal.
		assert!(matches!(
			classify("overrides/sub/../foo.json", "overrides", "/pack"),
			ExtractDecision::Unsafe
		));
	}

	#[test]
	fn absolute_path_does_not_match_prefix() {
		// An absolute zip entry name does not start with the relative prefix
		// in component form, so it is silently skipped rather than extracted.
		assert!(matches!(
			classify("/overrides/foo.txt", "overrides", "/pack"),
			ExtractDecision::Skip
		));
	}

	#[test]
	fn allows_curdir_components() {
		// `./` segments inside the relative path are harmless and get
		// normalized away by PathClean.
		let out =
			extracted(classify("overrides/./foo.txt", "overrides", "/pack"))
				.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("/pack/foo.txt"));
	}

	#[test]
	fn dest_root_with_dot_prefix_still_matches() {
		// Regression: when the modpack root is passed as `./pack`, the
		// previous string-based check rejected every otherwise-valid file.
		let out =
			extracted(classify("overrides/foo.txt", "overrides", "./pack"))
				.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("pack/foo.txt"));
	}

	#[test]
	fn dest_root_with_trailing_slash_still_matches() {
		let out =
			extracted(classify("overrides/foo.txt", "overrides", "/pack/"))
				.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("/pack/foo.txt"));
	}

	#[test]
	fn ympk_config_prefix_extracts_correctly() {
		// YMPK passes `dest_root = root.join("config")` and prefix "config".
		let out = extracted(classify(
			"config/sodium/options.json",
			"config",
			"/pack/config",
		))
		.expect("expected extract decision");
		assert_eq!(out, PathBuf::from("/pack/config/sodium/options.json"));
	}
}
