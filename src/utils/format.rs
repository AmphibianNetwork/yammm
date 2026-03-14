//! Formatting utilities: sizes, dates, error chains, truncation.

use std::time::{SystemTime, UNIX_EPOCH};

/// Truncate a string to `max_chars` characters, appending `suffix` if truncated.
pub fn truncate_str(
	s: &str,
	max_chars: usize,
	suffix: &str,
) -> String {
	if s.chars().count() <= max_chars {
		return s.to_string();
	}
	let end = s
		.char_indices()
		.take(max_chars)
		.last()
		.map_or(0, |(i, c)| i + c.len_utf8());
	s[..end].to_string() + suffix
}

/// Print an `anyhow::Error` with its full cause chain.
pub fn print_error(e: &anyhow::Error) {
	crate::output::error(e.to_string());
	for cause in e.chain().skip(1) {
		crate::output::dim(format!("  Caused by: {}", cause));
	}
}

/// Format bytes as human-readable (binary prefixes: 1 KB = 1024 B).
pub fn format_size(bytes: u64) -> String {
	const KB: u64 = 1024;
	const MB: u64 = KB * 1024;
	const GB: u64 = MB * 1024;

	if bytes >= GB {
		format!("{:.2} GB", bytes as f64 / GB as f64)
	} else if bytes >= MB {
		format!("{:.2} MB", bytes as f64 / MB as f64)
	} else if bytes >= KB {
		format!("{:.2} KB", bytes as f64 / KB as f64)
	} else {
		format!("{} B", bytes)
	}
}

pub fn system_time_to_date(t: SystemTime) -> String {
	let secs = t
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let offset = time::OffsetDateTime::from_unix_timestamp(secs as i64)
		.unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
	offset.date().to_string()
}

pub fn today_iso8601() -> String {
	time::OffsetDateTime::now_utc().date().to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_format_size() {
		assert_eq!(format_size(500), "500 B");
		assert_eq!(format_size(1024), "1.00 KB");
		assert_eq!(format_size(1024 * 1024), "1.00 MB");
		assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GB");
	}

	#[test]
	fn test_format_size_zero() {
		assert_eq!(format_size(0), "0 B");
	}

	#[test]
	fn test_format_size_fractional_kb() {
		let result = format_size(1536);
		assert!(result.contains("KB"));
	}

	#[test]
	fn test_print_error_basic() {
		let err = anyhow::anyhow!("test error message");
		print_error(&err);
	}

	#[test]
	fn test_print_error_with_chain() {
		use anyhow::Context;
		let result: Result<(), anyhow::Error> =
			Err(anyhow::anyhow!("root cause")).context("outer context");
		if let Err(e) = result {
			print_error(&e);
		}
	}
}
