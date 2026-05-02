use crate::api::ModrinthClient;
use crate::app::AppContext;
use crate::commands::export::mrpack::{mrpack_env_to_mod_env, MrpackIndex};
use crate::output;
use crate::services::deps_install::{
	categorize_deps, present_incompatible_warnings, prompt_and_install_deps,
	record_dep_edges, DepInstallContext,
};
use crate::services::resolver::DependencyResolver;
use crate::types::{HashType, ModEnv, ModSource, ProjectType, TrackedMod};
use crate::utils::slugify;
use anyhow::{Context, Result};
use clap::Parser;
use path_clean::PathClean;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Import a modpack from an `.mrpack` or `.ympk` archive.
#[derive(Parser, Debug)]
pub struct ImportCommand {
	pub file: PathBuf,

	#[arg(short = 'o', long)]
	pub output: Option<PathBuf>,

	#[arg(short = 'y', long)]
	pub yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFormat {
	Mrpack,
	Ympk,
}

impl ImportCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		tracing::debug!("ImportCommand running");

		if !self.file.exists() {
			return Err(crate::errors::YammmError::invalid_args(format!(
				"File not found: {}",
				self.file.display()
			))
			.into());
		}

		let format = detect_format(&self.file)?;

		match format {
			ImportFormat::Mrpack => self.import_mrpack(ctx).await,
			ImportFormat::Ympk => self.import_ympk(),
		}
	}

	async fn import_mrpack(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let output_dir = if let Some(ref dir) = self.output {
			dir.clone()
		} else if let Some(ref app) = ctx.modpack {
			app.root_dir.clone()
		} else {
			let name =
				self.file.file_stem().unwrap_or_default().to_string_lossy();
			PathBuf::from(name.to_string())
		};

		output::info(format!("Importing: {}", self.file.display()));
		output::bullet("Format: MRPACK (creates or updates modpack)");

		if output_dir.exists() && !self.yes {
			let proceed = dialoguer::Confirm::new()
				.with_prompt(
					"Destination directory already exists. Import anyway?",
				)
				.default(false)
				.interact()?;

			if !proceed {
				output::cancelled("Import");
				return Ok(());
			}
		}

		let file = std::fs::File::open(&self.file)?;
		let mut archive =
			zip::ZipArchive::new(file).context("Not a valid ZIP archive")?;

		let mut index_bytes = Vec::new();
		archive
			.by_name("modrinth.index.json")
			.context("No modrinth.index.json found in MRPACK")?
			.read_to_end(&mut index_bytes)?;
		let index: MrpackIndex = serde_json::from_slice(&index_bytes)
			.context("Failed to parse modrinth.index.json")?;

		output::bullet(format!("Found {} mods in MRPACK:", index.files.len()));

		let override_data =
			extract_overrides_to_memory(&mut archive, &output_dir)?;

		std::fs::create_dir_all(&output_dir)?;

		let modpack_path = output_dir.join("modpack.toml");
		let cache_dir = ctx.cache_dir();
		let jar_cache =
			crate::storage::cache::JarCache::new(cache_dir.join("jars"));
		jar_cache.init()?;

		let app = if modpack_path.exists() {
			crate::app::App::load(output_dir.clone(), jar_cache)?
		} else {
			let mut config = crate::config::ModpackManifest::new();
			config.name = index.name.clone();
			config.description = index.summary.clone().unwrap_or_default();
			if !index.version_id.is_empty() {
				config.version = index.version_id.clone();
			}
			config.apply_index_dependencies(&index.dependencies);
			crate::app::App::from_parts(output_dir.clone(), config, jar_cache)
		};

		let storage = &app.storage;
		let modrinth_client =
			ModrinthClient::new().with_client(ctx.http_client.clone());

		let mut added = 0usize;
		let mut skipped = 0usize;
		let mut unresolved_count = 0usize;
		let mut imported_for_deps: Vec<(String, ModSource, ProjectType)> =
			Vec::new();

		for mrpack_file in &index.files {
			let content_type = if mrpack_file.path.starts_with("resourcepacks/")
			{
				ProjectType::ResourcePack
			} else if mrpack_file.path.starts_with("shaderpacks/") {
				ProjectType::Shader
			} else {
				ProjectType::Mod
			};

			let download_url =
				mrpack_file.downloads.first().cloned().unwrap_or_default();
			let sha512 =
				mrpack_file.hashes.sha512.clone().filter(|h| !h.is_empty());
			let sha1 =
				mrpack_file.hashes.sha1.clone().filter(|h| !h.is_empty());
			let filename =
				mrpack_file.path.rsplit('/').next().map(|s| s.to_string());

			let resolved = resolve_mrpack_mod(
				&modrinth_client,
				&mrpack_file.downloads,
				&mrpack_file.path,
				sha512.as_deref(),
				sha1.as_deref(),
			)
			.await;

			let slug = resolved.slug;

			let already_exists = storage.exists(content_type, &slug);

			if already_exists {
				output::bullet(format!("{} (already installed)", slug));
				skipped += 1;
				continue;
			}

			let env = mrpack_file
				.env
				.as_ref()
				.map(mrpack_env_to_mod_env)
				.unwrap_or(resolved.env);

			let mod_ron = TrackedMod::builder(&slug, resolved.source.clone())
				.name(&resolved.name)
				.description(&resolved.description)
				.version(&resolved.version)
				.url(resolved.url.as_deref().unwrap_or(""))
				.download_url(&download_url)
				.hash(resolved.hash)
				.hash_type(resolved.hash_type)
				.sha1(sha1)
				.project_type(content_type)
				.env(env)
				.filename(filename)
				.unresolved(resolved.unresolved)
				.build();

			storage.save(content_type, &slug, &mod_ron)?;

			if resolved.unresolved {
				output::warning(format!(
					"{} (unresolved - could not find on Modrinth)",
					slug
				));
				unresolved_count += 1;
			} else {
				output::bullet(format!("{} (new)", slug));
				if resolved.source.requires_api() {
					imported_for_deps.push((
						slug.clone(),
						resolved.source.clone(),
						content_type,
					));
				}
			}
			added += 1;
		}

		let mut modpack = app.config.clone();
		let mut updated = false;

		if !index.name.is_empty() && modpack.name.is_empty() {
			modpack.name = index.name.clone();
			updated = true;
		}

		if !index.version_id.is_empty() && modpack.version.is_empty() {
			modpack.version = index.version_id.clone();
			updated = true;
		}

		if let Some(summary) = &index.summary {
			if modpack.description.is_empty() {
				modpack.description = summary.clone();
				updated = true;
			}
		}

		updated |= modpack.apply_index_dependencies(&index.dependencies);

		if updated || !modpack_path.exists() {
			storage.save_modpack(&modpack)?;
			output::success("Updated modpack.toml");
		}

		if !imported_for_deps.is_empty() {
			output::info("Resolving dependencies...");
			self.resolve_import_deps(
				&imported_for_deps,
				storage,
				&ctx,
				&modpack,
			)
			.await?;
		}

		write_override_data(&override_data)?;

		if unresolved_count > 0 {
			output::warning(format!(
				"{} mod(s) could not be resolved and were marked as unresolved. Use 'yammm update' or edit them manually.",
				unresolved_count
			));
		}

		output::success(format!(
			"Import complete: {} added, {} skipped, {} unresolved",
			added, skipped, unresolved_count
		));

		Ok(())
	}

	fn import_ympk(self) -> Result<()> {
		let output_dir = self.output.unwrap_or_else(|| {
			let name =
				self.file.file_stem().unwrap_or_default().to_string_lossy();
			PathBuf::from(name.to_string())
		});

		output::info(format!("Importing: {}", self.file.display()));
		output::bullet("Format: YMPK (extracts profile directory)");
		output::bullet(format!("Extracting to: {}", output_dir.display()));

		if output_dir.exists() && !self.yes {
			let proceed = dialoguer::Confirm::new()
				.with_prompt(
					"Destination directory already exists. Extract anyway?",
				)
				.default(false)
				.interact()?;

			if !proceed {
				output::cancelled("Import");
				return Ok(());
			}
		}

		std::fs::create_dir_all(&output_dir)?;

		let file = std::fs::File::open(&self.file)?;
		let mut archive =
			zip::ZipArchive::new(file).context("Not a valid ZIP archive")?;

		let mut found_modpack_toml = false;
		let mut mod_count = 0usize;

		for i in 0..archive.len() {
			let mut zip_file = archive.by_index(i)?;
			let name = zip_file.name().to_string();
			let outpath = match zip_file.enclosed_name() {
				Some(path) => output_dir.join(path).clean(),
				None => continue,
			};

			if !outpath.starts_with(&output_dir) {
				tracing::warn!("Skipping zip entry with path escape: {}", name);
				continue;
			}

			if name.ends_with('/') {
				std::fs::create_dir_all(&outpath)?;
			} else {
				if let Some(p) = outpath.parent() {
					if !p.exists() {
						std::fs::create_dir_all(p)?;
					}
				}
				let mut outfile = std::fs::File::create(&outpath)?;
				std::io::copy(&mut zip_file, &mut outfile)?;

				if name == "modpack.toml" {
					found_modpack_toml = true;
				}
				if (name.starts_with("mods/")
					|| name.starts_with("resourcepacks/")
					|| name.starts_with("shaderpacks/"))
					&& name.ends_with("/mod.ron")
				{
					mod_count += 1;
				}
			}
		}

		if !found_modpack_toml {
			std::fs::remove_dir_all(&output_dir).ok();
			return Err(crate::errors::YammmError::invalid_args(
				"No modpack.toml found in YMPK - archive may be corrupted",
			)
			.into());
		}

		output::success("Validated modpack.toml");
		output::success(format!("Found {} mods", mod_count));
		output::success(format!(
			"Done! Modpack ready at: {}",
			output_dir.display()
		));

		Ok(())
	}

	async fn resolve_import_deps(
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

		let mut all_resolved: Vec<crate::services::resolver::ResolvedMod> =
			Vec::new();
		let mut root_ids: Vec<String> = Vec::new();

		for (slug, source, _project_type) in imported {
			let mod_id = source.source_id().to_string();
			root_ids.push(mod_id.clone());

			let mut resolver = DependencyResolver::new(ctx.registry.clone());
			if let Some(v) = mc_version {
				resolver = resolver.with_minecraft_version(v);
			}
			if let Some(l) = loader {
				resolver = resolver.with_loader(l);
			}

			match resolver.resolve(&mod_id, source.clone()).await {
				Ok(mods) => all_resolved.extend(mods),
				Err(e) => {
					output::warning(format!(
						"Could not resolve dependencies for {}: {}",
						slug, e
					));
				}
			}
		}

		if all_resolved.is_empty() {
			return Ok(());
		}

		let install_ctx = DepInstallContext {
			storage,
			registry: ctx.registry.clone(),
			mc_version,
			loader,
		};

		let categorized = categorize_deps(all_resolved, &root_ids, storage);

		present_incompatible_warnings(&categorized.incompatible_warnings);

		if !categorized.missing_required.is_empty() {
			prompt_and_install_deps(
				"Required dependencies",
				&categorized.missing_required,
				true,
				self.yes,
				false,
				&install_ctx,
			)
			.await?;
		}

		if !categorized.missing_optional.is_empty() {
			prompt_and_install_deps(
				"Optional dependencies",
				&categorized.missing_optional,
				false,
				self.yes,
				false,
				&install_ctx,
			)
			.await?;
		}

		let mut dep_by_parent: HashMap<String, Vec<crate::types::Dependency>> =
			HashMap::new();
		for dep in &categorized.dep_entries {
			if let Some(ref parent) = dep.required_by {
				for root_id in &root_ids {
					if parent.contains(root_id) {
						dep_by_parent
							.entry(root_id.clone())
							.or_default()
							.push(dep.clone());
					}
				}
			}
		}

		for (slug, source, project_type) in imported {
			let mod_id = source.source_id().to_string();
			if let Some(deps) = dep_by_parent.get(&mod_id) {
				record_dep_edges(storage, *project_type, slug, deps)?;
			}
		}

		Ok(())
	}
}

