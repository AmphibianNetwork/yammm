//! Shared dependency installation logic used by `add` and `import` commands.
//!
//! Provides categorization of resolved dependencies, presentation to the user,
//! interactive prompting, and installation via the service layer.

use std::sync::Arc;

use super::mod_install::install_mod;
use crate::output;
use crate::providers::SourceRegistry;
use crate::storage::Storage;
use crate::types::{
	Dependency, DependencyKind, LoaderType, ModSource, ProjectType,
};
use crate::utils::slugify;

use super::resolver::ResolvedMod;

/// Info about a single dependency to present to the user.
#[derive(Debug, Clone)]
pub struct DepInfo {
	pub identifier: String,
	pub name: String,
	pub description: String,
	pub url: String,
	pub version: Option<String>,
	pub required_by: Option<String>,
}

/// Categorized result of dependency resolution.
///
/// Separates deps into buckets for different handling:
/// - `missing_required`: need to be installed, will prompt user
/// - `missing_optional`: can be skipped, will prompt user
/// - `dep_entries`: metadata to record in the parent mod's .ron
/// - `incompatible_warnings`: mods that conflict with installed ones
#[derive(Debug)]
pub struct CategorizedDeps {
	pub missing_required: Vec<DepInfo>,
	pub missing_optional: Vec<DepInfo>,
	pub dep_entries: Vec<Dependency>,
	pub incompatible_warnings: Vec<String>,
}

/// Context for installing resolved dependencies.
pub struct DepInstallContext {
	pub storage: Arc<Storage>,
	pub registry: Arc<SourceRegistry>,
	pub mc_version: Option<String>,
	pub loader: Option<LoaderType>,
}

/// Pure categorization of resolved mods into required/optional/incompatible.
///
/// `root_mod_ids` contains the IDs of root mods that should be excluded from
/// the results (they're already being added/imported). For single-root resolution,
/// this is typically one ID; for batched resolution, multiple.
pub fn categorize_deps(
	resolved_mods: Vec<ResolvedMod>,
	root_mod_ids: &[String],
	storage: &Storage,
) -> CategorizedDeps {
	let mut missing_required: Vec<DepInfo> = Vec::new();
	let mut missing_optional: Vec<DepInfo> = Vec::new();
	let mut dep_entries: Vec<Dependency> = Vec::new();
	let mut incompatible_warnings: Vec<String> = Vec::new();
	let mut seen_dep_slugs: std::collections::HashSet<String> =
		std::collections::HashSet::new();

	for resolved in resolved_mods {
		if root_mod_ids.contains(&resolved.mod_id) {
			continue;
		}

		if resolved.dependency_type == DependencyKind::Embedded {
			continue;
		}

		if resolved.dependency_type == DependencyKind::Incompatible {
			let slug = slugify(&resolved.mod_id);
			if storage.exists(ProjectType::Mod, &slug) {
				incompatible_warnings.push(format!(
					"Incompatible mod '{}' is installed — this may cause conflicts with {}",
					resolved.name.as_deref().unwrap_or(&resolved.mod_id),
					resolved
						.required_by
						.as_deref()
						.unwrap_or("the mod you are adding")
				));
			}
			continue;
		}

		let slug = slugify(&resolved.mod_id);

		if seen_dep_slugs.contains(&slug) {
			if let Some(existing) =
				dep_entries.iter_mut().find(|d| d.mod_id == slug)
			{
				append_required_by(
					&mut existing.required_by,
					&resolved.required_by,
				);
			}
			let target = if resolved.dependency_type.is_required() {
				&mut missing_required
			} else {
				&mut missing_optional
			};
			if let Some(list) = target.iter_mut().find(|d| {
				slugify(&d.name) == slug
					|| d.identifier.ends_with(&format!(":{}", slug))
			}) {
				append_required_by(
					&mut list.required_by,
					&resolved.required_by,
				);
			}
			continue;
		}

		seen_dep_slugs.insert(slug.clone());

		let dep = Dependency::new(
			slug.clone(),
			resolved.source.clone(),
			resolved.dependency_type,
		)
		.with_required_by(
			resolved
				.required_by
				.clone()
				.unwrap_or_else(|| root_mod_ids.join(", ")),
		);
		dep_entries.push(dep);

		if storage.exists(ProjectType::Mod, &slug) {
			continue;
		}

		if !resolved.source.requires_api() {
			continue;
		}

		let dep_info = DepInfo {
			identifier: format!("{}:{}", resolved.source.as_str(), slug),
			name: resolved.name.unwrap_or_else(|| resolved.mod_id.clone()),
			description: resolved
				.description
				.unwrap_or_default()
				.chars()
				.take(80)
				.collect(),
			url: resolved.url.unwrap_or_default(),
			version: resolved.version.clone(),
			required_by: resolved.required_by,
		};

		if resolved.dependency_type.is_required() {
			missing_required.push(dep_info);
		} else {
			missing_optional.push(dep_info);
		}
	}

	CategorizedDeps {
		missing_required,
		missing_optional,
		dep_entries,
		incompatible_warnings,
	}
}

