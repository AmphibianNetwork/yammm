//! Filesystem helpers: symlinks (with Windows fallbacks), recursive file finding,
//! and secure file writing (restricted permissions on Unix).

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Open options for the temp file used by `atomic_write_bytes`.
///
/// On Unix the caller can request mode `0o600` so the file is created
/// non-readable by other users from the very first byte. On Windows the
/// `mode` field is ignored.
#[derive(Debug, Clone, Copy)]
pub struct AtomicWriteOptions {
	#[cfg(unix)]
	pub mode: Option<u32>,
	/// Whether to fsync the parent directory after rename (Unix only).
	/// Defaults to true; durability matters for auth tokens and cache manifests.
	pub fsync_parent: bool,
}

impl Default for AtomicWriteOptions {
	fn default() -> Self {
		Self {
			#[cfg(unix)]
			mode: None,
			fsync_parent: true,
		}
	}
}

/// Atomically write `data` to `path`.
///
/// Writes to `<path>.tmp` first, fsyncs the data, then renames into place.
/// Optionally fsyncs the parent directory so the rename is durable across
/// power loss (Unix only — Windows doesn't expose directory fsync).
///
/// If anything fails, the temp file is cleaned up.
pub fn atomic_write_bytes(
	path: &Path,
	data: &[u8],
	opts: AtomicWriteOptions,
) -> std::io::Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	let tmp_path = path.with_extension("tmp");

	// Scope: ensure the file handle is closed before the rename so Windows
	// doesn't trip on a sharing violation, and so fsync_all completes.
	let write_result = {
		let mut open_opts = std::fs::OpenOptions::new();
		open_opts.write(true).create(true).truncate(true);

		#[cfg(unix)]
		if let Some(mode) = opts.mode {
			use std::os::unix::fs::OpenOptionsExt;
			open_opts.mode(mode);
		}

		(|| -> std::io::Result<()> {
			let mut file = open_opts.open(&tmp_path)?;
			file.write_all(data)?;
			file.sync_all()
		})()
	};

	if let Err(e) = write_result {
		let _ = std::fs::remove_file(&tmp_path);
		return Err(e);
	}

	if let Err(e) = std::fs::rename(&tmp_path, path) {
		let _ = std::fs::remove_file(&tmp_path);
		return Err(e);
	}

	#[cfg(unix)]
	if opts.fsync_parent
		&& let Some(parent) = path.parent()
		&& let Ok(dir) = std::fs::File::open(parent)
	{
		// Best-effort: on filesystems that don't support dir fsync (some FUSE)
		// this returns EINVAL — not worth aborting a successful write.
		let _ = dir.sync_all();
	}

	#[cfg(not(unix))]
	let _ = opts.fsync_parent;

	Ok(())
}

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

/// Atomically write `data` to a file with restricted permissions.
///
/// On Unix, the temp file is created with `0o600` from the first byte (no
/// window where the file is world-readable), data is fsynced, then renamed
/// into place, and the parent directory is fsynced so the rename survives
/// power loss.
///
/// On Windows, the file inherits the per-user ACL of the parent directory
/// (typically `%APPDATA%`), which provides equivalent protection. The
/// write is still atomic via write-tmp + rename.
pub fn write_secret_file(
	path: &Path,
	data: &str,
) -> Result<()> {
	let opts = AtomicWriteOptions {
		#[cfg(unix)]
		mode: Some(0o600),
		fsync_parent: true,
	};
	atomic_write_bytes(path, data.as_bytes(), opts)
		.context("Failed to write secret file")
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
