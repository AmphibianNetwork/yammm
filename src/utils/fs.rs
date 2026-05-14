//! Filesystem helpers: symlinks (with Windows fallbacks), recursive file finding,
//! and secure file writing (restricted permissions on Unix).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Recursively search for a file by name, starting from `dir`.
pub fn find_file_recursive(
	dir: &Path,
	filename: &str,
) -> Option<PathBuf> {
	if !dir.exists() {
		return None;
	}
	for entry in std::fs::read_dir(dir).ok()? {
		let entry = entry.ok()?;
		let path = entry.path();
		if path.is_dir() {
			if let Some(found) = find_file_recursive(&path, filename) {
				return Some(found);
			}
		} else if path
			.file_name()
			.and_then(|n| n.to_str())
			.is_some_and(|n| n == filename)
		{
			return Some(path);
		}
	}
	None
}

/// Create a symlink (or fallback on Windows).
///
/// On Unix, creates a standard symlink. On Windows, tries symlink_file
/// (or symlink_dir for directories), then falls back to a hard link,
/// then to a full file copy if even hard links aren't supported
/// (e.g. cross-drive).
pub fn create_symlink(
	original: &Path,
	link: &Path,
) -> std::io::Result<()> {
	#[cfg(unix)]
	{
		std::os::unix::fs::symlink(original, link)
	}
	#[cfg(windows)]
	{
		if original.is_dir() {
			if std::os::windows::fs::symlink_dir(original, link).is_err() {
				std::fs::create_dir_all(link)?;
				for entry in std::fs::read_dir(original)? {
					let entry = entry?;
					let src = entry.path();
					let dst = link.join(entry.file_name());
					create_symlink(&src, &dst)?;
				}
			}
		} else if std::os::windows::fs::symlink_file(original, link).is_err() {
			if std::fs::hard_link(original, link).is_err() {
				std::fs::copy(original, link)?;
			}
		}
		Ok(())
	}
}

fn list_files_recursive(
	vec: &mut Vec<PathBuf>,
	path: &Path,
	include_symlinks: bool,
) -> std::io::Result<()> {
	if let Ok(metadata) = std::fs::symlink_metadata(path)
		&& metadata.file_type().is_dir()
	{
		for entry in std::fs::read_dir(path)? {
			let full_path = entry?.path();
			let metadata = std::fs::symlink_metadata(&full_path)?;

			if !include_symlinks && metadata.file_type().is_symlink() {
				continue;
			}

			if metadata.file_type().is_dir() {
				list_files_recursive(vec, &full_path, include_symlinks)?;
			} else {
				vec.push(full_path);
			}
		}
	}
	Ok(())
}

/// Write data to a file with restricted permissions.
///
/// On Unix, creates the file with `0o600` permissions from the start (no
/// window where the file is world-readable), then writes data.
/// On Windows, the file inherits the per-user ACL of the parent directory
/// (typically `%APPDATA%`), which provides equivalent protection.
pub fn write_secret_file(
	path: &Path,
	data: &str,
) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)
			.context("Failed to create parent directory")?;
	}

	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		std::fs::OpenOptions::new()
			.write(true)
			.create(true)
			.truncate(true)
			.mode(0o600)
			.open(path)
			.and_then(|mut f| {
				use std::io::Write;
				f.write_all(data.as_bytes())
			})
			.context("Failed to write secret file")?;
	}

	#[cfg(not(unix))]
	{
		std::fs::write(path, data).context("Failed to write file")?;
	}

	Ok(())
}

/// RAII guard that removes a directory tree when dropped.
///
/// Used for temporary directories that need deterministic cleanup
/// (e.g. installer extraction dirs) without depending on the `tempfile` crate
/// in production code.
pub struct TempDirCleanup<'a>(pub &'a Path);

impl Drop for TempDirCleanup<'_> {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(self.0);
	}
}

/// List all files in a directory tree, optionally including symlinks.
///
/// Uses `symlink_metadata` instead of `metadata` to avoid following
/// symlinks, which prevents infinite loops in symlink cycles.
pub fn list_files(
	path: &Path,
	include_symlinks: bool,
) -> Vec<PathBuf> {
	let mut vec = Vec::new();
	if let Err(e) = list_files_recursive(&mut vec, path, include_symlinks) {
		tracing::warn!("Error listing files in {}: {}", path.display(), e);
	}
	vec
}
