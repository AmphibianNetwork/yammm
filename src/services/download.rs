//! Download service for mod JARs with retry and hash verification.
//!
//! - `download_jar`: single JAR with retry + hash check
//! - `download_missing_mods`: batch-download with concurrency semaphore
//!
//! Downloads are streamed chunk-by-chunk into a temporary file in the cache
//! directory, hashed from disk, and only renamed into the final location if
//! the hash matches. This keeps memory bounded regardless of JAR size and
//! ensures no partially-written file is ever visible at the final path.

use crate::api::retry::{RetryConfig, retry_request};
use crate::output;
use crate::storage::JarCache;
use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

#[derive(Debug)]
pub struct DownloadSummary {
	/// Number of JARs successfully downloaded
	pub downloaded: usize,
	/// JARs that failed, with their name and the error
	pub failed: Vec<(String, anyhow::Error)>,
}

impl DownloadSummary {
	#[allow(dead_code)] // surfaced in summary lines that aren't wired in yet
	pub fn total(&self) -> usize {
		self.downloaded + self.failed.len()
	}

	/// Consume the summary and return `Ok(())` only if everything succeeded.
	///
	/// On any failure, returns the first error wrapped in an aggregate
	/// `context()` so its variant (and hence `exit_code()`) survives. This
	/// replaces the boilerplate pattern of `if !summary.failed.is_empty() { bail!(...) }`
	/// that every caller previously had to remember.
	pub fn into_result(self) -> anyhow::Result<()> {
		if self.failed.is_empty() {
			return Ok(());
		}
		let count = self.failed.len();
		let mut iter = self.failed.into_iter();
		let (first_name, first_err) = iter.next().expect("non-empty guard");
		// Log remaining errors so the user can see *all* failures, not just the
		// first — the chain only carries the head error for exit-code purposes.
		for (name, err) in iter {
			tracing::error!(name = %name, error = ?err, "download failed");
		}
		Err(first_err.context(format!(
			"{} file(s) failed to download (first failure: {})",
			count, first_name
		)))
	}
}

/// Parameters for downloading a JAR to the cache.
#[derive(Debug)]
pub struct DownloadJarParams<'a> {
	pub cache: &'a JarCache,
	pub client: &'a reqwest::Client,
	pub download_url: &'a str,
	pub hash_type: crate::types::HashType,
	pub hash: &'a str,
	pub name: &'a str,
}

/// Download a single JAR to the cache if not already present.
///
/// Returns immediately if cached. Retries transient failures with
/// exponential backoff. Hash mismatches are retryable (CDN may serve stale content).
///
/// Fails fast if `hash` is empty — we refuse to write a JAR we cannot verify.
/// Callers without a hash must obtain one before invoking this function.
pub async fn download_jar(
	params: DownloadJarParams<'_>
) -> Result<(std::path::PathBuf, String)> {
	let DownloadJarParams {
		cache,
		client,
		download_url,
		hash_type,
		hash,
		name,
	} = params;

	if hash.is_empty() {
		return Err(crate::errors::YammmError::download_failed(format!(
			"Refusing to download {}: no integrity hash available. \
			 Re-resolve the version metadata so a hash is recorded \
			 before downloading.",
			name
		))
		.into());
	}

	// Fast path: already cached from a previous download.
	let cached = cache.jar_path(hash_type, hash);
	if cached.exists() {
		tracing::debug!(
			name = %name,
			hash_type = ?hash_type,
			path = %cached.display(),
			"jar cache hit"
		);
		// Keep the LRU honest so frequently-used cached JARs survive eviction.
		cache.mark_used(hash_type, hash);
		return Ok((cached, hash.to_string()));
	}

	tracing::info!(
		name = %name,
		url = %download_url,
		hash_type = ?hash_type,
		"downloading jar"
	);

	let config = RetryConfig::default();
	let client = client.clone();
	let download_url = download_url.to_string();
	let hash = hash.to_string();
	let name_owned = name.to_string();
	retry_request(&config, || {
		let client = client.clone();
		let download_url = download_url.clone();
		let hash = hash.clone();
		let name = name_owned.clone();
		let cache = cache.clone();
		async move {
			download_once(client, cache, download_url, hash_type, hash, name)
				.await
		}
	})
	.await
}

