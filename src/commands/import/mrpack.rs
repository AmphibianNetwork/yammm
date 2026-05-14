use super::ImportCommand;
use super::helpers::{extract_slug_from_path, extract_version_from_path};
use crate::api::ModrinthClient;
use crate::app::AppContext;
use crate::commands::export::mrpack::{MrpackIndex, mrpack_env_to_mod_env};
use crate::output;
use crate::services::connector::is_connector_installed;
use crate::types::{
	HashType, LoaderType, ModEnv, ModSource, ProjectType, TrackedMod,
};
use crate::utils::slugify;
use anyhow::{Context, Result};
use path_clean::PathClean;
use std::io::Read;
use std::path::{Path, PathBuf};

impl ImportCommand {
	pub(super) async fn import_mrpack(
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
		let jar_cache = ctx.jar_cache().clone();

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

		let modpack_loader = app.config.loader.loader;
		let connector_available = matches!(
			modpack_loader,
			Some(LoaderType::Forge | LoaderType::NeoForge)
		) && is_connector_installed(storage);

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

			let is_fabric_only = resolved.loaders.iter().any(|l| l == "fabric")
				&& !resolved
					.loaders
					.iter()
					.any(|l| l == "forge" || l == "neoforge");
			let needs_connector = connector_available && is_fabric_only;

			let mod_ron = TrackedMod::builder(&slug, resolved.source.clone())
				.name(&resolved.name)
				.description(&resolved.description)
				.version(&resolved.version)
				.url(resolved.url.as_deref().unwrap_or(""))
				.download_url(&download_url)
				.hash(resolved.hash)
				.hash_type(resolved.hash_type)
				.project_type(content_type)
				.env(env)
				.filename(filename)
				.unresolved(resolved.unresolved)
				.connector_compat(needs_connector)
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
				if resolved.source.requires_api()
					&& content_type == ProjectType::Mod
				{
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

		if let Some(summary) = &index.summary
			&& modpack.description.is_empty()
		{
			modpack.description = summary.clone();
			updated = true;
		}

		updated |= modpack.apply_index_dependencies(&index.dependencies);

		if updated || !modpack_path.exists() {
			storage.save_modpack(&modpack)?;
			output::success("Updated modpack.toml");
		}

		if !imported_for_deps.is_empty() {
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
	loaders: Vec<String>,
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
		loaders: Vec::new(),
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
		.map_err(|e| {
			tracing::warn!("Failed to resolve hash {}: {e}", hash);
			e
		})
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
		loaders: version_data.loaders.clone(),
	})
}

async fn try_resolve_by_search(
	modrinth_client: &ModrinthClient,
	slug: &str,
	version: &str,
	hash: &Option<String>,
	hash_type: HashType,
) -> Option<ImportedMod> {
	let hits = modrinth_client
		.search(slug, Some(5))
		.await
		.map_err(|e| {
			tracing::warn!("Failed to search for '{slug}': {e}");
			e
		})
		.ok()?;

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
		loaders: hit
			.categories
			.iter()
			.filter(|c| {
				matches!(c.as_str(), "fabric" | "forge" | "neoforge" | "quilt")
			})
			.cloned()
			.collect(),
	})
}

pub fn determine_source(
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
	ModSource::modrinth(slug)
}

pub fn extract_project_id_from_cdn_url(url: &str) -> Option<String> {
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
		if let Some(p) = outpath.parent()
			&& !p.exists()
		{
			std::fs::create_dir_all(p)?;
		}

		std::fs::write(outpath, data)?;
		extracted += 1;
	}

	if extracted > 0 {
		output::success(format!("Extracted {} override file(s)", extracted));
	}

	Ok(())
}
