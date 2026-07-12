//! Conditional-GET cache for API metadata responses.
//!
//! Modrinth and CurseForge serve `ETag` (and sometimes `Last-Modified`)
//! headers on project / version metadata. Sending the saved validator
//! back as `If-None-Match` lets the server respond with a bodyless `304
//! Not Modified` when the resource hasn't changed — useful for `update`,
//! which re-reads the same metadata on every run.
//!
//! Cache layout:
//!
//! ```text
//! ~/.cache/yammm/http-meta/
//!   <sha256(url)[..16]>.json    # one file per cached URL
//! ```
//!
//! Each file holds the original URL (for collision detection), the
//! response body, the validators, and a fetched-at timestamp. JAR
//! downloads are *not* cached here — the content-addressed [`JarCache`]
//! handles those.
//!
//! [`JarCache`]: crate::storage::JarCache

#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Discard cached entries older than this. Even if a server keeps
/// claiming `304 Not Modified` indefinitely, callers will refetch fresh
/// once a day.
const MAX_AGE_SECS: u64 = 24 * 60 * 60;

/// Public accessor for [`MAX_AGE_SECS`] so callers (the CLI dispatcher,
/// integration tests) can report the threshold without depending on
/// the constant directly.
pub fn default_max_age_secs() -> u64 {
	MAX_AGE_SECS
}

/// Process-global bypass for the HTTP metadata cache. When set,
/// [`conditional_fetch_json`] skips both lookup and store — every
/// request goes straight to the network. Driven by the global
/// `--no-http-cache` CLI flag for debugging upstream weirdness.
static BYPASS: AtomicBool = AtomicBool::new(false);

pub fn set_bypass(enabled: bool) {
	BYPASS.store(enabled, Ordering::Relaxed);
}

pub fn is_bypassed() -> bool {
	BYPASS.load(Ordering::Relaxed)
}

/// Refuse to cache bodies bigger than this. Metadata responses are
/// small; anything larger is likely a binary or a misuse.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Soft cap on the number of files kept in the cache directory.
/// `store` runs a best-effort LRU eviction once the count exceeds
/// this. The cap is generous enough to cover months of typical
/// modpack work; the goal is bounding worst-case growth, not tight
/// memory accounting.
const MAX_ENTRIES: usize = 2_000;

/// One cached HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedResponse {
	/// The full request URL. Stored alongside the body so a hash collision
	/// doesn't silently return the wrong cached payload.
	pub url: String,
	pub body: String,
	#[serde(default)]
	pub etag: Option<String>,
	#[serde(default)]
	pub last_modified: Option<String>,
	pub fetched_at: u64,
}

impl CachedResponse {
	/// Has this entry exceeded [`MAX_AGE_SECS`]?
	pub fn is_stale(&self) -> bool {
		epoch_secs().saturating_sub(self.fetched_at) > MAX_AGE_SECS
	}
}

/// Snapshot of HTTP metadata cache usage. Surfaced by
/// [`HttpMetaCache::stats`] and rendered by `cache status`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpMetaStats {
	pub count: usize,
	pub total_bytes: u64,
}

/// On-disk HTTP metadata cache. Cheap to clone; thread-safe.
pub struct HttpMetaCache {
	root: PathBuf,
}

impl HttpMetaCache {
	pub fn new(root: PathBuf) -> Self {
		Self { root }
	}

