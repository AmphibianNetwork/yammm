//! Content-addressed cache for mod JAR files.
//!
//! JARs stored as `{hash_type}_{hash}.jar` — duplicate downloads are
//! automatically deduplicated, and lookup is O(1).
//!
//! A `cache_manifest.json` file tracks last-access timestamps for LRU
//! eviction, since filesystem atime is unreliable (noatime mounts, etc.).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MANIFEST_FILE: &str = "cache_manifest.json";

/// Debounce window: how long the writer thread waits after a flush signal
/// before actually writing, coalescing any further signals that arrive in
/// the interim. 200ms is short enough to feel "live" for interactive use
/// and long enough to absorb a burst of cache touches from a parallel
/// download batch.
const FLUSH_DEBOUNCE: Duration = Duration::from_millis(200);

/// Maximum time `sync()` will wait for the writer to confirm a flush
/// landed before giving up. The writer log-warns on failure, so a timeout
/// here means the I/O genuinely wedged.
const SYNC_TIMEOUT: Duration = Duration::from_secs(5);

/// In-memory representation of the LRU manifest.
///
/// `dirty` tracks whether any mutation has happened since the last successful
/// save, so that `save()` becomes a cheap no-op when called from the flush path
/// after a batch of touches that produced no net change. Persisting JSON on
/// every cache touch made batch installs do O(N) fsyncs; now touches stay in
/// memory and we flush on explicit boundaries (eviction, user commands) and
/// when the last `JarCache` handle drops.
#[derive(Debug, Default, Deserialize)]
struct CacheManifest {
	/// `Arc<BTreeMap>` so the writer thread can snapshot in O(1) (refcount
	/// bump) instead of cloning the whole map. Mutations go through
	/// `Arc::make_mut`, which only deep-copies when a snapshot is still
	/// outstanding — i.e., during the brief window where the writer is
	/// mid-serialize. Serialization is done by [`CacheManifestRef`] from a
	/// snapshotted `Arc` to keep the writer's lock window minimal.
	#[serde(deserialize_with = "deserialize_arc_btreemap")]
	entries: Arc<BTreeMap<String, u64>>,
	#[serde(skip)]
	dirty: bool,
}

fn deserialize_arc_btreemap<'de, D>(
	deserializer: D
) -> Result<Arc<BTreeMap<String, u64>>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	BTreeMap::<String, u64>::deserialize(deserializer).map(Arc::new)
}

impl CacheManifest {
	fn load(dir: &Path) -> Self {
		let path = dir.join(MANIFEST_FILE);
		if path.exists()
			&& let Ok(data) = fs::read_to_string(&path)
			&& let Ok(mut manifest) = serde_json::from_str::<Self>(&data)
		{
			manifest.dirty = false;
			return manifest;
		}
		Self::default()
	}

	fn touch(
		&mut self,
		key: &str,
	) {
		let now = epoch_secs();
		Arc::make_mut(&mut self.entries).insert(key.to_string(), now);
		self.dirty = true;
	}

	fn remove(
		&mut self,
		key: &str,
	) {
		if Arc::make_mut(&mut self.entries).remove(key).is_some() {
			self.dirty = true;
		}
	}

	fn prune_missing(
		&mut self,
		dir: &Path,
	) {
		let before = self.entries.len();
		Arc::make_mut(&mut self.entries).retain(|key, _| {
			let path = dir.join(format!("{}.jar", key));
			path.exists()
		});
		if self.entries.len() != before {
			self.dirty = true;
		}
	}
}

fn epoch_secs() -> u64 {
	use std::time::{SystemTime, UNIX_EPOCH};
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

/// Borrowed view of [`CacheManifest`] used by the writer thread to serialize
/// without cloning the full `entries` map. Mirrors the on-disk schema of
/// `CacheManifest` (which has `#[serde(skip)]` on `dirty`).
#[derive(Serialize)]
struct CacheManifestRef<'a> {
	entries: &'a BTreeMap<String, u64>,
}

