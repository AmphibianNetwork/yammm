//! Offline integration tests for command-level flows.
//!
//! These spawn the `yammm` binary and exercise end-to-end command flows
//! that do not require network access:
//!   - YMPK export → import round-trip
//!   - `info`, `remove`, `cache status` on a hand-built modpack
//!   - Import path-traversal rejection (verifies the hardening in
//!     `commands/import/helpers.rs::classify_archive_entry`).
//!
//! Network-bound flows live in `integration_tests.rs` under `#[ignore]`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn target_dir() -> PathBuf {
	let mut dir = std::env::current_exe().unwrap();
	dir.pop();
	dir
}

fn yammm_bin() -> PathBuf {
	let target = target_dir();
	if target.file_name().unwrap_or_default() == "debug" {
		target.join("yammm")
	} else {
		target.join("../yammm")
	}
}

fn run_yammm(
	work_dir: &Path,
	args: &[&str],
) -> std::process::Output {
	Command::new(yammm_bin())
		.args(args)
		.current_dir(work_dir)
		.output()
		.expect("Failed to spawn yammm")
}

fn init_pack(
	work_dir: &Path,
	name: &str,
) {
	let out = run_yammm(
		work_dir,
		&[
			"init",
			"--name",
			name,
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		],
	);
	assert!(
		out.status.success(),
		"init failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}

/// Place a TrackedMod entry on disk without going through `add` (which
/// needs network). The entry format is RON; matching the on-disk shape
/// used by `Storage::save` is enough to make `info`, `remove`, and the
/// YMPK exporter see this mod. Fields that default via `#[serde(default)]`
/// are omitted intentionally.
fn stage_local_mod(
	work_dir: &Path,
	slug: &str,
	display_name: &str,
) {
	let mod_dir = work_dir.join("mods").join(slug);
	fs::create_dir_all(&mod_dir).unwrap();
	let entry = format!(
		r#"(
	id: "{slug}",
	name: "{display_name}",
	description: "test mod",
	version: "1.0.0",
	source: (type: "url", url: "https://example.com/{slug}.jar"),
	url: "https://example.com/{slug}",
	download_url: "https://example.com/{slug}.jar",
)
"#,
		slug = slug,
		display_name = display_name,
	);
	fs::write(mod_dir.join("entry.ron"), entry).unwrap();
}

#[test]
fn info_shows_staged_mod() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "info-test");
	stage_local_mod(tmp.path(), "test-mod", "Test Mod");

	let out = run_yammm(tmp.path(), &["info"]);
	assert!(
		out.status.success(),
		"info failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stdout.contains("info-test"),
		"info output should name the pack, got: {stdout}"
	);
	assert!(
		stdout.contains("test-mod") || stdout.contains("Test Mod"),
		"info output should list the staged mod, got: {stdout}"
	);
}

#[test]
fn remove_deletes_staged_mod() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "remove-test");
	stage_local_mod(tmp.path(), "remove-me", "Removable");

	let mod_dir = tmp.path().join("mods").join("remove-me");
	assert!(mod_dir.exists(), "precondition: mod dir should exist");

	let out = run_yammm(tmp.path(), &["remove", "remove-me", "--yes"]);
	assert!(
		out.status.success(),
		"remove failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(
		!mod_dir.exists(),
		"mod directory should be gone after remove"
	);
}

#[test]
fn cache_status_runs_on_fresh_pack() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "cache-test");

	let cache_dir = tmp.path().join("yammm-cache");
	let out = Command::new(yammm_bin())
		.env("YAMMM_CACHE_DIR", &cache_dir)
		.args(["cache", "status"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"cache status failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stdout.contains("Root:") || stdout.contains("cache"),
		"cache status output unexpected: {stdout}"
	);
}