fn detect_format(path: &std::path::Path) -> Result<ImportFormat> {
	let ext = path
		.extension()
		.map(|e| e.to_string_lossy().to_lowercase())
		.unwrap_or_default();

	match ext.as_str() {
		"mrpack" => Ok(ImportFormat::Mrpack),
		"ympk" => Ok(ImportFormat::Ympk),
		"zip" => detect_zip_format(path),
		_ => Err(crate::errors::YammmError::invalid_args(format!(
			"Unsupported format: .{}",
			ext
		))
		.into()),
	}
}

fn detect_zip_format(path: &std::path::Path) -> Result<ImportFormat> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file).with_context(|| {
		format!("{} is not a valid ZIP archive", path.display())
	})?;

	if archive.by_name("modrinth.index.json").is_ok() {
		return Ok(ImportFormat::Mrpack);
	}

	if archive.by_name("modpack.toml").is_ok() {
		return Ok(ImportFormat::Ympk);
	}

	Err(crate::errors::YammmError::invalid_args(
		format!(
			"Cannot determine format of {}. Neither modrinth.index.json nor modpack.toml found inside.",
			path.display()
		),
	).into())
}

fn extract_overrides_to_memory(
	archive: &mut zip::ZipArchive<std::fs::File>,
	root_dir: &Path,
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
	let override_dirs = ["overrides", "client-overrides", "server-overrides"];
	let mut files = Vec::new();

	for override_dir in &override_dirs {
		for i in 0..archive.len() {
			let mut zip_file = archive.by_index(i)?;
			let name = zip_file.name().to_string();
			let name_normalized = name.replace('\\', "/");

			if !name_normalized.starts_with(&format!("{}/", override_dir)) {
				continue;
			}

			let relative_path = &name_normalized[override_dir.len() + 1..];
			if relative_path.is_empty() {
				continue;
			}

			let enclosed = zip_file.enclosed_name();
			let outpath: PathBuf = match enclosed {
				Some(path) => {
					let stripped = path.iter().skip(1).collect::<PathBuf>();
					root_dir.join(stripped).clean()
				}
				None => continue,
			};

			if !outpath.starts_with(root_dir) {
				tracing::warn!("Skipping override with path escape: {}", name);
				continue;
			}

			if name.ends_with('/') {
				continue;
			}

			if outpath.exists() {
				continue;
			}

			let mut data = Vec::new();
			std::io::Read::read_to_end(&mut zip_file, &mut data)?;
			files.push((outpath, data));
		}
	}

	Ok(files)
}

