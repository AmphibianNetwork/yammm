use crate::storage::Storage;
use crate::types::{DependencyKind, ModSource, ProjectType, TrackedMod};
use crate::utils::slugify;

use super::app::DepNode;

pub fn build_dep_tree(
	storage: &Storage,
	root_id: &str,
	root_source: &ModSource,
	max_depth: usize,
) -> Vec<DepNode> {
	let mut visited = std::collections::HashSet::new();
	build_dep_tree_recursive(
		storage,
		root_id,
		root_source,
		None,
		max_depth,
		&mut visited,
	)
}

fn build_dep_tree_recursive(
	storage: &Storage,
	mod_id: &str,
	source: &ModSource,
	kind: Option<DependencyKind>,
	remaining_depth: usize,
	visited: &mut std::collections::HashSet<String>,
) -> Vec<DepNode> {
	if remaining_depth == 0 {
		return Vec::new();
	}

	if visited.contains(mod_id) {
		return Vec::new();
	}
	visited.insert(mod_id.to_string());

	let tracked = find_mod_by_source(storage, mod_id, source);

	let name = tracked
		.as_ref()
		.map(|m| m.name.clone())
		.unwrap_or_else(|| mod_id.to_string());
	let version = tracked
		.as_ref()
		.map(|m| m.version.clone())
		.unwrap_or_default();

	let deps = tracked
		.as_ref()
		.map(|m| m.dependencies.clone())
		.unwrap_or_default();

	let children: Vec<DepNode> = deps
		.iter()
		.flat_map(|dep| {
			let dep_slug = slugify(&dep.mod_id);
			let mut sub = build_dep_tree_recursive(
				storage,
				&dep_slug,
				&dep.source,
				Some(dep.kind),
				remaining_depth - 1,
				visited,
			);
			for node in &mut sub {
				if node.kind.is_none() {
					node.kind = Some(dep.kind);
				}
			}
			sub
		})
		.collect();

	visited.remove(mod_id);

	vec![DepNode {
		mod_id: mod_id.to_string(),
		name,
		version,
		kind,
		children,
		expanded: true,
	}]
}

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

pub fn cleanup_stale_deps_after_remove(
	storage: &Storage,
	removed_slug: &str,
	removed_source: &ModSource,
) {
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
				let _ = storage.save(*project_type, &mod_ron.id, &mod_ron);
			}
		}
	}
}

fn find_mod_by_source(
	storage: &Storage,
	slug: &str,
	source: &ModSource,
) -> Option<TrackedMod> {
	for pt in ProjectType::VARIANTS {
		if let Ok(m) = storage.load(*pt, slug) {
			return Some(m);
		}
	}
	if let Ok((_, m)) = storage.find_any(slug) {
		return Some(m);
	}
	let _ = source;
	None
}
