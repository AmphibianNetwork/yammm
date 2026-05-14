//! Reverse-dependency lookup and stale-dependency cleanup.
//!
//! Shared by the `remove` command and the `manage` TUI so both use the
//! same logic for finding dependents and cleaning up dangling references.

use crate::storage::Storage;
use crate::types::{ModSource, ProjectType};
use crate::utils::slugify;

/// Find all installed mods that declare a dependency on `target_slug`.
///
/// Returns `(slug, name)` pairs for each dependent mod.
pub fn find_reverse_deps(
	storage: &Storage,
	target_slug: &str,
	target_source: &ModSource,
) -> Vec<(String, String)> {
	let mut dependents = Vec::new();
	let all_items = storage.list_all().unwrap_or_default();

	for other_mod in all_items {
		if other_mod.id == target_slug {
			continue;
		}

		for dep in &other_mod.dependencies {
			let dep_slug = slugify(&dep.mod_id);
			let matches_slug =
				dep_slug == target_slug || dep.mod_id == target_slug;
			let matches_source =
				dep.source.source_id() == target_source.source_id();
			if matches_slug || matches_source {
				dependents.push((other_mod.id.clone(), other_mod.name.clone()));
				break;
			}
		}
	}

	dependents
}

/// Remove dangling dependency references from all remaining mods.
///
/// When a mod is removed, other mods that listed it as a dependency still
/// have that entry in their `.ron` file. This function scans all mods and
/// removes references to the deleted mod.
pub fn cleanup_stale_deps(
	storage: &Storage,
	removed_slug: &str,
	removed_source: &ModSource,
) -> anyhow::Result<()> {
	for project_type in ProjectType::VARIANTS {
		for mut mod_ron in
			storage.store_for(*project_type).list().unwrap_or_default()
		{
			let before_len = mod_ron.dependencies.len();
			mod_ron.dependencies.retain(|d| {
				let slug_match = slugify(&d.mod_id) != removed_slug
					&& d.mod_id != removed_slug;
				let source_match =
					d.source.source_id() != removed_source.source_id();
				slug_match && source_match
			});
			if mod_ron.dependencies.len() < before_len {
				storage.save(*project_type, &mod_ron.id, &mod_ron)?;
			}
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::ModpackManifest;
	use crate::types::{Dependency, DependencyKind, ModEnv, TrackedMod};

	fn test_storage() -> (tempfile::TempDir, Storage) {
		let tmp = tempfile::TempDir::new().unwrap();
		let config = ModpackManifest::new();
		let storage = Storage::new(tmp.path(), &config);
		std::fs::create_dir_all(&storage.mods_dir).unwrap();
		(tmp, storage)
	}

	fn make_mod(
		id: &str,
		deps: Vec<Dependency>,
	) -> TrackedMod {
		let mut m = TrackedMod::builder(id, ModSource::modrinth(id))
			.name(id)
			.version("1.0.0")
			.env(ModEnv::Both)
			.build();
		m.dependencies = deps;
		m
	}

	#[test]
	fn test_find_reverse_deps_none() {
		let (_tmp, storage) = test_storage();
		let sodium = make_mod("sodium", vec![]);
		storage.save(ProjectType::Mod, "sodium", &sodium).unwrap();

		let result = find_reverse_deps(
			&storage,
			"lithium",
			&ModSource::modrinth("lithium"),
		);
		assert!(result.is_empty());
	}

	#[test]
	fn test_find_reverse_deps_by_slug() {
		let (_tmp, storage) = test_storage();
		let sodium = make_mod("sodium", vec![]);
		storage.save(ProjectType::Mod, "sodium", &sodium).unwrap();

		let dep = Dependency::new(
			"sodium",
			ModSource::modrinth("sodium"),
			DependencyKind::Required,
		);
		let iris = make_mod("iris", vec![dep]);
		storage.save(ProjectType::Mod, "iris", &iris).unwrap();

		let result = find_reverse_deps(
			&storage,
			"sodium",
			&ModSource::modrinth("sodium"),
		);
		assert_eq!(result.len(), 1);
		assert_eq!(result[0].0, "iris");
	}

	#[test]
	fn test_find_reverse_deps_excludes_self() {
		let (_tmp, storage) = test_storage();
		let dep = Dependency::new(
			"sodium",
			ModSource::modrinth("sodium"),
			DependencyKind::Required,
		);
		let sodium = make_mod("sodium", vec![dep]);
		storage.save(ProjectType::Mod, "sodium", &sodium).unwrap();

		let result = find_reverse_deps(
			&storage,
			"sodium",
			&ModSource::modrinth("sodium"),
		);
		assert!(result.is_empty());
	}

	#[test]
	fn test_cleanup_stale_deps_removes_reference() {
		let (_tmp, storage) = test_storage();
		let sodium = make_mod("sodium", vec![]);
		storage.save(ProjectType::Mod, "sodium", &sodium).unwrap();

		let dep = Dependency::new(
			"sodium",
			ModSource::modrinth("sodium"),
			DependencyKind::Required,
		);
		let iris = make_mod("iris", vec![dep]);
		storage.save(ProjectType::Mod, "iris", &iris).unwrap();

		cleanup_stale_deps(&storage, "sodium", &ModSource::modrinth("sodium"))
			.unwrap();

		let updated = storage.load(ProjectType::Mod, "iris").unwrap();
		assert!(updated.dependencies.is_empty());
	}

	#[test]
	fn test_cleanup_stale_deps_preserves_other_deps() {
		let (_tmp, storage) = test_storage();
		let sodium = make_mod("sodium", vec![]);
		storage.save(ProjectType::Mod, "sodium", &sodium).unwrap();

		let lithium = make_mod("lithium", vec![]);
		storage.save(ProjectType::Mod, "lithium", &lithium).unwrap();

		let dep_sodium = Dependency::new(
			"sodium",
			ModSource::modrinth("sodium"),
			DependencyKind::Required,
		);
		let dep_lithium = Dependency::new(
			"lithium",
			ModSource::modrinth("lithium"),
			DependencyKind::Optional,
		);
		let iris = make_mod("iris", vec![dep_sodium, dep_lithium]);
		storage.save(ProjectType::Mod, "iris", &iris).unwrap();

		cleanup_stale_deps(&storage, "sodium", &ModSource::modrinth("sodium"))
			.unwrap();

		let updated = storage.load(ProjectType::Mod, "iris").unwrap();
		assert_eq!(updated.dependencies.len(), 1);
		assert_eq!(updated.dependencies[0].mod_id, "lithium");
	}
}
