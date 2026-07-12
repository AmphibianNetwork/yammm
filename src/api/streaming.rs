//! Streaming downloader used by all non-JAR-cache download paths.
//!
//! The JAR cache has its own content-addressed download routine in
//! [`crate::services::download`]; this module is for downloads that need to
//! land at a specific path (Minecraft assets, Forge/NeoForge libraries,
//! Adoptium JDK archives) instead of being keyed by hash.
//!
//! Common guarantees:
//!
//! - **Memory-bounded.** Chunks stream through `Response::chunk()` into a
//!   `tokio::fs::File`, never accumulating the full body in memory.
//! - **Atomic on disk.** Bytes land in a `.<pid>.<n>.tmp` neighbor file,
//!   fsynced, then renamed into place. Partial writes are removed by a
//!   `Drop` guard on error or task cancellation.
//! - **Integrity-verified when the caller provides a hash.** Pass
//!   `HashPolicy::Required(...)` and the file is re-hashed from disk
//!   (page cache still warm) and rejected on mismatch. Callers that
//!   legitimately can't supply a hash must opt out with
//!   `HashPolicy::AcceptedUnhashed { reason }` — the reason is logged so
//!   the integrity gap is explicit at the call site, not hidden behind a
//!   `None`.
//! - **Existence-skipping.** If `dest` already exists and the on-disk
//!   bytes match the expected hash (when one is provided), we return
//!   without touching the network. A mismatch deletes the stale file
//!   and falls through to a fresh download.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;

/// Integrity hash for a streaming download.
#[derive(Debug, Clone)]
pub struct ExpectedHash<'a> {
	pub hash_type: crate::types::HashType,
	pub hex: &'a str,
}

/// Whether `download_to_file` verifies the bytes after download.
///
/// `AcceptedUnhashed` is an explicit opt-out — every caller that can't
/// produce a hash must spell out the reason here so the gap is visible at
/// the call site. The reason is logged when a download begins.
#[derive(Debug, Clone)]
pub enum HashPolicy<'a> {
	/// Verify the file against this hash; reject and clean up on mismatch.
	Required(ExpectedHash<'a>),
	/// The caller has no hash to verify against and accepts the integrity
	/// gap. The `reason` documents *why* (manifest doesn't carry one,
	/// upstream API doesn't expose one, etc.) and is logged once per call.
	AcceptedUnhashed { reason: &'static str },
}

impl<'a> HashPolicy<'a> {
	/// Convenience for callers whose manifest carries an optional hash:
	/// if `hash` is `Some`, verify; otherwise accept unhashed for `reason`.
	pub fn from_optional(
		hash: Option<ExpectedHash<'a>>,
		reason: &'static str,
	) -> Self {
		match hash {
			Some(h) => HashPolicy::Required(h),
			None => HashPolicy::AcceptedUnhashed { reason },
		}
	}

	fn expected(&self) -> Option<&ExpectedHash<'a>> {
		match self {
			HashPolicy::Required(h) => Some(h),
			HashPolicy::AcceptedUnhashed { .. } => None,
		}
	}
}

/// Outcome of a streaming download. Useful for callers that want to log or
/// surface "we hit the cache" differently from "we just fetched bytes."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadOutcome {
	/// `dest` already existed; no network call was made.
	AlreadyPresent,
	/// Bytes were streamed from the network and verified (if a hash was given).
	Downloaded,
}

