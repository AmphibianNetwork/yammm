//! Download service for mod JARs with retry and hash verification.
//!
//! - `download_jar`: single JAR with retry + hash check
//! - `download_missing_mods`: batch-download with concurrency semaphore

use crate::api::retry::{retry_request, RetryConfig};
use crate::output;
use crate::storage::JarCache;
use anyhow::{Context, Result};

#[derive(Debug)]
pub struct DownloadSummary {
	/// Number of JARs successfully downloaded
	pub downloaded: usize,
	/// JARs that failed, with their name and the error
	pub failed: Vec<(String, anyhow::Error)>,
}

impl DownloadSummary {
	pub fn total(&self) -> usize {
		self.downloaded + self.failed.len()
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

	// Fast path: already cached from a previous download.
	let cached = cache.jar_path(hash_type, hash);
	if cached.exists() {
		tracing::debug!("JAR already cached: {}", cached.display());
		return Ok((cached, hash.to_string()));
	}

	tracing::info!("Downloading {}...", name);

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
async fn download_once(
	client: reqwest::Client,
	cache: JarCache,
	download_url: String,
	hash_type: crate::types::HashType,
	expected_hash: String,
	name: String,
) -> Result<(std::path::PathBuf, String)> {
	let response =
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

	let bytes = response
		.bytes()
		.await
		.with_context(|| format!("Failed to read response for {}", name))?;

	let computed = hash_type.compute_for_bytes(&bytes);

	// Hash verification.
	if !expected_hash.is_empty() && computed != expected_hash {
		return Err(crate::errors::YammmError::HashMismatch {
			name,
			expected: expected_hash,
			actual: computed,
		}
		.into());
	}

	if expected_hash.is_empty() {
		tracing::warn!("No hash provided for {}, skipping verification", name);
	}

	cache.write_bytes(hash_type, &computed, &bytes, &name)
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

	// Filter to mods whose hash isn't in the cache yet.
	let to_download: Vec<&crate::types::TrackedMod> = all_items
		.iter()
		.filter(|m| {
			m.hash
				.as_ref()
				.is_some_and(|h| cache.get(m.hash_type, h).is_none())
		})
		.collect();

	if to_download.is_empty() {
		return Ok(DownloadSummary {
			downloaded: 0,
			failed: vec![],
		});
	}

	let pb = output::download_progress(to_download.len() as u64);
	pb.set_message("Downloading mods");

	let mut downloaded = 0usize;
	let mut failed: Vec<(String, anyhow::Error)> =
		Vec::with_capacity(to_download.len());

	// Semaphore limits concurrent network requests.
	let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
	let mut tasks = tokio::task::JoinSet::new();

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

		tasks.spawn(async move {
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
	}

	while let Some(result) = tasks.join_next().await {
		pb.inc(1);
		match result {
			Ok((name, Ok((_path, _computed_hash)))) => {
				downloaded += 1;
				pb.set_message(format!("Downloaded {}", name));
			}
			Ok((name, Err(e))) => {
				failed.push((name, e));
			}
			Err(e) => {
				failed.push((
					"unknown".to_string(),
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
}