/// Messages routed from `JarCache` clones to the single writer thread.
enum WriterMsg {
	/// A touch has marked the manifest dirty. The writer debounces these
	/// and eventually writes once; the actual write happens *outside* the
	/// manifest lock so concurrent touches don't block on disk I/O.
	Flush,
	/// Caller wants confirmation that the next write has landed.
	/// The writer responds on `ack` once it has tried to persist.
	Sync(mpsc::Sender<()>),
	/// Drain remaining dirty state, then exit.
	Shutdown,
}

/// Owns the writer thread that serializes all manifest persistence.
///
/// The thread is dedicated (one per `JarCache` lineage), so writes are
/// naturally serialized — no two threads ever race on the `cache_manifest.json.tmp`
/// rename. It also debounces touches: a burst of cache touches during a
/// parallel download produces *one* write at the end, not many.
struct ManifestWriter {
	tx: mpsc::Sender<WriterMsg>,
	handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl ManifestWriter {
	fn spawn(
		manifest: Arc<Mutex<CacheManifest>>,
		cache_dir: PathBuf,
	) -> Self {
		let (tx, rx) = mpsc::channel::<WriterMsg>();
		let handle = thread::Builder::new()
			.name("yammm-cache-writer".to_string())
			.spawn(move || writer_loop(rx, manifest, cache_dir))
			.expect("failed to spawn cache manifest writer thread");
		Self {
			tx,
			handle: Mutex::new(Some(handle)),
		}
	}

	/// Non-blocking: signal that the manifest has dirty in-memory state.
	/// Multiple rapid signals are coalesced into a single eventual write.
	fn signal_dirty(&self) {
		// Errors here only happen if the writer thread has already exited
		// (e.g., during shutdown). The lost signal is recoverable: the next
		// touch will set the dirty bit again, and any caller that needs a
		// real durability guarantee uses `sync()` instead.
		let _ = self.tx.send(WriterMsg::Flush);
	}

	/// Synchronously wait for the manifest to be persisted to disk. Used
	/// at eager-flush boundaries (eviction, `remove`) and at program exit.
	fn sync(&self) {
		let (ack_tx, ack_rx) = mpsc::channel();
		if self.tx.send(WriterMsg::Sync(ack_tx)).is_err() {
			// Writer already shut down — nothing to wait on.
			return;
		}
		match ack_rx.recv_timeout(SYNC_TIMEOUT) {
			Ok(()) => {}
			Err(_) => {
				tracing::warn!(
					"Cache manifest sync timed out after {:?}",
					SYNC_TIMEOUT
				);
			}
		}
	}

	/// Drain remaining dirty state and shut down the writer thread.
	/// Joins the thread so callers can observe completion before exit.
	fn shutdown(&self) {
		if self.tx.send(WriterMsg::Shutdown).is_err() {
			return;
		}
		if let Some(handle) =
			self.handle.lock().unwrap_or_else(|e| e.into_inner()).take()
		{
			let _ = handle.join();
		}
	}
}

impl std::fmt::Debug for ManifestWriter {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		f.debug_struct("ManifestWriter").finish_non_exhaustive()
	}
}

