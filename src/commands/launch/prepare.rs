use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::api::minecraft::VersionInfo;
use crate::output;
use crate::storage::Storage;

use super::java::resolve_java;
use super::libraries::download_mc_libraries;
use super::loader::check_modloader_deps;

pub(super) struct LaunchContext {
	pub(super) java_path: PathBuf,
	pub(super) mc_cache: PathBuf,
	pub(super) loader_lib_dir: PathBuf,
	pub(super) natives_dir: PathBuf,
	pub(super) version_info: VersionInfo,
	pub(super) main_class: String,
	pub(super) classpath_jars: Vec<PathBuf>,
	pub(super) loader_jvm_args: Vec<String>,
	pub(super) loader_game_args: Vec<String>,
	pub(super) mc_jvm_args: Vec<String>,
	pub(super) skip_merge_libs: bool,
}

struct LoaderResult {
	main_class: String,
	classpath_jars: Vec<PathBuf>,
	jvm_args: Vec<String>,
	game_args: Vec<String>,
	lib_dir: PathBuf,
	replaces_mc_jar: bool,
	dedup_names: Vec<String>,
}

pub(super) async fn prepare_launch(
	modpack: &crate::config::ModpackManifest,
	storage: &Storage,
	cache_dir: &Path,
	side: super::LaunchSide,
	http_client: &reqwest::Client,
	java_override: Option<&Path>,
	root_dir: Option<&Path>,
) -> Result<LaunchContext> {
	check_modloader_deps(storage, modpack);

	let (java_path, java_major) = resolve_java(
		cache_dir,
		&modpack.minecraft_version,
		&modpack.loader.loader_or_default(),
		http_client,
		java_override,
	)
	.await?;
	output::bullet(format!(
		"Using Java {} at {}",
		java_major,
		java_path.display()
	));

	let mc_cache = cache_dir.join("minecraft");
	let loader_cache = cache_dir.join("loaders");

	let mc_client =
		crate::api::MinecraftClient::new().with_client(http_client.clone());
	let spin = output::spinner("Fetching version info...");
	let version_info = mc_client
		.get_version_info(&modpack.minecraft_version)
		.await?;
	spin.finish_and_clear();

	let download = match side {
		super::LaunchSide::Client => version_info.downloads.client.as_ref(),
		super::LaunchSide::Server => version_info.downloads.server.as_ref(),
	}
	.ok_or_else(|| {
		anyhow::anyhow!(
			"No {} download for MC {}",
			side,
			modpack.minecraft_version
		)
	})?;

	let mc_jar = mc_client
		.download_jar(
			download,
			&modpack.minecraft_version,
			side.as_str(),
			&mc_cache,
		)
		.await?;

	if side == super::LaunchSide::Client {
		if let Some(ref asset_index) = version_info.asset_index {
			let assets_dir = mc_cache.join("assets");
			mc_client.download_assets(asset_index, &assets_dir).await?;
		}
	}

	let mut classpath_jars = vec![mc_jar.clone()];

	let (mc_lib_paths, natives_dir) = download_mc_libraries(
		&version_info.libraries,
		&modpack.minecraft_version,
		&mc_cache,
		http_client,
	)
	.await?;

	let is_fabric_like = modpack.loader.loader_or_default().is_fabric_like();
	let skip_mc_libs = is_fabric_like
		&& side == super::LaunchSide::Server
		&& is_bundler_jar(&mc_jar);

	if !skip_mc_libs {
		classpath_jars.extend(mc_lib_paths);
	}

	let loader_result = install_loader(
		&modpack.loader.loader_or_default(),
		&modpack.loader.version,
		&modpack.minecraft_version,
		side,
		&mc_jar,
		&loader_cache,
		root_dir,
		&java_path,
		http_client,
	)
	.await?;

	if loader_result.replaces_mc_jar {
		classpath_jars.remove(0);
	}
	deduplicate_classpath(&mut classpath_jars, &loader_result.dedup_names);
	classpath_jars.extend(loader_result.classpath_jars);

	let mc_jvm_args = crate::api::minecraft::resolve_mc_jvm_args(
		&version_info,
		&natives_dir,
		&mc_cache.join("libraries"),
		&modpack.minecraft_version,
	);

	Ok(LaunchContext {
		java_path,
		mc_cache,
		loader_lib_dir: loader_result.lib_dir,
		natives_dir,
		version_info,
		main_class: loader_result.main_class,
		classpath_jars,
		loader_jvm_args: loader_result.jvm_args,
		loader_game_args: loader_result.game_args,
		mc_jvm_args,
		skip_merge_libs: is_fabric_like && side == super::LaunchSide::Server,
	})
}

