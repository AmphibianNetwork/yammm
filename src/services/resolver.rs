//! BFS dependency resolver with cycle detection and optional-dep propagation.
//!
//! Required deps are dequeued before optional ones (fail fast).
//! Optional parents downgrade all transitive deps to optional.
//! Cycles are detected via ancestor tracking per queue entry.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::Arc;

use crate::providers::registry::SourceRegistry;
use crate::types::{
	Dependency, DependencyKind, LoaderType, ModIdentity, ModSource, Version,
	VersionFilters,
};

/// A resolved mod from dependency resolution.
#[derive(Debug, Clone)]
pub struct ResolvedMod {
	pub mod_id: String,
	pub name: Option<String>,
	pub description: Option<String>,
	pub url: Option<String>,
	pub source: ModSource,
	pub version: Option<String>,
	pub version_id: Option<String>,
	pub dependency_type: DependencyKind,
	pub required_by: Option<String>,
}

/// Internal queue entry for BFS traversal. `ancestors` enables cycle detection.
struct QueueEntry {
	dep: Dependency,
	ancestors: HashSet<String>,
}

/// Dependency resolver backed by a `SourceRegistry`.
pub struct DependencyResolver {
	registry: Arc<SourceRegistry>,
	minecraft_version: Option<String>,
	loader: Option<LoaderType>,
}

impl DependencyResolver {
	pub fn new(registry: Arc<SourceRegistry>) -> Self {
		Self {
			registry,
			minecraft_version: None,
			loader: None,
		}
	}

	pub fn with_minecraft_version(
		mut self,
		version: impl Into<String>,
	) -> Self {
		self.minecraft_version = Some(version.into());
		self
	}

	pub fn with_loader(
		mut self,
		loader: LoaderType,
	) -> Self {
		self.loader = Some(loader);
		self
	}

	/// Resolve all dependencies for a mod using BFS.
	///
	/// Required deps fail fast; optional deps are silently skipped on error.
	/// Circular deps produce `YammmError::CircularDependency`.
	pub async fn resolve(
		&self,
		mod_id: &str,
		source: ModSource,
	) -> Result<Vec<ResolvedMod>> {
		let mut resolved: HashSet<ModIdentity> = HashSet::new();
		let mut result: Vec<ResolvedMod> = Vec::new();
		let mut queue: Vec<QueueEntry> = Vec::new();

		queue.push(QueueEntry {
			dep: Dependency::new(mod_id, source, DependencyKind::Required),
			ancestors: HashSet::new(),
		});

		while !queue.is_empty() {
			// Priority: required deps first (fail fast).
			let idx = queue
				.iter()
				.position(|e| e.dep.kind.is_required())
				.unwrap_or(0);
			let entry = queue.remove(idx);
			let dep = entry.dep;

			let key = ModIdentity {
				mod_id: dep.mod_id.clone(),
				source: dep.source.clone(),
			};

			// If this mod appears in the ancestor chain, we've got a cycle.
			if entry.ancestors.contains(&key.to_string()) {
				return Err(crate::errors::YammmError::circular_dep(
					key.to_string(),
					format!("{} → ... → {}", mod_id, dep.mod_id),
				)
				.into());
			}

			// Skip if already resolved (deduplicate across multiple parents).
			if resolved.contains(&key) {
				continue;
			}

			match self.resolve_mod(&dep).await {
				Ok(resolved_mod) => {
					resolved.insert(key.clone());
					result.push(resolved_mod.clone());

					// Embedded mods are bundled in the parent JAR — skip their deps.
					if resolved_mod.dependency_type == DependencyKind::Embedded
					{
						continue;
					}

					if let Err(e) = self
						.queue_dependencies(
							&resolved_mod,
							&mut queue,
							&entry.ancestors,
							&key,
						)
						.await
					{
						tracing::warn!(
							"Could not fetch dependencies for {}: {}",
							dep.mod_id,
							e
						);
					}
				}
				Err(e) => {
					if dep.kind.is_required() {
						tracing::warn!(
							"Required dependency resolution failed: {}",
							e
						);
						return Err(e);
					} else {
						tracing::debug!(
							"Optional dependency resolution failed: {}",
							e
						);
					}
				}
			}
		}

		Ok(result)
	}

