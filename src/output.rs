//! Terminal output formatting layer.
//!
//! Every public helper (`success`, `info`, `heading`, …) constructs an
//! [`OutputEvent`] and routes it through [`dispatch`]. `dispatch` is the
//! one place that decides whether to:
//!
//! - **render to stdout / stderr** (default mode)
//! - **append to a thread-local capture buffer** (TUI inline mode)
//! - **silence the stdout helpers** (`--quiet`)
//! - **silence all non-JSON output** (`--json`, leaving stdout for the
//!   one JSON document the command emits)
//!
//! The split between the typed enum and the rendering functions
//! (`render_styled`, `render_plain`) keeps coloring out of the gating
//! decision: a single match handles routing; styling sits on the
//! side, used only when the destination is a TTY.

use console::Style;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::services::download::DownloadSummary;
use crate::types::{DependencyKind, ModSource};
use comfy_table::{
	Color as TColor, ContentArrangement, Table, modifiers::UTF8_ROUND_CORNERS,
	presets::UTF8_FULL,
};

static OUTPUT_CAPTURED: AtomicBool = AtomicBool::new(false);
static OUTPUT_QUIET: AtomicBool = AtomicBool::new(false);
static OUTPUT_JSON: AtomicBool = AtomicBool::new(false);

thread_local! {
	static CAPTURED_LINES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn start_capture() {
	OUTPUT_CAPTURED.store(true, Ordering::Relaxed);
	CAPTURED_LINES.with(|lines| lines.borrow_mut().clear());
}

pub fn stop_capture() -> Vec<String> {
	OUTPUT_CAPTURED.store(false, Ordering::Relaxed);
	CAPTURED_LINES.with(|lines| lines.borrow_mut().drain(..).collect())
}

fn is_captured() -> bool {
	OUTPUT_CAPTURED.load(Ordering::Relaxed)
}

fn capture_line(line: String) {
	CAPTURED_LINES.with(|lines| lines.borrow_mut().push(line));
}

/// Globally enable or disable quiet mode. When quiet, every stdout helper
/// (success, info, heading, bullet, raw_line, etc.) and the progress-bar /
/// spinner factories become no-ops. Stderr helpers (error, warning) are
/// unaffected — diagnostic output must remain visible regardless.
pub fn set_quiet(enabled: bool) {
	OUTPUT_QUIET.store(enabled, Ordering::Relaxed);
}

pub fn is_quiet() -> bool {
	OUTPUT_QUIET.load(Ordering::Relaxed)
}

/// Enable or disable JSON output mode.
///
/// In JSON mode every stdout status helper (success, info, heading,
/// bullet, blank_line, progress bars, raw_block) becomes a no-op so the
/// command's *result* — a single JSON document emitted via
/// [`emit_json`] — is the only thing on stdout. Stderr helpers (error,
/// warning) keep emitting so failures stay visible.
///
/// Commands that haven't been wired for JSON should detect this flag
/// via [`is_json_mode`] and return an explicit error rather than
/// silently emit nothing. See [`require_json_support`] for the canned
/// error.
pub fn set_json_mode(enabled: bool) {
	OUTPUT_JSON.store(enabled, Ordering::Relaxed);
}

pub fn is_json_mode() -> bool {
	OUTPUT_JSON.load(Ordering::Relaxed)
}

/// Helper for command handlers: if the user passed `--json` and the
/// command doesn't yet have a JSON output path, return a clear error
/// instead of producing no output.
pub fn require_json_support(command: &str) -> anyhow::Result<()> {
	if is_json_mode() {
		Err(crate::errors::YammmError::invalid_args(format!(
			"command '{command}' does not yet support --json output. \
			 Run without --json, or open an issue requesting JSON for this command."
		))
		.into())
	} else {
		Ok(())
	}
}

/// Status messages on stdout — silenced by `--quiet` and `--json`.
#[derive(Debug, Clone, Copy)]
pub enum StatusKind {
	Heading,
	Info,
	Success,
	Bullet,
	Dim,
}

/// Diagnostics on stderr — never silenced; visible even in `--quiet`
/// and `--json` modes.
#[derive(Debug, Clone, Copy)]
pub enum DiagnosticKind {
	Warning,
	Error,
}

/// Typed representation of every line the program might emit.
///
/// Every public helper in this module produces one of these and routes
/// it through [`dispatch`], which is the single place that decides
/// whether the event should render to the terminal, be captured, or be
/// silenced.
#[derive(Debug, Clone)]
pub enum OutputEvent {
	/// Pre-formatted status text. The styled rendering applies colour
	/// and any symbol prefix (`✓`, `  •`, etc.) via [`render_styled`].
	Status(StatusKind, String),
	/// Pre-formatted diagnostic text destined for stderr.
	Diagnostic(DiagnosticKind, String),
	/// An empty line. Treated like a status event for gating.
	BlankLine,
	/// Raw, unstyled stdout content representing the *result* of a
	/// command (a rendered table, search hits, etc.). Survives `--quiet`
	/// but is suppressed in `--json` (where the only legitimate stdout
	/// artifact is the JSON document).
	Raw(String),
	/// Same as [`Raw`] but the payload contains newlines. Capture splits
	/// it line-by-line so the buffer stays line-oriented.
	///
	/// [`Raw`]: OutputEvent::Raw
	RawBlock(String),
	/// A JSON document. Only emitted when `is_json_mode()` is true (or
	/// during capture). The string is the already-serialised pretty
	/// JSON; [`dispatch`] does not re-encode.
	JsonDocument(String),
}

/// Central routing for every output event. The only stdout/stderr
/// writes in this module happen here.
fn dispatch(event: OutputEvent) {
	if is_captured() {
		capture_event(&event);
		return;
	}

	match event {
		OutputEvent::Diagnostic(kind, msg) => {
			eprintln!("{}", render_diagnostic_styled(kind, &msg));
		}
		OutputEvent::JsonDocument(s) => {
			// Constructed only via emit_json, which already gates on
			// JSON mode. Emit unconditionally so non-JSON callers
			// that bypass that guard still see their output.
			println!("{}", s);
		}
		OutputEvent::Raw(s) => {
			if !is_json_mode() {
				println!("{}", s);
			}
		}
		OutputEvent::RawBlock(s) => {
			if !is_json_mode() {
				println!("{}", s);
			}
		}
		OutputEvent::BlankLine => {
			if !is_quiet() && !is_json_mode() {
				println!();
			}
		}
		OutputEvent::Status(kind, msg) => {
			if !is_quiet() && !is_json_mode() {
				println!("{}", render_status_styled(kind, &msg));
			}
		}
	}
}

/// Capture path: store the unstyled form so test harnesses and the TUI
/// see the same bytes a non-coloured terminal would.
fn capture_event(event: &OutputEvent) {
	match event {
		OutputEvent::Status(kind, msg) => {
			capture_line(render_status_plain(*kind, msg));
		}
		OutputEvent::Diagnostic(kind, msg) => {
			capture_line(render_diagnostic_plain(*kind, msg));
		}
		OutputEvent::BlankLine => capture_line(String::new()),
		OutputEvent::Raw(s) => capture_line(s.clone()),
		OutputEvent::RawBlock(s) | OutputEvent::JsonDocument(s) => {
			for line in s.lines() {
				capture_line(line.to_string());
			}
		}
	}
}

static SUCCESS_CHECK: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().green().bold().apply_to("✓"));
static ERROR_CROSS: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().red().bold().apply_to("✗"));
static WARN_SYMBOL: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().yellow().bold().apply_to("⚠"));
static BULLET_POINT: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().dim().apply_to("  •"));
static HEADING_STYLE: LazyLock<Style> =
	LazyLock::new(|| Style::new().bold().cyan());
