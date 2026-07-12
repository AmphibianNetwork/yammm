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
	ResolvedClasspath, build_java_command, install_signal_handlers,
	resolve_classpath, spawn_java_process, wait_for_child,
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

/// Auth state to pass into `build_client_java_args` without dragging in the
/// full `auth::AuthToken` (which carries refresh tokens / expiry that the
/// launcher arg vector doesn't need to know about).
enum ClientAuth {
	Offline,
	Online {
		username: String,
		access_token: String,
		uuid: String,
	},
}

/// All inputs needed to assemble the Minecraft client launch arg vector.
/// Borrows everything so tests can stack-allocate the bits they care about.
/// Deliberately does NOT carry a full `LaunchContext` — only the fields the
/// args builder reads — so tests don't need to construct a `VersionInfo`.
struct ClientLaunchInputs<'a> {
	natives_dir: &'a Path,
	mc_cache: &'a Path,
	mc_jvm_args: &'a [String],
	args: &'a ClientArgs,
	resolved: &'a ResolvedClasspath,
	client_dir: &'a Path,
	mc_version: &'a str,
	asset_index_id: Option<&'a str>,
	/// Args from `write_log4j_config` if the MC version is affected by
	/// CVE-2021-44228; empty otherwise. Lifted out of this fn because it
	/// performs I/O — the caller owns the side effect, we own the pure
	/// args assembly.
	log4j_config_args: &'a [String],
	auth: &'a ClientAuth,
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
		download_missing_mods(storage, &app.cache, ctx.http_client(), 4)
			.await?;
	output::present_download_summary(&summary);
	summary.into_result()?;

	let java_override = args.java.as_deref();
	let launch_ctx = prepare_launch(
		modpack,
		storage,
		ctx.cache_dir(),
		super::LaunchSide::Client,
		ctx.http_client(),
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

	// Side-effect I/O: write the log4j patch config if needed. Kept out of
	// the args builder so the builder remains a pure function we can test.
	let log4j_config_args =
		if super::java::needs_log4j_config_override(&modpack.minecraft_version)
		{
			super::java::write_log4j_config(&client_dir).unwrap_or_default()
		} else {
			Vec::new()
		};

	let auth = if args.offline {
		ClientAuth::Offline
	} else {
		// get_valid_token refreshes expired access tokens via the stored
		// refresh token, falling back to a full interactive login. Using
		// load_token() directly would hand Minecraft a stale token after the
		// ~1h MSA access-token TTL elapsed.
		let token = crate::auth::get_valid_token(ctx.http_client()).await?;
		ClientAuth::Online {
			username: token.username,
			access_token: token.access_token,
			uuid: token.uuid,
		}
	};

	let asset_index_id = launch_ctx
		.version_info
		.asset_index
		.as_ref()
		.map(|a| a.id.as_str());

	let java_args = build_client_java_args(ClientLaunchInputs {
		natives_dir: &launch_ctx.natives_dir,
		mc_cache: &launch_ctx.mc_cache,
		mc_jvm_args: &launch_ctx.mc_jvm_args,
		args: &args,
		resolved: &resolved,
		client_dir: &client_dir,
		mc_version: &modpack.minecraft_version,
		asset_index_id,
		log4j_config_args: &log4j_config_args,
		auth: &auth,
	});

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

/// Assemble the full Minecraft client launch arg vector.
///
/// Pure function — no I/O, no globals. All inputs flow in through
/// `ClientLaunchInputs`. Output order matters: Minecraft is sensitive to the
/// position of `-cp` (must immediately precede the main class), so the layout
/// here is significant and is the main thing the tests pin down.
fn build_client_java_args(inputs: ClientLaunchInputs<'_>) -> Vec<String> {
	let ClientLaunchInputs {
		natives_dir,
		mc_cache,
		mc_jvm_args,
		args,
		resolved,
		client_dir,
		mc_version,
		asset_index_id,
		log4j_config_args,
		auth,
	} = inputs;

	let mut java_args: Vec<String> = Vec::new();

	#[cfg(target_os = "macos")]
	java_args.push("-XstartOnFirstThread".to_string());

	java_args.push(format!(
		"-Djava.library.path={}",
		natives_dir.to_string_lossy()
	));

	java_args.extend(super::java::log4j_mitigation_args(mc_version));
	java_args.extend(log4j_config_args.iter().cloned());

	if let Some(resolved_jvm) = &resolved.resolved_jvm_args {
		java_args.extend(resolved_jvm.iter().cloned());
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
	java_args.push(resolved.classpath.clone());
	java_args.push(resolved.main_class.clone());

	if !resolved.game_args.is_empty() {
		java_args.extend(resolved.game_args.iter().cloned());
	}

	java_args.push("--gameDir".to_string());
	java_args.push(client_dir.to_string_lossy().to_string());
	java_args.push("--assetsDir".to_string());
	java_args.push(mc_cache.join("assets").to_string_lossy().to_string());
	if let Some(asset_index) = asset_index_id {
		java_args.push("--assetIndex".to_string());
		java_args.push(asset_index.to_string());
	}
	java_args.push("--version".to_string());
	java_args.push(mc_version.to_string());

	match auth {
		ClientAuth::Offline => {
			java_args.push("--username".to_string());
			java_args.push("Player".to_string());
			java_args.push("--accessToken".to_string());
			java_args.push("0".to_string());
			java_args.push("--uuid".to_string());
			java_args.push("00000000000000000000000000000000".to_string());
		}
		ClientAuth::Online {
			username,
			access_token,
			uuid,
		} => {
			java_args.push("--username".to_string());
			java_args.push(username.clone());
			java_args.push("--accessToken".to_string());
			java_args.push(access_token.clone());
			java_args.push("--uuid".to_string());
			java_args.push(uuid.clone());
		}
	}

	java_args.push("--versionType".to_string());
	java_args.push("yammm".to_string());

	java_args
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

#[cfg(test)]
mod tests {
	use super::*;

	fn make_client_args(
		offline: bool,
		jvm_args: Option<&str>,
	) -> ClientArgs {
		ClientArgs {
			offline,
			jvm_args: jvm_args.map(|s| s.to_string()),
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

	/// Build an inputs struct with empty/default values that tests then
	/// override field-by-field — keeps each test focused on what it asserts.
	fn default_inputs<'a>(
		args: &'a ClientArgs,
		resolved: &'a ResolvedClasspath,
		auth: &'a ClientAuth,
	) -> ClientLaunchInputs<'a> {
		ClientLaunchInputs {
			natives_dir: Path::new("/cache/natives"),
			mc_cache: Path::new("/cache/mc"),
			mc_jvm_args: &[],
			args,
			resolved,
			client_dir: Path::new("/pack/client"),
			mc_version: "1.20.4",
			asset_index_id: None,
			log4j_config_args: &[],
			auth,
		}
	}

	#[test]
	fn test_offline_emits_player_credentials() {
		let args = make_client_args(true, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		// Find `--username` and check the three offline values follow.
		let idx = result.iter().position(|a| a == "--username").unwrap();
		assert_eq!(result[idx + 1], "Player");
		assert_eq!(result[idx + 2], "--accessToken");
		assert_eq!(result[idx + 3], "0");
		assert_eq!(result[idx + 4], "--uuid");
		assert_eq!(result[idx + 5], "00000000000000000000000000000000");
	}

	#[test]
	fn test_online_emits_token_fields() {
		let args = make_client_args(false, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let auth = ClientAuth::Online {
			username: "alice".to_string(),
			access_token: "tok-abc".to_string(),
			uuid: "uuid-xyz".to_string(),
		};
		let result =
			build_client_java_args(default_inputs(&args, &resolved, &auth));

		let idx = result.iter().position(|a| a == "--username").unwrap();
		assert_eq!(result[idx + 1], "alice");
		assert_eq!(result[idx + 3], "tok-abc");
		assert_eq!(result[idx + 5], "uuid-xyz");
	}

	#[test]
	fn test_cp_immediately_precedes_main_class() {
		let args = make_client_args(true, None);
		let resolved = make_resolved(
			"/lib/a.jar:/lib/b.jar",
			None,
			"net.minecraft.client.main.Main",
			vec![],
		);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		let cp_idx = result.iter().position(|a| a == "-cp").unwrap();
		assert_eq!(result[cp_idx + 1], "/lib/a.jar:/lib/b.jar");
		assert_eq!(result[cp_idx + 2], "net.minecraft.client.main.Main");
	}

	#[test]
	fn test_java_library_path_present() {
		let args = make_client_args(true, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		assert!(
			result
				.iter()
				.any(|a| a == "-Djava.library.path=/cache/natives")
		);
	}

	#[test]
	fn test_asset_index_only_when_some() {
		let args = make_client_args(true, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let auth = ClientAuth::Offline;

		// None → no --assetIndex flag
		let result =
			build_client_java_args(default_inputs(&args, &resolved, &auth));
		assert!(!result.iter().any(|a| a == "--assetIndex"));

		// Some → flag + value appear
		let mut inputs = default_inputs(&args, &resolved, &auth);
		inputs.asset_index_id = Some("17");
		let result = build_client_java_args(inputs);
		let idx = result.iter().position(|a| a == "--assetIndex").unwrap();
		assert_eq!(result[idx + 1], "17");
	}

	#[test]
	fn test_version_type_yammm_appended() {
		let args = make_client_args(true, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		let vt_idx = result.iter().position(|a| a == "--versionType").unwrap();
		assert_eq!(result[vt_idx + 1], "yammm");
	}

	#[test]
	fn test_resolved_jvm_path_emits_add_opens_arg_and_filters_mc_jvm_args() {
		let args = make_client_args(true, None);
		let resolved = make_resolved(
			"cp",
			Some(vec!["-Dprop=v".to_string()]),
			"Main",
			vec![],
		);
		let mc_jvm = vec![
			"--add-opens=java.base/sun.foo=ALL".to_string(),
			"-Xmx1G".to_string(), // not an --add-opens / --add-exports → filtered
		];

		let mut inputs = default_inputs(&args, &resolved, &ClientAuth::Offline);
		inputs.mc_jvm_args = &mc_jvm;
		let result = build_client_java_args(inputs);

		assert!(result.iter().any(|a| a == "-Dprop=v"));
		assert!(result.iter().any(|a| a == crate::utils::ADD_OPENS_ARG));
		assert!(
			result
				.iter()
				.any(|a| a == "--add-opens=java.base/sun.foo=ALL")
		);
		assert!(
			!result.iter().any(|a| a == "-Xmx1G"),
			"non-module mc_jvm_args entries are filtered on the resolved path"
		);
	}

	#[test]
	fn test_no_resolved_jvm_path_filters_library_path_from_mc_jvm_args() {
		let args = make_client_args(true, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let mc_jvm = vec![
			"-Djava.library.path=stale".to_string(), // duplicated by us; must be filtered
			"-Dthrough.passthrough=ok".to_string(),
		];

		let mut inputs = default_inputs(&args, &resolved, &ClientAuth::Offline);
		inputs.mc_jvm_args = &mc_jvm;
		let result = build_client_java_args(inputs);

		assert!(result.iter().any(|a| a == "-Dthrough.passthrough=ok"));
		// We set our own -Djava.library.path=/cache/natives; the stale one must
		// not appear in the final args.
		assert_eq!(
			result
				.iter()
				.filter(|a| a.starts_with("-Djava.library.path="))
				.count(),
			1
		);
	}

	#[test]
	fn test_user_jvm_args_are_space_split() {
		let args = make_client_args(
			true,
			Some("-Xmx4G -XX:+UseG1GC -XX:MaxGCPauseMillis=50"),
		);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		assert!(result.iter().any(|a| a == "-Xmx4G"));
		assert!(result.iter().any(|a| a == "-XX:+UseG1GC"));
		assert!(result.iter().any(|a| a == "-XX:MaxGCPauseMillis=50"));
	}

	#[test]
	fn test_game_args_appear_before_game_dir() {
		let args = make_client_args(true, None);
		let resolved = make_resolved(
			"cp",
			None,
			"Main",
			vec!["--demo".to_string(), "--quickPlay".to_string()],
		);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		let demo_idx = result.iter().position(|a| a == "--demo").unwrap();
		let game_dir_idx =
			result.iter().position(|a| a == "--gameDir").unwrap();
		assert!(demo_idx < game_dir_idx);
	}

	#[test]
	fn test_log4j_config_args_appear_before_resolved_jvm() {
		let args = make_client_args(true, None);
		let resolved = make_resolved(
			"cp",
			Some(vec!["-Dlate=arg".to_string()]),
			"Main",
			vec![],
		);
		let log4j = vec!["-Dlog4j.configurationFile=/path".to_string()];

		let mut inputs = default_inputs(&args, &resolved, &ClientAuth::Offline);
		inputs.log4j_config_args = &log4j;
		let result = build_client_java_args(inputs);

		let log4j_idx = result
			.iter()
			.position(|a| a.starts_with("-Dlog4j.configurationFile="))
			.unwrap();
		let late_idx = result.iter().position(|a| a == "-Dlate=arg").unwrap();
		assert!(log4j_idx < late_idx);
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn test_macos_prepends_start_on_first_thread() {
		let args = make_client_args(true, None);
		let resolved = make_resolved("cp", None, "Main", vec![]);
		let result = build_client_java_args(default_inputs(
			&args,
			&resolved,
			&ClientAuth::Offline,
		));

		assert_eq!(
			result.first().map(|s| s.as_str()),
			Some("-XstartOnFirstThread")
		);
	}
}
