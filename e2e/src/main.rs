mod matrix;
mod report;
mod runner;
mod tui;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;
use console::style;

use crate::matrix::{LaunchSide, Loader, TestCase};
use crate::runner::{Runner, RunnerConfig};

#[derive(Parser, Debug)]
#[command(name = "yammm-e2e")]
#[command(about = "End-to-end test runner for yammm")]
#[command(after_help = "EXAMPLES:\n  \
    yammm-e2e                          Run all tests\n  \
    yammm-e2e -l fabric                Run only Fabric tests\n  \
    yammm-e2e -l fabric -l forge       Run Fabric + Forge tests\n  \
    yammm-e2e -v 1.21.1               Run only MC 1.21.1 tests\n  \
    yammm-e2e -v 1.21.1 -v 1.20.1     Run 1.21.1 + 1.20.1 tests\n  \
    yammm-e2e -l fabric -v 1.21.1     Combine loader + version filters\n  \
    yammm-e2e --side server            Only test server launches\n  \
    yammm-e2e --side client            Only test client launches\n  \
    yammm-e2e --mod sodium             Use 'sodium' as the test mod\n  \
    yammm-e2e -i                       Interactive test selection (TUI)")]
struct Cli {
	/// Filter by mod loader (can be repeated)
	#[arg(short = 'l', long = "loader", value_parser = parse_loader)]
	loaders: Vec<Loader>,

	/// Filter by Minecraft version (can be repeated)
	#[arg(short = 'v', long = "version")]
	versions: Vec<String>,

	/// Which side to test (server, client, or both)
	#[arg(short = 's', long = "side", default_value = "both", value_parser = clap::value_parser!(LaunchSide))]
	side: LaunchSide,

	/// Mod slug to use for add/remove tests
	#[arg(short = 'm', long = "mod")]
	mod_slug: Option<String>,

	/// Interactive TUI test selection
	#[arg(short = 'i', long = "interactive")]
	interactive: bool,

	/// List all test cases without running
	#[arg(long = "list")]
	list: bool,

	/// Path to yammm binary
	#[arg(long = "bin")]
	yammm_bin: Option<PathBuf>,

	/// Timeout per test in seconds
	#[arg(long = "timeout", default_value = "90")]
	timeout: u64,

	/// Keep test directories after run
	#[arg(long = "no-cleanup")]
	no_cleanup: bool,

	/// Skip building yammm (use existing binary)
	#[arg(long = "skip-build")]
	skip_build: bool,
}

fn parse_loader(s: &str) -> Result<Loader> {
	s.parse::<Loader>().map_err(|e| anyhow::anyhow!("{e}"))
}

fn main() -> Result<()> {
	let cli = Cli::parse();

	let all_tests = matrix::test_matrix();
	let filtered =
		matrix::filter_tests(&all_tests, &cli.loaders, &cli.versions);

	if cli.list {
		list_tests(&filtered);
		return Ok(());
	}

	let tests = if cli.interactive {
		let selected = tui::interactive_select(filtered)?;
		if selected.is_empty() {
			println!("No tests selected.");
			return Ok(());
		}
		selected
	} else {
		if filtered.is_empty() {
			println!("No tests match the given filters.");
			return Ok(());
		}
		filtered
	};

	let yammm_bin = if let Some(bin) = cli.yammm_bin {
		if !bin.exists() {
			bail!("yammm binary not found at {}", bin.display());
		}
		bin
	} else if cli.skip_build {
		runner::find_yammm_bin()?
	} else {
		runner::build_yammm()?
	};

	let work_dir =
		runner::find_project_root(&yammm_bin).join("target/e2e-work");

	println!();
	println!(
		"{}",
		style("╔═══════════════════════════════════════════╗").bold()
	);
	println!(
		"{}",
		style("║        yammm e2e test suite               ║").bold()
	);
	println!(
		"{}",
		style("╚═══════════════════════════════════════════╝").bold()
	);
	println!();

	let java_ver =
		runner::detect_java_version().unwrap_or_else(|| "unknown".to_string());
	log(&format!("Java version: {java_ver}"));
	log(&format!("yammm binary:  {}", yammm_bin.display()));
	log(&format!(
		"Side:          {}",
		match cli.side {
			LaunchSide::Server => "server",
			LaunchSide::Client => "client",
			LaunchSide::Both => "both",
		}
	));
	log(&format!("Timeout:       {}s", cli.timeout));
	log(&format!(
		"Mod slug:      {}",
		cli.mod_slug.as_deref().unwrap_or("<per-loader default>")
	));
	log(&format!(
		"Cleanup:       {}",
		if cli.no_cleanup {
			"off (--no-cleanup)"
		} else {
			"on"
		}
	));

	std::fs::create_dir_all(&work_dir)?;

	let work_dir_clone = work_dir.clone();
	let no_cleanup = cli.no_cleanup;
	ctrlc::set_handler(move || {
		println!();
		println!("{}", style("Interrupted — stopping...").yellow());
		if !no_cleanup {
			let _ = std::fs::remove_dir_all(&work_dir_clone);
		}
		std::process::exit(if cfg!(unix) { 130 } else { 1 });
	})?;

	let config = RunnerConfig {
		yammm_bin,
		timeout_secs: cli.timeout,
		no_cleanup: cli.no_cleanup,
		work_dir: work_dir.clone(),
		side: cli.side,
		mod_slug: cli.mod_slug,
	};

	let mut runner = Runner::new(config);
	let report = runner.run_all(&tests);

	report.print_summary();

	if !cli.no_cleanup {
		let _ = std::fs::remove_dir_all(&work_dir);
	}

	if report.has_failures() {
		std::process::exit(1);
	}

	Ok(())
}

fn list_tests(tests: &[TestCase]) {
	if tests.is_empty() {
		println!("No tests match the given filters.");
		return;
	}

	println!("Test cases ({} total):\n", tests.len());
	for (i, test) in tests.iter().enumerate() {
		let known = test
			.known_issue
			.map_or(String::new(), |issue| format!("  [known: {issue}]"));
		let loader_display = match test.loader {
			Loader::Fabric => style("Fabric").cyan(),
			Loader::Forge => style("Forge").red(),
			Loader::NeoForge => style("NeoForge").magenta(),
			Loader::Quilt => style("Quilt").color256(165),
		};
		println!(
			"  {:3}. {:<10} {}  java {}{}",
			i + 1,
			test.mc_version,
			loader_display,
			test.min_java,
			known,
		);
	}
}

fn log(msg: &str) {
	println!("{} {msg}", style("[e2e]").cyan());
}