fn writer_loop(
	rx: mpsc::Receiver<WriterMsg>,
	manifest: Arc<Mutex<CacheManifest>>,
	cache_dir: PathBuf,
) {
	loop {
		let first = match rx.recv() {
			Ok(m) => m,
			Err(_) => return,
		};

		let mut pending_acks: Vec<mpsc::Sender<()>> = Vec::new();
		let mut shutdown = false;

		match first {
			WriterMsg::Flush => {
				// Debounce: absorb additional signals during the window so
				// a burst of touches produces one write.
				let deadline = Instant::now() + FLUSH_DEBOUNCE;
				loop {
					let remaining =
						deadline.saturating_duration_since(Instant::now());
					if remaining.is_zero() {
						break;
					}
					match rx.recv_timeout(remaining) {
						Ok(WriterMsg::Flush) => continue,
						Ok(WriterMsg::Sync(ack)) => {
							pending_acks.push(ack);
							// A sync request collapses the debounce window —
							// the caller is waiting, so write immediately.
							break;
						}
						Ok(WriterMsg::Shutdown) => {
							shutdown = true;
							break;
						}
						Err(_) => break,
					}
				}
			}
			WriterMsg::Sync(ack) => pending_acks.push(ack),
			WriterMsg::Shutdown => shutdown = true,
		}

		// Snapshot under the lock; serialize and write outside it. With
		// `Arc<BTreeMap>` the snapshot is an O(1) refcount bump — no full-map
		// clone, and the lock window collapses to a couple of pointer writes.
		// Concurrent touches that arrive during serialize/write trigger a
		// copy-on-write via `Arc::make_mut`, so they neither block nor
		// corrupt the snapshot being persisted.
		let snapshot = {
			let mut m = manifest.lock().unwrap_or_else(|e| e.into_inner());
			if !m.dirty {
				None
			} else {
				// Optimistically mark clean. If the write fails, we re-mark
				// dirty below so the next signal retries.
				m.dirty = false;
				Some(Arc::clone(&m.entries))
			}
		};

		if let Some(entries) = snapshot {
			let path = cache_dir.join(MANIFEST_FILE);
			let result = serde_json::to_string_pretty(&CacheManifestRef {
				entries: &entries,
			})
			.context("Failed to serialize cache manifest")
			.and_then(|json| {
				crate::utils::fs::atomic_write_bytes(
					&path,
					json.as_bytes(),
					crate::utils::fs::AtomicWriteOptions::default(),
				)
				.context("Failed to write cache manifest")
			});

			if let Err(e) = result {
				tracing::warn!("Failed to persist cache manifest: {}", e);
				let mut m = manifest.lock().unwrap_or_else(|e| e.into_inner());
				m.dirty = true;
			}
		}

		for ack in pending_acks {
			let _ = ack.send(());
		}

		if shutdown {
			return;
		}
	}
}

/// Content-addressed cache for mod JAR files.
///
/// Path: `~/.cache/yammm/jars/sha512_abc123.jar`
#[derive(Debug)]
pub struct JarCache {
	cache_dir: PathBuf,
	manifest: Arc<Mutex<CacheManifest>>,
	writer: Arc<ManifestWriter>,
}

impl Clone for JarCache {
	fn clone(&self) -> Self {
		Self {
			cache_dir: self.cache_dir.clone(),
			manifest: Arc::clone(&self.manifest),
			writer: Arc::clone(&self.writer),
		}
	}
}

impl JarCache {
	pub fn new(cache_dir: PathBuf) -> Self {
		let manifest = Arc::new(Mutex::new(CacheManifest::load(&cache_dir)));
		let writer = Arc::new(ManifestWriter::spawn(
			Arc::clone(&manifest),
			cache_dir.clone(),
		));
		Self {
			cache_dir,
			manifest,
			writer,
		}
	}

	pub fn with_default() -> Self {
		let cache_dir = Self::default_cache_dir();
		Self::new(cache_dir)
	}

	fn default_cache_dir() -> PathBuf {
		dirs::cache_dir()
			.map(|dir| dir.join("yammm").join("jars"))
			.unwrap_or_else(|| PathBuf::from("./.cache/yammm/jars"))
	}

	pub fn init(&self) -> Result<()> {
		fs::create_dir_all(&self.cache_dir)
			.context("Failed to create JAR cache directory")
	}

	pub fn cache_dir(&self) -> &Path {
		&self.cache_dir
	}

	/// Synchronously persist the LRU manifest. Used by eager-flush sites
	/// (eviction, `remove`) where the caller has changed user-visible state
	/// and must not observe a stale manifest after the call returns.
	///
	/// Cheap no-op if the manifest is clean. Errors are logged but not
	/// propagated — a stale manifest only degrades eviction quality.
	pub fn flush(&self) {
		self.writer.sync();
	}

	fn touch_in_memory(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) {
		let key = format!("{}_{}", hash_type.as_str(), hash);
		{
			let mut manifest =
				self.manifest.lock().unwrap_or_else(|e| e.into_inner());
			manifest.touch(&key);
		}
		// Signal the background writer that we have dirty state. The actual
		// write happens off this thread, after the debounce window.
		self.writer.signal_dirty();
	}

	/// Look up a cached JAR by hash. Returns `Some(path)` if it exists.
	/// Records access time in the manifest for LRU eviction (kept in memory;
	/// flushed lazily on user commands or when the last `JarCache` handle drops).
	pub fn get(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) -> Option<PathBuf> {
		let path = self.jar_path(hash_type, hash);
		if path.exists() {
			self.touch_in_memory(hash_type, hash);
			Some(path)
		} else {
			None
		}
	}