static INFO_STYLE: LazyLock<Style> = LazyLock::new(|| Style::new().cyan());
static DIM_STYLE: LazyLock<Style> = LazyLock::new(|| Style::new().dim());

fn render_status_styled(
	kind: StatusKind,
	msg: &str,
) -> String {
	match kind {
		StatusKind::Success => format!("{} {}", *SUCCESS_CHECK, msg),
		StatusKind::Bullet => format!("{} {}", *BULLET_POINT, msg),
		StatusKind::Heading => format!("{}", HEADING_STYLE.apply_to(msg)),
		StatusKind::Info => format!("{}", INFO_STYLE.apply_to(msg)),
		StatusKind::Dim => format!("{}", DIM_STYLE.apply_to(msg)),
	}
}

fn render_status_plain(
	kind: StatusKind,
	msg: &str,
) -> String {
	match kind {
		StatusKind::Success => format!("✓ {}", msg),
		StatusKind::Bullet => format!("  • {}", msg),
		StatusKind::Heading | StatusKind::Info | StatusKind::Dim => {
			msg.to_string()
		}
	}
}

fn render_diagnostic_styled(
	kind: DiagnosticKind,
	msg: &str,
) -> String {
	match kind {
		DiagnosticKind::Error => format!("{} {}", *ERROR_CROSS, msg),
		DiagnosticKind::Warning => format!("{} {}", *WARN_SYMBOL, msg),
	}
}

