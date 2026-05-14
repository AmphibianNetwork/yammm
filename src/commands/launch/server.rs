use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::output;
use crate::services::download_missing_mods;
use crate::storage::JarCache;
use clap::Parser;

use super::java::java_launch_prefix;
use super::prepare::{merge_libraries, prepare_launch};
use super::vfs::VfsTree;
use super::{
	build_java_command, install_signal_handlers, resolve_classpath,
	spawn_java_process, wait_for_child,
};

#[derive(Parser, Debug)]
pub struct ServerArgs {
	#[arg(long, default_value = "25565")]
	pub port: u16,

	#[arg(long)]
	pub jvm_args: Option<String>,

	#[arg(long)]
	pub eula: bool,

	#[arg(long)]
	pub java: Option<PathBuf>,
}

pub async fn run(
	args: ServerArgs,
	ctx: crate::app::AppContext,
) -> Result<()> {
	let app = ctx.require_modpack()?;
	let modpack = &app.config;
	let storage = &app.storage;

	output::heading(format!(
		"Launching Minecraft server for: {}",
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

	let server_dir = app.root_dir.join("server");
	std::fs::create_dir_all(&server_dir)?;

	let java_override = args.java.as_deref();
	let launch_ctx = prepare_launch(
		modpack,
		storage,
		ctx.cache_dir(),
		super::LaunchSide::Server,
		&ctx.http_client,
		java_override,
		Some(&server_dir),
	)
	.await?;

	if args.eula {
		std::fs::write(server_dir.join("eula.txt"), "eula=true\n")?;
	}

	let root_dir = app.root_dir.as_path();
	let vfs_tree = build_server_vfs(storage, &app.cache, root_dir)?;
	crate::commands::launch::vfs::realize_vfs(&vfs_tree, &server_dir)?;

	let unix_args_path = if launch_ctx.loader_lib_dir.as_os_str().is_empty() {
		None
	} else {
		find_unix_args_path(
			&launch_ctx.loader_lib_dir,
			&modpack.minecraft_version,
		)
	};

	if let Some(ref unix_args) = unix_args_path {
		launch_with_unix_args(
			&args,
			&launch_ctx,
			&server_dir,
			unix_args,
			&modpack.minecraft_version,
		)?;
	} else {
		launch_with_classpath(
			&args,
			&launch_ctx,
			&server_dir,
			storage,
			&app.cache,
			&modpack.minecraft_version,
		)?;
	}

	Ok(())
}

fn launch_with_unix_args(
	args: &ServerArgs,
	launch_ctx: &super::prepare::LaunchContext,
	server_dir: &Path,
	unix_args: &Path,
	mc_version: &str,
) -> Result<()> {
	let server_lib_dir = server_dir.join("libraries");

	if !launch_ctx.loader_lib_dir.as_os_str().is_empty() {
		merge_libraries(&server_lib_dir, &launch_ctx.loader_lib_dir)?;
	}

	let lib_dir = if launch_ctx.loader_lib_dir.as_os_str().is_empty() {
		server_lib_dir
	} else {
		launch_ctx.loader_lib_dir.clone()
	};
	link_shim_jars(server_dir, &lib_dir, unix_args)?;

	let java_prefix = java_launch_prefix(&launch_ctx.java_path);

	let user_jvm_args_path = server_dir.join("user_jvm_args.txt");
	if !user_jvm_args_path.exists() {
		let mut defaults = Vec::new();
		defaults.push("-Xmx2G".to_string());
		if let Some(ref jvm) = args.jvm_args {
			for arg in jvm.split_whitespace() {
				defaults.push(arg.to_string());
			}
		}
		std::fs::write(&user_jvm_args_path, defaults.join("\n"))?;
	}

	let mut java_args = Vec::new();
	java_args.extend(super::java::log4j_mitigation_args(mc_version));

	if super::java::needs_log4j_config_override(mc_version)
		&& let Ok(config_args) = super::java::write_log4j_config(server_dir)
	{
		java_args.extend(config_args);
	}

	for arg in &launch_ctx.mc_jvm_args {
		if (arg.starts_with("--add-opens") || arg.starts_with("--add-exports"))
			&& !java_args.contains(arg)
		{
			java_args.push(arg.clone());
		}
	}

	java_args.push("@user_jvm_args.txt".to_string());
	java_args.push(format!("@{}", unix_args.to_string_lossy()));
	java_args.push("--port".to_string());
	java_args.push(args.port.to_string());
	java_args.push("nogui".to_string());

	output::blank_line();
	output::info(format!(
		"Starting Minecraft server on port {}...",
		args.port
	));
	install_signal_handlers();

	let cmd = build_java_command(
		&launch_ctx.java_path,
		&java_prefix,
		java_args,
		Some(server_dir),
	)?;
	let mut child = spawn_java_process(cmd)?;
	report_exit(wait_for_child(&mut child)?, "Server")
}

fn launch_with_classpath(
	args: &ServerArgs,
	launch_ctx: &super::prepare::LaunchContext,
	server_dir: &Path,
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
	minecraft_version: &str,
) -> Result<()> {
	let server_jar = launch_ctx.classpath_jars.first().ok_or_else(|| {
		crate::errors::YammmError::download_failed("No classpath jars found")
	})?;
	let dest_jar = server_dir.join("server.jar");
	if dest_jar.exists() {
		std::fs::remove_file(&dest_jar)?;
	}
	std::fs::copy(server_jar, &dest_jar)?;

	if super::java::needs_log4j_config_override(minecraft_version) {
		super::java::patch_log4j_config_in_jar(&dest_jar)?;
	}

	if !launch_ctx.skip_merge_libs
		&& !launch_ctx.loader_lib_dir.as_os_str().is_empty()
	{
		let server_lib_dir = server_dir.join("libraries");
		merge_libraries(&server_lib_dir, &launch_ctx.loader_lib_dir)?;
	}

	let resolved =
		resolve_classpath(launch_ctx, storage, cache, minecraft_version)?;
	let java_prefix = java_launch_prefix(&launch_ctx.java_path);
	let mut java_args = Vec::new();

	java_args.extend(super::java::log4j_mitigation_args(minecraft_version));

	if super::java::needs_log4j_config_override(minecraft_version)
		&& let Ok(config_args) = super::java::write_log4j_config(server_dir)
	{
		java_args.extend(config_args);
	}

	if let Some(resolved_args) = &resolved.resolved_jvm_args {
		java_args.extend(resolved_args.iter().cloned());
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

	if super::java::needs_log4j_config_override(minecraft_version) {
		let original = server_jar.to_string_lossy();
		let patched = dest_jar.to_string_lossy();
		java_args.push(
			resolved
				.classpath
				.replace(original.as_ref(), patched.as_ref()),
		);
	} else {
		java_args.push(resolved.classpath);
	}

	java_args.push(resolved.main_class);

	if !resolved.game_args.is_empty() {
		java_args.extend(resolved.game_args);
	}

	java_args.push("--port".to_string());
	java_args.push(args.port.to_string());
	java_args.push("nogui".to_string());

	output::blank_line();
	output::info(format!(
		"Starting Minecraft server on port {}...",
		args.port
	));
	install_signal_handlers();

	let cmd = build_java_command(
		&launch_ctx.java_path,
		&java_prefix,
		java_args,
		Some(server_dir),
	)?;
	let mut child = spawn_java_process(cmd)?;
	report_exit(wait_for_child(&mut child)?, "Server")
}

fn report_exit(
	status: std::process::ExitStatus,
	label: &str,
) -> Result<()> {
	if status.success() {
		output::success(format!("{} exited normally", label));
	} else {
		output::warning(format!("{} exited with: {}", label, status));
	}
	Ok(())
}

fn build_server_vfs(
	storage: &crate::storage::Storage,
	cache: &JarCache,
	root_dir: &Path,
) -> Result<VfsTree> {
	super::build_mod_vfs(storage, cache, super::LaunchSide::Server, root_dir)
}

fn find_unix_args_path(
	lib_dir: &Path,
	game_version: &str,
) -> Option<PathBuf> {
	find_unix_args_in_libraries(lib_dir, game_version)
}

fn find_unix_args_in_libraries(
	dir: &Path,
	_game_version: &str,
) -> Option<PathBuf> {
	let args_filename = if cfg!(windows) {
		"win_args.txt"
	} else {
		"unix_args.txt"
	};
	crate::utils::find_file_recursive(dir, args_filename)
		.or_else(|| crate::utils::find_file_recursive(dir, "unix_args.txt"))
}

fn link_shim_jars(
	server_dir: &Path,
	lib_dir: &Path,
	unix_args_path: &Path,
) -> Result<()> {
	let contents = std::fs::read_to_string(unix_args_path)?;
	for line in contents.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		for arg in line.split_whitespace() {
			if arg.ends_with(".jar") && !arg.starts_with('-') {
				let jar_path = PathBuf::from(arg);
				if !jar_path.is_absolute()
					&& let Some(jar_name) = jar_path.file_name()
					&& let Some(found) = find_file_in_libraries(
						lib_dir,
						jar_name.to_str().unwrap_or(""),
					) {
					let link = server_dir.join(jar_name);
					if !link.exists() {
						if let Some(parent) = link.parent() {
							std::fs::create_dir_all(parent)?;
						}
						crate::utils::create_symlink(
							&found.canonicalize().unwrap_or(found),
							&link,
						)?;
					}
				}
			}
		}
	}
	Ok(())
}

fn find_file_in_libraries(
	dir: &Path,
	filename: &str,
) -> Option<PathBuf> {
	crate::utils::find_file_recursive(dir, filename)
}