	/// Record a cache hit without returning the path. Use this from hot read
	/// paths (e.g. download fast-path) that have already constructed the path
	/// themselves but want to keep the LRU honest.
	pub fn mark_used(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) {
		self.touch_in_memory(hash_type, hash);
	}

	/// Return the expected cache path for a hash. Does **not** create the file.
	pub fn path_for(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) -> PathBuf {
		self.jar_path(hash_type, hash)
	}

	/// Store a JAR from a local file path. Re-hashes with SHA-512.
	/// Uses atomic writes (write to `.tmp`, then rename).
	pub fn put<P: AsRef<Path>>(
		&self,
		source: P,
	) -> Result<String> {
		let source = source.as_ref();
		let hash_type = crate::types::HashType::Sha512;
		let hash = hash_type.compute_for_file(source)?;
		let dest = self.jar_path(hash_type, &hash);

		if let Some(parent) = dest.parent() {
			fs::create_dir_all(parent)
				.context("Failed to create cache directory")?;
		}

		if !dest.exists() {
			let tmp = dest.with_extension("tmp");
			fs::copy(source, &tmp).context("Failed to copy JAR to cache")?;
			fs::rename(&tmp, &dest).or_else(|_| {
				if dest.exists() {
					let _ = fs::remove_file(&tmp);
					Ok::<(), anyhow::Error>(())
				} else {
					Err(crate::errors::YammmError::general(
						"Failed to rename temp file to cache destination",
					)
					.into())
				}
			})?;
		}

		// Touch only in memory; flush happens on Drop or explicit `flush()`.
		self.touch_in_memory(hash_type, &hash);

		Ok(hash)
	}

	/// Remove a cached JAR. No-op if it doesn't exist.
	pub fn remove(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) -> Result<()> {
		let path = self.jar_path(hash_type, hash);
		if path.exists() {
			fs::remove_file(&path)
				.context(format!("Failed to remove cached JAR: {}", hash))?;
		}
		let key = format!("{}_{}", hash_type.as_str(), hash);
		{
			let mut manifest =
				self.manifest.lock().unwrap_or_else(|e| e.into_inner());
			manifest.remove(&key);
		}
		// `remove` is a user-visible destructive op — flush immediately
		// so a crash before exit doesn't resurrect a stale LRU entry.
		self.flush();
		Ok(())
	}

	/// Check whether a cached JAR exists.
	pub fn contains(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) -> bool {
		self.jar_path(hash_type, hash).exists()
	}

	pub fn jar_path(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) -> PathBuf {
		self.cache_dir
			.join(format!("{}_{}.jar", hash_type.as_str(), hash))
	}

	/// Check if a path looks like a cached JAR file.
	/// Matches `{hash_prefix}_{hash}.jar` to exclude stray files.
	pub(crate) fn is_jar_file(path: &Path) -> bool {
		let name = match path.file_name().and_then(|n| n.to_str()) {
			Some(n) => n,
			None => return false,
		};
		name.ends_with(".jar")
			&& name
				.split_once('_')
				.map(|(prefix, _)| {
					matches!(prefix, "sha1" | "sha256" | "sha512" | "md5")
				})
				.unwrap_or(false)
	}

	/// Count the number of JAR files in the cache.
	pub fn count(&self) -> Result<usize> {
		if !self.cache_dir.exists() {
			return Ok(0);
		}
		let count = fs::read_dir(&self.cache_dir)
			.context("Failed to read cache directory")?
			.filter(|e| {
				e.as_ref()
					.map(|e| e.path().is_file() && Self::is_jar_file(&e.path()))
					.unwrap_or(false)
			})
			.count();
		Ok(count)
	}