fn render_diagnostic_plain(
	kind: DiagnosticKind,
	msg: &str,
) -> String {
	match kind {
		DiagnosticKind::Error => format!("✗ {}", msg),
		DiagnosticKind::Warning => format!("⚠ {}", msg),
	}
}

/// Emit a single JSON document on stdout (no trailing decoration).
///
/// Use this from command handlers to report their result when
/// [`is_json_mode`] is on. Errors during serialization are propagated so
/// the caller can choose to fall back to text output.
pub fn emit_json<T: serde::Serialize + ?Sized>(
	value: &T
) -> anyhow::Result<()> {
	let s = serde_json::to_string_pretty(value).map_err(|e| {
		anyhow::anyhow!("failed to serialize JSON output: {}", e)
	})?;
	dispatch(OutputEvent::JsonDocument(s));
	Ok(())
}

pub fn success(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Status(StatusKind::Success, msg.to_string()));
}

pub fn error(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Diagnostic(
		DiagnosticKind::Error,
		msg.to_string(),
	));
}

pub fn warning(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Diagnostic(
		DiagnosticKind::Warning,
		msg.to_string(),
	));
}

pub fn heading(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Status(StatusKind::Heading, msg.to_string()));
}

pub fn info(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Status(StatusKind::Info, msg.to_string()));
}

pub fn dim(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Status(StatusKind::Dim, msg.to_string()));
}

/// Prints an empty line.
pub fn blank_line() {
	dispatch(OutputEvent::BlankLine);
}

/// Print one line of raw, unstyled data to stdout — for command output that
/// is the *result* of a command (tables, config values, search hits), not a
/// status message. Respects the capture machinery so the TUI can intercept
/// downloads/spinners triggered by these commands.
///
/// Note: `raw_line` represents the *result* of a command and is therefore
/// **not** silenced by quiet mode. Callers that want quiet to hide their
/// output should use [`info`]/[`bullet`]/[`success`] instead.
///
/// In JSON mode `raw_line` *is* silenced — the JSON document is the only
/// legitimate stdout artifact, and unstructured tables would corrupt it.
pub fn raw_line(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Raw(msg.to_string()));
}

/// Print pre-formatted multi-line output (e.g. a comfy_table render) to
/// stdout, splitting on newlines so capture stays line-oriented.
///
/// Like [`raw_line`], this carries command results and is not silenced by
/// quiet mode but *is* silenced by JSON mode.
pub fn raw_block(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::RawBlock(msg.to_string()));
}

pub fn bullet(msg: impl std::fmt::Display) {
	dispatch(OutputEvent::Status(StatusKind::Bullet, msg.to_string()));
}

/// Returns a colour-styled source label based on the provider name.
pub fn source_tag(source: &str) -> console::StyledObject<&str> {
	match source {
		"modrinth" => Style::new().green().apply_to(source),
		"curseforge" => Style::new().magenta().apply_to(source),
		"github" => Style::new().white().apply_to(source),
		"url" => Style::new().yellow().apply_to(source),
		"file" => Style::new().cyan().apply_to(source),
		_ => Style::new().dim().apply_to(source),
	}
}

/// Formats a version upgrade arrow: `old → new` with colour coding.
pub fn version_arrow(
	old: &str,
	new: &str,
) -> String {
	let old_style = Style::new().red().apply_to(old);
	let arrow = Style::new().yellow().apply_to("→");
	let new_style = Style::new().green().apply_to(new);
	format!("{} {} {}", old_style, arrow, new_style)
}