/// Stream a URL into `dest`, atomically and with optional hash verification.
///
/// The caller is responsible for creating any directories above `dest`. The
/// `name` parameter only flavors error messages.
pub async fn download_to_file(
	client: &reqwest::Client,
	url: &str,
	dest: &Path,
	policy: HashPolicy<'_>,
	name: &str,
) -> Result<DownloadOutcome> {
	if let HashPolicy::AcceptedUnhashed { reason } = &policy {
		tracing::debug!(
			"Streaming {} without integrity check (reason: {})",
			name,
			reason
		);
	}

	if tokio::fs::try_exists(dest)
		.await
		.with_context(|| format!("Failed to stat {}", name))?
	{
		// If a hash was provided, verify the on-disk bytes before trusting them.
		// A truncated or externally-modified file would otherwise be considered
		// valid forever. Mismatch deletes the stale file and falls through to
		// re-download.
		if let Some(expected) = policy.expected() {
			let path_for_hash = dest.to_path_buf();
			let hash_type = expected.hash_type;
			let computed = tokio::task::spawn_blocking(move || {
				hash_type.compute_for_file(&path_for_hash)
			})
			.await
			.map_err(|e| {
				crate::errors::YammmError::general(format!(
					"hashing task panicked: {}",
					e
				))
			})??;
			if computed == expected.hex {
				return Ok(DownloadOutcome::AlreadyPresent);
			}
			tracing::warn!(
				"Existing {} at {} has hash {} but expected {}; re-downloading",
				name,
				dest.display(),
				computed,
				expected.hex
			);
			tokio::fs::remove_file(dest)
				.await
				.with_context(|| format!("Failed to remove stale {}", name))?;
		} else {
			return Ok(DownloadOutcome::AlreadyPresent);
		}
	}

	if let Some(parent) = dest.parent() {
		tokio::fs::create_dir_all(parent).await.with_context(|| {
			format!("Failed to create parent directory for {}", name)
		})?;
	}

	let tmp_path = unique_tmp_path(dest);
	let guard = TmpFileGuard::new(tmp_path.clone());

	let mut response = crate::api::retry::send_retried(client, url, Vec::new())
		.await
		.map_err(|e| {
			crate::errors::YammmError::network_error(format!(
				"Failed to fetch {}: {}",
				name, e
			))
		})?;
	let status = response.status();
	if !status.is_success() {
		return Err(crate::errors::YammmError::download_failed(format!(
			"Failed to download {}: HTTP {}",
			name, status
		))
		.into());
	}

	{
		let mut file =
			tokio::fs::File::create(&tmp_path).await.with_context(|| {
				format!("Failed to create tmp file for {}", name)
			})?;
		while let Some(chunk) = response.chunk().await.with_context(|| {
			format!("Network error while downloading {}", name)
		})? {
			file.write_all(&chunk).await.with_context(|| {
				format!("Failed to write tmp file for {}", name)
			})?;
		}
		file.sync_all().await.with_context(|| {
			format!("Failed to fsync tmp file for {}", name)
		})?;
	}

	if let Some(expected) = policy.expected() {
		let hash_path = tmp_path.clone();
		let hash_type = expected.hash_type;
		let computed = tokio::task::spawn_blocking(move || {
			hash_type.compute_for_file(&hash_path)
		})
		.await
		.map_err(|e| {
			crate::errors::YammmError::general(format!(
				"hashing task panicked: {}",
				e
			))
		})??;
		if computed != expected.hex {
			return Err(crate::errors::YammmError::HashMismatch {
				name: name.to_string(),
				expected: expected.hex.to_string(),
				actual: computed,
			}
			.into());
		}
	}

	let tmp_for_rename = tmp_path.clone();
	let dest_for_rename = dest.to_path_buf();
	let name_for_rename = name.to_string();
	tokio::task::spawn_blocking(move || -> std::io::Result<()> {
		// If a concurrent task wrote the destination between our existence
		// check and now, prefer the existing file and discard our tmp.
		if dest_for_rename.exists() {
			let _ = std::fs::remove_file(&tmp_for_rename);
			return Ok(());
		}
		std::fs::rename(&tmp_for_rename, &dest_for_rename)
	})
	.await
	.map_err(|e| {
		crate::errors::YammmError::general(format!(
			"rename task panicked for {}: {}",
			name_for_rename, e
		))
	})?
	.with_context(|| format!("Failed to commit {}", name))?;

	guard.disarm();
	Ok(DownloadOutcome::Downloaded)
}

fn unique_tmp_path(dest: &Path) -> PathBuf {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let n = COUNTER.fetch_add(1, Ordering::Relaxed);
	let pid = std::process::id();
	let parent = dest.parent().map(Path::to_path_buf).unwrap_or_default();
	let base = dest
		.file_name()
		.and_then(|s| s.to_str())
		.unwrap_or("download");
	parent.join(format!(".{base}.{pid}.{n}.tmp"))
}

struct TmpFileGuard {
	path: PathBuf,
	armed: bool,
}

impl TmpFileGuard {
	fn new(path: PathBuf) -> Self {
		Self { path, armed: true }
	}

	fn disarm(mut self) {
		self.armed = false;
	}
}