/// Build a minimal but valid YMPK archive in `path`, with two mod entries
/// and a nested config file. Mirrors what `commands/export::export_ympk`
/// would emit, minus the per-mod JAR (which the exporter only includes
/// when the JAR is in the cache — irrelevant for import-side coverage).
fn write_minimal_ympk(path: &Path) {
	let file = fs::File::create(path).unwrap();
	let mut zip = zip::ZipWriter::new(file);
	let opts = zip::write::SimpleFileOptions::default();

	zip.start_file::<_, ()>("modpack.toml", opts).unwrap();
	zip.write_all(
		b"name = \"imported\"\nversion = \"1.0.0\"\ndescription = \"\"\n\n[loader]\nloader = \"fabric\"\nminecraft_version = \"1.20.4\"\n",
	).unwrap();

	for slug in ["alpha", "beta"] {
		zip.start_file::<_, ()>(format!("mods/{slug}/entry.ron"), opts)
			.unwrap();
		zip.write_all(
			format!(
				r#"(
	id: "{slug}",
	name: "{slug}",
	description: "imported {slug}",
	version: "1.0.0",
	source: (type: "url", url: "https://example.com/{slug}.jar"),
	url: "https://example.com/{slug}",
	download_url: "https://example.com/{slug}.jar",
)
"#
			)
			.as_bytes(),
		)
		.unwrap();
	}

	zip.start_file::<_, ()>("config/alpha/options.json", opts)
		.unwrap();
	zip.write_all(b"{\"flag\":true}").unwrap();

	zip.finish().unwrap();
}

#[test]
fn import_ympk_extracts_mods_and_nested_config() {
	let tmp = tempfile::tempdir().unwrap();
	let ympk = tmp.path().join("pack.ympk");
	let dst = tmp.path().join("dst");

	write_minimal_ympk(&ympk);

	let out = run_yammm(
		tmp.path(),
		&[
			"import",
			ympk.to_str().unwrap(),
			"-o",
			dst.to_str().unwrap(),
			"--yes",
		],
	);
	assert!(
		out.status.success(),
		"import failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);

	assert!(
		dst.join("modpack.toml").exists(),
		"imported pack should have a modpack.toml"
	);
	assert!(
		dst.join("mods/alpha/entry.ron").exists(),
		"alpha entry should be present"
	);
	assert!(
		dst.join("mods/beta/entry.ron").exists(),
		"beta entry should be present"
	);
	assert!(
		dst.join("config/alpha/options.json").exists(),
		"nested config file should be extracted"
	);
}

/// Build a malicious YMPK that contains a `config/../escape.txt` entry.
/// The hardened importer must refuse to write outside the destination.
#[test]
fn info_json_emits_machine_readable_payload() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "json-test");
	stage_local_mod(tmp.path(), "alpha", "Alpha Mod");
	stage_local_mod(tmp.path(), "beta", "Beta Mod");

	let out = run_yammm(tmp.path(), &["info", "--json"]);
	assert!(
		out.status.success(),
		"info --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("info --json should emit valid JSON");

	assert_eq!(parsed["name"], "json-test");
	assert_eq!(parsed["minecraft_version"], "1.20.4");
	assert_eq!(parsed["loader"]["name"], "fabric");
	assert_eq!(parsed["counts"]["mods"], 2);
	assert_eq!(parsed["counts"]["resource_packs"], 0);

	let mods = parsed["mods"].as_array().expect("mods should be an array");
	assert_eq!(mods.len(), 2);
	let slugs: Vec<_> = mods
		.iter()
		.map(|m| m["id"].as_str().unwrap().to_string())
		.collect();
	assert!(slugs.contains(&"alpha".to_string()));
	assert!(slugs.contains(&"beta".to_string()));
}

#[test]
fn info_mod_json_emits_single_mod_payload() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "single-mod-json");
	stage_local_mod(tmp.path(), "alpha", "Alpha Mod");

	let out = run_yammm(tmp.path(), &["info", "--json", "mod", "alpha"]);
	assert!(
		out.status.success(),
		"info --json mod failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("info mod --json should emit valid JSON");

	assert_eq!(parsed["id"], "alpha");
	assert_eq!(parsed["name"], "Alpha Mod");
	assert_eq!(parsed["version"], "1.0.0");
}

