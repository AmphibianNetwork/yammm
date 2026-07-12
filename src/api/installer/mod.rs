//! Forge/NeoForge installer logic for extracting install profiles,
//! downloading libraries, and running post-install processors.

pub mod libraries;
pub mod processors;
pub mod profile;
pub mod templates;

pub use templates::TemplateContext;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::output;

pub struct LoaderInstallResult {
	pub main_class: String,
	pub classpath_jars: Vec<PathBuf>,
	pub jvm_args: Vec<String>,
	pub game_args: Vec<String>,
	pub lib_dir: PathBuf,
	#[allow(dead_code)]
	// set during install for future relative-path resolution
	pub root_dir: PathBuf,
}

/// Parameters for running a loader installer pipeline.
pub struct InstallParams<'a> {
	pub game_version: &'a str,
	pub loader_version: &'a str,
	pub side: &'a str,
	pub mc_jar: &'a Path,
	pub cache_dir: &'a Path,
	pub root_dir: Option<&'a Path>,
	pub java_path: &'a Path,
}

struct InstallRunContext<'a> {
	side: &'a str,
	mc_jar: &'a Path,
	installer_jar: &'a Path,
	cache_dir: &'a Path,
	root_dir: &'a Path,
	java_path: &'a Path,
	http_client: &'a reqwest::Client,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_install(
	profile: &profile::InstallProfile,
	version_json: &serde_json::Value,
	side: &str,
	mc_jar: &Path,
	installer_jar: &Path,
	cache_dir: &Path,
	root_dir: &Path,
	java_path: &Path,
	http_client: &reqwest::Client,
) -> Result<LoaderInstallResult> {
	let ctx = InstallRunContext {
		side,
		mc_jar,
		installer_jar,
		cache_dir,
		root_dir,
		java_path,
		http_client,
	};
	run_install_inner(profile, version_json, &ctx).await
}

async fn run_install_inner(
	profile: &profile::InstallProfile,
	version_json: &serde_json::Value,
	ctx: &InstallRunContext<'_>,
) -> Result<LoaderInstallResult> {
	let lib_dir = ctx.cache_dir.join("libraries");

	let installer_temp = std::env::temp_dir()
		.join(format!("yammm-installer-{}", std::process::id()));
	std::fs::create_dir_all(&installer_temp)?;
	let _cleanup = crate::utils::TempDirCleanup(&installer_temp);

	let profile_libs = libraries::download_profile_libraries(
		profile,
		ctx.cache_dir,
		ctx.http_client,
	)
	.await?;

	let version_libs = version_json
		.get("libraries")
		.and_then(|v| v.as_array())
		.cloned()
		.unwrap_or_default();

	let mut classpath_jars = libraries::collect_version_libs(
		&version_libs,
		&lib_dir,
		ctx.http_client,
	)
	.await?;

	let processors_for_side =
		processors::filter_processors_by_side(&profile.processors, ctx.side);
	if !processors_for_side.is_empty() {
		let total = processors_for_side.len();
		output::info(format!("Running {} installer processor(s)...", total));

		let tmpl_ctx = TemplateContext {
			data: &profile.data,
			side: ctx.side,
			library_dir: &lib_dir,
			installer_jar: ctx.installer_jar,
			mc_jar: ctx.mc_jar,
			root_dir: ctx.root_dir,
			temp_dir: &installer_temp,
		};

		for (i, processor) in processors_for_side.iter().enumerate() {
			output::bullet(format!(
				"Processor {}/{}: {}",
				i + 1,
				total,
				processor.jar
			));

			let resolved_args =
				templates::resolve_template_args(&processor.args, &tmpl_ctx)?;

			let all_libs: Vec<PathBuf> = classpath_jars
				.iter()
				.chain(profile_libs.iter())
				.chain(std::iter::once(&lib_dir.to_path_buf()))
				.cloned()
				.collect();

			processors::run_processor(
				processor,
				ctx.side,
				&all_libs,
				&resolved_args,
				ctx.java_path,
			)?;
		}

		let post_processor_jars = libraries::collect_version_libs(
			&version_libs,
			&lib_dir,
			ctx.http_client,
		)
		.await?;
		for jar in post_processor_jars {
			if !classpath_jars.contains(&jar) {
				classpath_jars.push(jar);
			}
		}
	}

	let main_class = version_json
		.get("mainClass")
		.and_then(|v| v.as_str())
		.unwrap_or("net.minecraft.client.main.Main")
		.to_string();

	let jvm_args = version_json
		.get("arguments")
		.and_then(|a| a.get("jvm"))
		.and_then(|j| j.as_array())
		.map(|arr| crate::api::minecraft::resolve_args_array(arr))
		.unwrap_or_default();

	let game_args = version_json
		.get("arguments")
		.and_then(|a| a.get("game"))
		.and_then(|g| g.as_array())
		.map(|arr| crate::api::minecraft::resolve_args_array(arr))
		.unwrap_or_default();

	profile::extract_launch_args_from_installer(ctx.installer_jar, &lib_dir)?;

	Ok(LoaderInstallResult {
		main_class,
		classpath_jars,
		jvm_args,
		game_args,
		lib_dir: lib_dir.clone(),
		root_dir: ctx.root_dir.to_path_buf(),
	})
}

#[allow(clippy::too_many_arguments)]
pub async fn download_and_run_installer(
	loader_name: &str,
	installer_url: &str,
	installer_filename: &str,
	side: &str,
	mc_jar: &Path,
	cache_dir: &Path,
	root_dir: Option<&Path>,
	java_path: &Path,
	http_client: &reqwest::Client,
) -> Result<LoaderInstallResult> {
	std::fs::create_dir_all(cache_dir)?;

	let lib_dir = cache_dir.join("libraries");
	std::fs::create_dir_all(&lib_dir)?;

	let effective_root = root_dir.unwrap_or(cache_dir);
	if effective_root != cache_dir {
		let root_lib = effective_root.join("libraries");
		if !root_lib.is_symlink() {
			if root_lib.exists() {
				std::fs::remove_dir_all(&root_lib)?;
			}
			crate::utils::create_symlink(&lib_dir, &root_lib)?;
		}
	}

	let installer_jar = cache_dir.join(installer_filename);

	if !installer_jar.exists() {
		output::info(format!("Downloading {} installer...", loader_name));
		// Forge/NeoForge installer URLs come from the loader's own metadata
		// API, which doesn't surface a checksum — we can't integrity-check
		// today. Streaming still keeps memory bounded for the multi-MB
		// installer jar and the atomic rename avoids leaving a partial
		// installer that subsequent runs would try to extract.
		crate::api::streaming::download_to_file(
			http_client,
			installer_url,
			&installer_jar,
			crate::api::streaming::HashPolicy::AcceptedUnhashed {
				reason: "Forge/NeoForge installer metadata does not expose a checksum",
			},
			&format!("{} installer", loader_name),
		)
		.await?;
	}

	output::info(format!("Extracting {} install profile...", loader_name));
	let (profile, version_json) =
		profile::extract_install_profile(&installer_jar).with_context(
			|| format!("Failed to extract {} install profile", loader_name),
		)?;

	run_install(
		&profile,
		&version_json,
		side,
		mc_jar,
		&installer_jar,
		cache_dir,
		effective_root,
		java_path,
		http_client,
	)
	.await
}
