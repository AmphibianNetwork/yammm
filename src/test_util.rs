//! Shared test helpers to reduce duplication across test modules.
//!
//! Provides factory functions (`make_mod_info`, `make_version`, etc.) that
//! create domain objects with sensible defaults for use in unit tests.
//! These are only compiled under `#[cfg(test)]`.

use crate::types::SourceDependency;
use crate::types::{
	DependencyKind, HashType, ModInfo, ModSource, ModVersion, ProjectType,
	TrackedMod,
};

pub fn make_mod_info(id: &str) -> ModInfo {
	ModInfo {
		id: id.to_string(),
		name: id.to_string(),
		description: format!("Test mod {}", id),
		source: ModSource::modrinth(id),
		minecraft_versions: vec![],
		loaders: vec![],
		downloads: 0,
		url: format!("https://example.com/{}", id),
		project_type: None,
		client_side: None,
		server_side: None,
	}
}

pub fn make_version(
	version: &str,
	version_id: &str,
) -> ModVersion {
	ModVersion {
		version_id: Some(version_id.to_string()),
		version: version.to_string(),
		minecraft_versions: vec!["1.20.4".to_string()],
		loaders: vec!["fabric".to_string()],
		download_url: format!("https://example.com/mod-{}.jar", version),
		hash: None,
		hash_type: HashType::Sha512,
		file_size: 1000,
		release_date: "2024-01-01".to_string(),
	}
}

pub fn make_dep(
	mod_id: &str,
	dep_type: DependencyKind,
) -> SourceDependency {
	SourceDependency {
		mod_id: mod_id.to_string(),
		version_id: None,
		dep_type,
		source: None,
	}
}

pub fn make_dep_with_source(
	mod_id: &str,
	dep_type: DependencyKind,
	source: ModSource,
) -> SourceDependency {
	SourceDependency {
		mod_id: mod_id.to_string(),
		version_id: None,
		dep_type,
		source: Some(source),
	}
}

pub fn make_test_mod(
	id: &str,
	name: &str,
) -> TrackedMod {
	TrackedMod::builder(id, ModSource::modrinth(id))
		.name(name)
		.description(format!("desc for {}", id))
		.version("1.0.0")
		.url(format!("https://modrinth.com/mod/{}", id))
		.download_url(format!("https://cdn.modrinth.com/{}.jar", id))
		.hash(Some("a".repeat(128)))
		.hash_type(HashType::Sha512)
		.project_type(ProjectType::Mod)
		.build()
}