impl Drop for TmpFileGuard {
	fn drop(&mut self) {
		if self.armed {
			let _ = std::fs::remove_file(&self.path);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::HashType;

	#[tokio::test]
	async fn test_download_to_file_streams_and_verifies() {
		let mut server = mockito::Server::new_async().await;
		let body = b"streaming download test body bytes";
		let sha1 = HashType::Sha1.compute_for_bytes(body);
		let _mock = server
			.mock("GET", "/lib.jar")
			.with_status(200)
			.with_body(body)
			.create_async()
			.await;

		let tmp = tempfile::TempDir::new().unwrap();
		let dest = tmp.path().join("nested/lib.jar");
		let client = reqwest::Client::new();
		let outcome = download_to_file(
			&client,
			&format!("{}/lib.jar", server.url()),
			&dest,
			HashPolicy::Required(ExpectedHash {
				hash_type: HashType::Sha1,
				hex: &sha1,
			}),
			"test-lib",
		)
		.await
		.expect("download should succeed");
		assert_eq!(outcome, DownloadOutcome::Downloaded);
		assert_eq!(std::fs::read(&dest).unwrap(), body);
	}

	#[tokio::test]
	async fn test_download_to_file_existing_short_circuits() {
		let tmp = tempfile::TempDir::new().unwrap();
		let dest = tmp.path().join("lib.jar");
		std::fs::write(&dest, b"already here").unwrap();
		let client = reqwest::Client::new();
		let outcome = download_to_file(
			&client,
			"http://example.invalid/should-not-fetch",
			&dest,
			HashPolicy::AcceptedUnhashed {
				reason: "test fixture has no hash",
			},
			"test-lib",
		)
		.await
		.unwrap();
		assert_eq!(outcome, DownloadOutcome::AlreadyPresent);
		assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
	}

	#[tokio::test]
	async fn test_download_to_file_hash_mismatch_cleans_up() {
		let mut server = mockito::Server::new_async().await;
		let body = b"actual body";
		let wrong = HashType::Sha1.compute_for_bytes(b"different");
		let _mock = server
			.mock("GET", "/x.jar")
			.with_status(200)
			.with_body(body)
			.expect_at_least(1)
			.create_async()
			.await;

		let tmp = tempfile::TempDir::new().unwrap();
		let dest = tmp.path().join("x.jar");
		let client = reqwest::Client::new();
		let result = download_to_file(
			&client,
			&format!("{}/x.jar", server.url()),
			&dest,
			HashPolicy::Required(ExpectedHash {
				hash_type: HashType::Sha1,
				hex: &wrong,
			}),
			"x",
		)
		.await;
		assert!(result.is_err());
		assert!(!dest.exists(), "dest must not exist after mismatch");
		// And the tmp file is gone too.
		let dirents: Vec<_> = std::fs::read_dir(tmp.path())
			.unwrap()
			.filter_map(|e| e.ok())
			.collect();
		assert!(dirents.is_empty(), "leftovers: {:?}", dirents);
	}

	#[tokio::test]
	async fn test_download_to_file_existing_corrupt_is_rejected_then_redownloaded()
	 {
		let mut server = mockito::Server::new_async().await;
		let body = b"the real bytes";
		let sha1 = HashType::Sha1.compute_for_bytes(body);
		let _mock = server
			.mock("GET", "/lib.jar")
			.with_status(200)
			.with_body(body)
			.create_async()
			.await;

		let tmp = tempfile::TempDir::new().unwrap();
		let dest = tmp.path().join("lib.jar");
		std::fs::write(&dest, b"corrupt").unwrap();

		let client = reqwest::Client::new();
		let outcome = download_to_file(
			&client,
			&format!("{}/lib.jar", server.url()),
			&dest,
			HashPolicy::Required(ExpectedHash {
				hash_type: HashType::Sha1,
				hex: &sha1,
			}),
			"test-lib",
		)
		.await
		.expect("re-download should succeed");
		assert_eq!(outcome, DownloadOutcome::Downloaded);
		assert_eq!(std::fs::read(&dest).unwrap(), body);
	}

	#[tokio::test]
	async fn test_download_to_file_existing_valid_short_circuits() {
		let tmp = tempfile::TempDir::new().unwrap();
		let dest = tmp.path().join("lib.jar");
		let body = b"already-correct bytes";
		std::fs::write(&dest, body).unwrap();
		let sha1 = HashType::Sha1.compute_for_bytes(body);

		let client = reqwest::Client::new();
		let outcome = download_to_file(
			&client,
			"http://example.invalid/should-not-fetch",
			&dest,
			HashPolicy::Required(ExpectedHash {
				hash_type: HashType::Sha1,
				hex: &sha1,
			}),
			"test-lib",
		)
		.await
		.unwrap();
		assert_eq!(outcome, DownloadOutcome::AlreadyPresent);
		assert_eq!(std::fs::read(&dest).unwrap(), body);
	}

	#[tokio::test]
	async fn test_download_to_file_http_error_is_propagated() {
		let mut server = mockito::Server::new_async().await;
		let _mock = server
			.mock("GET", "/nope.jar")
			.with_status(404)
			.expect_at_least(1)
			.create_async()
			.await;

		let tmp = tempfile::TempDir::new().unwrap();
		let dest = tmp.path().join("nope.jar");
		let client = reqwest::Client::new();
		let result = download_to_file(
			&client,
			&format!("{}/nope.jar", server.url()),
			&dest,
			HashPolicy::AcceptedUnhashed {
				reason: "404 test fixture",
			},
			"nope",
		)
		.await;
		assert!(result.is_err());
		assert!(!dest.exists());
	}
}