/// Single download attempt (no retry — handled by `retry_request` wrapper).
///
/// Streams the response body chunk-by-chunk into a unique `.tmp` file in the
/// cache directory, then re-hashes the file from disk. If the hash matches,
/// the tmp file is renamed into place; otherwise it's deleted and an error
/// is returned. This keeps peak memory at ~64 KB regardless of JAR size.
async fn download_once(
	client: reqwest::Client,
	cache: JarCache,
	download_url: String,
	hash_type: crate::types::HashType,
	expected_hash: String,
	name: String,
) -> Result<(std::path::PathBuf, String)> {
	let mut response =
		client.get(&download_url).send().await.with_context(|| {
			format!("Failed to download {}: {}", name, download_url)
		})?;

	let status = response.status();
	if status.is_server_error() {
		return Err(crate::errors::YammmError::network_error(format!(
			"Server error downloading {}: HTTP {}",
			name, status
		))
		.into());
	}

	if !status.is_success() {
		return Err(crate::errors::YammmError::download_failed(format!(
			"Failed to download {}: HTTP {}",
			name, status
		))
		.into());
	}

	let dest = cache.jar_path(hash_type, &expected_hash);
	if let Some(parent) = dest.parent() {
		tokio::fs::create_dir_all(parent).await.with_context(|| {
			format!("Failed to create cache directory for {}", name)
		})?;
	}
	let tmp_path = unique_tmp_path(&dest);

	// `TmpFileGuard` ensures the partial file is cleaned up on any early return
	// (hash mismatch, network error mid-stream, task cancellation).
	let guard = TmpFileGuard::new(tmp_path.clone());

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

	// Hash the file from disk. Pages are still in the buffer cache after the
	// write, so this is effectively free I/O — and it sidesteps any
	// inconsistency between what we streamed and what's actually persisted.
	let hash_path = tmp_path.clone();
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

	if computed != expected_hash {
		return Err(crate::errors::YammmError::HashMismatch {
			name,
			expected: expected_hash,
			actual: computed,
		}
		.into());
	}

	let dest_clone = dest.clone();
	let tmp_clone = tmp_path.clone();
	let name_for_rename = name.clone();
	let computed_for_rename = computed.clone();
	let final_path =
		tokio::task::spawn_blocking(move || -> Result<std::path::PathBuf> {
			cache.commit_tmp(
				hash_type,
				&computed_for_rename,
				&tmp_clone,
				&dest_clone,
				&name_for_rename,
			)
		})
		.await
		.map_err(|e| {
			crate::errors::YammmError::general(format!(
				"rename task panicked: {}",
				e
			))
		})??;

	// On success the tmp path no longer exists (it was renamed) — disarm
	// the guard so its drop is a no-op.
	guard.disarm();
	Ok((final_path, computed))
}

fn unique_tmp_path(dest: &std::path::Path) -> std::path::PathBuf {
	use std::sync::atomic::{AtomicU64, Ordering};
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let n = COUNTER.fetch_add(1, Ordering::Relaxed);
	let pid = std::process::id();
	let parent = dest
		.parent()
		.map(std::path::Path::to_path_buf)
		.unwrap_or_default();
	let base = dest
		.file_name()
		.and_then(|s| s.to_str())
		.unwrap_or("download");
	parent.join(format!(".{base}.{pid}.{n}.tmp"))
}

/// RAII guard that deletes a partial tmp file on drop unless disarmed.
struct TmpFileGuard {
	path: std::path::PathBuf,
	armed: bool,
}

impl TmpFileGuard {
	fn new(path: std::path::PathBuf) -> Self {
		Self { path, armed: true }
	}

	fn disarm(mut self) {
		self.armed = false;
	}
}

impl Drop for TmpFileGuard {
	/// Best-effort cleanup of the partial tmp file. Synchronous on purpose:
	/// removing a single small file is well under a millisecond, and
	/// spawning an async cleanup from `drop` introduces timing surprises
	/// (the task may not complete before the next observation point) that
	/// callers — including tests — rely on not seeing.
	fn drop(&mut self) {
		if self.armed {
			let _ = std::fs::remove_file(&self.path);
		}
	}
}