	/// Process-global instance rooted at `$XDG_CACHE_HOME/yammm/http-meta`
	/// (or the platform-appropriate equivalent). Created on first use.
	pub fn shared() -> &'static HttpMetaCache {
		static INSTANCE: OnceLock<HttpMetaCache> = OnceLock::new();
		INSTANCE.get_or_init(|| {
			let root = std::env::var("YAMMM_CACHE_DIR")
				.ok()
				.map(PathBuf::from)
				.or_else(dirs::cache_dir)
				.map(|d| d.join("yammm").join("http-meta"))
				.unwrap_or_else(|| {
					PathBuf::from(".yammm-cache").join("http-meta")
				});
			HttpMetaCache::new(root)
		})
	}

	#[cfg(test)]
	pub fn root(&self) -> &Path {
		&self.root
	}

	fn path_for(
		&self,
		url: &str,
	) -> PathBuf {
		let mut hasher = Sha256::new();
		hasher.update(url.as_bytes());
		let digest = hasher.finalize();
		let hex = hex::encode(digest);
		self.root.join(format!("{}.json", &hex[..16]))
	}

	/// Read the cache entry for `url`, if any. Returns `None` on any
	/// error (missing file, parse error, URL collision) — the caller
	/// will refetch unconditionally in that case.
	pub fn lookup(
		&self,
		url: &str,
	) -> Option<CachedResponse> {
		let path = self.path_for(url);
		let data = std::fs::read_to_string(&path).ok()?;
		let cached: CachedResponse = serde_json::from_str(&data).ok()?;
		// Collision guard: if the SHA-256-prefix landed on the same file
		// for a different URL, the stored URL won't match and we discard.
		if cached.url != url {
			return None;
		}
		Some(cached)
	}

	/// Persist a response. Silently no-ops if the body is too large or
	/// neither validator is present (without a validator there is
	/// nothing for the server to compare against on the next request).
	/// Write failures are logged but never propagated.
	pub fn store(
		&self,
		url: &str,
		body: &str,
		etag: Option<String>,
		last_modified: Option<String>,
	) {
		if body.len() > MAX_BODY_BYTES {
			return;
		}
		if etag.is_none() && last_modified.is_none() {
			return;
		}
		if let Err(e) = std::fs::create_dir_all(&self.root) {
			tracing::debug!(
				root = %self.root.display(),
				error = %e,
				"failed to create http meta cache dir; skipping store"
			);
			return;
		}
		let entry = CachedResponse {
			url: url.to_string(),
			body: body.to_string(),
			etag,
			last_modified,
			fetched_at: epoch_secs(),
		};
		let path = self.path_for(url);
		let data = match serde_json::to_string(&entry) {
			Ok(d) => d,
			Err(e) => {
				tracing::debug!(error = %e, "failed to serialize http cache entry");
				return;
			}
		};
		if let Err(e) = crate::utils::fs::atomic_write_bytes(
			&path,
			data.as_bytes(),
			crate::utils::fs::AtomicWriteOptions::default(),
		) {
			tracing::debug!(
				path = %path.display(),
				error = %e,
				"failed to persist http cache entry"
			);
			return;
		}

		// Best-effort soft cap: keep the directory bounded under heavy
		// use. Errors here are non-fatal — the write succeeded; eviction
		// will retry next time.
		if let Err(e) = self.evict_if_over_cap() {
			tracing::debug!(
				root = %self.root.display(),
				error = %e,
				"http meta cache eviction failed; continuing"
			);
		}
	}

	/// Aggregate stats for the on-disk HTTP metadata cache.
	pub fn stats(&self) -> std::io::Result<HttpMetaStats> {
		if !self.root.exists() {
			return Ok(HttpMetaStats::default());
		}
		let mut count = 0usize;
		let mut total_bytes = 0u64;
		for entry in std::fs::read_dir(&self.root)? {
			let entry = match entry {
				Ok(e) => e,
				Err(_) => continue,
			};
			if entry.path().is_file()
				&& let Ok(meta) = entry.metadata()
			{
				count += 1;
				total_bytes += meta.len();
			}
		}
		Ok(HttpMetaStats { count, total_bytes })
	}

	/// If the directory holds more than [`MAX_ENTRIES`] files, remove
	/// the oldest by `fetched_at` until we are back at the cap. Called
	/// after a successful [`store`] so eviction amortizes naturally
	/// with the writes that caused the growth.
	fn evict_if_over_cap(&self) -> std::io::Result<usize> {
		let mut entries: Vec<(PathBuf, u64)> = Vec::new();
		for entry in std::fs::read_dir(&self.root)? {
			let entry = match entry {
				Ok(e) => e,
				Err(_) => continue,
			};
			let path = entry.path();
			if !path.is_file() {
				continue;
			}
			let fetched_at = match std::fs::read_to_string(&path)
				.ok()
				.and_then(|d| serde_json::from_str::<CachedResponse>(&d).ok())
			{
				Some(c) => c.fetched_at,
				// Unparseable entries are treated as oldest so they get
				// evicted first.
				None => 0,
			};
			entries.push((path, fetched_at));
		}

		if entries.len() <= MAX_ENTRIES {
			return Ok(0);
		}

		entries.sort_by_key(|(_, t)| *t);
		let to_remove = entries.len() - MAX_ENTRIES;
		let mut removed = 0usize;
		for (path, _) in entries.into_iter().take(to_remove) {
			if std::fs::remove_file(&path).is_ok() {
				removed += 1;
			}
		}
		Ok(removed)
	}

	/// Remove every cached entry. Used by `cache clear-http-meta`.
	pub fn clear(&self) -> std::io::Result<()> {
		if self.root.exists() {
			std::fs::remove_dir_all(&self.root)?;
		}
		Ok(())
	}

	/// Remove every cached entry past its [`MAX_AGE_SECS`] window.
	///
	/// Convenience wrapper around [`Self::clear_older_than`] for the
	/// default policy. `cache clear-http-meta --stale` calls this.
	pub fn clear_stale(&self) -> std::io::Result<usize> {
		self.clear_older_than(MAX_AGE_SECS)
	}

	/// Remove every cached entry older than `max_age_secs` seconds (by
	/// embedded `fetched_at`, not filesystem mtime). Use this when the
	/// caller wants a custom threshold — e.g. `cache clear-http-meta
	/// --max-age 7d` for weekly maintenance.
	///
	/// Unparseable files are always evicted so accumulated corruption
	/// gets cleaned up regardless of threshold.
	///
	/// Errors reading individual entries are logged and the walk
	/// continues — a single unreadable file should not prevent the rest
	/// of the cleanup.
	pub fn clear_older_than(
		&self,
		max_age_secs: u64,
	) -> std::io::Result<usize> {
		if !self.root.exists() {
			return Ok(0);
		}
		let now = epoch_secs();
		let mut removed = 0usize;
		for entry in std::fs::read_dir(&self.root)? {
			let entry = match entry {
				Ok(e) => e,
				Err(e) => {
					tracing::debug!(error = %e, "skipping unreadable dir entry");
					continue;
				}
			};
			let path = entry.path();
			if !path.is_file() {
				continue;
			}
			let data = match std::fs::read_to_string(&path) {
				Ok(d) => d,
				Err(e) => {
					tracing::debug!(
						path = %path.display(),
						error = %e,
						"failed to read cache file during stale prune"
					);
					continue;
				}
			};
			let cached: CachedResponse = match serde_json::from_str(&data) {
				Ok(c) => c,
				Err(_) => {
					// Unparseable file — treat as junk and remove it so
					// the cache doesn't accumulate corruption over time.
					if std::fs::remove_file(&path).is_ok() {
						removed += 1;
					}
					continue;
				}
			};
			let age = now.saturating_sub(cached.fetched_at);
			if age > max_age_secs && std::fs::remove_file(&path).is_ok() {
				removed += 1;
			}
		}
		Ok(removed)
	}
}

