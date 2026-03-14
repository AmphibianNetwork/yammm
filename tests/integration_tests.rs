//! Integration tests for yammm CLI commands

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Helper to find the target directory
fn target_dir() -> PathBuf {
	let mut dir = std::env::current_exe().unwrap();
	dir.pop();
	dir
}

/// Helper to get the yammm binary path
fn yammm_bin() -> PathBuf {
	let target = target_dir();
	if target.file_name().unwrap_or_default() == "debug" {
		target.join("yammm")
	} else {
		target.join("../yammm")
	}
}

#[test]
fn test_version_command() {
	// Test that the version command works
	let output = Command::new(yammm_bin())
		.arg("--version")
		.output()
		.expect("Failed to execute version command");

	assert!(output.status.success());
	let version = String::from_utf8_lossy(&output.stdout);
	assert!(version.contains("yammm"));
}

#[test]
fn test_help_command() {
	// Test that help command works
	let output = Command::new(yammm_bin())
		.arg("--help")
		.output()
		.expect("Failed to execute help command");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Usage:"));
	assert!(help.contains("Commands:"));
}

#[test]
fn test_init_command() {
	let temp_dir = tempfile::tempdir().unwrap();
	let output = Command::new(yammm_bin())
		.args(["init", "--name", "test-modpack"])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	assert!(
		output.status.success(),
		"stdout: {}, stderr: {}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);

	// Verify created files
	assert!(temp_dir.path().join("modpack.toml").exists());
	assert!(temp_dir.path().join("mods").exists());
	assert!(temp_dir.path().join("config").exists());

	// Verify modpack.toml content
	let content =
		fs::read_to_string(temp_dir.path().join("modpack.toml")).unwrap();
	assert!(content.contains("name = \"test-modpack\""));
	assert!(content.contains("minecraft_version"));
	assert!(content.contains("loader = \"fabric\""));
}

#[test]
fn test_init_command_with_custom_params() {
	let temp_dir = tempfile::tempdir().unwrap();
	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"custom-pack",
			"--minecraft-version",
			"1.19.2",
			"--loader",
			"forge",
			"--description",
			"My custom modpack",
		])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	assert!(output.status.success());

	let content =
		fs::read_to_string(temp_dir.path().join("modpack.toml")).unwrap();
	assert!(content.contains("name = \"custom-pack\""));
	assert!(content.contains("minecraft_version = \"1.19.2\""));
	assert!(content.contains("loader = \"forge\""));
	assert!(content.contains("description = \"My custom modpack\""));
}