/// Batch-download all mod JARs not yet in the cache.
/// Caps concurrent downloads with a semaphore.
pub async fn download_missing_mods(
	storage: &crate::storage::Storage,
	cache: &JarCache,
	http_client: &reqwest::Client,
	max_concurrent: usize,
) -> Result<DownloadSummary> {
	let all_items = storage.list_all()?;

	let mut failed: Vec<(String, anyhow::Error)> = Vec::new();

	// Partition into:
	//  - missing hash: surface as a failure (we won't write unverifiable JARs)
	//  - cached: skip
	//  - to download: actually fetch
	// Use `cache.contains()` rather than `cache.get()` so we don't touch the
	// LRU manifest (and trigger an fsync) for every mod in the modpack.
	let mut to_download: Vec<&crate::types::TrackedMod> = Vec::new();
	for m in &all_items {
		match m.hash.as_deref() {
			None | Some("") => {
				failed.push((
					m.name.clone(),
					crate::errors::YammmError::download_failed(format!(
						"No integrity hash recorded for {}; \
						 re-resolve the mod to obtain one",
						m.name
					))
					.into(),
				));
			}
			Some(h) if cache.contains(m.hash_type, h) => {}
			Some(_) => to_download.push(m),
		}
	}

	if to_download.is_empty() {
		return Ok(DownloadSummary {
			downloaded: 0,
			failed,
		});
	}

	let pb = output::download_progress(to_download.len() as u64);
	pb.set_message("Downloading mods");

	let mut downloaded = 0usize;
	failed.reserve(to_download.len());

	// Semaphore limits concurrent network requests.
	let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
	let mut tasks = tokio::task::JoinSet::new();
	// Track mod name per task id so a panic/cancellation in the task can
	// still be reported against the mod it was downloading.
	let mut names_by_id: std::collections::HashMap<tokio::task::Id, String> =
		std::collections::HashMap::with_capacity(to_download.len());

	for m in &to_download {
		let permit = sem
			.clone()
			.acquire_owned()
			.await
			.context("semaphore closed unexpectedly")?;
		let cache = cache.clone();
		let client = http_client.clone();
		let name = m.name.clone();
		let download_url = m.download_url.clone();
		let hash_type = m.hash_type;
		let hash = m.hash.clone();
		let task_name = name.clone();

		let abort = tasks.spawn(async move {
			let _permit = permit;
			let result = download_jar(DownloadJarParams {
				cache: &cache,
				client: &client,
				download_url: &download_url,
				hash_type,
				hash: hash.as_deref().unwrap_or_default(),
				name: &name,
			})
			.await;
			(name, result)
		});
		names_by_id.insert(abort.id(), task_name);
	}

	while let Some(result) = tasks.join_next_with_id().await {
		pb.inc(1);
		match result {
			Ok((id, (name, Ok((_path, _computed_hash))))) => {
				names_by_id.remove(&id);
				downloaded += 1;
				pb.set_message(format!("Downloaded {}", name));
			}
			Ok((id, (name, Err(e)))) => {
				names_by_id.remove(&id);
				failed.push((name, e));
			}
			Err(e) => {
				let name = names_by_id
					.remove(&e.id())
					.unwrap_or_else(|| "unknown".to_string());
				failed.push((
					name,
					crate::errors::YammmError::download_failed(format!(
						"{}",
						e
					))
					.into(),
				));
			}
		}
	}

	pb.finish_and_clear();

	Ok(DownloadSummary { downloaded, failed })
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::HashType;

	#[test]
	fn test_download_summary_total() {
		let summary = DownloadSummary {
			downloaded: 3,
			failed: vec![],
		};
		assert_eq!(summary.total(), 3);

		let summary_with_failures = DownloadSummary {
			downloaded: 2,
			failed: vec![("mod-a".to_string(), anyhow::anyhow!("timeout"))],
		};
		assert_eq!(summary_with_failures.total(), 3);
	}

	#[test]
	fn test_download_summary_into_result_preserves_first_error_kind() {
		let summary = DownloadSummary {
			downloaded: 0,
			failed: vec![
				(
					"mod-a".to_string(),
					crate::errors::YammmError::mod_not_found("mod-a").into(),
				),
				(
					"mod-b".to_string(),
					crate::errors::YammmError::network_error("net").into(),
				),
			],
		};
		let err = summary.into_result().unwrap_err();
		// The structured first-error must survive the wrapping context so
		// `exit_code()` returns 3 (mod_not_found), not 4 (download_failed).
		assert_eq!(crate::errors::exit_code(&err), 3);
	}

	#[test]
	fn test_download_summary_into_result_empty_is_ok() {
		let summary = DownloadSummary {
			downloaded: 5,
			failed: vec![],
		};
		assert!(summary.into_result().is_ok());
	}

	#[test]
	fn test_download_summary_all_failed() {
		let summary = DownloadSummary {
			downloaded: 0,
			failed: vec![
				("mod-a".to_string(), anyhow::anyhow!("err1")),
				("mod-b".to_string(), anyhow::anyhow!("err2")),
			],
		};
		assert_eq!(summary.total(), 2);
		assert_eq!(summary.downloaded, 0);
	}

	#[tokio::test]
	async fn test_download_jar_already_cached() {
		let temp_dir = tempfile::TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let hash_type = HashType::Sha512;
		let hash = hash_type.compute_for_bytes(b"cached content");
		cache
			.write_bytes(hash_type, &hash, b"cached content", "test.jar")
			.unwrap();

		let client = reqwest::Client::new();
		let result = download_jar(DownloadJarParams {
			cache: &cache,
			client: &client,
			download_url: "https://example.com/test.jar",
			hash_type,
			hash: &hash,
			name: "test",
		})
		.await
		.unwrap();

		assert_eq!(result.1, hash);
		assert!(result.0.exists());
	}

	#[tokio::test]
	async fn test_download_jar_empty_hash_rejected() {
		let temp_dir = tempfile::TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let client = reqwest::Client::new();
		// An empty hash should never reach the network; we refuse it up front.
		let result = download_jar(DownloadJarParams {
			cache: &cache,
			client: &client,
			download_url: "https://example.invalid/test.jar",
			hash_type: HashType::Sha512,
			hash: "",
			name: "test",
		})
		.await;

		assert!(result.is_err(), "empty hash must be rejected");
		let err = result.unwrap_err();
		let msg = format!("{:#}", err);
		assert!(msg.contains("no integrity hash"), "unexpected error: {msg}");
	}

	#[tokio::test]
	async fn test_download_jar_streams_and_verifies_hash() {
		use crate::types::HashType;

		let mut server = mockito::Server::new_async().await;
		let body = b"hello world body bytes for streaming download test";
		let hash_type = HashType::Sha512;
		let expected = hash_type.compute_for_bytes(body);

		let mock = server
			.mock("GET", "/mod.jar")
			.with_status(200)
			.with_header("content-type", "application/java-archive")
			.with_body(body)
			.create_async()
			.await;

		let temp_dir = tempfile::TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let url = format!("{}/mod.jar", server.url());
		let client = reqwest::Client::new();
		let (path, hash) = download_jar(DownloadJarParams {
			cache: &cache,
			client: &client,
			download_url: &url,
			hash_type,
			hash: &expected,
			name: "stream-test",
		})
		.await
		.expect("download should succeed");

		mock.assert_async().await;
		assert_eq!(hash, expected);
		assert!(path.exists());
		assert_eq!(std::fs::read(&path).unwrap(), body);

		// No leftover .tmp files should remain in the cache dir.
		let leftover: Vec<_> = std::fs::read_dir(temp_dir.path().join("cache"))
			.unwrap()
			.filter_map(|e| e.ok())
			.filter(|e| {
				e.path()
					.file_name()
					.and_then(|n| n.to_str())
					.is_some_and(|n| n.contains(".tmp"))
			})
			.collect();
		assert!(leftover.is_empty(), "leftover tmp files: {:?}", leftover);
	}

	#[tokio::test]
	async fn test_download_jar_hash_mismatch_cleans_tmp() {
		use crate::types::HashType;

		let mut server = mockito::Server::new_async().await;
		let body = b"actual body";
		let hash_type = HashType::Sha512;
		let wrong_hash = hash_type.compute_for_bytes(b"different body");

		let _mock = server
			.mock("GET", "/mismatch.jar")
			.with_status(200)
			.with_body(body)
			// retry layer will hit this a few times before giving up
			.expect_at_least(1)
			.create_async()
			.await;

		let temp_dir = tempfile::TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let url = format!("{}/mismatch.jar", server.url());
		let client = reqwest::Client::new();
		let result = download_jar(DownloadJarParams {
			cache: &cache,
			client: &client,
			download_url: &url,
			hash_type,
			hash: &wrong_hash,
			name: "mismatch-test",
		})
		.await;

		assert!(result.is_err(), "hash mismatch should fail");

		let leftover: Vec<_> = std::fs::read_dir(temp_dir.path().join("cache"))
			.unwrap()
			.filter_map(|e| e.ok())
			.collect();
		assert!(
			leftover.is_empty()
				|| leftover.iter().all(|e| e
					.path()
					.file_name()
					.and_then(|n| n.to_str())
					.is_some_and(|n| n == "cache_manifest.json")),
			"unexpected leftover files: {:?}",
			leftover
		);
	}
}