	/// Commit a pre-populated temp file to its final cache location.
	///
	/// The caller is expected to have already streamed the bytes into `tmp_path`
	/// and verified the hash. This method just performs the atomic rename and
	/// updates the LRU manifest. If `dest` already exists (another task won the
	/// race), the tmp file is removed and the existing destination is returned.
	pub fn commit_tmp(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
		tmp_path: &Path,
		dest: &Path,
		name: &str,
	) -> anyhow::Result<PathBuf> {
		if let Some(parent) = dest.parent() {
			fs::create_dir_all(parent)
				.context("Failed to create cache directory")?;
		}
		if dest.exists() {
			let _ = fs::remove_file(tmp_path);
		} else if let Err(e) = fs::rename(tmp_path, dest) {
			// Cleanup our partial file before propagating.
			let _ = fs::remove_file(tmp_path);
			return Err(e).with_context(|| {
				format!("Failed to commit cached JAR: {}", name)
			});
		}
		self.touch_in_memory(hash_type, hash);
		Ok(dest.to_path_buf())
	}

	/// Write raw bytes into the cache using atomic writes (.tmp + rename).
	/// Returns the path and computed hash.
	pub fn write_bytes(
		&self,
		hash_type: crate::types::HashType,
		computed_hash: &str,
		bytes: &[u8],
		name: &str,
	) -> anyhow::Result<(PathBuf, String)> {
		fs::create_dir_all(&self.cache_dir)
			.context("Failed to create cache directory")?;
		let dest = self.jar_path(hash_type, computed_hash);
		if dest.exists() {
			return Ok((dest, computed_hash.to_string()));
		}
		let tmp = dest.with_extension("tmp");
		fs::write(&tmp, bytes)
			.with_context(|| format!("Failed to write cached JAR: {}", name))?;
		fs::rename(&tmp, &dest).or_else(|_| {
			if dest.exists() {
				let _ = fs::remove_file(&tmp);
				Ok::<(), anyhow::Error>(())
			} else {
				Err(crate::errors::YammmError::general(format!(
					"Failed to commit cached JAR: {}",
					name
				))
				.into())
			}
		})?;
		self.touch_in_memory(hash_type, computed_hash);
		Ok((dest, computed_hash.to_string()))
	}

	/// Total size (in bytes) of all cached JAR files.
	pub fn size(&self) -> Result<u64> {
		let mut total = 0u64;
		if self.cache_dir.exists() {
			for entry in fs::read_dir(&self.cache_dir)
				.context("Failed to read cache directory")?
			{
				let entry = entry.context("Failed to read cache entry")?;
				let path = entry.path();
				if path.is_file()
					&& Self::is_jar_file(&path)
					&& let Ok(metadata) = entry.metadata()
				{
					total += metadata.len();
				}
			}
		}
		Ok(total)
	}

	/// Delete all cached JARs and recreate the cache directory.
	pub fn clear(&self) -> Result<()> {
		if self.cache_dir.exists() {
			fs::remove_dir_all(&self.cache_dir)
				.context("Failed to remove cache directory")?;
		}
		self.init()
	}

	/// Evict files until the cache is at or below `max_size_bytes`.
	/// Uses manifest-based LRU (oldest recorded access time removed first).
	/// More reliable than atime, which is often disabled (noatime mounts).
	pub fn cleanup(
		&self,
		max_size_bytes: u64,
	) -> Result<u64> {
		if !self.cache_dir.exists() {
			return Ok(0);
		}

		let current_size = self.size()?;
		if current_size <= max_size_bytes {
			return Ok(0);
		}

		let mut manifest =
			self.manifest.lock().unwrap_or_else(|e| e.into_inner());
		manifest.prune_missing(&self.cache_dir);

		let mut entries: Vec<(String, u64, u64)> = Vec::new();

		for entry in fs::read_dir(&self.cache_dir)
			.context("Failed to read cache directory")?
		{
			let entry = entry.context("Failed to read cache entry")?;
			let path = entry.path();

			if path.is_file()
				&& Self::is_jar_file(&path)
				&& let Ok(metadata) = entry.metadata()
			{
				let file_size = metadata.len();
				let filename =
					path.file_name().and_then(|n| n.to_str()).unwrap_or("");
				let key = filename
					.strip_suffix(".jar")
					.unwrap_or(filename)
					.to_string();
				let last_access =
					manifest.entries.get(&key).copied().unwrap_or(0);
				entries.push((key, file_size, last_access));
			}
		}

		entries.sort_by_key(|a| a.2);

		let mut removed = 0u64;
		let mut remaining = current_size;

		for (key, size, _) in entries {
			if remaining <= max_size_bytes {
				break;
			}
			let path = self.cache_dir.join(format!("{}.jar", key));
			if fs::remove_file(&path).is_ok() {
				manifest.remove(&key);
				removed += size;
				remaining -= size;
			}
		}

		drop(manifest);
		// Eviction is a user-visible destructive op — flush eagerly.
		self.flush();
		Ok(removed)
	}
}

