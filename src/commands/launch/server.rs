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
	ResolvedClasspath, build_java_command, install_signal_handlers,
	resolve_classpath, spawn_java_process, wait_for_child,
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
		download_missing_mods(storage, &app.cache, ctx.http_client(), 4)
			.await?;
	output::present_download_summary(&summary);
	summary.into_result()?;

	let server_dir = app.root_dir.join("server");
	std::fs::create_dir_all(&server_dir)?;

	let java_override = args.java.as_deref();
	let launch_ctx = prepare_launch(
		modpack,
		storage,
		ctx.cache_dir(),
		super::LaunchSide::Server,
		ctx.http_client(),
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

	// I/O: ensure user_jvm_args.txt exists. Pure args assembly is below.
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

	let log4j_config_args =
		if super::java::needs_log4j_config_override(mc_version) {
			super::java::write_log4j_config(server_dir).unwrap_or_default()
		} else {
			Vec::new()
		};

	let java_args = build_unix_args_java_args(
		args,
		&launch_ctx.mc_jvm_args,
		unix_args,
		mc_version,
		&log4j_config_args,
	);

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

/// Pure assembly of the server java arg vector when the loader provides a
/// `@unix_args.txt` (Forge/NeoForge installer style). Logging side effects
/// and user_jvm_args.txt writing are the caller's responsibility.
///
/// Takes only the slices the builder reads (not a full `LaunchContext`) so
/// tests don't need to construct a `VersionInfo` to exercise this code.
fn build_unix_args_java_args(
	args: &ServerArgs,
	mc_jvm_args: &[String],
	unix_args: &Path,
	mc_version: &str,
	log4j_config_args: &[String],
) -> Vec<String> {
	let mut java_args = Vec::new();
	java_args.extend(super::java::log4j_mitigation_args(mc_version));
	java_args.extend(log4j_config_args.iter().cloned());

	for arg in mc_jvm_args {
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

	java_args
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

	let log4j_config_args =
		if super::java::needs_log4j_config_override(minecraft_version) {
			super::java::write_log4j_config(server_dir).unwrap_or_default()
		} else {
			Vec::new()
		};

	let java_args = build_classpath_java_args(
		args,
		&launch_ctx.mc_jvm_args,
		&resolved,
		server_jar,
		&dest_jar,
		minecraft_version,
		&log4j_config_args,
	);

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

/// Pure assembly of the server java arg vector when launching via classpath
/// (vanilla / Fabric / Quilt). The caller is responsible for copying
/// `server_jar` to `dest_jar` and patching log4j inside the jar; this fn
/// only decides which path appears in the final `-cp` argument.
#[allow(clippy::too_many_arguments)]
fn build_classpath_java_args(
	args: &ServerArgs,
	mc_jvm_args: &[String],
	resolved: &ResolvedClasspath,
	server_jar: &Path,
	dest_jar: &Path,
	mc_version: &str,
	log4j_config_args: &[String],
) -> Vec<String> {
	let mut java_args = Vec::new();

	java_args.extend(super::java::log4j_mitigation_args(mc_version));
	java_args.extend(log4j_config_args.iter().cloned());

	if let Some(resolved_args) = &resolved.resolved_jvm_args {
		java_args.extend(resolved_args.iter().cloned());
		java_args.push(crate::utils::ADD_OPENS_ARG.to_string());
		for arg in mc_jvm_args {
			if (arg.starts_with("--add-opens")
				|| arg.starts_with("--add-exports"))
				&& !java_args.contains(arg)
			{
				java_args.push(arg.clone());
			}
		}
	} else {
		java_args.extend(
			mc_jvm_args
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

	if super::java::needs_log4j_config_override(mc_version) {
		let original = server_jar.to_string_lossy();
		let patched = dest_jar.to_string_lossy();
		java_args.push(
			resolved
				.classpath
				.replace(original.as_ref(), patched.as_ref()),
		);
	} else {
		java_args.push(resolved.classpath.clone());
	}

	java_args.push(resolved.main_class.clone());

	if !resolved.game_args.is_empty() {
		java_args.extend(resolved.game_args.iter().cloned());
	}

	java_args.push("--port".to_string());
	java_args.push(args.port.to_string());
	java_args.push("nogui".to_string());

	java_args
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
	for jar_name in parse_shim_jar_filenames(&contents) {
		if let Some(found) = find_file_in_libraries(lib_dir, &jar_name) {
			let link = server_dir.join(&jar_name);
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
	Ok(())
}

/// Parse a Forge/NeoForge `unix_args.txt` file and return the bare filenames
/// of every relative `.jar` reference. Pure; no I/O.
///
/// Rules (mirroring the legacy inline parser):
/// - Lines beginning with `#` and blank lines are skipped.
/// - Tokens not ending in `.jar` are skipped.
/// - Tokens starting with `-` are flags, not paths — skipped.
/// - Absolute paths are skipped — the launcher already resolves those.
/// - For relative paths, only the final file-name component is kept (these
///   become symlink names in the server dir).
fn parse_shim_jar_filenames(contents: &str) -> Vec<String> {
	let mut names = Vec::new();
	for line in contents.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		for arg in line.split_whitespace() {
			if !arg.ends_with(".jar") || arg.starts_with('-') {
				continue;
			}
			let jar_path = PathBuf::from(arg);
			if jar_path.is_absolute() {
				continue;
			}
			if let Some(file_name) = jar_path.file_name()
				&& let Some(name_str) = file_name.to_str()
			{
				names.push(name_str.to_string());
			}
		}
	}
	names
}

fn find_file_in_libraries(
	dir: &Path,
	filename: &str,
) -> Option<PathBuf> {
	crate::utils::find_file_recursive(dir, filename)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_server_args(
		port: u16,
		jvm_args: Option<&str>,
	) -> ServerArgs {
		ServerArgs {
			port,
			jvm_args: jvm_args.map(|s| s.to_string()),
			eula: false,
			java: None,
		}
	}

	fn make_resolved(
		classpath: &str,
		resolved_jvm: Option<Vec<String>>,
		main_class: &str,
		game_args: Vec<String>,
	) -> ResolvedClasspath {
		ResolvedClasspath {
			classpath: classpath.to_string(),
			resolved_jvm_args: resolved_jvm,
			main_class: main_class.to_string(),
			game_args,
		}
	}

	// ---- parse_shim_jar_filenames ----

	#[test]
	fn test_parse_shim_skips_comments_and_blanks() {
		let content = "# comment line\n\n  \nfoo.jar\n";
		assert_eq!(parse_shim_jar_filenames(content), vec!["foo.jar"]);
	}

	#[test]
	fn test_parse_shim_skips_non_jar_tokens() {
		let content = "main.class some-arg foo.jar bar.txt baz.JAR\n";
		assert_eq!(parse_shim_jar_filenames(content), vec!["foo.jar"]);
	}

	#[test]
	fn test_parse_shim_skips_flag_tokens() {
		let content = "-Xmx2G --some=foo.jar real.jar\n";
		assert_eq!(parse_shim_jar_filenames(content), vec!["real.jar"]);
	}

	#[test]
	fn test_parse_shim_skips_absolute_paths() {
		let content = "/abs/path/skipme.jar relative/keepme.jar\n";
		assert_eq!(parse_shim_jar_filenames(content), vec!["keepme.jar"]);
	}

	#[test]
	fn test_parse_shim_strips_directory_components() {
		let content = "libs/sub/dir/cool.jar\n";
		assert_eq!(parse_shim_jar_filenames(content), vec!["cool.jar"]);
	}

	#[test]
	fn test_parse_shim_handles_multiple_jars_per_line() {
		let content = "a.jar b.jar  c.jar\n";
		assert_eq!(
			parse_shim_jar_filenames(content),
			vec!["a.jar", "b.jar", "c.jar"]
		);
	}

	// ---- build_unix_args_java_args ----

	#[test]
	fn test_unix_args_appends_port_and_nogui_at_tail() {
		let args = make_server_args(25566, None);
		let result = build_unix_args_java_args(
			&args,
			&[],
			Path::new("/loader/unix_args.txt"),
			"1.20.4",
			&[],
		);

		let tail = &result[result.len() - 5..];
		assert_eq!(tail[0], "@user_jvm_args.txt");
		assert!(tail[1].starts_with("@/loader/unix_args.txt"));
		assert_eq!(tail[2], "--port");
		assert_eq!(tail[3], "25566");
		assert_eq!(tail[4], "nogui");
	}

	#[test]
	fn test_unix_args_filters_mc_jvm_args_to_module_flags() {
		let mc_jvm = vec![
			"--add-opens=java.base/sun.nio=ALL-UNNAMED".to_string(),
			"-Xmx1G".to_string(),
			"--add-exports=java.base/sun.foo=ALL-UNNAMED".to_string(),
		];
		let args = make_server_args(25565, None);
		let result = build_unix_args_java_args(
			&args,
			&mc_jvm,
			Path::new("u.txt"),
			"1.20.4",
			&[],
		);

		assert!(result.iter().any(|a| a.starts_with("--add-opens=")));
		assert!(result.iter().any(|a| a.starts_with("--add-exports=")));
		assert!(
			!result.iter().any(|a| a == "-Xmx1G"),
			"only --add-opens/--add-exports survive the filter"
		);
	}

	#[test]
	fn test_unix_args_prepends_log4j_config_args() {
		let args = make_server_args(25565, None);
		let log4j_args = vec!["-Dlog4j.configurationFile=/foo".to_string()];
		let result = build_unix_args_java_args(
			&args,
			&[],
			Path::new("u.txt"),
			"1.20.4",
			&log4j_args,
		);

		let user_jvm_idx = result
			.iter()
			.position(|a| a == "@user_jvm_args.txt")
			.unwrap();
		let log4j_idx = result
			.iter()
			.position(|a| a.starts_with("-Dlog4j.configurationFile="))
			.unwrap();
		assert!(log4j_idx < user_jvm_idx);
	}

	// ---- build_classpath_java_args ----

	#[test]
	fn test_classpath_args_no_log4j_uses_original_classpath() {
		let args = make_server_args(25565, None);
		let resolved = make_resolved(
			"/orig/server.jar:/lib/a.jar",
			None,
			"net.minecraft.server.Main",
			vec![],
		);
		let server_jar = PathBuf::from("/orig/server.jar");
		let dest_jar = PathBuf::from("/dest/server.jar");

		let result = build_classpath_java_args(
			&args,
			&[],
			&resolved,
			&server_jar,
			&dest_jar,
			"1.20.4",
			&[],
		);

		let cp_idx = result.iter().position(|a| a == "-cp").unwrap();
		assert_eq!(result[cp_idx + 1], "/orig/server.jar:/lib/a.jar");
		assert_eq!(result[cp_idx + 2], "net.minecraft.server.Main");
	}

	#[test]
	fn test_classpath_args_log4j_swaps_server_jar_to_dest() {
		let args = make_server_args(25565, None);
		let resolved = make_resolved(
			"/orig/server.jar:/lib/a.jar",
			None,
			"net.minecraft.server.Main",
			vec![],
		);
		let server_jar = PathBuf::from("/orig/server.jar");
		let dest_jar = PathBuf::from("/dest/server.jar");

		let result = build_classpath_java_args(
			&args,
			&[],
			&resolved,
			&server_jar,
			&dest_jar,
			"1.17.1",
			&[],
		);

		let cp_idx = result.iter().position(|a| a == "-cp").unwrap();
		assert_eq!(result[cp_idx + 1], "/dest/server.jar:/lib/a.jar");
	}

	#[test]
	fn test_classpath_args_user_jvm_args_split_and_appended_before_cp() {
		let args = make_server_args(25565, Some("-Xmx2G -XX:+UseG1GC"));
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let result = build_classpath_java_args(
			&args,
			&[],
			&resolved,
			Path::new("/s.jar"),
			Path::new("/d.jar"),
			"1.20.4",
			&[],
		);

		let xmx_idx = result.iter().position(|a| a == "-Xmx2G").unwrap();
		let gc_idx = result.iter().position(|a| a == "-XX:+UseG1GC").unwrap();
		let cp_idx = result.iter().position(|a| a == "-cp").unwrap();
		assert!(xmx_idx < cp_idx && gc_idx < cp_idx);
	}

	#[test]
	fn test_classpath_args_resolved_jvm_path_includes_add_opens_arg() {
		let mc_jvm = vec!["--add-opens=java.base/sun.foo=ALL".to_string()];
		let args = make_server_args(25565, None);
		let resolved = make_resolved(
			"cp",
			Some(vec!["-Dprop=v".to_string()]),
			"Main",
			vec![],
		);
		let result = build_classpath_java_args(
			&args,
			&mc_jvm,
			&resolved,
			Path::new("/s.jar"),
			Path::new("/d.jar"),
			"1.20.4",
			&[],
		);

		assert!(result.iter().any(|a| a == "-Dprop=v"));
		assert!(result.iter().any(|a| a == crate::utils::ADD_OPENS_ARG));
		assert!(
			result
				.iter()
				.any(|a| a == "--add-opens=java.base/sun.foo=ALL")
		);
	}

	#[test]
	fn test_classpath_args_tail_is_port_and_nogui() {
		let args = make_server_args(25577, None);
		let resolved =
			make_resolved("cp", None, "Main", vec!["--demo".to_string()]);
		let result = build_classpath_java_args(
			&args,
			&[],
			&resolved,
			Path::new("/s.jar"),
			Path::new("/d.jar"),
			"1.20.4",
			&[],
		);

		let tail = &result[result.len() - 3..];
		assert_eq!(
			tail,
			&[
				"--port".to_string(),
				"25577".to_string(),
				"nogui".to_string()
			]
		);
		assert!(
			result.iter().position(|a| a == "--demo").unwrap()
				< result.len() - 3
		);
	}
}