fn write_override_data(files: &[(PathBuf, Vec<u8>)]) -> Result<()> {
	let mut extracted = 0usize;

	for (outpath, data) in files {
		if let Some(p) = outpath.parent() {
			if !p.exists() {
				std::fs::create_dir_all(p)?;
			}
		}

		std::fs::write(outpath, data)?;
		extracted += 1;
	}

	if extracted > 0 {
		output::success(format!("Extracted {} override file(s)", extracted));
	}

	Ok(())
}

fn extract_slug_from_path(path: &str) -> String {
	let path = path.replace('\\', "/");
	let filename = path.rsplit('/').next().unwrap_or(path.as_str());
	filename
		.strip_suffix(".jar")
		.or_else(|| filename.strip_suffix(".zip"))
		.unwrap_or(filename)
		.to_string()
}

fn extract_version_from_path(path: &str) -> String {
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

struct ImportedMod {
	slug: String,
	name: String,
	description: String,
	version: String,
	source: ModSource,
	hash: Option<String>,
	hash_type: HashType,
	env: ModEnv,
	url: Option<String>,
	unresolved: bool,
}

async fn resolve_mrpack_mod(
	modrinth_client: &ModrinthClient,
	downloads: &[String],
	path: &str,
	sha512: Option<&str>,
	sha1: Option<&str>,
) -> ImportedMod {
	if let Some(result) =
		try_resolve_by_hash(modrinth_client, sha512, "sha512", HashType::Sha512)
			.await
	{
		return result;
	}

	if let Some(result) =
		try_resolve_by_hash(modrinth_client, sha1, "sha1", HashType::Sha1).await
	{
		return result;
	}

	if sha512.is_some() || sha1.is_some() {
		tracing::warn!(
			"Modrinth hash lookup failed for {}, falling back to search",
			path
		);
	}

	let path_slug = slugify(&extract_slug_from_path(path));
	let version = extract_version_from_path(path);

	let hash = sha512
		.map(|h| h.to_string())
		.or(sha1.map(|h| h.to_string()));
	let hash_type = sha512
		.map(|_| HashType::Sha512)
		.or(sha1.map(|_| HashType::Sha1))
		.unwrap_or_default();

	if let Some(result) = try_resolve_by_search(
		modrinth_client,
		&path_slug,
		&version,
		&hash,
		hash_type,
	)
	.await
	{
		return result;
	}

	tracing::warn!(
		"Could not resolve mod from path '{}', marking as unresolved",
		path
	);

	let source = determine_source(downloads, &path_slug);

	ImportedMod {
		slug: path_slug,
		name: extract_slug_from_path(path),
		description: String::new(),
		version,
		source,
		hash,
		hash_type,
		env: ModEnv::Both,
		url: None,
		unresolved: true,
	}
}

async fn try_resolve_by_hash(
	modrinth_client: &ModrinthClient,
	hash: Option<&str>,
	algorithm: &str,
	hash_type: HashType,
) -> Option<ImportedMod> {
	let hash = hash?;
	let version_data = modrinth_client
		.get_version_by_hash(hash, algorithm)
		.await
		.ok()?;

	let project_result = modrinth_client
		.get_project_direct(&version_data.mod_id)
		.await;

	let slug = match &project_result {
		Ok(project) => {
			if project.slug.is_empty() {
				slugify(&version_data.mod_id)
			} else {
				project.slug.clone()
			}
		}
		Err(_) => slugify(&version_data.mod_id),
	};

	let (name, description, url, env) = match &project_result {
		Ok(project) => {
			let env = crate::providers::modrinth::mod_env_from_modrinth_sides(
				project.client_side.as_deref(),
				project.server_side.as_deref(),
			);
			(
				project.title.clone(),
				project.description.clone(),
				Some(format!("https://modrinth.com/mod/{}", slug)),
				env,
			)
		}
		Err(_) => (
			version_data.version_number.clone(),
			String::new(),
			None,
			ModEnv::Both,
		),
	};

	Some(ImportedMod {
		slug: slug.clone(),
		name,
		description,
		version: version_data.version_number,
		source: ModSource::modrinth(&slug),
		hash: Some(hash.to_string()),
		hash_type,
		env,
		url,
		unresolved: false,
	})
}

async fn try_resolve_by_search(
	modrinth_client: &ModrinthClient,
	slug: &str,
	version: &str,
	hash: &Option<String>,
	hash_type: HashType,
) -> Option<ImportedMod> {
	let hits = modrinth_client.search(slug, Some(5)).await.ok()?;

	let normalized_slug = slug.to_lowercase().replace('-', " ");
	let hit = hits.iter().find(|h| {
		h.slug == slug
			|| h.slug.to_lowercase() == slug.to_lowercase()
			|| h.title.to_lowercase().replace('-', " ") == normalized_slug
	})?;

	let resolved_slug = if hit.slug.is_empty() {
		slug.to_string()
	} else {
		hit.slug.clone()
	};

	let env = crate::providers::modrinth::mod_env_from_modrinth_sides(
		hit.client_side.as_deref(),
		hit.server_side.as_deref(),
	);

	Some(ImportedMod {
		slug: resolved_slug.clone(),
		name: hit.title.clone(),
		description: hit.description.clone(),
		version: version.to_string(),
		source: ModSource::modrinth(&resolved_slug),
		hash: hash.clone(),
		hash_type,
		env,
		url: Some(format!("https://modrinth.com/mod/{}", resolved_slug)),
		unresolved: false,
	})
}

fn determine_source(
	downloads: &[String],
	slug: &str,
) -> ModSource {
	if let Some(url) = downloads.first() {
		let host = extract_host(url);
		if let Some(h) = host {
			if h.ends_with("modrinth.com") {
				if let Some(project_id) = extract_project_id_from_cdn_url(url) {
					return ModSource::modrinth(project_id);
				}
				return ModSource::modrinth(slug);
			}
			if h.ends_with("curseforge.com") {
				return ModSource::curseforge(slug);
			}
			if h.ends_with("github.com") {
				return ModSource::url(url.clone());
			}
		}
		return ModSource::url(url.clone());
	}
	ModSource::url(String::new())
}

fn extract_project_id_from_cdn_url(url: &str) -> Option<String> {
	let path = url.split("://").nth(1)?;
	let path = path.split('/').skip(1).collect::<Vec<_>>();
	if path.len() >= 2 && path[0] == "data" {
		return Some(path[1].to_string());
	}
	None
}

fn extract_host(url: &str) -> Option<String> {
	let after_scheme = url.split("://").nth(1)?;
	let host_port = after_scheme.split('/').next()?;
	let host = host_port.split(':').next()?;
	Some(host.to_lowercase())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_extract_slug_from_path() {
		assert_eq!(extract_slug_from_path("mods/sodium.jar"), "sodium");
		assert_eq!(
			extract_slug_from_path("mods/lithium-2.0.jar"),
			"lithium-2.0"
		);
		assert_eq!(
			extract_slug_from_path("mods\\windows-mod.jar"),
			"windows-mod"
		);
		assert_eq!(
			extract_slug_from_path("resourcepacks/some-pack.zip"),
			"some-pack"
		);
		assert_eq!(
			extract_slug_from_path("shaderpacks/bsl-shaders.zip"),
			"bsl-shaders"
		);
	}

	#[test]
	fn test_extract_version_from_path() {
		assert_eq!(extract_version_from_path("mods/sodium-0.5.8.jar"), "0.5.8");
		assert_eq!(
			extract_version_from_path("mods/fabric-api-0.92.0+1.20.4.jar"),
			"0.92.0+1.20.4"
		);
		assert_eq!(extract_version_from_path("mods/simplemod.jar"), "0.0.0");
		assert_eq!(
			extract_version_from_path("mods/iris-mc1.20.4-1.6.17.jar"),
			"1.6.17"
		);
	}

	#[test]
	fn test_determine_source_modrinth_cdn() {
		let source = determine_source(
			&[
				"https://cdn.modrinth.com/data/fRiHVvU7/versions/1.0/mod.jar"
					.to_string(),
			],
			"abc",
		);
		match source {
			ModSource::Modrinth { id } => assert_eq!(id, "fRiHVvU7"),
			_ => panic!("Expected Modrinth source with project ID"),
		}
	}

	#[test]
	fn test_determine_source_modrinth_no_cdn() {
		let source = determine_source(
			&["https://api.modrinth.com/some/path.jar".to_string()],
			"abc",
		);
		match source {
			ModSource::Modrinth { id } => assert_eq!(id, "abc"),
			_ => panic!("Expected Modrinth source with slug fallback"),
		}
	}

	#[test]
	fn test_determine_source_curseforge() {
		let source = determine_source(
			&["https://edge.forge.curseforge.com/files/123/mod.jar"
				.to_string()],
			"abc",
		);
		assert!(matches!(source, ModSource::CurseForge { .. }));
	}

	#[test]
	fn test_determine_source_url() {
		let source = determine_source(
			&["https://example.com/mod.jar".to_string()],
			"abc",
		);
		assert!(matches!(source, ModSource::Url { .. }));
	}

	#[test]
	fn test_extract_project_id_from_cdn_url() {
		assert_eq!(
			extract_project_id_from_cdn_url(
				"https://cdn.modrinth.com/data/fRiHVvU7/versions/abc123/mod.jar"
			),
			Some("fRiHVvU7".to_string())
		);
		assert_eq!(
			extract_project_id_from_cdn_url(
				"https://api.modrinth.com/v2/project/emi"
			),
			None
		);
		assert_eq!(
			extract_project_id_from_cdn_url("https://example.com/mod.jar"),
			None
		);
	}
}
