use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

mod client;
mod java;
mod libraries;
mod loader;
mod prepare;
mod server;
mod vfs;

pub use client::ClientArgs;
pub use server::ServerArgs;

const SHUTDOWN_TIMEOUT_SECS: u64 = 10;
const PROCESS_POLL_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LaunchSide {
	Client,
	Server,
}

impl LaunchSide {
	fn as_str(&self) -> &'static str {
		match self {
			LaunchSide::Client => "client",
			LaunchSide::Server => "server",
		}
	}
}

impl std::fmt::Display for LaunchSide {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
static CHILD_PID: std::sync::atomic::AtomicU32 =
	std::sync::atomic::AtomicU32::new(0);
#[cfg(windows)]
static CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);

fn install_signal_handlers() {
	#[cfg(unix)]
	{
		let _ = ctrlc::set_handler(|| {
			INTERRUPTED.store(true, Ordering::Relaxed);
			let pid = CHILD_PID.load(Ordering::Relaxed);
			if pid > 0 {
				unsafe {
					libc::kill(pid as i32, libc::SIGINT);
				}
			}
		});

		unsafe {
			libc::signal(
				libc::SIGTERM,
				sigterm_handler as *const () as libc::sighandler_t,
			);
		}
	}
	#[cfg(windows)]
	{
		let _ = ctrlc::set_handler(|| {
			INTERRUPTED.store(true, Ordering::Relaxed);
			if let Ok(mut guard) = CHILD.lock() {
				if let Some(child) = guard.as_mut() {
					let _ = child.kill();
				}
			}
		});
	}
}

#[cfg(unix)]
extern "C" fn sigterm_handler(_sig: libc::c_int) {
	let pid = CHILD_PID.load(Ordering::Relaxed);
	if pid > 0 {
		unsafe {
			libc::kill(pid as i32, libc::SIGTERM);
		}
	}
	std::process::exit(143);
}

fn spawn_java_process(
	mut cmd: std::process::Command
) -> Result<std::process::Child> {
	let child = cmd.spawn()?;
	#[cfg(unix)]
	{
		CHILD_PID.store(child.id(), Ordering::Relaxed);
	}
	#[cfg(windows)]
	{
		if let Ok(mut guard) = CHILD.lock() {
			*guard = Some(child.try_clone()?);
		}
	}
	Ok(child)
}

fn wait_for_child(
	child: &mut std::process::Child
) -> Result<std::process::ExitStatus> {
	let mut deadline: Option<std::time::Instant> = None;
	loop {
		match child.try_wait() {
			Ok(Some(status)) => return Ok(status),
			Ok(None) => {}
			Err(e) => {
				if e.kind() == std::io::ErrorKind::InvalidData {
					std::thread::sleep(std::time::Duration::from_millis(
						PROCESS_POLL_INTERVAL_MS,
					));
					continue;
				}
				return Err(e.into());
			}
		}

		if INTERRUPTED.load(Ordering::Relaxed) && deadline.is_none() {
			crate::output::info("Shutting down...");
			deadline = Some(
				std::time::Instant::now()
					+ std::time::Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
			);
		}

		if let Some(dl) = deadline {
			if std::time::Instant::now() > dl {
				crate::output::warning(
					"Server did not exit in time, force killing",
				);
				let _ = child.kill();
				return Ok(child.wait()?);
			}
		}

		std::thread::sleep(std::time::Duration::from_millis(
			PROCESS_POLL_INTERVAL_MS,
		));
	}
}

#[derive(Parser, Debug)]
pub struct LaunchCommand {
	#[command(subcommand)]
	pub command: Option<LaunchSubcommand>,
}

#[derive(Parser, Debug)]
pub enum LaunchSubcommand {
	Client(ClientArgs),
	Server(ServerArgs),
}

impl LaunchCommand {
	pub async fn run(
		self,
		ctx: crate::app::AppContext,
	) -> Result<()> {
		if let Some(command) = self.command {
			match command {
				LaunchSubcommand::Client(args) => client::run(args, ctx).await,
				LaunchSubcommand::Server(args) => server::run(args, ctx).await,
			}
		} else {
			client::run(ClientArgs::parse(), ctx).await
		}
	}
}

fn build_mod_vfs(
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
	side: LaunchSide,
	root_dir: &Path,
) -> Result<vfs::VfsTree> {
	let mut tree = vfs::VfsTree::new();

	let mods_dir = PathBuf::from("mods");
	tree.add_dir(&mods_dir);

	let mods = storage.list(crate::types::ProjectType::Mod)?;
	for mod_ron in &mods {
		if let Some(jar_path) = mod_ron
			.hash
			.as_ref()
			.and_then(|h| cache.get(mod_ron.hash_type, h))
		{
			let slug = crate::utils::slugify(&mod_ron.name);
			tree.add_file(&mods_dir.join(format!("{}.jar", slug)), jar_path);
		}
	}

	let config_dest = PathBuf::from("config");
	tree.add_dir(&config_dest);
	populate_config_vfs(&mut tree, &config_dest, storage, side, root_dir)?;

	let resources_dir = root_dir.join("resources").join(side.as_str());
	if resources_dir.exists() {
		tree.add_dir_from_source(&PathBuf::from("."), &resources_dir);
	}

	Ok(tree)
}

