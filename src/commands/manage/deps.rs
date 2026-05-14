use crate::services::{cleanup_stale_deps, find_reverse_deps};
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
	let installed = tracked.is_some();

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
		installed,
		source: source.clone(),
		children,
	}]
}

pub fn find_reverse_dependents(
	storage: &Storage,
	target_slug: &str,
	target_source: &ModSource,
) -> Vec<(String, String)> {
	find_reverse_deps(storage, target_slug, target_source)
}

pub fn flatten_dep_tree(
	tree: &[super::app::DepNode],
	indent: usize,
) -> Vec<super::app::DepEntry> {
	let mut entries = Vec::new();
	for node in tree {
		entries.push(super::app::DepEntry {
			mod_id: node.mod_id.clone(),
			name: node.name.clone(),
			version: node.version.clone(),
			kind: node.kind,
			installed: node.installed,
			source: node.source.clone(),
			indent,
		});
		entries.extend(flatten_dep_tree(&node.children, indent + 1));
	}
	entries
}

pub fn cleanup_stale_deps_after_remove(
	storage: &Storage,
	removed_slug: &str,
	removed_source: &ModSource,
) {
	if let Err(e) = cleanup_stale_deps(storage, removed_slug, removed_source) {
		tracing::warn!("Failed to cleanup stale deps: {e}");
	}
}

fn find_mod_by_source(
	storage: &Storage,
	slug: &str,
	source: &ModSource,
) -> Option<TrackedMod> {
	for pt in ProjectType::VARIANTS {
		if let Ok(m) = storage.load(*pt, slug)
			&& m.source == *source
		{
			return Some(m);
		}
	}
	if let Ok((_, m)) = storage.find_any(slug)
		&& m.source == *source
	{
		return Some(m);
	}
	None
}