	/// Queue dependencies for a resolved mod.
	/// Optional parents downgrade children to optional.
	async fn queue_dependencies(
		&self,
		resolved: &ResolvedMod,
		queue: &mut Vec<QueueEntry>,
		parent_ancestors: &HashSet<String>,
		parent_key: &ModIdentity,
	) -> Result<()> {
		let provider = self.registry.get(&resolved.source)?;

		let version_id = match &resolved.version_id {
			Some(vid) => vid.clone(),
			None => {
				let filters = VersionFilters {
					minecraft_version: self.minecraft_version.clone(),
					loader: self.loader,
				};
				let version = provider
					.get_latest_version(&resolved.mod_id, &filters)
					.await
					.context(format!(
						"Could not resolve version for {}",
						resolved.mod_id
					))?;
				let version_id = match version.version_id {
					Some(id) => id,
					None => {
						tracing::warn!(
							"No version ID for {}, skipping dependency resolution",
							resolved.mod_id
						);
						return Ok(());
					}
				};
				version_id
			}
		};

		let deps = provider
			.get_dependencies(&resolved.mod_id, &version_id)
			.await
			.context("Failed to fetch dependencies")?;

		// Build ancestor set for the next level: parent's ancestors + parent.
		let mut new_ancestors = parent_ancestors.clone();
		new_ancestors.insert(parent_key.to_string());

		for dep in deps {
			// A mod should not depend on itself.
			if dep.mod_id == resolved.mod_id {
				continue;
			}

			match dep.dep_type {
				DependencyKind::Embedded => {
					tracing::debug!(
						"Skipping embedded dependency: {}",
						dep.mod_id
					);
					continue;
				}
				DependencyKind::Incompatible => {
					tracing::debug!(
						"Skipping incompatible dependency: {}",
						dep.mod_id
					);
					continue;
				}
				_ => {}
			}

			let dep_type = dep.dep_type;

			// Inherit parent's source if the dependency doesn't specify one.
			let dep_source =
				dep.source.unwrap_or_else(|| resolved.source.clone());

			// Downgrade: children of optional parents become optional.
			let effective_dep_type =
				if resolved.dependency_type == DependencyKind::Optional {
					DependencyKind::Optional
				} else {
					dep_type
				};

			queue.push(QueueEntry {
				dep: Dependency {
					mod_id: dep.mod_id,
					source: dep_source,
					kind: effective_dep_type,
					version: None,
					required_by: Some(
						resolved
							.name
							.clone()
							.unwrap_or(resolved.mod_id.clone()),
					),
				},
				ancestors: new_ancestors.clone(),
			});
		}

		Ok(())
	}