#[allow(clippy::too_many_arguments)]
async fn install_loader(
	loader: &crate::types::LoaderType,
	loader_version: &str,
	mc_version: &str,
	side: super::LaunchSide,
	mc_jar: &Path,
	loader_cache: &Path,
	root_dir: Option<&Path>,
	java_path: &Path,
	http_client: &reqwest::Client,
) -> Result<LoaderResult> {
	if loader.is_fabric_like() {
		install_fabric_like(
			loader,
			loader_version,
			mc_version,
			side,
			loader_cache,
			http_client,
		)
		.await
	} else {
		install_forge_like(
			loader,
			loader_version,
			mc_version,
			side,
			mc_jar,
			loader_cache,
			root_dir,
			java_path,
			http_client,
		)
		.await
	}
}

async fn install_fabric_like(
	loader: &crate::types::LoaderType,
	loader_version: &str,
	mc_version: &str,
	side: super::LaunchSide,
	loader_cache: &Path,
	http_client: &reqwest::Client,
) -> Result<LoaderResult> {
	let loader_version = if loader_version.is_empty() {
		output::info(format!(
			"Fetching latest {} loader version...",
			loader.display_name()
		));
		match loader {
			crate::types::LoaderType::Fabric => {
				let client = crate::api::FabricClient::new()
					.with_client(http_client.clone());
				client.get_latest_loader_version(mc_version).await?
			}
			crate::types::LoaderType::Quilt => {
				let client = crate::api::QuiltClient::new()
					.with_client(http_client.clone());
				client.get_latest_loader_version(mc_version).await?
			}
			_ => unreachable!(),
		}
	} else {
		loader_version.to_string()
	};
	output::bullet(format!(
		"{} loader version: {}",
		loader.display_name(),
		loader_version
	));

	let (main_class, lib_dir, classpath_jars, dedup_names) = match loader {
		crate::types::LoaderType::Fabric => {
			let fabric = crate::api::FabricClient::new()
				.with_client(http_client.clone());
			let profile =
				fabric.get_profile(mc_version, &loader_version).await?;

			let main_class = if side == super::LaunchSide::Server {
				"net.fabricmc.loader.impl.launch.knot.KnotServer".to_string()
			} else {
				profile.main_class.clone()
			};

			let fabric_cache = loader_cache.join(loader.as_str());
			let lib_dir = fabric_cache.join(mc_version).join(&loader_version);

			let dedup_names: Vec<String> = profile
				.libraries
				.iter()
				.map(|l| crate::utils::maven::artifact_version_stem(&l.name))
				.collect();

			let lib_paths = fabric
				.download_libraries(
					&profile,
					mc_version,
					&loader_version,
					&fabric_cache,
				)
				.await?;
			let classpath_jars = lib_paths;

			(main_class, lib_dir, classpath_jars, dedup_names)
		}
		crate::types::LoaderType::Quilt => {
			let quilt =
				crate::api::QuiltClient::new().with_client(http_client.clone());
			let profile =
				quilt.get_profile(mc_version, &loader_version).await?;

			let main_class = profile.main_class.for_side(side.as_str());

			let quilt_cache = loader_cache.join(loader.as_str());
			let lib_dir = quilt_cache.join(mc_version).join(&loader_version);

			let dedup_names: Vec<String> = profile
				.libraries
				.for_side(side.as_str())
				.iter()
				.map(|l| crate::utils::maven::artifact_version_stem(&l.name))
				.collect();

			let lib_paths = quilt
				.download_libraries(
					&profile,
					side.as_str(),
					mc_version,
					&loader_version,
					&quilt_cache,
				)
				.await?;
			let classpath_jars = lib_paths;

			(main_class, lib_dir, classpath_jars, dedup_names)
		}
		_ => unreachable!(),
	};

	Ok(LoaderResult {
		main_class,
		classpath_jars,
		jvm_args: Vec::new(),
		game_args: Vec::new(),
		lib_dir,
		replaces_mc_jar: false,
		dedup_names,
	})
}