/// Parse a duration string into seconds. Accepts a positive integer
/// followed by one of `s`, `m`, `h`, `d` (e.g. `"30s"`, `"5m"`, `"2h"`,
/// `"7d"`). A bare integer is treated as seconds.
///
/// Used by `cache clear-http-meta --max-age <duration>` so users can
/// pick the threshold without depending on a separate duration crate.
pub fn parse_duration_secs(input: &str) -> Result<u64, String> {
	let trimmed = input.trim();
	if trimmed.is_empty() {
		return Err("duration cannot be empty".to_string());
	}
	let (number_part, multiplier) =
		match trimmed.chars().last().expect("non-empty checked above") {
			's' => (&trimmed[..trimmed.len() - 1], 1u64),
			'm' => (&trimmed[..trimmed.len() - 1], 60),
			'h' => (&trimmed[..trimmed.len() - 1], 60 * 60),
			'd' => (&trimmed[..trimmed.len() - 1], 60 * 60 * 24),
			c if c.is_ascii_digit() => (trimmed, 1),
			_ => {
				return Err(format!(
					"unknown duration suffix in '{trimmed}'; expected s, m, h, or d"
				));
			}
		};
	let n: u64 = number_part
		.parse()
		.map_err(|_| format!("invalid duration number in '{trimmed}'"))?;
	n.checked_mul(multiplier).ok_or_else(|| {
		format!("duration '{trimmed}' overflows when converted to seconds")
	})
}

