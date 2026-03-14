//! Content-addressed cache for mod JAR files.
//!
//! JARs stored as `{hash_type}_{hash}.jar` — duplicate downloads are
//! automatically deduplicated, and lookup is O(1).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Content-addressed cache for mod JAR files.
///
/// Path: `~/.cache/yammm/jars/sha512_abc123.jar`
#[derive(Debug, Clone)]
pub struct JarCache {
	cache_dir: PathBuf,
}

impl JarCache {
	pub fn new(cache_dir: PathBuf) -> Self {
		Self { cache_dir }
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

	/// Look up a cached JAR by hash. Returns `Some(path)` if it exists.
	pub fn get(
		&self,
		hash_type: crate::types::HashType,
		hash: &str,
	) -> Option<PathBuf> {
		let path = self.jar_path(hash_type, hash);
		if path.exists() {
			Some(path)
		} else {
			None
		}
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
					Ok(())
				} else {
					Err(anyhow::anyhow!(
						"Failed to rename temp file to cache destination"
					))
				}
			})?;
		}

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

	/// Write raw bytes into the cache, returning the path and computed hash.
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
		fs::write(&dest, bytes)
			.with_context(|| format!("Failed to write cached JAR: {}", name))?;
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
				if path.is_file() && Self::is_jar_file(&path) {
					if let Ok(metadata) = entry.metadata() {
						total += metadata.len();
					}
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
	/// Uses atime-based LRU (oldest access time removed first).
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

		let mut entries: Vec<(PathBuf, u64, SystemTime)> = Vec::new();

		for entry in fs::read_dir(&self.cache_dir)
			.context("Failed to read cache directory")?
		{
			let entry = entry.context("Failed to read cache entry")?;
			let path = entry.path();

			if path.is_file() && Self::is_jar_file(&path) {
				if let Ok(metadata) = entry.metadata() {
					let accessed = metadata.accessed().unwrap_or(UNIX_EPOCH);
					entries.push((path, metadata.len(), accessed));
				}
			}
		}

		entries.sort_by_key(|a| a.2);

		// Remove least-recently-accessed files first
		let mut removed = 0u64;
		let mut remaining = current_size;

		for (path, size, _) in entries {
			if remaining <= max_size_bytes {
				break;
			}
			if fs::remove_file(&path).is_ok() {
				removed += size;
				remaining -= size;
			}
		}

		Ok(removed)
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
	fn test_is_jar_file() {
		assert!(JarCache::is_jar_file(Path::new("sha512_abc.jar")));
		assert!(JarCache::is_jar_file(Path::new("sha1_def.jar")));
		assert!(JarCache::is_jar_file(Path::new("md5_ghi.jar")));
		assert!(!JarCache::is_jar_file(Path::new("random.txt")));
		assert!(!JarCache::is_jar_file(Path::new("nohash.jar")));
	}
}