#[allow(clippy::too_many_arguments)]
async fn install_forge_like(
	loader: &crate::types::LoaderType,
	loader_version: &str,
	mc_version: &str,
	side: super::LaunchSide,
	mc_jar: &Path,
	loader_cache: &Path,
	root_dir: Option<&Path>,
	java_path: &Path,
	http_client: &reqwest::Client,
) -> Result<LoaderResult> {
	let params = crate::api::installer::InstallParams {
		game_version: mc_version,
		loader_version,
		side: side.as_str(),
		mc_jar,
		cache_dir: loader_cache,
		root_dir,
		java_path,
	};

	let result = match loader {
		crate::types::LoaderType::Forge => {
			let forge =
				crate::api::ForgeClient::new().with_client(http_client.clone());
			forge.install(&params).await?
		}
		crate::types::LoaderType::NeoForge => {
			let neoforge = crate::api::NeoForgeClient::new()
				.with_client(http_client.clone());
			neoforge.install(&params).await?
		}
		_ => unreachable!(),
	};

	Ok(LoaderResult {
		main_class: result.main_class,
		classpath_jars: result.classpath_jars,
		jvm_args: result.jvm_args,
		game_args: result.game_args,
		lib_dir: result.lib_dir,
		replaces_mc_jar: true,
		dedup_names: Vec::new(),
	})
}

fn is_bundler_jar(jar_path: &Path) -> bool {
	let Ok(file) = std::fs::File::open(jar_path) else {
		return false;
	};
	let Ok(mut archive) = zip::ZipArchive::new(file) else {
		return false;
	};
	let result = archive.by_name("META-INF/versions.list").is_ok();
	result
}

fn deduplicate_classpath(
	classpath_jars: &mut Vec<PathBuf>,
	dedup_names: &[String],
) {
	classpath_jars.retain(|jar| {
		let jar_str = jar.to_string_lossy().to_string();
		!dedup_names.iter().any(|name| jar_str.contains(name))
	});
}

pub(super) fn merge_libraries(
	mc_lib_dir: &Path,
	loader_lib_dir: &Path,
) -> Result<()> {
	if !loader_lib_dir.exists() {
		return Ok(());
	}
	merge_libraries_recursive(mc_lib_dir, loader_lib_dir)
}

fn merge_libraries_recursive(
	mc_lib_dir: &Path,
	loader_lib_dir: &Path,
) -> Result<()> {
	for entry in std::fs::read_dir(loader_lib_dir)? {
		let entry = entry?;
		let name = entry.file_name();
		let src_path = entry.path();
		let dest_path = mc_lib_dir.join(&name);

		if src_path.is_dir() {
			if !dest_path.exists() {
				std::fs::create_dir_all(&dest_path)?;
			}
			merge_libraries_recursive(&dest_path, &src_path)?;
		} else if src_path.is_file() && !dest_path.exists() {
			if let Some(parent) = dest_path.parent() {
				std::fs::create_dir_all(parent)?;
			}
			crate::utils::create_symlink(
				&src_path.canonicalize().unwrap_or(src_path),
				&dest_path,
			)?;
		}
	}
	Ok(())
}