	/// Resolve a single dependency to a `ResolvedMod`.
	async fn resolve_mod(
		&self,
		dep: &Dependency,
	) -> Result<ResolvedMod> {
		let provider = self
			.registry
			.get(&dep.source)
			.with_context(|| format!("Mod not found: {}", dep.mod_id))?;

		let mod_info = provider
			.get_mod(&dep.mod_id)
			.await
			.with_context(|| format!("Mod not found: {}", dep.mod_id))?;

		let filters = VersionFilters {
			minecraft_version: self.minecraft_version.clone(),
			loader: self.loader,
		};

		let version = provider
			.get_latest_version(&dep.mod_id, &filters)
			.await
			.with_context(|| {
				format!("No matching version for {}", dep.mod_id)
			})?;

		if let Some(ref req) = dep.version {
			let ver = Version::parse(&version.version)
				.context("Invalid version string from provider")?;
			if !req.satisfies(&ver) {
				return Err(crate::errors::YammmError::version_conflict(
					format!("No version of {} satisfies {}", dep.mod_id, req),
				)
				.into());
			}
		}

		Ok(ResolvedMod {
			mod_id: mod_info.id.clone(),
			name: Some(mod_info.name.clone()),
			description: Some(mod_info.description.clone()),
			url: Some(mod_info.url.clone()),
			source: mod_info.source.clone(),
			version: Some(version.version.clone()),
			version_id: version.version_id.clone(),
			dependency_type: dep.kind,
			required_by: dep.required_by.clone(),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::providers::mock::MockSource;
	use crate::providers::registry::SourceRegistry;
	use crate::test_util;

	fn setup_resolver() -> (DependencyResolver, MockSource) {
		let mock = MockSource::new();
		let registry = SourceRegistry::new_with_mock(mock.clone());
		let resolver = DependencyResolver::new(Arc::new(registry));
		(resolver, mock)
	}

	#[test]
	fn test_embedded_not_collapsed_to_required() {
		let dt: DependencyKind = "embedded".parse().unwrap();
		assert_eq!(dt, DependencyKind::Embedded);
		assert_ne!(dt, DependencyKind::Required);
	}

	#[tokio::test]
	async fn test_bfs_resolution_with_required_deps() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("A", test_util::make_mod_info("A"));
		mock.add_mod("B", test_util::make_mod_info("B"));
		mock.add_mod("C", test_util::make_mod_info("C"));

		mock.add_versions("A", vec![test_util::make_version("1.0", "vid-a")]);
		mock.add_versions("B", vec![test_util::make_version("1.0", "vid-b")]);
		mock.add_versions("C", vec![test_util::make_version("1.0", "vid-c")]);

		mock.add_deps(
			"vid-a",
			vec![test_util::make_dep("B", DependencyKind::Required)],
		);
		mock.add_deps(
			"vid-b",
			vec![test_util::make_dep("C", DependencyKind::Required)],
		);
		mock.add_deps("vid-c", vec![]);

		let result = resolver
			.resolve("A", ModSource::modrinth("A"))
			.await
			.unwrap();

		assert_eq!(result.len(), 3);

		let ids: Vec<&str> = result.iter().map(|m| m.mod_id.as_str()).collect();
		assert!(ids.contains(&"A"));
		assert!(ids.contains(&"B"));
		assert!(ids.contains(&"C"));

		for m in &result {
			assert_eq!(m.dependency_type, DependencyKind::Required);
		}
	}

	#[tokio::test]
	async fn test_optional_dep_downgrade() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("A", test_util::make_mod_info("A"));
		mock.add_mod("B", test_util::make_mod_info("B"));
		mock.add_mod("C", test_util::make_mod_info("C"));

		mock.add_versions("A", vec![test_util::make_version("1.0", "vid-a")]);
		mock.add_versions("B", vec![test_util::make_version("1.0", "vid-b")]);
		mock.add_versions("C", vec![test_util::make_version("1.0", "vid-c")]);

		mock.add_deps(
			"vid-a",
			vec![test_util::make_dep("B", DependencyKind::Optional)],
		);
		mock.add_deps(
			"vid-b",
			vec![test_util::make_dep("C", DependencyKind::Required)],
		);
		mock.add_deps("vid-c", vec![]);

		let result = resolver
			.resolve("A", ModSource::modrinth("A"))
			.await
			.unwrap();

		assert_eq!(result.len(), 3);

		let a = result.iter().find(|m| m.mod_id == "A").unwrap();
		assert_eq!(a.dependency_type, DependencyKind::Required);

		let b = result.iter().find(|m| m.mod_id == "B").unwrap();
		assert_eq!(b.dependency_type, DependencyKind::Optional);

		let c = result.iter().find(|m| m.mod_id == "C").unwrap();
		assert_eq!(
			c.dependency_type,
			DependencyKind::Optional,
			"children of optional parents should be downgraded to Optional"
		);
	}

	#[tokio::test]
	async fn test_circular_dependency_detection() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("A", test_util::make_mod_info("A"));
		mock.add_mod("B", test_util::make_mod_info("B"));

		mock.add_versions("A", vec![test_util::make_version("1.0", "vid-a")]);
		mock.add_versions("B", vec![test_util::make_version("1.0", "vid-b")]);

		mock.add_deps(
			"vid-a",
			vec![test_util::make_dep("B", DependencyKind::Required)],
		);
		mock.add_deps(
			"vid-b",
			vec![test_util::make_dep("A", DependencyKind::Required)],
		);

		let result = resolver.resolve("A", ModSource::modrinth("A")).await;

		assert!(result.is_err());
		let err = result.unwrap_err();
		let yammm_err = err.downcast_ref::<crate::errors::YammmError>();
		assert!(
			matches!(
				yammm_err,
				Some(crate::errors::YammmError::CircularDependency { .. })
			),
			"expected CircularDependency error, got: {:?}",
			yammm_err
		);
	}