impl Drop for JarCache {
	fn drop(&mut self) {
		// When the last clone of a JarCache goes away, drain the writer.
		// Strong count == 1 on the writer means `self` holds the only
		// remaining reference; nothing else can be racing us. Shutdown
		// blocks until the worker thread has processed the final write,
		// so callers observe a fully persisted manifest at process exit.
		if Arc::strong_count(&self.writer) == 1 {
			self.writer.shutdown();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::HashType;
	use std::fs;
	use tempfile::TempDir;

	fn make_cache() -> (TempDir, JarCache) {
		let temp_dir = TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();
		(temp_dir, cache)
	}

	fn write_fake_jar(
		cache: &JarCache,
		hash_type: HashType,
		hash: &str,
		content: &[u8],
	) {
		cache.write_bytes(hash_type, hash, content, "test").unwrap();
	}

	#[test]
	fn test_jar_cache_init() {
		let temp_dir = TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().to_path_buf());
		cache.init().unwrap();
		assert!(cache.cache_dir.exists());
	}

	#[test]
	fn test_jar_cache_put_get() {
		let temp_dir = TempDir::new().unwrap();

		let test_file = temp_dir.path().join("test.jar");
		fs::write(&test_file, "test content").unwrap();

		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let hash = cache.put(&test_file).unwrap();
		assert!(cache.contains(HashType::Sha512, &hash));
		assert!(cache.get(HashType::Sha512, &hash).is_some());
	}

	#[test]
	fn test_jar_cache_remove() {
		let temp_dir = TempDir::new().unwrap();

		let test_file = temp_dir.path().join("test.jar");
		fs::write(&test_file, "test content").unwrap();

		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let hash = cache.put(&test_file).unwrap();
		assert!(cache.contains(HashType::Sha512, &hash));

		cache.remove(HashType::Sha512, &hash).unwrap();
		assert!(!cache.contains(HashType::Sha512, &hash));
	}

	#[test]
	fn test_jar_cache_get_nonexistent() {
		let temp_dir = TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		assert!(cache.get(HashType::Sha512, "nonexistent").is_none());
		assert!(!cache.contains(HashType::Sha512, "nonexistent"));
	}

	#[test]
	fn test_jar_cache_write_bytes() {
		let (_temp_dir, cache) = make_cache();
		let hash = "a".repeat(128);
		write_fake_jar(&cache, HashType::Sha512, &hash, b"hello world");
		assert!(cache.contains(HashType::Sha512, &hash));
		let path = cache.get(HashType::Sha512, &hash).unwrap();
		let content = fs::read(&path).unwrap();
		assert_eq!(content, b"hello world");
	}

	#[test]
	fn test_jar_cache_count_empty() {
		let (_temp_dir, cache) = make_cache();
		assert_eq!(cache.count().unwrap(), 0);
	}

	#[test]
	fn test_jar_cache_count_with_files() {
		let (_temp_dir, cache) = make_cache();
		write_fake_jar(&cache, HashType::Sha512, &"a".repeat(128), b"a");
		write_fake_jar(&cache, HashType::Sha512, &"b".repeat(128), b"b");
		assert_eq!(cache.count().unwrap(), 2);
	}

	#[test]
	fn test_jar_cache_size_empty() {
		let (_temp_dir, cache) = make_cache();
		assert_eq!(cache.size().unwrap(), 0);
	}

	#[test]
	fn test_jar_cache_size_with_files() {
		let (_temp_dir, cache) = make_cache();
		write_fake_jar(&cache, HashType::Sha512, &"a".repeat(128), b"abc");
		let size = cache.size().unwrap();
		assert!(size > 0);
	}

	#[test]
	fn test_jar_cache_clear() {
		let (_temp_dir, cache) = make_cache();
		write_fake_jar(&cache, HashType::Sha512, &"a".repeat(128), b"data");
		assert_eq!(cache.count().unwrap(), 1);

		cache.clear().unwrap();
		assert_eq!(cache.count().unwrap(), 0);
		assert!(cache.cache_dir.exists());
	}

	#[test]
	fn test_jar_cache_cleanup_evicts_oldest() {
		let (_temp_dir, cache) = make_cache();
		let hash_a = "a".repeat(128);
		let hash_b = "b".repeat(128);
		write_fake_jar(&cache, HashType::Sha512, &hash_a, b"aaa");
		write_fake_jar(&cache, HashType::Sha512, &hash_b, b"bbb");

		let _size_before = cache.size().unwrap();
		assert_eq!(cache.count().unwrap(), 2);

		let removed = cache.cleanup(1).unwrap();
		assert!(removed > 0);
		assert!(cache.count().unwrap() < 2);
	}

	#[test]
	fn test_jar_cache_cleanup_no_eviction_if_under_limit() {
		let (_temp_dir, cache) = make_cache();
		write_fake_jar(&cache, HashType::Sha512, &"a".repeat(128), b"tiny");

		let removed = cache.cleanup(u64::MAX).unwrap();
		assert_eq!(removed, 0);
		assert_eq!(cache.count().unwrap(), 1);
	}

	#[test]
	fn test_jar_cache_remove_nonexistent() {
		let (_temp_dir, cache) = make_cache();
		assert!(cache.remove(HashType::Sha512, "nonexistent").is_ok());
	}

	#[test]
	fn test_jar_cache_path_for() {
		let (_temp_dir, cache) = make_cache();
		let path = cache.path_for(HashType::Sha512, "abc");
		assert!(path.to_string_lossy().contains("sha512_abc"));
		assert!(!path.exists());
	}

	#[test]
	fn test_manifest_persisted_by_flush_and_drop() {
		let temp_dir = TempDir::new().unwrap();
		let cache_dir = temp_dir.path().join("cache");
		let manifest_path = cache_dir.join(MANIFEST_FILE);

		let hash = "a".repeat(128);
		{
			let cache = JarCache::new(cache_dir.clone());
			cache.init().unwrap();
			write_fake_jar(&cache, HashType::Sha512, &hash, b"data");

			// Note: with the background writer, the manifest may or may not
			// be on disk at this point — touches debounce and write
			// asynchronously. Callers that need a durability guarantee call
			// `flush()`, which blocks until the writer has caught up.
			cache.flush();
			assert!(
				manifest_path.exists(),
				"explicit flush should ensure the manifest landed"
			);
		}

		// Drop of the last handle drains the writer; the final state must
		// be on disk by the time the JarCache lineage is gone.
		assert!(manifest_path.exists());
		let contents = fs::read_to_string(&manifest_path).unwrap();
		assert!(contents.contains(&hash));
	}

	#[test]
	fn test_flush_is_no_op_when_clean() {
		let temp_dir = TempDir::new().unwrap();
		let cache = JarCache::new(temp_dir.path().join("cache"));
		cache.init().unwrap();

		let manifest_path = temp_dir.path().join("cache").join(MANIFEST_FILE);
		cache.flush();
		assert!(
			!manifest_path.exists(),
			"flush with no dirty state must not touch the disk"
		);
	}

	#[test]
	fn test_clone_drop_does_not_drain_writer() {
		// Dropping a non-final clone must NOT shut down the writer thread:
		// the original handle is still in use, so the writer must keep
		// running to absorb future touches.
		let temp_dir = TempDir::new().unwrap();
		let cache_dir = temp_dir.path().join("cache");

		let cache = JarCache::new(cache_dir.clone());
		cache.init().unwrap();
		let hash = "b".repeat(128);
		write_fake_jar(&cache, HashType::Sha512, &hash, b"data");

		// Drop a clone — strong count returns to 1 on the writer Arc, but
		// the writer thread itself is owned by the surviving handle's Arc
		// and stays alive.
		drop(cache.clone());

		// The surviving handle can still flush successfully, proving the
		// writer thread didn't shut down when the clone went away.
		write_fake_jar(&cache, HashType::Sha512, &"c".repeat(128), b"more");
		cache.flush();

		let manifest_path = cache_dir.join(MANIFEST_FILE);
		assert!(manifest_path.exists());
		let contents = fs::read_to_string(&manifest_path).unwrap();
		assert!(contents.contains(&hash));
		assert!(contents.contains("c".repeat(128).as_str()));
	}

	#[test]
	fn test_flush_blocks_until_write_lands() {
		// Synchronous `flush()` must guarantee the manifest is on disk by
		// the time it returns. This is the contract eager-flush sites
		// (`remove`, eviction) depend on.
		let temp_dir = TempDir::new().unwrap();
		let cache_dir = temp_dir.path().join("cache");
		let manifest_path = cache_dir.join(MANIFEST_FILE);

		let cache = JarCache::new(cache_dir.clone());
		cache.init().unwrap();
		let hash = "d".repeat(128);
		write_fake_jar(&cache, HashType::Sha512, &hash, b"data");
		cache.flush();

		assert!(
			manifest_path.exists(),
			"manifest must exist immediately after flush returns"
		);
		let contents = fs::read_to_string(&manifest_path).unwrap();
		assert!(contents.contains(&hash));
	}

	#[test]
	fn test_concurrent_touches_all_land_on_flush() {
		// Many threads touching the cache concurrently must not lose any
		// updates — the in-memory BTreeMap is mutex-guarded and the writer
		// snapshots it atomically.
		let temp_dir = TempDir::new().unwrap();
		let cache_dir = temp_dir.path().join("cache");
		let cache = JarCache::new(cache_dir.clone());
		cache.init().unwrap();

		const THREADS: usize = 16;
		const PER_THREAD: usize = 25;

		let mut handles = Vec::with_capacity(THREADS);
		for t in 0..THREADS {
			let c = cache.clone();
			handles.push(std::thread::spawn(move || {
				for i in 0..PER_THREAD {
					let hash = format!("{:0>128}", t * PER_THREAD + i);
					c.write_bytes(HashType::Sha512, &hash, b"x", "concurrent")
						.unwrap();
				}
			}));
		}
		for h in handles {
			h.join().unwrap();
		}

		cache.flush();

		let manifest_path = cache_dir.join(MANIFEST_FILE);
		let contents = fs::read_to_string(&manifest_path).unwrap();
		let parsed: serde_json::Value =
			serde_json::from_str(&contents).unwrap();
		let entries = parsed["entries"].as_object().unwrap();
		assert_eq!(
			entries.len(),
			THREADS * PER_THREAD,
			"every concurrent touch should land in the manifest"
		);
	}

	#[test]
	fn test_writer_debounces_burst_into_one_write() {
		// A burst of touches within the debounce window should produce
		// far fewer writes than touches. We can't directly count writes,
		// but we can verify that the manifest reflects the final state
		// after the burst — the contract callers care about.
		let temp_dir = TempDir::new().unwrap();
		let cache_dir = temp_dir.path().join("cache");
		let cache = JarCache::new(cache_dir.clone());
		cache.init().unwrap();

		for i in 0..50 {
			let hash = format!("{:0>128}", i);
			cache
				.write_bytes(HashType::Sha512, &hash, b"x", "burst")
				.unwrap();
		}
		cache.flush();

		let manifest_path = cache_dir.join(MANIFEST_FILE);
		let contents = fs::read_to_string(&manifest_path).unwrap();
		let parsed: serde_json::Value =
			serde_json::from_str(&contents).unwrap();
		let entries = parsed["entries"].as_object().unwrap();
		assert_eq!(entries.len(), 50);
	}

	#[test]
	fn test_is_jar_file() {
		assert!(JarCache::is_jar_file(Path::new("sha512_abc.jar")));
		assert!(JarCache::is_jar_file(Path::new("sha1_def.jar")));
		assert!(JarCache::is_jar_file(Path::new("md5_ghi.jar")));
		assert!(!JarCache::is_jar_file(Path::new("random.txt")));
		assert!(!JarCache::is_jar_file(Path::new("nohash.jar")));
	}
}
