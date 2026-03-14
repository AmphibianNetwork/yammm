//! Add a mod, resource pack, or shader pack to the modpack.
//!
//! The full flow:
//! 1. Resolve the identifier to a `ModSource` (handles `modrinth:`, `cf:`, URLs, etc.)
//! 2. Fetch mod metadata from the provider
//! 3. If already installed, offer to update (or overwrite with `--force`)
//! 4. Resolve the best matching version
//! 5. Save the `.ron` metadata file
//! 6. Resolve and install transitive dependencies (see `resolve_dependencies`)
//!
//! Dependency installation is split into required vs optional:
//! - Required deps are auto-confirmed with `--yes`
//! - Optional deps are skipped with `--yes` unless explicitly asked

use clap::Parser;

use super::CliSource;
use crate::app::AppContext;
use crate::output;
use crate::services::resolver::{DependencyResolver, ResolvedMod};
use crate::types::{
	Dependency, DependencyKind, LoaderType, ModEnv, ModSource, ProjectType,
};
use crate::utils::slugify;

mod sources;
use sources::AddContext;

/// Add a mod, resource pack, or shader pack to the modpack.
#[derive(Parser, Debug)]
pub struct AddCommand {
	/// Mod slug, ID, URL, or `curseforge:`/`modrinth:` prefixed identifier.
	pub identifier: String,

	#[arg(short = 'v', long)]
	pub version: Option<String>,

	#[arg(short = 'f', long)]
	pub force: bool,

	#[arg(short = 's', long, default_value = "modrinth")]
	pub source: CliSource,

	#[arg(short = 'l', long)]
	pub loader: Option<String>,

	#[arg(long)]
	pub env: Option<ModEnv>,

	#[arg(long)]
	pub project_type: Option<ProjectType>,

	#[arg(short = 't', long, value_delimiter = ',')]
	pub categories: Vec<String>,

	#[arg(short = 'y', long)]
	pub yes: bool,
}

impl AddCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> anyhow::Result<()> {
		tracing::debug!("AddCommand running");
		tracing::debug!("Identifier: {}", self.identifier);
		tracing::debug!("Version: {:?}", self.version);

		let app = ctx.require_modpack()?;

		let modpack_config = &app.config;

		let filters = modpack_config.version_filters();

		let minecraft_version = filters
			.minecraft_version
			.as_deref()
			.filter(|s| !s.is_empty());
		let loader = self
			.loader
			.as_deref()
			.and_then(|s| s.parse::<LoaderType>().ok())
			.or(filters.loader);

		tracing::debug!("Using minecraft_version: {:?}", minecraft_version);
		tracing::debug!("Using loader: {:?}", loader);

		let storage = &app.storage;

		let resolved_source = self.source.to_mod_source(&self.identifier);

		let ctx = AddContext {
			source: &resolved_source,
			version_req: AddContext::parse_version(self.version.as_deref())?,
			force: self.force,
			storage,
			mc_version: minecraft_version,
			loader,
			registry: ctx.registry.clone(),
			env_override: self.env,
			project_type_override: self.project_type.as_ref(),
			categories: self.categories.clone(),
		};

		let (main_slug, main_project_type) = ctx.add(&self.identifier).await?;

		let install_ctx = InstallContext {
			storage,
			registry: ctx.registry.clone(),
			mc_version: minecraft_version,
			loader,
		};

		self.resolve_dependencies(
			&install_ctx,
			&main_slug,
			main_project_type,
			&resolved_source,
		)
		.await?;

		output::success("Done");