fn populate_config_vfs(
	tree: &mut vfs::VfsTree,
	config_dest: &Path,
	storage: &crate::storage::Storage,
	side: LaunchSide,
	root_dir: &Path,
) -> Result<()> {
	let mods = storage
		.list(crate::types::ProjectType::Mod)
		.unwrap_or_default();

	for mod_ron in &mods {
		let mod_dir = storage.mod_store().base_dir().join(&mod_ron.id);
		let config_dirs: Vec<PathBuf> = vec![
			mod_dir.join("config"),
			mod_dir.join(side.as_str()).join("config"),
		];

		for config_src in &config_dirs {
			if config_src.exists() && config_src.is_dir() {
				tree.add_dir_from_source(config_dest, config_src);
			}
		}
	}

	let global_config = root_dir.join("config");
	if global_config.exists() {
		tree.add_dir_from_source(config_dest, &global_config);
	}

	Ok(())
}

fn collect_mod_jars(
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
) -> Result<Vec<PathBuf>> {
	let mods = storage.list(crate::types::ProjectType::Mod)?;
	Ok(mods
		.iter()
		.filter_map(|m| m.hash.as_ref().and_then(|h| cache.get(m.hash_type, h)))
		.collect())
}

fn build_classpath(jars: &[PathBuf]) -> String {
	jars.iter()
		.map(|p| p.to_string_lossy().to_string())
		.collect::<Vec<_>>()
		.join(crate::utils::CLASSPATH_SEPARATOR)
}

fn resolve_jvm_args(
	args: &[String],
	loader_lib_dir: &Path,
	version_name: &str,
) -> Vec<String> {
	args.iter()
		.map(|arg| {
			arg.replace(
				"${library_directory}",
				&loader_lib_dir.to_string_lossy(),
			)
			.replace(
				"${classpath_separator}",
				crate::utils::CLASSPATH_SEPARATOR,
			)
			.replace("${version_name}", version_name)
		})
		.collect()
}

pub(super) fn extract_module_path_jars(
	resolved_args: &[String]
) -> Vec<PathBuf> {
	let mut jars = Vec::new();
	let mut iter = resolved_args.iter();
	while let Some(arg) = iter.next() {
		if arg == "-p" {
			if let Some(path_str) = iter.next() {
				for part in path_str.split(crate::utils::CLASSPATH_SEPARATOR) {
					let p = PathBuf::from(part);
					if p.exists() {
						jars.push(p);
					}
				}
			}
		}
	}
	jars
}

struct ResolvedClasspath {
	classpath: String,
	resolved_jvm_args: Option<Vec<String>>,
	main_class: String,
	game_args: Vec<String>,
}

fn resolve_classpath(
	launch_ctx: &prepare::LaunchContext,
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
	minecraft_version: &str,
) -> Result<ResolvedClasspath> {
	let mut jars = launch_ctx.classpath_jars.clone();
	if launch_ctx.loader_jvm_args.is_empty() {
		jars.extend(collect_mod_jars(storage, cache)?);
	}

	let resolved_jvm_args = if !launch_ctx.loader_jvm_args.is_empty() {
		Some(resolve_jvm_args(
			&launch_ctx.loader_jvm_args,
			&launch_ctx.loader_lib_dir,
			minecraft_version,
		))
	} else {
		None
	};

	let module_path_jars = match &resolved_jvm_args {
		Some(resolved) => extract_module_path_jars(resolved),
		None => Vec::new(),
	};
	jars.retain(|jar| !module_path_jars.contains(jar));

	Ok(ResolvedClasspath {
		classpath: build_classpath(&jars),
		resolved_jvm_args,
		main_class: launch_ctx.main_class.clone(),
		game_args: launch_ctx.loader_game_args.clone(),
	})
}

fn build_java_command(
	_java_path: &Path,
	java_prefix: &[String],
	java_args: Vec<String>,
	current_dir: Option<&Path>,
) -> Result<std::process::Command> {
	let mut all_args = Vec::new();
	if java_prefix.len() > 1 {
		all_args.extend_from_slice(&java_prefix[1..]);
	}
	all_args.extend(java_args);

	let mut cmd = std::process::Command::new(&java_prefix[0]);
	cmd.args(&all_args)
		.stdin(std::process::Stdio::inherit())
		.stdout(std::process::Stdio::inherit())
		.stderr(std::process::Stdio::inherit());
	if let Some(dir) = current_dir {
		cmd.current_dir(dir);
	}
	Ok(cmd)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_build_classpath() {
		let jars = vec![PathBuf::from("/a/b.jar"), PathBuf::from("/c/d.jar")];
		let cp = build_classpath(&jars);
		assert_eq!(
			cp,
			format!("/a/b.jar{}/c/d.jar", crate::utils::CLASSPATH_SEPARATOR)
		);
	}
}
