use super::ImportCommand;
use crate::app::AppContext;
use crate::output;
use crate::services::connector::is_connector_installed;
use crate::services::deps_install::{
	categorize_deps, present_incompatible_warnings, record_dep_edges,
};
use crate::services::resolver::DependencyResolver;
use crate::types::{LoaderType, ModSource, ProjectType};
use anyhow::Result;

impl ImportCommand {
	pub(super) async fn resolve_import_deps(
		&self,
		imported: &[(String, ModSource, ProjectType)],
		storage: &crate::storage::Storage,
		ctx: &AppContext,
		modpack: &crate::config::ModpackManifest,
	) -> Result<()> {
		let filters = modpack.version_filters();
		let mc_version = filters
			.minecraft_version
			.as_deref()
			.filter(|s| !s.is_empty());
		let loader = filters.loader;

		let mut total_missing_required = 0usize;
		let mut total_missing_optional = 0usize;

		let mut missing_required_names: Vec<String> = Vec::new();

		let spin = output::spinner("Resolving dependencies...");

		for (slug, source, project_type) in imported {
			let mod_id = source.source_id().to_string();

			let resolved_mods = match self
				.resolve_with_fallback(
					&mod_id, source, mc_version, loader, ctx, storage,
				)
				.await
			{
				Ok(mods) => mods,
				Err(e) => {
					output::warning(format!(
						"Could not resolve dependencies for {}: {}",
						slug, e
					));
					continue;
				}
			};

			if resolved_mods.len() <= 1 {
				continue;
			}

			let canonical_id = resolved_mods
				.first()
				.map(|m| m.mod_id.clone())
				.unwrap_or_else(|| mod_id.clone());
			let root_ids = vec![mod_id.clone(), canonical_id.clone()];

			let categorized =
				categorize_deps(resolved_mods, &root_ids, storage);

			present_incompatible_warnings(&categorized.incompatible_warnings);

			total_missing_required += categorized.missing_required.len();
			total_missing_optional += categorized.missing_optional.len();

			for dep in &categorized.missing_required {
				let label = match &dep.required_by {
					Some(parent) => {
						format!("{} (required by {})", dep.name, parent)
					}
					None => dep.name.clone(),
				};
				missing_required_names.push(label);
			}

			record_dep_edges(
				storage,
				*project_type,
				slug,
				&categorized.dep_entries,
			)?;
		}

		spin.finish_and_clear();

		if total_missing_required > 0 || total_missing_optional > 0 {
			output::blank_line();
			if total_missing_required > 0 {
				output::warning(format!(
					"{} missing required dependencies (use 'yammm manage' to install):",
					total_missing_required
				));
				for name in &missing_required_names {
					output::bullet(name);
				}
			}
			if total_missing_optional > 0 {
				output::info(format!(
					"{} optional dependencies available (use 'yammm manage' to review)",
					total_missing_optional
				));
			}
		}

		Ok(())
	}

	async fn resolve_with_fallback(
		&self,
		mod_id: &str,
		source: &ModSource,
		mc_version: Option<&str>,
		loader: Option<LoaderType>,
		ctx: &AppContext,
		storage: &crate::storage::Storage,
	) -> Result<Vec<crate::services::resolver::ResolvedMod>> {
		let mut resolver = DependencyResolver::new(ctx.registry.clone());
		if let Some(v) = mc_version {
			resolver = resolver.with_minecraft_version(v);
		}
		if let Some(l) = loader {
			resolver = resolver.with_loader(l);
		}

		match resolver.resolve(mod_id, source.clone()).await {
			Ok(mods) => return Ok(mods),
			Err(_) if Self::should_try_connector(loader, storage) => {}
			Err(e) => return Err(e),
		}

		let mut fabric_resolver = DependencyResolver::new(ctx.registry.clone());
		if let Some(v) = mc_version {
			fabric_resolver = fabric_resolver.with_minecraft_version(v);
		}
		fabric_resolver = fabric_resolver.with_loader(LoaderType::Fabric);

		fabric_resolver.resolve(mod_id, source.clone()).await
	}

	fn should_try_connector(
		loader: Option<LoaderType>,
		storage: &crate::storage::Storage,
	) -> bool {
		matches!(loader, Some(LoaderType::Forge | LoaderType::NeoForge))
			&& is_connector_installed(storage)
	}
}