		Ok(())
	}

	/// Resolve and install transitive dependencies for the just-added mod.
	///
	/// URL sources are excluded from dependency resolution since they don't
	/// have a provider that can return dependency information.
	async fn resolve_dependencies(
		&self,
		ctx: &InstallContext<'_>,
		main_slug: &str,
		main_project_type: ProjectType,
		resolved_source: &ModSource,
	) -> anyhow::Result<()> {
		// URL-based mods don't have a provider that can resolve dependencies.
		if resolved_source.url_str().is_some() {
			return Ok(());
		}
		let mod_id = resolved_source.source_id().to_string();

		let mut resolver = DependencyResolver::new(ctx.registry.clone());

		if let Some(minecraft_version) = ctx.mc_version {
			resolver = resolver.with_minecraft_version(minecraft_version);
		}

		if let Some(loader) = ctx.loader {
			resolver = resolver.with_loader(loader);
		}

		let resolved_mods =
			match resolver.resolve(&mod_id, resolved_source.clone()).await {
				Ok(mods) => mods,
				Err(e) => {
					output::warning(format!(
						"Could not resolve dependencies: {}",
						e
					));
					return Ok(());
				}
			};

		let categorized = categorize_deps(resolved_mods, &mod_id, ctx.storage);

		present_incompatible_warnings(&categorized.incompatible_warnings);

		if !categorized.missing_required.is_empty() {
			prompt_and_install_deps(
				"Required dependencies",
				&categorized.missing_required,
				true,
				self.yes,
				self.force,
				ctx,
			)
			.await?;
		}

		if !categorized.missing_optional.is_empty() {
			prompt_and_install_deps(
				"Optional dependencies",
				&categorized.missing_optional,
				false,
				self.yes,
				self.force,
				ctx,
			)
			.await?;
		}

		// Record dependency edges in the main mod's .ron file.
		// This lets `yammm remove` check reverse dependencies later.
		if !categorized.dep_entries.is_empty() {
			if let Ok(mut main_mod) =
				ctx.storage.load(main_project_type, main_slug)
			{
				for dep in &categorized.dep_entries {
					if !main_mod
						.dependencies
						.iter()
						.any(|d| d.mod_id == dep.mod_id)
					{
						main_mod.dependencies.push(dep.clone());
					}
				}
				ctx.storage.save(main_project_type, main_slug, &main_mod)?;
			}
		}

		Ok(())
	}
}

/// Context for installing resolved dependencies (lighter than `AddContext`).
struct InstallContext<'a> {
	storage: &'a crate::storage::Storage,
	registry: std::sync::Arc<crate::providers::SourceRegistry>,
	mc_version: Option<&'a str>,
	loader: Option<LoaderType>,
}

/// Categorized result of dependency resolution.
///
/// Separates deps into buckets for different handling:
/// - `missing_required`: need to be installed, will prompt user
/// - `missing_optional`: can be skipped, will prompt user
/// - `dep_entries`: metadata to record in the parent mod's .ron
/// - `incompatible_warnings`: mods that conflict with installed ones
#[derive(Debug)]
struct CategorizedDeps {
	missing_required: Vec<DepInfo>,
	missing_optional: Vec<DepInfo>,
	dep_entries: Vec<Dependency>,
	incompatible_warnings: Vec<String>,
}

/// Info about a single dependency to present to the user.
#[derive(Debug)]
struct DepInfo {
	identifier: String,
	name: String,
	description: String,
	url: String,
	version: Option<String>,
	required_by: Option<String>,
}