#[test]
fn json_flag_is_rejected_on_unsupported_command() {
	// `completions` emits shell completion script text — JSON would be
	// nonsensical. The dispatcher should reject the flag with a clear
	// error rather than corrupt the output.
	let tmp = tempfile::tempdir().unwrap();
	let out = run_yammm(tmp.path(), &["--json", "completions", "bash"]);
	assert!(
		!out.status.success(),
		"completions --json should fail rather than emit garbage"
	);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("does not yet support --json"),
		"error message should explain the limitation, got: {stderr}"
	);
	assert!(
		out.stdout.is_empty(),
		"--json on unsupported command must not corrupt stdout"
	);
}

#[test]
fn cache_status_json_emits_structured_payload() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "cache-json");

	let cache_dir = tmp.path().join("yammm-cache");
	let out = Command::new(yammm_bin())
		.env("YAMMM_CACHE_DIR", &cache_dir)
		.args(["--json", "cache", "status"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"cache status --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("cache status --json should emit valid JSON");

	// Schema check: keys scripts will pin to.
	assert!(parsed["root"].is_string());
	assert_eq!(parsed["jars"]["file_count"], 0);
	assert_eq!(parsed["http_meta"]["file_count"], 0);
	assert!(parsed["http_meta"]["total_size"].is_number());
	assert_eq!(parsed["total"]["file_count"], 0);
	assert!(parsed["total"]["total_size"].is_number());
}

#[test]
fn info_json_with_quiet_emits_only_json() {
	// The canonical scripting combo: --quiet silences status, --json emits
	// the payload. Together, stdout must be exactly one JSON document.
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "quiet-json");

	let out = run_yammm(tmp.path(), &["--quiet", "info", "--json"]);
	assert!(
		out.status.success(),
		"quiet+json info failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("stdout under --quiet --json should be a single JSON document");
	assert_eq!(parsed["name"], "quiet-json");
}

#[test]
fn update_check_only_json_emits_drift_payload() {
	// With no mods installed the result is well-defined and requires
	// no network access — perfect smoke test for the JSON contract
	// and the early-return path.
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "drift-test");

	let out = run_yammm(tmp.path(), &["--json", "update", "--check-only"]);
	assert!(
		out.status.success(),
		"update --check-only --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("update --check-only --json should emit valid JSON");
	assert_eq!(parsed["command"], "update");
	assert_eq!(parsed["check_only"], true);
	let avail = parsed["updates_available"].as_array().unwrap();
	assert_eq!(avail.len(), 0, "fresh pack has no updates available");
	let failed = parsed["checks_failed"].as_array().unwrap();
	assert_eq!(failed.len(), 0, "fresh pack has no failed checks");
}

#[test]
fn no_http_cache_flag_is_accepted_globally() {
	// Smoke test: the global flag parses and the command still runs.
	// The bypass semantics are exercised in the http_cache unit tests
	// with mockito; here we just confirm the wiring exists.
	let tmp = tempfile::tempdir().unwrap();
	let cache_dir = tmp.path().join("yammm-cache");

	let out = Command::new(yammm_bin())
		.env("YAMMM_CACHE_DIR", &cache_dir)
		.args(["--no-http-cache", "cache", "status"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"--no-http-cache cache status failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}

#[test]
fn init_json_emits_creation_summary() {
	let tmp = tempfile::tempdir().unwrap();
	let out = run_yammm(
		tmp.path(),
		&[
			"--json",
			"init",
			"--name",
			"init-json",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		],
	);
	assert!(
		out.status.success(),
		"init --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("init --json should emit valid JSON");
	assert_eq!(parsed["name"], "init-json");
	assert_eq!(parsed["minecraft_version"], "1.20.4");
	assert_eq!(parsed["loader"]["name"], "fabric");
	assert_eq!(parsed["modpack_toml_created"], true);
	let created = parsed["created"].as_array().unwrap();
	assert!(
		created.iter().any(|v| v == "modpack.toml"),
		"created list should include modpack.toml"
	);
	assert!(
		tmp.path().join("modpack.toml").exists(),
		"init --json must still create modpack.toml"
	);
}

#[test]
fn info_tree_json_emits_node_edge_graph() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "tree-json");
	stage_local_mod(tmp.path(), "alpha", "Alpha");
	stage_local_mod(tmp.path(), "beta", "Beta");

	let out = run_yammm(tmp.path(), &["--json", "info", "tree"]);
	assert!(
		out.status.success(),
		"info tree --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("info tree --json should emit valid JSON");

	let nodes = parsed["nodes"].as_array().unwrap();
	assert_eq!(nodes.len(), 2);
	let ids: Vec<_> = nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
	assert!(ids.contains(&"alpha"));
	assert!(ids.contains(&"beta"));

	// Staged mods have no recorded deps, so the edge list is empty —
	// but the field must still be present in the schema.
	let edges = parsed["edges"].as_array().unwrap();
	assert_eq!(edges.len(), 0);
}

#[test]
fn config_show_json_redacts_api_key() {
	// `dirs::config_dir()` is platform-specific:
	//   Linux:   $XDG_CONFIG_HOME or $HOME/.config
	//   macOS:   $HOME/Library/Application Support
	//   Windows: %APPDATA%
	// We seed every relevant root so the test is portable.
	let tmp = tempfile::tempdir().unwrap();
	let payload = "[api_keys]\ncurseforge = \"super-secret-key-1234567890\"\n";

	let candidate_roots = [
		tmp.path().join("config").join("yammm"),
		tmp.path().join("Library/Application Support/yammm"),
		tmp.path().join("AppData/Roaming/yammm"),
	];
	for dir in &candidate_roots {
		fs::create_dir_all(dir).unwrap();
		fs::write(dir.join("config.toml"), payload).unwrap();
	}

	let out = Command::new(yammm_bin())
		.env("HOME", tmp.path())
		.env("USERPROFILE", tmp.path())
		.env("XDG_CONFIG_HOME", tmp.path().join("config"))
		.env("APPDATA", tmp.path().join("AppData/Roaming"))
		.args(["--json", "config", "show"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"config show --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("config show --json should emit valid JSON");

	let key = parsed["api_keys"]["curseforge"]
		.as_str()
		.unwrap_or_default();
	assert!(
		!key.is_empty(),
		"config show --json must report the api key field; got: {stdout}"
	);
	assert!(
		!key.contains("secret"),
		"api key must be redacted in JSON output, got: {key}"
	);
	assert!(
		key.contains("***"),
		"redaction marker missing from JSON key: {key}"
	);
}

#[test]
fn auth_status_json_reports_logged_out_when_no_token() {
	let tmp = tempfile::tempdir().unwrap();
	// Force a clean state via HOME / XDG_DATA_HOME redirection so we
	// don't depend on the developer's actual login state.
	let fake_home = tmp.path().join("home");
	fs::create_dir_all(&fake_home).unwrap();

	let out = Command::new(yammm_bin())
		.env("HOME", &fake_home)
		.env("XDG_DATA_HOME", fake_home.join("data"))
		.env("XDG_CACHE_HOME", fake_home.join("cache"))
		.env("XDG_CONFIG_HOME", fake_home.join("config"))
		.args(["--json", "auth", "status"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"auth status --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("auth status --json should emit valid JSON");
	assert_eq!(parsed["command"], "auth status");
	assert_eq!(parsed["logged_in"], false);
}

#[test]
fn remove_json_emits_removal_payload() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "rm-json");
	stage_local_mod(tmp.path(), "alpha", "Alpha Mod");

	let out = run_yammm(tmp.path(), &["--json", "remove", "alpha"]);
	assert!(
		out.status.success(),
		"remove --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("remove --json should emit valid JSON");
	assert_eq!(parsed["command"], "remove");
	let removed = parsed["removed"].as_array().unwrap();
	assert_eq!(removed.len(), 1);
	assert_eq!(removed[0]["id"], "alpha");
	assert_eq!(removed[0]["project_type"], "mod");
	assert!(
		!tmp.path().join("mods/alpha").exists(),
		"--json remove must still actually remove the mod dir"
	);
}

#[test]
fn import_ympk_json_emits_summary() {
	let tmp = tempfile::tempdir().unwrap();
	let ympk = tmp.path().join("pack.ympk");
	let dst = tmp.path().join("dst");
	write_minimal_ympk(&ympk);

	let out = run_yammm(
		tmp.path(),
		&[
			"--json",
			"import",
			ympk.to_str().unwrap(),
			"-o",
			dst.to_str().unwrap(),
		],
	);
	assert!(
		out.status.success(),
		"import --json failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("import --json should emit valid JSON");
	assert_eq!(parsed["command"], "import");
	assert_eq!(parsed["format"], "ympk");
	assert_eq!(parsed["added"], 2);
	assert_eq!(parsed["skipped"], 0);
	assert!(
		dst.join("modpack.toml").exists(),
		"--json import must still extract files"
	);
}

#[test]
fn cache_clear_http_meta_accepts_max_age_duration() {
	// `--max-age` with a parseable duration succeeds (no entries means
	// zero removed) and `--max-age` with garbage fails with a clear
	// error. The on-disk eviction logic is covered by the http_cache
	// unit tests; here we just verify the CLI plumbing.
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "max-age");
	let cache_dir = tmp.path().join("yammm-cache");

	let out = Command::new(yammm_bin())
		.env("YAMMM_CACHE_DIR", &cache_dir)
		.args(["--json", "cache", "clear-http-meta", "--max-age", "30s"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"--max-age 30s failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let parsed: serde_json::Value =
		serde_json::from_slice(&out.stdout).unwrap();
	assert_eq!(parsed["status"], "pruned");
	assert_eq!(parsed["threshold_secs"], 30);
	assert_eq!(parsed["removed"], 0);

	let bad = Command::new(yammm_bin())
		.env("YAMMM_CACHE_DIR", &cache_dir)
		.args(["cache", "clear-http-meta", "--max-age", "5xyz"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		!bad.status.success(),
		"--max-age garbage should fail loudly"
	);
	let stderr = String::from_utf8_lossy(&bad.stderr);
	assert!(
		stderr.contains("invalid --max-age"),
		"error should mention --max-age, got: {stderr}"
	);
}

#[test]
fn cache_clear_http_meta_is_idempotent() {
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "http-meta");
	let cache_dir = tmp.path().join("yammm-cache");

	// Run twice — second run should still succeed (idempotent: directory
	// may or may not exist, the command handles both).
	for _ in 0..2 {
		let out = Command::new(yammm_bin())
			.env("YAMMM_CACHE_DIR", &cache_dir)
			.args(["--json", "cache", "clear-http-meta"])
			.current_dir(tmp.path())
			.output()
			.expect("Failed to spawn yammm");
		assert!(
			out.status.success(),
			"cache clear-http-meta failed: {}",
			String::from_utf8_lossy(&out.stderr)
		);
		let stdout = String::from_utf8(out.stdout).unwrap();
		let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
			.expect("cache clear-http-meta --json should emit JSON");
		assert_eq!(parsed["status"], "cleared");
	}
}

#[test]
fn cache_clear_http_meta_stale_flag_emits_count() {
	// `--stale` keeps fresh entries and reports how many were pruned.
	// Without prior fetches the cache is empty, but the command must
	// still succeed with `removed: 0` (a useful "nothing to do" signal
	// for CI scripts).
	let tmp = tempfile::tempdir().unwrap();
	init_pack(tmp.path(), "stale-http-meta");
	let cache_dir = tmp.path().join("yammm-cache");

	let out = Command::new(yammm_bin())
		.env("YAMMM_CACHE_DIR", &cache_dir)
		.args(["--json", "cache", "clear-http-meta", "--stale"])
		.current_dir(tmp.path())
		.output()
		.expect("Failed to spawn yammm");
	assert!(
		out.status.success(),
		"cache clear-http-meta --stale failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8(out.stdout).unwrap();
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
		.expect("--stale --json should emit valid JSON");
	assert_eq!(parsed["status"], "pruned");
	assert_eq!(parsed["removed"], 0);
	// --stale without --max-age uses the default 24h threshold.
	assert_eq!(parsed["threshold_secs"], 24 * 60 * 60);
}

#[test]
fn quiet_flag_silences_stdout_status_output() {
	let tmp = tempfile::tempdir().unwrap();

	let baseline = run_yammm(
		tmp.path(),
		&[
			"init",
			"--name",
			"noisy",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		],
	);
	assert!(baseline.status.success());
	let baseline_stdout = String::from_utf8_lossy(&baseline.stdout);
	assert!(
		!baseline_stdout.trim().is_empty(),
		"baseline init should emit status lines; got empty stdout"
	);

	let quiet_dir = tempfile::tempdir().unwrap();
	let quiet = run_yammm(
		quiet_dir.path(),
		&[
			"--quiet",
			"init",
			"--name",
			"silent",
			"--minecraft-version",
			"1.20.4",
			"--loader",
			"fabric",
		],
	);
	assert!(
		quiet.status.success(),
		"quiet init failed: {}",
		String::from_utf8_lossy(&quiet.stderr)
	);
	assert!(
		quiet.stdout.is_empty(),
		"--quiet should suppress all stdout, got: {:?}",
		String::from_utf8_lossy(&quiet.stdout)
	);
	// The modpack must still be created — quiet must not change behavior.
	assert!(
		quiet_dir.path().join("modpack.toml").exists(),
		"quiet init should still create the modpack"
	);
}

#[test]
fn import_rejects_path_traversal_in_archive() {
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");

	let mal_path = tmp.path().join("malicious.ympk");
	{
		let file = fs::File::create(&mal_path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		let opts = zip::write::SimpleFileOptions::default();

		// A minimal valid modpack.toml so detect_format() picks YMPK.
		zip.start_file::<_, ()>("modpack.toml", opts).unwrap();
		zip.write_all(
			b"name = \"evil\"\nversion = \"1.0.0\"\ndescription = \"\"\n\n[loader]\nloader = \"fabric\"\nminecraft_version = \"1.20.4\"\n",
		)
		.unwrap();

		// The traversal payload: a config entry that lexically escapes.
		zip.start_file::<_, ()>("config/../escape.txt", opts)
			.unwrap();
		zip.write_all(b"owned").unwrap();

		// And a sibling-prefix attack (overrides_evil/-style for YMPK is
		// "config_evil/", which must not collide with the "config/" gate).
		zip.start_file::<_, ()>("config_evil/payload.txt", opts)
			.unwrap();
		zip.write_all(b"sibling").unwrap();

		zip.finish().unwrap();
	}

	let out = run_yammm(
		tmp.path(),
		&[
			"import",
			mal_path.to_str().unwrap(),
			"-o",
			dst.to_str().unwrap(),
			"--yes",
		],
	);
	assert!(
		out.status.success(),
		"import of malicious ympk should still complete (it just skips bad entries): {}",
		String::from_utf8_lossy(&out.stderr)
	);

	// Crucial: neither escape attempt should have landed anywhere.
	let escape_in_parent = tmp.path().join("escape.txt");
	let escape_in_dest = dst.join("escape.txt");
	let payload_in_dest = dst.join("config_evil/payload.txt");
	assert!(
		!escape_in_parent.exists(),
		"path traversal escaped the destination directory"
	);
	assert!(
		!escape_in_dest.exists(),
		"path traversal wrote to the destination root"
	);
	assert!(
		!payload_in_dest.exists(),
		"sibling-prefix attack should not match the config gate"
	);
}