fn append_required_by(
	field: &mut Option<String>,
	new_parent: &Option<String>,
) {
	if let Some(parent) = new_parent
		&& !field.as_ref().is_some_and(|p| p.contains(parent))
	{
		let updated = format!("{}, {}", field.as_deref().unwrap_or(""), parent);
		*field = Some(updated);
	}
}

pub fn present_incompatible_warnings(warnings: &[String]) {
	for warning in warnings {
		output::warning(warning);
	}
}

pub fn present_dep(dep: &DepInfo) {
	let ver = dep.version.as_deref().unwrap_or("latest");
	if let Some(ref parent) = dep.required_by {
		output::bullet(format!(
			"{} v{} (via {})",
			console::Style::new().bold().apply_to(&dep.name),
			console::Style::new().dim().apply_to(ver),
			parent
		));
	} else {
		output::bullet(format!(
			"{} v{}",
			console::Style::new().bold().apply_to(&dep.name),
			console::Style::new().dim().apply_to(ver)
		));
	}
	if !dep.description.is_empty() {
		output::dim(format!("      {}", dep.description));
	}
	if !dep.url.is_empty() {
		output::dim(format!("      {}", dep.url));
	}
}

pub async fn prompt_and_install_deps(
	heading: &str,
	deps: &[DepInfo],
	default_confirm: bool,
	yes: bool,
	force: bool,
	ctx: &DepInstallContext,
) -> anyhow::Result<Vec<DepInfo>> {
	if deps.is_empty() {
		return Ok(Vec::new());
	}

	output::blank_line();
	output::heading(heading);

	let auto_answer = yes && default_confirm;
	let mut accepted: Vec<DepInfo> = Vec::new();

	for dep in deps {
		present_dep(dep);

		let should_add = if auto_answer {
			true
		} else if yes && !default_confirm {
			false
		} else {
			dialoguer::Confirm::new()
				.with_prompt(format!("Install {}?", dep.name))
				.default(default_confirm)
				.interact()?
		};

		if should_add {
			accepted.push(dep.clone());
		}
	}

	if !accepted.is_empty() {
		install_deps(&accepted, force, ctx).await?;
	}

	Ok(accepted)
}

pub async fn install_deps(
	deps: &[DepInfo],
	force: bool,
	ctx: &DepInstallContext,
) -> anyhow::Result<()> {
	if deps.len() <= 1 {
		for dep in deps {
			install_single_dep(dep, force, ctx).await?;
		}
		return Ok(());
	}

	let sem = Arc::new(tokio::sync::Semaphore::new(8));
	let mut tasks = tokio::task::JoinSet::new();

	for dep in deps {
		let dep = dep.clone();
		let storage = ctx.storage.clone();
		let registry = ctx.registry.clone();
		let mc_version = ctx.mc_version.clone();
		let loader = ctx.loader;
		let permit = sem.clone().acquire_owned().await.map_err(|_| {
			crate::errors::YammmError::general("semaphore closed")
		})?;

		tasks.spawn(async move {
			let _permit = permit;
			let dep_mod_source = dep
				.identifier
				.parse::<ModSource>()
				.unwrap_or_else(|_| ModSource::modrinth(&dep.identifier));
			let version_req = dep
				.version
				.as_deref()
				.map(crate::types::VersionReq::parse)
				.transpose()?;
			install_mod(
				&dep_mod_source,
				version_req.as_ref(),
				force,
				&storage,
				mc_version.as_deref(),
				loader,
				registry,
			)
			.await
		});
	}

	let mut errors: Vec<anyhow::Error> = Vec::new();
	while let Some(result) = tasks.join_next().await {
		match result {
			Ok(Ok(_)) => {}
			Ok(Err(e)) => errors.push(e),
			Err(e) => {
				tracing::warn!("dep install task failed: {e}");
				errors.push(
					crate::errors::YammmError::general(format!(
						"task failed: {e}"
					))
					.into(),
				);
			}
		}
	}

	if let Some(first) = errors.into_iter().next() {
		return Err(first);
	}
	Ok(())
}

async fn install_single_dep(
	dep: &DepInfo,
	force: bool,
	ctx: &DepInstallContext,
) -> anyhow::Result<()> {
	let dep_mod_source = dep
		.identifier
		.parse::<ModSource>()
		.unwrap_or_else(|_| ModSource::modrinth(&dep.identifier));
	let version_req = match dep.version.as_deref() {
		Some(v) => Some(crate::types::VersionReq::parse(v)?),
		None => None,
	};
	install_mod(
		&dep_mod_source,
		version_req.as_ref(),
		force,
		&ctx.storage,
		ctx.mc_version.as_deref(),
		ctx.loader,
		ctx.registry.clone(),
	)
	.await?;
	Ok(())
}