/// Pure categorization of resolved mods into required/optional/incompatible.
fn categorize_deps(
	resolved_mods: Vec<ResolvedMod>,
	root_mod_id: &str,
	storage: &crate::storage::Storage,
) -> CategorizedDeps {
	let mut missing_required = Vec::new();
	let mut missing_optional = Vec::new();
	let mut dep_entries: Vec<Dependency> = Vec::new();
	let mut incompatible_warnings = Vec::new();

	for resolved in resolved_mods {
		if resolved.mod_id == root_mod_id {
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
					resolved.required_by.as_deref().unwrap_or("the mod you are adding")
				));
			}
			continue;
		}

		let slug = slugify(&resolved.mod_id);
		let dep = Dependency::new(
			slug.clone(),
			resolved.source.clone(),
			resolved.dependency_type,
		)
		.with_required_by(
			resolved
				.required_by
				.clone()
				.unwrap_or_else(|| root_mod_id.to_string()),
		);
		dep_entries.push(dep);

		if storage.exists(ProjectType::Mod, &slug) {
			continue;
		}

		if !resolved.source.requires_api() {
			continue;
		}

		let dep_info = DepInfo {
			identifier: format!(
				"{}:{}",
				resolved.source.as_str(),
				resolved.source.source_id()
			),
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

fn present_incompatible_warnings(warnings: &[String]) {
	for warning in warnings {
		output::warning(warning);
	}
}

fn present_dep_list(
	heading: &str,
	deps: &[DepInfo],
) {
	output::blank_line();
	output::heading(format!("{} ({})", heading, deps.len()));
	for dep in deps {
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
}

async fn prompt_and_install_deps(
	heading: &str,
	deps: &[DepInfo],
	default_confirm: bool,
	yes: bool,
	force: bool,
	ctx: &InstallContext<'_>,
) -> anyhow::Result<()> {
	present_dep_list(heading, deps);

	let auto_answer = yes && default_confirm;
	let should_add = if auto_answer {
		true
	} else if yes && !default_confirm {
		false
	} else {
		dialoguer::Confirm::new()
			.with_prompt(format!(
				"Download and add {}?",
				heading.to_lowercase()
			))
			.default(default_confirm)
			.interact()?
	};

	if should_add {
		install_deps(deps, force, ctx).await?;
	}

	Ok(())
}

async fn install_deps(
	deps: &[DepInfo],
	force: bool,
	ctx: &InstallContext<'_>,
) -> anyhow::Result<()> {
	for dep in deps {
		let ver = dep.version.as_deref();
		let dep_mod_source = dep
			.identifier
			.parse::<crate::types::ModSource>()
			.unwrap_or_else(|_| {
				crate::types::ModSource::modrinth(&dep.identifier)
			});
		let add_ctx = AddContext {
			source: &dep_mod_source,
			version_req: AddContext::parse_version(ver)?,
			force,
			storage: ctx.storage,
			mc_version: ctx.mc_version,
			loader: ctx.loader,
			registry: ctx.registry.clone(),
			env_override: None,
			project_type_override: None,
			categories: Vec::new(),
		};
		add_ctx.add(&dep.identifier).await?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::services::resolver::ResolvedMod;
	use crate::storage::Storage;
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
		let result = categorize_deps(resolved, "root-mod", &storage);
		assert!(result.missing_required.is_empty());
		assert!(result.missing_optional.is_empty());
	}

	#[test]
	fn test_categorize_deps_required_dep() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("dep-a", DependencyKind::Required)];
		let result = categorize_deps(resolved, "root-mod", &storage);
		assert_eq!(result.missing_required.len(), 1);
		assert_eq!(result.missing_required[0].name, "dep-a");
	}

	#[test]
	fn test_categorize_deps_optional_dep() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("dep-b", DependencyKind::Optional)];
		let result = categorize_deps(resolved, "root-mod", &storage);
		assert!(result.missing_required.is_empty());
		assert_eq!(result.missing_optional.len(), 1);
	}

	#[test]
	fn test_categorize_deps_skips_embedded() {
		let (_temp_dir, storage) = make_storage();
		let resolved =
			vec![make_resolved_mod("dep-c", DependencyKind::Embedded)];
		let result = categorize_deps(resolved, "root-mod", &storage);
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
		let result = categorize_deps(resolved, "root-mod", &storage);
		assert!(result.missing_required.is_empty());
	}

	#[test]
	fn test_categorize_deps_incompatible_warning() {
		let (_temp_dir, storage) = make_storage();
		let mod_ron = crate::test_util::make_test_mod("dep-e", "Dep E");
		storage.save(ProjectType::Mod, "dep-e", &mod_ron).unwrap();
		let resolved =
			vec![make_resolved_mod("dep-e", DependencyKind::Incompatible)];
		let result = categorize_deps(resolved, "root-mod", &storage);
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
		let result = categorize_deps(resolved, "root-mod", &storage);
		assert_eq!(result.missing_required.len(), 1);
		assert_eq!(result.missing_optional.len(), 1);
		assert_eq!(result.dep_entries.len(), 2);
	}
}
