//! Terminal output formatting layer.
//!
//! Styled print helpers, progress bars, spinners, and table construction.
//! All styling can be globally disabled via `set_colors_enabled(false)`.

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

static SUCCESS_CHECK: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().green().bold().apply_to("✓"));
static ERROR_CROSS: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().red().bold().apply_to("✗"));
static WARN_SYMBOL: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().yellow().bold().apply_to("⚠"));
static BULLET_POINT: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().dim().apply_to("  •"));
static ITEM_CHECK: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().green().apply_to("✓"));
static ITEM_CROSS: LazyLock<console::StyledObject<&str>> =
	LazyLock::new(|| Style::new().red().apply_to("✗"));

static HEADING_STYLE: LazyLock<Style> =
	LazyLock::new(|| Style::new().bold().cyan());
static INFO_STYLE: LazyLock<Style> = LazyLock::new(|| Style::new().cyan());
static DIM_STYLE: LazyLock<Style> = LazyLock::new(|| Style::new().dim());
static BOLD_STYLE: LazyLock<Style> = LazyLock::new(|| Style::new().bold());

/// Emit a styled line to stdout, or capture it if output capture is active.
macro_rules! emit {
	(stdout $line:expr, $styled:expr) => {
		if is_captured() {
			capture_line($line);
			return;
		}
		println!("{}", $styled);
	};
	(stderr $line:expr, $styled:expr) => {
		if is_captured() {
			capture_line($line);
			return;
		}
		eprintln!("{}", $styled);
	};
}

pub fn success(msg: impl std::fmt::Display) {
	let line = format!("{} {}", *SUCCESS_CHECK, msg);
	emit!(stdout line, line);
}

pub fn error(msg: impl std::fmt::Display) {
	let line = format!("{} {}", *ERROR_CROSS, msg);
	emit!(stderr line, line);
}

pub fn warning(msg: impl std::fmt::Display) {
	let line = format!("{} {}", *WARN_SYMBOL, msg);
	emit!(stderr line, line);
}

pub fn heading(msg: impl std::fmt::Display) {
	let line = format!("{}", msg);
	emit!(stdout line, HEADING_STYLE.apply_to(msg));
}

pub fn info(msg: impl std::fmt::Display) {
	let line = format!("{}", msg);
	emit!(stdout line, INFO_STYLE.apply_to(msg));
}

pub fn dim(msg: impl std::fmt::Display) {
	let line = format!("{}", msg);
	emit!(stdout line, DIM_STYLE.apply_to(msg));
}

/// Prints an empty line.
pub fn blank_line() {
	if is_captured() {
		capture_line(String::new());
		return;
	}
	println!();
}

pub fn styled(
	msg: impl std::fmt::Display,
	style: Style,
) {
	let line = format!("{}", msg);
	emit!(stdout line, style.apply_to(msg));
}

pub fn bullet(msg: impl std::fmt::Display) {
	let line = format!("{} {}", *BULLET_POINT, msg);
	emit!(stdout line, line);
}

fn print_item(
	icon: &console::StyledObject<&str>,
	name: &str,
	version: &str,
	source: &str,
) {
	let line = format!("  {} {} v{} [{}]", icon, name, version, source);
	let ver = DIM_STYLE.apply_to(format!("v{}", version));
	let src = Style::new().blue().apply_to(format!("[{}]", source));
	emit!(stdout line, format!("  {} {} {} {}", icon, BOLD_STYLE.apply_to(name), ver, src));
}

pub fn item_success(
	name: &str,
	version: &str,
	source: &str,
) {
	print_item(&ITEM_CHECK, name, version, source);
}

pub fn item_missing(
	name: &str,
	version: &str,
	source: &str,
) {
	print_item(&ITEM_CROSS, name, version, source);
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
	let pb = ProgressBar::new(total);
	pb.set_style(
		ProgressStyle::with_template(
			"{spinner:.green} {msg} [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
		)
		.unwrap()
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
	let pb = ProgressBar::new_spinner();
	pb.set_style(
		ProgressStyle::with_template("{spinner:.green} {msg}")
			.unwrap()
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
}