fn epoch_secs() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

/// Conditional-GET implementation shared by [`crate::api::ApiClient::fetch_json_cached`]
/// and the cache's own integration tests.
///
/// Decoupling this from the global [`HttpMetaCache::shared`] lets tests
/// inject a cache rooted at a temporary directory without contaminating
/// the developer's `~/.cache/yammm` state.
pub async fn conditional_fetch_json<C, T>(
	client: &C,
	cache: &HttpMetaCache,
	url: &str,
	headers: Vec<(&'static str, String)>,
) -> Result<T, crate::api::ApiError>
where
	C: crate::api::ApiClient + ?Sized,
	T: serde::de::DeserializeOwned,
{
	use crate::api::ApiError;

	// Bypass: skip cache entirely (lookup AND store). The caller still
	// pays the full network roundtrip — this is the escape hatch for
	// debugging upstream cache misbehaviour or testing without
	// snapshot contamination.
	if is_bypassed() {
		return client.fetch_json(url, headers).await;
	}

	let cached = cache.lookup(url).filter(|c| !c.is_stale());

	let mut req_headers = headers;
	if let Some(c) = cached.as_ref() {
		if let Some(etag) = &c.etag {
			req_headers.push(("If-None-Match", etag.clone()));
		}
		if let Some(lm) = &c.last_modified {
			req_headers.push(("If-Modified-Since", lm.clone()));
		}
	}

	let response = client.send_retried(url, req_headers).await?;
	let status = response.status().as_u16();

	if status == 304 {
		if let Some(c) = cached {
			tracing::debug!(url = %url, "http meta cache hit (304)");
			return serde_json::from_str(&c.body).map_err(Into::into);
		}
		tracing::warn!(
			url = %url,
			"server returned 304 but no cached body was available"
		);
		return Err(ApiError::http(
			304,
			"304 Not Modified without cached body".to_string(),
		));
	}

	let response = <C as crate::api::ApiClient>::ensure_success(response)?;
	let etag = response
		.headers()
		.get(reqwest::header::ETAG)
		.and_then(|v| v.to_str().ok())
		.map(String::from);
	let last_modified = response
		.headers()
		.get(reqwest::header::LAST_MODIFIED)
		.and_then(|v| v.to_str().ok())
		.map(String::from);

	let body = response.text().await.map_err(ApiError::from)?;
	if etag.is_some() || last_modified.is_some() {
		cache.store(url, &body, etag, last_modified);
	}
	serde_json::from_str(&body).map_err(Into::into)
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // deliberate: bypass_lock serializes
// tests that touch the global BYPASS flag. The lock is held only across
// mockito's localhost roundtrips — no deadlock risk in practice.
mod tests {
	use super::*;
	use tempfile::TempDir;

	fn cache() -> (TempDir, HttpMetaCache) {
		let tmp = TempDir::new().unwrap();
		let cache = HttpMetaCache::new(tmp.path().to_path_buf());
		(tmp, cache)
	}

	/// Tests that depend on the process-global `BYPASS` flag must lock
	/// this mutex so they don't race each other or with tests that
	/// implicitly assume bypass is off.
	fn bypass_lock() -> std::sync::MutexGuard<'static, ()> {
		use std::sync::Mutex;
		static M: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
		M.get_or_init(|| Mutex::new(()))
			.lock()
			.unwrap_or_else(|e| e.into_inner())
	}

	#[test]
	fn store_then_lookup_round_trip() {
		let (_tmp, cache) = cache();
		cache.store(
			"https://example.com/a",
			"{\"name\":\"a\"}",
			Some("W/\"abc\"".to_string()),
			None,
		);
		let got = cache.lookup("https://example.com/a").unwrap();
		assert_eq!(got.url, "https://example.com/a");
		assert_eq!(got.body, "{\"name\":\"a\"}");
		assert_eq!(got.etag.as_deref(), Some("W/\"abc\""));
	}

	#[test]
	fn lookup_returns_none_when_url_mismatches_stored_record() {
		// Forge a file with a different stored URL to simulate a hash
		// prefix collision; the loader must reject it.
		let (_tmp, cache) = cache();
		std::fs::create_dir_all(cache.root()).unwrap();
		let path = cache.path_for("https://example.com/a");
		let bogus = serde_json::to_string(&CachedResponse {
			url: "https://example.com/different".to_string(),
			body: "{}".to_string(),
			etag: None,
			last_modified: None,
			fetched_at: epoch_secs(),
		})
		.unwrap();
		std::fs::write(&path, bogus).unwrap();

		assert!(cache.lookup("https://example.com/a").is_none());
	}

	#[test]
	fn store_skips_when_no_validators_present() {
		let (_tmp, cache) = cache();
		cache.store("https://example.com/a", "{}", None, None);
		assert!(cache.lookup("https://example.com/a").is_none());
	}

	#[test]
	fn store_skips_oversized_bodies() {
		let (_tmp, cache) = cache();
		let big = "x".repeat(MAX_BODY_BYTES + 1);
		cache.store(
			"https://example.com/a",
			&big,
			Some("etag".to_string()),
			None,
		);
		assert!(cache.lookup("https://example.com/a").is_none());
	}

	#[test]
	fn stale_entries_are_detected() {
		let entry = CachedResponse {
			url: "u".to_string(),
			body: "b".to_string(),
			etag: None,
			last_modified: None,
			fetched_at: 0, // far in the past
		};
		assert!(entry.is_stale());

		let fresh = CachedResponse {
			fetched_at: epoch_secs(),
			..entry
		};
		assert!(!fresh.is_stale());
	}

	#[test]
	fn distinct_urls_get_distinct_files() {
		let (_tmp, cache) = cache();
		let a = cache.path_for("https://example.com/a");
		let b = cache.path_for("https://example.com/b");
		assert_ne!(a, b);
	}

	#[test]
	fn stats_round_trip_count_and_bytes() {
		let (_tmp, cache) = cache();
		// Empty cache: zeros.
		let s = cache.stats().unwrap();
		assert_eq!(s.count, 0);
		assert_eq!(s.total_bytes, 0);

		cache.store(
			"https://example.com/a",
			"{\"x\":1}",
			Some("etag-a".to_string()),
			None,
		);
		cache.store(
			"https://example.com/b",
			"{\"y\":2}",
			Some("etag-b".to_string()),
			None,
		);

		let s = cache.stats().unwrap();
		assert_eq!(s.count, 2);
		assert!(s.total_bytes > 0);
	}

	#[test]
	fn eviction_caps_directory_at_max_entries() {
		// Sanity-check size-cap LRU eviction. We can't write 2000+ files
		// per-test cheaply, so we plant files by hand with controlled
		// `fetched_at` values to exercise the sort + remove path.
		let (_tmp, cache) = cache();
		std::fs::create_dir_all(&cache.root).unwrap();

		// Seed MAX_ENTRIES + 5 entries, with the *oldest* timestamps
		// going to the first ones written so they should be evicted.
		let overflow = 5;
		for i in 0..(MAX_ENTRIES + overflow) {
			let url = format!("https://example.com/entry/{}", i);
			let entry = CachedResponse {
				url: url.clone(),
				body: "{}".to_string(),
				etag: Some(format!("e-{}", i)),
				last_modified: None,
				// Oldest first so they get evicted; everything else stays.
				fetched_at: i as u64,
			};
			let path = cache.path_for(&url);
			std::fs::write(&path, serde_json::to_string(&entry).unwrap())
				.unwrap();
		}

		let stats = cache.stats().unwrap();
		assert_eq!(stats.count, MAX_ENTRIES + overflow);

		let removed = cache.evict_if_over_cap().unwrap();
		assert_eq!(removed, overflow);

		let after = cache.stats().unwrap();
		assert_eq!(after.count, MAX_ENTRIES);

		// The five oldest entries (indices 0..5) should be the ones gone.
		for i in 0..overflow {
			let url = format!("https://example.com/entry/{}", i);
			assert!(
				cache.lookup(&url).is_none(),
				"entry {} should have been evicted",
				i
			);
		}
		// And anything newer should still be present.
		let url = format!("https://example.com/entry/{}", overflow + 10);
		assert!(
			cache.lookup(&url).is_some(),
			"newer entry should survive eviction"
		);
	}

	#[test]
	fn parse_duration_secs_accepts_unit_suffixes() {
		assert_eq!(parse_duration_secs("30s").unwrap(), 30);
		assert_eq!(parse_duration_secs("5m").unwrap(), 5 * 60);
		assert_eq!(parse_duration_secs("2h").unwrap(), 2 * 60 * 60);
		assert_eq!(parse_duration_secs("7d").unwrap(), 7 * 60 * 60 * 24);
		// Bare integer is treated as seconds.
		assert_eq!(parse_duration_secs("90").unwrap(), 90);
		// Whitespace tolerated.
		assert_eq!(parse_duration_secs("  3h  ").unwrap(), 3 * 60 * 60);
	}

	#[test]
	fn parse_duration_secs_rejects_garbage() {
		assert!(parse_duration_secs("").is_err());
		assert!(parse_duration_secs("abc").is_err());
		assert!(parse_duration_secs("5x").is_err());
		assert!(parse_duration_secs("-3s").is_err());
		// Overflow guard. u64::MAX seconds is ~5.85e11 years; multiplying
		// a >1e16 day count by 86400 overflows the multiplier.
		assert!(parse_duration_secs("99999999999999999d").is_err());
	}

	#[test]
	fn clear_older_than_respects_custom_threshold() {
		let (_tmp, cache) = cache();
		std::fs::create_dir_all(&cache.root).unwrap();
		let now = epoch_secs();

		// Three entries: 1h, 2h, 3h old.
		for (i, age_secs) in [3600, 7200, 10800].iter().enumerate() {
			let url = format!("https://example.com/age-{}", i);
			let entry = CachedResponse {
				url: url.clone(),
				body: "{}".to_string(),
				etag: Some(format!("e-{i}")),
				last_modified: None,
				fetched_at: now.saturating_sub(*age_secs),
			};
			let path = cache.path_for(&url);
			std::fs::write(&path, serde_json::to_string(&entry).unwrap())
				.unwrap();
		}

		// 90-minute threshold removes the 2h and 3h entries but keeps
		// the 1h one.
		let removed = cache.clear_older_than(90 * 60).unwrap();
		assert_eq!(removed, 2);
		assert!(cache.lookup("https://example.com/age-0").is_some());
		assert!(cache.lookup("https://example.com/age-1").is_none());
		assert!(cache.lookup("https://example.com/age-2").is_none());
	}

	#[test]
	fn eviction_no_op_when_under_cap() {
		let (_tmp, cache) = cache();
		cache.store(
			"https://example.com/single",
			"{}",
			Some("e".to_string()),
			None,
		);
		let removed = cache.evict_if_over_cap().unwrap();
		assert_eq!(removed, 0);
		assert_eq!(cache.stats().unwrap().count, 1);
	}

	#[tokio::test]
	async fn conditional_fetch_stores_then_uses_304() {
		let _lock = bypass_lock();
		// End-to-end: first call gets 200 + ETag; second call (with the
		// same cache) sends If-None-Match and receives 304, returning the
		// cached body without re-deserializing from the wire.
		let mut server = mockito::Server::new_async().await;
		let etag = "W/\"v1\"";
		let body = serde_json::json!({"slug": "sodium", "name": "Sodium"});

		let mock_first = server
			.mock("GET", "/project/sodium")
			.match_header("if-none-match", mockito::Matcher::Missing)
			.with_status(200)
			.with_header("etag", etag)
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.expect(1)
			.create_async()
			.await;

		let mock_second = server
			.mock("GET", "/project/sodium")
			.match_header("if-none-match", etag)
			.with_status(304)
			.expect(1)
			.create_async()
			.await;

		let (_tmp, cache) = cache();
		let client =
			crate::api::ModrinthClient::new().with_base_url(server.url());
		let url = format!("{}/project/sodium", server.url());

		#[derive(serde::Deserialize, Debug, PartialEq)]
		struct Payload {
			slug: String,
			name: String,
		}

		let first: Payload =
			conditional_fetch_json(&client, &cache, &url, Vec::new())
				.await
				.expect("first fetch should populate cache");
		assert_eq!(first.slug, "sodium");
		assert_eq!(first.name, "Sodium");

		let second: Payload =
			conditional_fetch_json(&client, &cache, &url, Vec::new())
				.await
				.expect(
					"second fetch should hit 304 and return cached payload",
				);
		assert_eq!(first, second);

		mock_first.assert_async().await;
		mock_second.assert_async().await;
	}

	#[tokio::test]
	async fn bypass_flag_skips_lookup_and_store() {
		let _lock = bypass_lock();
		// With bypass on, the server sees a plain GET (no If-None-Match)
		// and the cache is never written.
		let mut server = mockito::Server::new_async().await;
		let body = serde_json::json!({"slug": "iris", "name": "Iris"});

		let mock = server
			.mock("GET", "/project/iris")
			.match_header("if-none-match", mockito::Matcher::Missing)
			.with_status(200)
			.with_header("etag", "W/\"v9\"")
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.expect(2)
			.create_async()
			.await;

		let (_tmp, cache) = cache();
		let client =
			crate::api::ModrinthClient::new().with_base_url(server.url());
		let url = format!("{}/project/iris", server.url());

		// We only need a type for `conditional_fetch_json` to deserialize
		// into; the test cares about the side effects (or lack thereof),
		// not the payload contents.
		#[derive(serde::Deserialize)]
		struct Payload {
			#[allow(dead_code)]
			slug: String,
		}

		// First call: bypass on — nothing cached.
		let prev = is_bypassed();
		set_bypass(true);
		let _: Payload =
			conditional_fetch_json(&client, &cache, &url, Vec::new())
				.await
				.unwrap();
		assert!(
			cache.lookup(&url).is_none(),
			"bypass must not write to the cache"
		);

		// Second call: still bypassed. The mock expects 2 raw GETs and
		// no `If-None-Match` headers — verifies we genuinely skipped
		// the cache rather than just suppressing the write.
		let _: Payload =
			conditional_fetch_json(&client, &cache, &url, Vec::new())
				.await
				.unwrap();
		set_bypass(prev);

		mock.assert_async().await;
	}

	#[tokio::test]
	async fn conditional_fetch_skips_validators_when_cache_stale() {
		let _lock = bypass_lock();
		// A stale cache entry must not be used: the request goes out
		// without If-None-Match, and the server's fresh 200 wins.
		let mut server = mockito::Server::new_async().await;
		let body = serde_json::json!({"slug": "lithium", "name": "Lithium"});

		let mock = server
			.mock("GET", "/project/lithium")
			.match_header("if-none-match", mockito::Matcher::Missing)
			.with_status(200)
			.with_header("etag", "W/\"v2\"")
			.with_header("content-type", "application/json")
			.with_body(body.to_string())
			.expect(1)
			.create_async()
			.await;

		let (_tmp, cache) = cache();
		// Plant a stale entry that *would* match without the age guard.
		let url = format!("{}/project/lithium", server.url());
		let stale = CachedResponse {
			url: url.clone(),
			body: "{\"slug\":\"old\",\"name\":\"old\"}".to_string(),
			etag: Some("W/\"old\"".to_string()),
			last_modified: None,
			fetched_at: 0, // way past MAX_AGE_SECS
		};
		std::fs::create_dir_all(cache.root()).unwrap();
		let path = cache.path_for(&url);
		std::fs::write(&path, serde_json::to_string(&stale).unwrap()).unwrap();

		#[derive(serde::Deserialize)]
		struct Payload {
			slug: String,
		}

		let client =
			crate::api::ModrinthClient::new().with_base_url(server.url());
		let payload: Payload =
			conditional_fetch_json(&client, &cache, &url, Vec::new())
				.await
				.expect("stale cache must not be used");
		assert_eq!(payload.slug, "lithium");

		mock.assert_async().await;
	}

	#[test]
	fn clear_stale_removes_aged_entries_only() {
		let (_tmp, cache) = cache();
		std::fs::create_dir_all(cache.root()).unwrap();

		let fresh_url = "https://example.com/fresh";
		let stale_url = "https://example.com/stale";
		// Hand-write both files so we control `fetched_at` precisely —
		// the public `store` API stamps with now().
		let fresh = CachedResponse {
			url: fresh_url.to_string(),
			body: "{}".to_string(),
			etag: Some("e".to_string()),
			last_modified: None,
			fetched_at: epoch_secs(),
		};
		let stale = CachedResponse {
			url: stale_url.to_string(),
			body: "{}".to_string(),
			etag: Some("e".to_string()),
			last_modified: None,
			fetched_at: 0,
		};
		std::fs::write(
			cache.path_for(fresh_url),
			serde_json::to_string(&fresh).unwrap(),
		)
		.unwrap();
		std::fs::write(
			cache.path_for(stale_url),
			serde_json::to_string(&stale).unwrap(),
		)
		.unwrap();

		let removed = cache.clear_stale().unwrap();
		assert_eq!(removed, 1, "exactly one stale entry should be pruned");
		assert!(cache.path_for(fresh_url).exists());
		assert!(!cache.path_for(stale_url).exists());
	}

	#[test]
	fn clear_stale_drops_unparseable_files() {
		// Corrupt files get treated as junk and removed alongside stale
		// entries so the directory cannot accumulate garbage.
		let (_tmp, cache) = cache();
		std::fs::create_dir_all(cache.root()).unwrap();
		std::fs::write(
			cache.root().join("junk.json"),
			"not even close to JSON",
		)
		.unwrap();

		let removed = cache.clear_stale().unwrap();
		assert_eq!(removed, 1);
		assert!(!cache.root().join("junk.json").exists());
	}

	#[test]
	fn clear_stale_on_missing_root_is_noop() {
		// Calling on a never-created cache must return cleanly so the
		// CLI command doesn't error before the cache has been used.
		let tmp = TempDir::new().unwrap();
		let cache = HttpMetaCache::new(tmp.path().join("does-not-exist"));
		assert_eq!(cache.clear_stale().unwrap(), 0);
	}
}
