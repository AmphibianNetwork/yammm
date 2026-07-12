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

use std::collections::HashSet;

use clap::Parser;

use super::CliSource;
use crate::app::AppContext;
use crate::output;
use crate::services::deps_install::{
	DepInstallContext, categorize_deps, prompt_and_install_deps,
	record_dep_edges,
};
use crate::services::resolver::DependencyResolver;
use crate::types::{LoaderType, ModEnv, ModSource, ProjectType};

pub mod sources;
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

		// Snapshot installed slugs for every project type so we can
		// report what got newly added (the main mod plus any
		// transitively installed deps) once the command finishes. This
		// is used by --json output; it's cheap, so we always do it.
		let before: HashSet<(ProjectType, String)> =
			snapshot_installed(storage);

		let resolved_source = self.source.to_mod_source(&self.identifier);

		let add_ctx = AddContext {
			source: &resolved_source,
			version_req: AddContext::parse_version(self.version.as_deref())?,
			force: self.force,
			storage,
			mc_version: minecraft_version,
			loader,
			registry: ctx.registry().clone(),
			env_override: self.env,
			project_type_override: self.project_type.as_ref(),
			categories: self.categories.clone(),
		};

		let (main_slug, main_project_type) =
			add_ctx.add(&self.identifier).await?;

		let install_ctx = DepInstallContext {
			storage: std::sync::Arc::new((*storage).clone()),
			registry: add_ctx.registry.clone(),
			mc_version: minecraft_version.map(String::from),
			loader,
		};

		self.resolve_dependencies(
			&install_ctx,
			&main_slug,
			main_project_type,
			&resolved_source,
		)
		.await?;

		if output::is_json_mode() {
			let after = snapshot_installed(storage);
			let deps: Vec<_> = after
				.difference(&before)
				.filter(|(pt, slug)| {
					!(*pt == main_project_type && slug == &main_slug)
				})
				.map(|(pt, slug)| {
					serde_json::json!({
						"id": slug,
						"project_type": pt.as_str(),
					})
				})
				.collect();
			output::emit_json(&serde_json::json!({
				"command": "add",
				"added": {
					"id": main_slug,
					"project_type": main_project_type.as_str(),
					"source": resolved_source.as_str(),
				},
				"dependencies_installed": deps,
			}))?;
			return Ok(());
		}

		output::success("Done");

		Ok(())
	}

	/// Resolve and install transitive dependencies for the just-added mod.
	///
	/// URL sources are excluded from dependency resolution since they don't
	/// have a provider that can return dependency information.
	async fn resolve_dependencies(
		&self,
		ctx: &DepInstallContext,
		main_slug: &str,
		main_project_type: ProjectType,
		resolved_source: &ModSource,
	) -> anyhow::Result<()> {
		if resolved_source.url_str().is_some() {
			return Ok(());
		}
		let mod_id = resolved_source.source_id().to_string();

		let mut resolver = DependencyResolver::new(ctx.registry.clone());

		if let Some(ref minecraft_version) = ctx.mc_version {
			resolver =
				resolver.with_minecraft_version(minecraft_version.as_str());
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

		let categorized = categorize_deps(
			resolved_mods,
			std::slice::from_ref(&mod_id),
			&ctx.storage,
		);

		crate::services::deps_install::present_incompatible_warnings(
			&categorized.incompatible_warnings,
		);

		// JSON mode is non-interactive — treat it as if --yes were passed
		// so the dep installer doesn't try to prompt on a piped stdin.
		let yes = self.yes || output::is_json_mode();

		if !categorized.missing_required.is_empty() {
			prompt_and_install_deps(
				"Required dependencies",
				&categorized.missing_required,
				true,
				yes,
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
				yes,
				self.force,
				ctx,
			)
			.await?;
		}

		record_dep_edges(
			&ctx.storage,
			main_project_type,
			main_slug,
			&categorized.dep_entries,
		)?;

		Ok(())
	}
}

/// Snapshot every installed slug grouped by project type. Used by the
/// command's JSON output path to compute a precise "what got added"
/// diff after dependency resolution runs.
fn snapshot_installed(
	storage: &crate::storage::Storage
) -> HashSet<(ProjectType, String)> {
	let mut snapshot = HashSet::new();
	for pt in ProjectType::VARIANTS {
		for item in storage.list(*pt).unwrap_or_default() {
			snapshot.insert((*pt, item.id));
		}
	}
	snapshot
}