/// Creates a progress bar for file downloads.
pub fn download_progress(total: u64) -> ProgressBar {
	if is_captured() {
		capture_line(format!("Downloading 0/{}...", total));
		let pb = ProgressBar::hidden();
		pb.finish_and_clear();
		return pb;
	}
	if is_quiet() || is_json_mode() {
		let pb = ProgressBar::hidden();
		pb.finish_and_clear();
		return pb;
	}
	let pb = ProgressBar::new(total);
	pb.set_style(
		ProgressStyle::with_template(
			"{spinner:.green} {msg} [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
		)
		.expect("hardcoded download progress template is valid")
		.progress_chars("━╸─"),
	);
	pb
}

/// Creates a spinner for indeterminate progress.
pub fn spinner(msg: &str) -> ProgressBar {
	if is_captured() {
		capture_line(format!("⠋ {}", msg));
		let pb = ProgressBar::hidden();
		pb.finish_and_clear();
		return pb;
	}
	if is_quiet() || is_json_mode() {
		let pb = ProgressBar::hidden();
		pb.finish_and_clear();
		return pb;
	}
	let pb = ProgressBar::new_spinner();
	pb.set_style(
		ProgressStyle::with_template("{spinner:.green} {msg}")
			.expect("hardcoded spinner template is valid")
			.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
	);
	pb.set_message(msg.to_string());
	pb.enable_steady_tick(std::time::Duration::from_millis(80));
	pb
}

/// Returns a colour-styled label for a dependency kind.
pub fn dependency_kind_styled(
	kind: &DependencyKind
) -> console::StyledObject<&'static str> {
	use console::Style;
	match kind {
		DependencyKind::Required => Style::new().green().apply_to("required"),
		DependencyKind::Optional => Style::new().yellow().apply_to("optional"),
		DependencyKind::Incompatible => {
			Style::new().red().apply_to("incompatible")
		}
		DependencyKind::Embedded => Style::new().dim().apply_to("embedded"),
	}
}

/// Returns the display label for a mod source (e.g. "Modrinth", "CurseForge").
pub fn source_label(source: &ModSource) -> &'static str {
	match source {
		ModSource::Modrinth { .. } => "Modrinth",
		ModSource::CurseForge { .. } => "CurseForge",
		ModSource::Url { .. } => "URL",
	}
}

/// Returns the display color for a mod source.
pub fn source_color(source: &ModSource) -> TColor {
	match source {
		ModSource::Modrinth { .. } => TColor::Green,
		ModSource::CurseForge { .. } => TColor::Magenta,
		ModSource::Url { .. } => TColor::DarkGrey,
	}
}

/// Creates a new table with the standard yammm styling preset.
pub fn new_table() -> Table {
	let mut table = Table::new();
	table
		.load_preset(UTF8_FULL)
		.apply_modifier(UTF8_ROUND_CORNERS)
		.set_content_arrangement(ContentArrangement::Dynamic);
	table
}

/// Globally enables or disables coloured output for stdout and stderr.
pub fn set_colors_enabled(enabled: bool) {
	console::set_colors_enabled(enabled);
	console::set_colors_enabled_stderr(enabled);
}

/// Prompts the user with a yes/no confirmation. Returns `true` if the user
/// confirms, `false` if they decline.
pub fn confirm(
	prompt: impl std::fmt::Display,
	default: bool,
) -> anyhow::Result<bool> {
	Ok(dialoguer::Confirm::new()
		.with_prompt(prompt.to_string())
		.default(default)
		.interact()?)
}

/// Prints a "cancelled" message.
pub fn cancelled(action: &str) {
	warning(format!("{} cancelled.", action));
}

