use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::output;
use crate::services::download_missing_mods;
use crate::storage::JarCache;
use clap::Parser;

use super::java::java_launch_prefix;
use super::prepare::prepare_launch;
use super::vfs::VfsTree;
use super::{
	build_java_command, install_signal_handlers, resolve_classpath,
	spawn_java_process, wait_for_child,
};

#[derive(Parser, Debug)]
pub struct ClientArgs {
	#[arg(long)]
	pub offline: bool,

	#[arg(long)]
	pub jvm_args: Option<String>,

	#[arg(long)]
	pub java: Option<PathBuf>,
}

pub async fn run(
	args: ClientArgs,
	ctx: crate::app::AppContext,
) -> Result<()> {
	let app = ctx.require_modpack()?;
	let modpack = &app.config;
	let storage = &app.storage;

	output::heading(format!(
		"Launching Minecraft client for: {}",
		modpack.name
	));
	output::blank_line();

	let summary =
		download_missing_mods(storage, &app.cache, &ctx.http_client, 4).await?;
	output::present_download_summary(&summary);
	if !summary.failed.is_empty() {
		anyhow::bail!(
			"{} file(s) could not be downloaded",
			summary.failed.len()
		);
	}

	let java_override = args.java.as_deref();
	let launch_ctx = prepare_launch(
		modpack,
		storage,
		ctx.cache_dir(),
		super::LaunchSide::Client,
		&ctx.http_client,
		java_override,
		None,
	)
	.await?;

	let client_dir = app.root_dir.join("client");
	let root_dir = app.root_dir.as_path();
	let first_jar = launch_ctx.classpath_jars.first().ok_or_else(|| {
		crate::errors::YammmError::download_failed("No classpath jars found")
	})?;
	let mut vfs_tree = build_client_vfs(storage, &app.cache, root_dir)?;
	extract_client_icon(first_jar, &mut vfs_tree);

	crate::commands::launch::vfs::realize_vfs(&vfs_tree, &client_dir)?;

	let resolved = resolve_classpath(
		&launch_ctx,
		storage,
		&app.cache,
		&modpack.minecraft_version,
	)?;
	let java_prefix = java_launch_prefix(&launch_ctx.java_path);

	let mut java_args = Vec::new();

	#[cfg(target_os = "macos")]
	java_args.push("-XstartOnFirstThread".to_string());

	java_args.push(format!(
		"-Djava.library.path={}",
		launch_ctx.natives_dir.to_string_lossy()
	));

	java_args.extend(super::java::log4j_mitigation_args(
		&modpack.minecraft_version,
	));

	if super::java::needs_log4j_config_override(&modpack.minecraft_version)
		&& let Ok(config_args) = super::java::write_log4j_config(&client_dir)
	{
		java_args.extend(config_args);
	}

	if let Some(resolved) = &resolved.resolved_jvm_args {
		java_args.extend(resolved.iter().cloned());
		java_args.push(crate::utils::ADD_OPENS_ARG.to_string());
		for arg in &launch_ctx.mc_jvm_args {
			if (arg.starts_with("--add-opens")
				|| arg.starts_with("--add-exports"))
				&& !java_args.contains(arg)
			{
				java_args.push(arg.clone());
			}
		}
	} else {
		java_args.extend(
			launch_ctx
				.mc_jvm_args
				.iter()
				.filter(|arg| !arg.starts_with("-Djava.library.path="))
				.cloned(),
		);
	}

	if let Some(ref jvm) = args.jvm_args {
		for arg in jvm.split_whitespace() {
			java_args.push(arg.to_string());
		}
	}

	java_args.push("-cp".to_string());
	java_args.push(resolved.classpath);
	java_args.push(resolved.main_class);

	if !resolved.game_args.is_empty() {
		java_args.extend(resolved.game_args);
	}

	java_args.push("--gameDir".to_string());
	java_args.push(client_dir.to_string_lossy().to_string());
	java_args.push("--assetsDir".to_string());
	java_args.push(
		launch_ctx
			.mc_cache
			.join("assets")
			.to_string_lossy()
			.to_string(),
	);
	if let Some(ref asset_index) = launch_ctx.version_info.asset_index {
		java_args.push("--assetIndex".to_string());
		java_args.push(asset_index.id.clone());
	}
	java_args.push("--version".to_string());
	java_args.push(modpack.minecraft_version.clone());

	if args.offline {
		java_args.push("--username".to_string());
		java_args.push("Player".to_string());
		java_args.push("--accessToken".to_string());
		java_args.push("0".to_string());
		java_args.push("--uuid".to_string());
		java_args.push("00000000000000000000000000000000".to_string());
	} else {
		match crate::auth::load_token()? {
			Some(token) => {
				java_args.push("--username".to_string());
				java_args.push(token.username);
				java_args.push("--accessToken".to_string());
				java_args.push(token.access_token);
				java_args.push("--uuid".to_string());
				java_args.push(token.uuid);
			}
			None => {
				java_args.push("--username".to_string());
				java_args.push("Player".to_string());
				java_args.push("--accessToken".to_string());
				java_args.push("0".to_string());
			}
		}
	}

	java_args.push("--versionType".to_string());
	java_args.push("yammm".to_string());

	output::blank_line();
	output::info("Starting Minecraft...");
	install_signal_handlers();

	let cmd = build_java_command(
		&launch_ctx.java_path,
		&java_prefix,
		java_args,
		Some(&client_dir),
	)?;
	let mut child = spawn_java_process(cmd)?;

	let status = wait_for_child(&mut child)?;
	if status.success() {
		output::success("Minecraft exited normally");
	} else {
		output::warning(format!("Minecraft exited with: {}", status));
	}

	Ok(())
}

fn build_client_vfs(
	storage: &crate::storage::Storage,
	cache: &JarCache,
	root_dir: &Path,
) -> Result<VfsTree> {
	let mut tree = super::build_mod_vfs(
		storage,
		cache,
		super::LaunchSide::Client,
		root_dir,
	)?;

	let rp_src = storage.resourcepacks_dir.clone();
	if rp_src.exists() {
		tree.add_dir_from_source(&PathBuf::from("resourcepacks"), &rp_src);
	}

	let sp_src = storage.shaderpacks_dir.clone();
	if sp_src.exists() {
		tree.add_dir_from_source(&PathBuf::from("shaderpacks"), &sp_src);
	}

	Ok(tree)
}

fn extract_client_icon(
	mc_jar: &Path,
	tree: &mut VfsTree,
) -> Option<()> {
	let file = match std::fs::File::open(mc_jar) {
		Ok(f) => f,
		Err(_) => return None,
	};
	let mut archive = match zip::ZipArchive::new(file) {
		Ok(a) => a,
		Err(_) => return None,
	};

	for icon_path in &["icons/minecraft.icns", "icons/icon_128x128.png"] {
		if let Ok(mut icon_file) = archive.by_name(icon_path) {
			let mut buf = Vec::new();
			if std::io::Read::read_to_end(&mut icon_file, &mut buf).is_ok() {
				let icon_tmp = std::env::temp_dir()
					.join(format!("yammm-icon-{}", std::process::id()));
				let _ = std::fs::create_dir_all(&icon_tmp);
				let tmp_path = icon_tmp.join(icon_path.replace('/', "_"));
				if std::fs::write(&tmp_path, &buf).is_ok() {
					tree.add_file(Path::new(icon_path), tmp_path);
					return Some(());
				}
			}
		}
	}
	None
}