	#[tokio::test]
	async fn test_multiple_sources_for_same_mod() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("A", test_util::make_mod_info("A"));
		mock.add_mod("B", test_util::make_mod_info("B"));
		mock.add_mod("C", test_util::make_mod_info("C"));

		mock.add_versions("A", vec![test_util::make_version("1.0", "vid-a")]);
		mock.add_versions("B", vec![test_util::make_version("1.0", "vid-b")]);
		mock.add_versions("C", vec![test_util::make_version("1.0", "vid-c")]);

		mock.add_deps(
			"vid-a",
			vec![
				test_util::make_dep_with_source(
					"B",
					DependencyKind::Required,
					ModSource::modrinth("B"),
				),
				test_util::make_dep_with_source(
					"C",
					DependencyKind::Required,
					ModSource::modrinth("C"),
				),
			],
		);
		mock.add_deps(
			"vid-b",
			vec![test_util::make_dep_with_source(
				"C",
				DependencyKind::Required,
				ModSource::modrinth("C"),
			)],
		);
		mock.add_deps("vid-c", vec![]);

		let result = resolver
			.resolve("A", ModSource::modrinth("A"))
			.await
			.unwrap();

		let c_count = result.iter().filter(|m| m.mod_id == "C").count();
		assert_eq!(c_count, 1, "mod C should appear exactly once");

		let c = result.iter().find(|m| m.mod_id == "C").unwrap();
		assert_eq!(c.dependency_type, DependencyKind::Required);
	}

	#[tokio::test]
	async fn test_empty_mod_list() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("standalone", test_util::make_mod_info("standalone"));
		mock.add_versions(
			"standalone",
			vec![test_util::make_version("1.0", "vid-standalone")],
		);
		mock.add_deps("vid-standalone", vec![]);

		let result = resolver
			.resolve("standalone", ModSource::modrinth("standalone"))
			.await
			.unwrap();

		assert_eq!(result.len(), 1);
		assert_eq!(result[0].mod_id, "standalone");
		assert_eq!(result[0].dependency_type, DependencyKind::Required);
	}

	#[tokio::test]
	async fn test_optional_deps_of_required_parents_stay_optional() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("A", test_util::make_mod_info("A"));
		mock.add_mod("B", test_util::make_mod_info("B"));

		mock.add_versions("A", vec![test_util::make_version("1.0", "vid-a")]);
		mock.add_versions("B", vec![test_util::make_version("1.0", "vid-b")]);

		mock.add_deps(
			"vid-a",
			vec![test_util::make_dep("B", DependencyKind::Optional)],
		);
		mock.add_deps("vid-b", vec![]);

		let result = resolver
			.resolve("A", ModSource::modrinth("A"))
			.await
			.unwrap();

		assert_eq!(result.len(), 2);

		let a = result.iter().find(|m| m.mod_id == "A").unwrap();
		assert_eq!(a.dependency_type, DependencyKind::Required);

		let b = result.iter().find(|m| m.mod_id == "B").unwrap();
		assert_eq!(
			b.dependency_type,
			DependencyKind::Optional,
			"optional deps of required parents should stay Optional"
		);
	}

	#[tokio::test]
	async fn test_longer_circular_dependency() {
		let (resolver, mock) = setup_resolver();

		mock.add_mod("A", test_util::make_mod_info("A"));
		mock.add_mod("B", test_util::make_mod_info("B"));
		mock.add_mod("C", test_util::make_mod_info("C"));

		mock.add_versions("A", vec![test_util::make_version("1.0", "vid-a")]);
		mock.add_versions("B", vec![test_util::make_version("1.0", "vid-b")]);
		mock.add_versions("C", vec![test_util::make_version("1.0", "vid-c")]);

		mock.add_deps(
			"vid-a",
			vec![test_util::make_dep("B", DependencyKind::Required)],
		);
		mock.add_deps(
			"vid-b",
			vec![test_util::make_dep("C", DependencyKind::Required)],
		);
		mock.add_deps(
			"vid-c",
			vec![test_util::make_dep("A", DependencyKind::Required)],
		);

		let result = resolver.resolve("A", ModSource::modrinth("A")).await;

		assert!(result.is_err());
		let err = result.unwrap_err();
		let yammm_err = err.downcast_ref::<crate::errors::YammmError>();
		assert!(matches!(
			yammm_err,
			Some(crate::errors::YammmError::CircularDependency { .. })
		));
	}
}