/// Prints a download batch summary — failures first, then total count.
pub fn present_download_summary(summary: &DownloadSummary) {
	if !summary.failed.is_empty() {
		for (name, e) in &summary.failed {
			error(format!("{} download failed: {}", name, e));
		}
	}

	if summary.downloaded > 0 {
		success(format!("{} file(s) downloaded", summary.downloaded));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Tests that exercise the process-global capture / quiet / json
	/// flags must hold this lock — `cargo test` runs lib tests in
	/// parallel by default and the atomics aren't isolated per test.
	fn output_state_lock() -> std::sync::MutexGuard<'static, ()> {
		use std::sync::Mutex;
		static M: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
		M.get_or_init(|| Mutex::new(()))
			.lock()
			.unwrap_or_else(|e| e.into_inner())
	}

	#[test]
	fn test_source_label() {
		assert_eq!(source_label(&ModSource::modrinth("x")), "Modrinth");
		assert_eq!(source_label(&ModSource::curseforge("123")), "CurseForge");
		assert_eq!(source_label(&ModSource::url("https://example.com")), "URL");
	}

	#[test]
	fn test_source_color() {
		assert_eq!(source_color(&ModSource::modrinth("x")), TColor::Green);
		assert_eq!(
			source_color(&ModSource::curseforge("123")),
			TColor::Magenta
		);
		assert_eq!(
			source_color(&ModSource::url("https://example.com")),
			TColor::DarkGrey
		);
	}

	#[test]
	fn test_version_arrow() {
		let arrow = version_arrow("1.0.0", "2.0.0");
		assert!(arrow.contains("1.0.0"));
		assert!(arrow.contains("2.0.0"));
	}

	#[test]
	fn test_new_table() {
		let table = new_table();
		let rendered = table.to_string();
		assert!(!rendered.is_empty());
	}

	#[test]
	fn test_download_summary_with_failures() {
		let summary = DownloadSummary {
			downloaded: 1,
			failed: vec![("mod-a".to_string(), anyhow::anyhow!("timeout"))],
		};
		assert_eq!(summary.total(), 2);
	}

	#[test]
	fn test_set_colors_enabled() {
		set_colors_enabled(false);
		set_colors_enabled(true);
	}

	#[test]
	fn quiet_mode_toggle_is_observable() {
		let _lock = output_state_lock();
		// The flag is a process-wide global, so other tests may have touched
		// it. Snapshot, mutate, assert, restore — and don't rely on the
		// initial state.
		let prev = is_quiet();
		set_quiet(true);
		assert!(is_quiet());
		set_quiet(false);
		assert!(!is_quiet());
		set_quiet(prev);
	}

	#[test]
	fn captured_lines_ignore_quiet_for_visibility() {
		let _lock = output_state_lock();
		// Capture wins over quiet — the capture buffer must still see the
		// line so the TUI / test harness can inspect what would have been
		// printed. Drive both helpers through the macro path.
		let prev = is_quiet();
		set_quiet(true);
		start_capture();
		success("captured message");
		bullet("captured bullet");
		let lines = stop_capture();
		set_quiet(prev);

		assert!(
			lines.iter().any(|l| l.contains("captured message")),
			"capture should record success lines even in quiet mode: {lines:?}"
		);
		assert!(
			lines.iter().any(|l| l.contains("captured bullet")),
			"capture should record bullet lines even in quiet mode: {lines:?}"
		);
	}

	#[test]
	fn capture_records_plain_rendering_for_all_status_kinds() {
		let _lock = output_state_lock();
		start_capture();
		heading("H");
		info("I");
		dim("D");
		success("S");
		bullet("B");
		blank_line();
		raw_line("R");
		warning("W");
		error("E");
		let lines = stop_capture();

		// Capture stores unstyled text — no ANSI escapes — so the
		// substrings here would survive any colour setting.
		assert!(lines.iter().any(|l| l == "H"));
		assert!(lines.iter().any(|l| l == "I"));
		assert!(lines.iter().any(|l| l == "D"));
		assert!(lines.iter().any(|l| l == "✓ S"));
		assert!(lines.iter().any(|l| l == "  • B"));
		assert!(lines.iter().any(|l| l.is_empty()));
		assert!(lines.iter().any(|l| l == "R"));
		assert!(lines.iter().any(|l| l == "⚠ W"));
		assert!(lines.iter().any(|l| l == "✗ E"));
	}

	#[test]
	fn emit_json_routes_through_dispatch() {
		let _lock = output_state_lock();
		start_capture();
		emit_json(&serde_json::json!({ "k": "v" })).unwrap();
		let lines = stop_capture();
		let joined = lines.join("\n");
		assert!(joined.contains("\"k\""));
		assert!(joined.contains("\"v\""));
	}
}