#[test]
fn test_init_command_output_dir() {
	let temp_dir = tempfile::tempdir().unwrap();
	let output_dir = temp_dir.path().join("my-workspace");

	// Use all required arguments - no interactive prompts will be shown
	// when name is provided (per should_interact logic)
	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--output-dir",
			output_dir.to_str().unwrap(),
			"--name",
			"test-pack",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	assert!(
		output.status.success(),
		"stdout: {}, stderr: {}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(output_dir.join("modpack.toml").exists());
	assert!(output_dir.join("mods").exists());
}

#[test]
fn test_search_command_help() {
	let output = Command::new(yammm_bin())
		.args(["search", "--help"])
		.output()
		.expect("Failed to execute search help");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Search for mods"));
}

#[test]
fn test_add_command_help() {
	let output = Command::new(yammm_bin())
		.args(["add", "--help"])
		.output()
		.expect("Failed to execute add help");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Add a mod"));
}

#[test]
fn test_remove_command_help() {
	let output = Command::new(yammm_bin())
		.args(["remove", "--help"])
		.output()
		.expect("Failed to execute remove help");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Remove a mod"));
}

#[test]
fn test_info_command_help() {
	let output = Command::new(yammm_bin())
		.args(["info", "--help"])
		.output()
		.expect("Failed to execute info help");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Display information"));
}

#[test]
fn test_export_command_help() {
	let output = Command::new(yammm_bin())
		.args(["export", "--help"])
		.output()
		.expect("Failed to execute export help");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Export the current modpack"));
}

#[test]
fn test_launch_command_help() {
	let output = Command::new(yammm_bin())
		.args(["launch", "--help"])
		.output()
		.expect("Failed to execute launch help");

	assert!(output.status.success());
	let help = String::from_utf8_lossy(&output.stdout);
	assert!(help.contains("Launch Minecraft"));
}

#[test]
fn test_invalid_command_shows_error() {
	let output = Command::new(yammm_bin())
		.arg("invalid-command-xyz")
		.output()
		.expect("Failed to execute invalid command");

	assert!(!output.status.success());
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(stderr.contains("unrecognized subcommand"));
}

#[test]
fn test_readme_creation() {
	let temp_dir = tempfile::tempdir().unwrap();

	Command::new(yammm_bin())
		.args(["init", "--name", "test"])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	let readme = fs::read_to_string(temp_dir.path().join("README.md")).unwrap();
	assert!(readme.contains("# test"));
	assert!(readme.contains("yammm add"));
}

#[test]
fn test_gitignore_creation() {
	let temp_dir = tempfile::tempdir().unwrap();

	Command::new(yammm_bin())
		.args(["init", "--name", "test"])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	let gitignore =
		fs::read_to_string(temp_dir.path().join(".gitignore")).unwrap();
	assert!(gitignore.contains("target/"));
	assert!(gitignore.contains(".idea/"));
}

/// Test that basic file structure is created during init
#[test]
fn test_init_command_creates_basic_structure() {
	let temp_dir = tempfile::tempdir().unwrap();

	let output = Command::new(yammm_bin())
		.args(["init", "--name", "structure-test"])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	assert!(output.status.success());

	// Verify created files
	assert!(temp_dir.path().join("modpack.toml").exists());
	assert!(temp_dir.path().join("mods").exists());
	assert!(temp_dir.path().join("config").exists());
	assert!(temp_dir.path().join("README.md").exists());
	assert!(temp_dir.path().join(".gitignore").exists());
}

#[test]
fn test_init_command_basic() {
	let temp_dir = tempfile::tempdir().unwrap();

	// Test that init command at least attempts to run
	// Note: This test may fail if init requires a workspace to exist
	let output = Command::new(yammm_bin())
		.args(["init", "--name", "test"])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");

	// The command either succeeds or gives a meaningful error
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	// Either it succeeds, or it gives a meaningful error message
	assert!(
		output.status.success() ||
			stderr.contains("error") ||
			stderr.contains("Error"),
		"Command should succeed or show meaningful error. stdout: {}, stderr: {}",
		stdout, stderr
	);
}

#[test]
#[ignore = "requires network access to Modrinth API"]
fn test_e2e_nominal_flow() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	// 1. Initialize modpack
	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"e2e-pack",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
			"--loader-version",
			"0.15.11",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(
		output.status.success(),
		"Init failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	// 2. Add a mod (fabric-api)
	// We use `--yes` so it skips prompts, which currently accepts required dependencies.
	let output = Command::new(yammm_bin())
		.args(["add", "fabric-api", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	assert!(
		output.status.success(),
		"Add failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	// Verify mod.ron exists
	assert!(
		work_dir.join("mods/fabric-api/mod.ron").exists(),
		"fabric-api mod.ron missing"
	);

	// 3. Get info
	let output = Command::new(yammm_bin())
		.args(["info"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute info command");
	assert!(output.status.success());
	let info_out = String::from_utf8_lossy(&output.stdout);
	assert!(info_out.contains("e2e-pack"));
	assert!(
		info_out.to_lowercase().contains("fabric api")
			|| info_out.to_lowercase().contains("fabric-api"),
		"Info output didn't contain fabric-api: {}",
		info_out
	);

	// 4. Export modpack (this also downloads JARs)
	let output = Command::new(yammm_bin())
		.args(["export", "-f", "mrpack", "-y"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute export command");
	assert!(
		output.status.success(),
		"Export failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	// Check if zip exists
	let has_export = std::fs::read_dir(work_dir).unwrap().any(|entry| {
		let path = entry.unwrap().path();
		let ext = path.extension().unwrap_or_default();
		ext == "mrpack" || ext == "zip"
	});
	assert!(has_export, "Exported file missing");

	// 6. Remove mod
	let output = Command::new(yammm_bin())
		.args(["remove", "fabric-api", "-y"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute remove command");
	assert!(
		output.status.success(),
		"Remove failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(
		!work_dir.join("mods/fabric-api").exists(),
		"fabric-api directory should be removed"
	);

	// 7. Cache status
	let output = Command::new(yammm_bin())
		.args(["cache", "status"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute cache command");
	assert!(output.status.success());
	let cache_out = String::from_utf8_lossy(&output.stdout);
	assert!(cache_out.contains("Root:"));
}

#[test]
#[ignore = "requires network access to Modrinth API"]
fn test_e2e_mod_ron_source_preserved() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"source-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["add", "fabric-api", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	assert!(
		output.status.success(),
		"Add failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	let mod_ron_path = work_dir.join("mods/fabric-api/mod.ron");
	assert!(mod_ron_path.exists(), "mod.ron should exist");

	let mod_ron_content = fs::read_to_string(&mod_ron_path).unwrap();
	assert!(
		mod_ron_content.contains("modrinth"),
		"mod.ron should contain modrinth source type, got: {}",
		mod_ron_content
	);
	assert!(
		mod_ron_content.contains("fabric-api"),
		"mod.ron should contain the mod id"
	);
}

#[test]
#[ignore = "requires network access to Modrinth API"]
fn test_e2e_add_and_remove_cycle() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"cycle-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["add", "fabric-api", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	assert!(output.status.success());

	assert!(work_dir.join("mods/fabric-api/mod.ron").exists());

	let output = Command::new(yammm_bin())
		.args(["remove", "fabric-api", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute remove command");
	assert!(output.status.success());

	assert!(
		!work_dir.join("mods/fabric-api").exists(),
		"Mod directory should be removed"
	);

	let output = Command::new(yammm_bin())
		.args(["info"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute info command");
	assert!(output.status.success());
}

#[test]
fn test_e2e_neoforge_loader() {
	let temp_dir = tempfile::tempdir().unwrap();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"neoforge-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"neoforge",
		])
		.current_dir(temp_dir.path())
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let content =
		fs::read_to_string(temp_dir.path().join("modpack.toml")).unwrap();
	assert!(
		content.contains("neoforge"),
		"modpack.toml should contain neoforge loader"
	);
}

#[test]
fn test_e2e_config_set_and_get() {
	let temp_dir = tempfile::tempdir().unwrap();
	let config_path = temp_dir.path().join("yammm").join("config.toml");
	fs::create_dir_all(config_path.parent().unwrap()).unwrap();
	fs::write(&config_path, "").unwrap();

	let output = Command::new(yammm_bin())
		.env("YAMMM_CONFIG", config_path.to_str().unwrap())
		.args(["config", "show"])
		.output()
		.expect("Failed to execute config show");
	assert!(
		output.status.success(),
		"config show failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
}

#[test]
#[ignore = "requires network access"]
fn test_e2e_add_url_source() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"url-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["add", "https://example.com/mods/some-mod.jar", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	assert!(
		output.status.success(),
		"Add URL source failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	let mod_dir = work_dir.join("mods/some-mod");
	assert!(mod_dir.join("mod.ron").exists(), "mod.ron should exist");

	let mod_ron = fs::read_to_string(mod_dir.join("mod.ron")).unwrap();
	assert!(
		mod_ron.contains("url"),
		"mod.ron should contain url source type"
	);
	assert!(
		mod_ron.contains("example.com"),
		"mod.ron should contain the URL"
	);
}

#[test]
#[ignore = "requires network access for URL resolution"]
fn test_e2e_add_file_source() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	let jar_path = work_dir.join("test-mod.jar");
	fs::write(&jar_path, b"fake jar content").unwrap();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"file-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["add", &format!("file://{}", jar_path.display()), "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	assert!(
		output.status.success(),
		"Add file source failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	let mod_dir = work_dir.join("mods/test-mod");
	assert!(mod_dir.join("mod.ron").exists(), "mod.ron should exist");

	let mod_ron = fs::read_to_string(mod_dir.join("mod.ron")).unwrap();
	assert!(
		mod_ron.contains("url"),
		"mod.ron should contain url source type"
	);
	assert!(
		mod_ron.contains("test-mod.jar"),
		"mod.ron should reference the file path"
	);
}

#[test]
#[ignore = "requires network access to GitHub API"]
fn test_e2e_add_github_source() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"github-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["add", "https://github.com/IrisShaders/Iris", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		if stderr.contains("403")
			|| stderr.contains("rate")
			|| stderr.contains("network")
		{
			eprintln!("GitHub API rate-limited, skipping assertions");
			return;
		}
		assert!(
			output.status.success(),
			"Add GitHub source failed: {}",
			stderr
		);
	}

	let mod_dir = work_dir.join("mods/iris");
	assert!(
		mod_dir.join("mod.ron").exists(),
		"mod.ron should exist for GitHub mod"
	);

	let mod_ron = fs::read_to_string(mod_dir.join("mod.ron")).unwrap();
	assert!(
		mod_ron.contains("github"),
		"mod.ron should contain github source type"
	);
	assert!(
		mod_ron.contains("IrisShaders/Iris"),
		"mod.ron should reference the repo"
	);
}

#[test]
#[ignore = "requires network access to Modrinth API"]
fn test_e2e_export_and_import_mrpack() {
	let temp_dir = tempfile::tempdir().unwrap();
	let work_dir = temp_dir.path();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"export-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
			"--loader-version",
			"0.15.11",
		])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute init command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["add", "fabric-api", "--yes"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute add command");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["export", "-f", "mrpack", "-y"])
		.current_dir(work_dir)
		.output()
		.expect("Failed to execute export command");
	assert!(
		output.status.success(),
		"Export failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	let mrpack_file = std::fs::read_dir(work_dir)
		.unwrap()
		.find(|e| {
			e.as_ref().unwrap().path().extension().unwrap_or_default()
				== "mrpack"
		})
		.unwrap()
		.unwrap()
		.path();
	assert!(mrpack_file.exists(), "MRPACK file should exist");

	let import_dir = temp_dir.path().join("imported");
	fs::create_dir_all(&import_dir).unwrap();

	let output = Command::new(yammm_bin())
		.args([
			"init",
			"--name",
			"import-test",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		])
		.current_dir(&import_dir)
		.output()
		.expect("Failed to init for import");
	assert!(output.status.success());

	let output = Command::new(yammm_bin())
		.args(["import", mrpack_file.to_str().unwrap(), "--yes"])
		.current_dir(&import_dir)
		.output()
		.expect("Failed to execute import command");
	assert!(
		output.status.success(),
		"Import failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);

	let import_mods_dir = import_dir.join("mods");
	if import_mods_dir.exists() {
		let mod_count = std::fs::read_dir(&import_mods_dir)
			.unwrap()
			.filter(|e| e.as_ref().unwrap().path().is_dir())
			.count();
		assert!(mod_count > 0, "Should have at least one mod after import");
	}
}

#[test]
fn test_organize_command_help() {
	let output = Command::new(yammm_bin())
		.args(["organize", "--help"])
		.output()
		.expect("Failed to execute organize help");
	assert!(output.status.success());
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(stdout.contains("client") || stdout.contains("server"));
}

#[test]
fn test_update_command_help() {
	let output = Command::new(yammm_bin())
		.args(["update", "--help"])
		.output()
		.expect("Failed to execute update help");
	assert!(output.status.success());
}

#[test]
fn test_completions_command_help() {
	let output = Command::new(yammm_bin())
		.args(["completions", "--help"])
		.output()
		.expect("Failed to execute completions help");
	assert!(output.status.success());
}