/// Record dependency edges in a parent mod's .ron file.
pub fn record_dep_edges(
	storage: &Storage,
	project_type: ProjectType,
	slug: &str,
	dep_entries: &[Dependency],
) -> anyhow::Result<()> {
	if dep_entries.is_empty() {
		return Ok(());
	}
	if let Ok(mut mod_ron) = storage.load(project_type, slug) {
		for dep in dep_entries {
			if !mod_ron.dependencies.iter().any(|d| d.mod_id == dep.mod_id) {
				mod_ron.dependencies.push(dep.clone());
			}
		}
		storage.save(project_type, slug, &mod_ron)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::ModSource;
	use tempfile::TempDir;

	fn make_resolved_mod(
		mod_id: &str,
		dep_type: DependencyKind,
	) -> ResolvedMod {
		ResolvedMod {
			mod_id: mod_id.to_string(),
			name: Some(mod_id.to_string()),
			description: Some(String::new()),
			url: Some(format!("https://example.com/{}", mod_id)),
			source: ModSource::modrinth(mod_id),
			version: Some("1.0.0".to_string()),
			version_id: None,
			dependency_type: dep_type,
			required_by: None,
		}
	}

	fn make_storage() -> (TempDir, Storage) {
		let temp_dir = TempDir::new().unwrap();
		let config = crate::config::ModpackManifest::new();
		let storage = Storage::new(temp_dir.path(), &config);
		(temp_dir, storage)
	}

	#[test]
	fn test_categorize_deps_skips_root_mod() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("root-mod", DependencyKind::Required)];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert!(result.missing_required.is_empty());
		assert!(result.missing_optional.is_empty());
	}

	#[test]
	fn test_categorize_deps_required_dep() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("dep-a", DependencyKind::Required)];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert_eq!(result.missing_required.len(), 1);
		assert_eq!(result.missing_required[0].name, "dep-a");
	}

	#[test]
	fn test_categorize_deps_optional_dep() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("dep-b", DependencyKind::Optional)];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert!(result.missing_required.is_empty());
		assert_eq!(result.missing_optional.len(), 1);
	}

	#[test]
	fn test_categorize_deps_skips_embedded() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("dep-c", DependencyKind::Embedded)];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert!(result.missing_required.is_empty());
		assert!(result.missing_optional.is_empty());
	}

	#[test]
	fn test_categorize_deps_skips_already_installed() {
		let (_temp_dir, storage) = make_storage();
		let mod_ron = crate::test_util::make_test_mod("dep-d", "Dep D");
		storage.save(ProjectType::Mod, "dep-d", &mod_ron).unwrap();
		let resolved =
			vec![make_resolved_mod("dep-d", DependencyKind::Required)];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert!(result.missing_required.is_empty());
	}

	#[test]
	fn test_categorize_deps_incompatible_warning() {
		let (_temp_dir, storage) = make_storage();
		let mod_ron = crate::test_util::make_test_mod("dep-e", "Dep E");
		storage.save(ProjectType::Mod, "dep-e", &mod_ron).unwrap();
		let resolved =
			vec![make_resolved_mod("dep-e", DependencyKind::Incompatible)];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert_eq!(result.incompatible_warnings.len(), 1);
	}

	#[test]
	fn test_categorize_deps_mixed() {
		let (_temp_dir, storage) = make_storage();
		let resolved = vec![
			make_resolved_mod("req-a", DependencyKind::Required),
			make_resolved_mod("opt-b", DependencyKind::Optional),
			make_resolved_mod("emb-c", DependencyKind::Embedded),
		];
		let result =
			categorize_deps(resolved, &["root-mod".to_string()], &storage);
		assert_eq!(result.missing_required.len(), 1);
		assert_eq!(result.missing_optional.len(), 1);
		assert_eq!(result.dep_entries.len(), 2);
	}

	#[test]
	fn test_categorize_deps_multiple_roots() {
		let (_temp_dir, storage) = make_storage();
		let resolved = vec![
			make_resolved_mod("root-a", DependencyKind::Required),
			make_resolved_mod("root-b", DependencyKind::Required),
			make_resolved_mod("dep-a", DependencyKind::Required),
		];
		let result = categorize_deps(
			resolved,
			&["root-a".to_string(), "root-b".to_string()],
			&storage,
		);
		assert_eq!(result.missing_required.len(), 1);
		assert_eq!(result.missing_required[0].name, "dep-a");
	}
}
