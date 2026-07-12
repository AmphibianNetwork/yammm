//! Unified cache management across all cache subdirectories.
//!
//! ```text
//! {cache_root}/
//!   jars/       ← JarCache (hash-based content-addressed storage)
//!   minecraft/  ← MC version JARs, libraries, assets
//!   loaders/    ← Fabric/Quilt/Forge/NeoForge libraries
//! ```

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::cache::JarCache;

/// Statistics for a single cache subdirectory.
#[derive(Debug, Clone)]
pub struct DirStats {
	#[allow(dead_code)] // populated for display, not currently shown
	pub path: PathBuf,
	pub file_count: usize,
	pub total_size: u64,
}

/// Breakdown of cache usage across the three subdirectories.
#[derive(Debug, Clone)]
pub struct CacheStatus {
	pub jars: DirStats,
	pub minecraft: DirStats,
	pub loaders: DirStats,
}

impl CacheStatus {
	pub fn total_files(&self) -> usize {
		self.jars.file_count
			+ self.minecraft.file_count
			+ self.loaders.file_count
	}

	pub fn total_size(&self) -> u64 {
		self.jars.total_size
			+ self.minecraft.total_size
			+ self.loaders.total_size
	}
}

#[derive(Debug, Clone)]
pub struct CacheManager {
	cache_root: PathBuf,
	jar_cache: JarCache,
}

impl CacheManager {
	pub fn new(cache_root: PathBuf) -> Self {
		let jar_cache = JarCache::new(cache_root.join("jars"));
		Self {
			cache_root,
			jar_cache,
		}
	}

	pub fn init(&self) -> Result<()> {
		self.jar_cache.init()?;
		Ok(())
	}

	pub fn cache_root(&self) -> &Path {
		&self.cache_root
	}

	#[allow(dead_code)] // exposes the inner cache for callers outside `cache` operations
	pub fn jar_cache(&self) -> &JarCache {
		&self.jar_cache
	}

	/// Gather file counts and sizes for each cache subdirectory.
	pub fn status(&self) -> Result<CacheStatus> {
		let jars_count = self.jar_cache.count()?;
		let jars_size = self.jar_cache.size()?;

		let mc_dir = self.cache_root.join("minecraft");
		let (mc_count, mc_size) = dir_stats_recursive(&mc_dir);

		let loaders_dir = self.cache_root.join("loaders");
		let (loaders_count, loaders_size) = dir_stats_recursive(&loaders_dir);

		Ok(CacheStatus {
			jars: DirStats {
				path: self.cache_root.join("jars"),
				file_count: jars_count,
				total_size: jars_size,
			},
			minecraft: DirStats {
				path: mc_dir,
				file_count: mc_count,
				total_size: mc_size,
			},
			loaders: DirStats {
				path: loaders_dir,
				file_count: loaders_count,
				total_size: loaders_size,
			},
		})
	}

	/// Evict cached files until total size is at or below `max_size_bytes`.
	///
	/// Priority: JARs (individual) → Minecraft versions (whole dir) → Loaders (whole dir).
	pub fn clean(
		&self,
		max_size_bytes: u64,
	) -> Result<u64> {
		let status = self.status()?;
		let current_size = status.total_size();
		if current_size <= max_size_bytes {
			return Ok(0);
		}

		let mut remaining = current_size;
		let non_jar_size =
			status.minecraft.total_size + status.loaders.total_size;
		let jar_budget = max_size_bytes.saturating_sub(non_jar_size);

		let removed_jars = self.jar_cache.cleanup(jar_budget)?;
		remaining -= removed_jars;
		if remaining <= max_size_bytes {
			return Ok(removed_jars);
		}

		let removed_mc = self.clean_version_dirs(
			"minecraft",
			&mut remaining,
			max_size_bytes,
		)?;
		if remaining <= max_size_bytes {
			return Ok(removed_jars + removed_mc);
		}

		let removed_loaders =
			self.clean_version_dirs("loaders", &mut remaining, max_size_bytes)?;

		Ok(removed_jars + removed_mc + removed_loaders)
	}

	/// Evict whole version directories at a time.
	/// Directories sorted by newest file's mtime — least-recently-modified removed first.
	/// Uses mtime instead of atime because atime is unreliable (noatime mounts).
	fn clean_version_dirs(
		&self,
		subdir: &str,
		remaining: &mut u64,
		max_size_bytes: u64,
	) -> Result<u64> {
		let base = self.cache_root.join(subdir);
		if !base.exists() {
			return Ok(0);
		}

		let mut version_dirs: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
		for entry in fs::read_dir(&base).with_context(|| {
			format!("Failed to read {} cache directory", subdir)
		})? {
			let entry = entry.with_context(|| {
				format!("Failed to read {} cache entry", subdir)
			})?;
			let path = entry.path();
			if path.is_dir() {
				let (count, size) = dir_stats_recursive(&path);
				if count == 0 {
					continue;
				}
				let newest_mtime =
					newest_modified_time(&path).unwrap_or(UNIX_EPOCH);
				version_dirs.push((path, size, newest_mtime));
			}
		}

		// Sort by newest mtime ascending — evict LRU versions first
		version_dirs.sort_by_key(|a| a.2);

		let mut removed = 0u64;
		for (path, size, _) in version_dirs {
			if *remaining <= max_size_bytes {
				break;
			}
			if fs::remove_dir_all(&path).is_ok() {
				removed += size;
				*remaining -= size;
			}
		}

		Ok(removed)
	}

	/// Delete the entire cache directory tree.
	pub fn obliterate(&self) -> Result<()> {
		if self.cache_root.exists() {
			fs::remove_dir_all(&self.cache_root)
				.context("Failed to obliterate cache")?;
		}
		Ok(())
	}
}

fn dir_stats_recursive(dir: &Path) -> (usize, u64) {
	if !dir.exists() {
		return (0, 0);
	}
	let mut count = 0usize;
	let mut size = 0u64;
	if let Ok(entries) = fs::read_dir(dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_file() {
				count += 1;
				if let Ok(meta) = entry.metadata() {
					size += meta.len();
				}
			} else if path.is_dir() {
				let (c, s) = dir_stats_recursive(&path);
				count += c;
				size += s;
			}
		}
	}
	(count, size)
}

fn newest_modified_time(dir: &Path) -> Option<SystemTime> {
	let mut newest = None;
	if let Ok(entries) = fs::read_dir(dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_file() {
				if let Ok(meta) = entry.metadata() {
					let mtime = meta.modified().ok()?;
					newest = Some(newest.unwrap_or(mtime).max(mtime));
				}
			} else if path.is_dir()
				&& let Some(sub_mtime) = newest_modified_time(&path)
			{
				newest = Some(newest.unwrap_or(sub_mtime).max(sub_mtime));
			}
		}
	}
	newest
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::TempDir;

	#[test]
	fn test_cache_manager_init() {
		let temp_dir = TempDir::new().unwrap();
		let mgr = CacheManager::new(temp_dir.path().to_path_buf());
		mgr.init().unwrap();
		assert!(temp_dir.path().join("jars").exists());
	}

	#[test]
	fn test_cache_manager_status_empty() {
		let temp_dir = TempDir::new().unwrap();
		let mgr = CacheManager::new(temp_dir.path().to_path_buf());
		mgr.init().unwrap();

		let status = mgr.status().unwrap();
		assert_eq!(status.jars.file_count, 0);
		assert_eq!(status.minecraft.file_count, 0);
		assert_eq!(status.loaders.file_count, 0);
		assert_eq!(status.total_files(), 0);
		assert_eq!(status.total_size(), 0);
	}

	#[test]
	fn test_cache_manager_status_with_files() {
		let temp_dir = TempDir::new().unwrap();
		let mgr = CacheManager::new(temp_dir.path().to_path_buf());
		mgr.init().unwrap();

		let mc_dir = temp_dir.path().join("minecraft").join("1.21.1");
		fs::create_dir_all(&mc_dir).unwrap();
		fs::write(mc_dir.join("client.jar"), "fake jar").unwrap();

		let loader_dir = temp_dir
			.path()
			.join("loaders")
			.join("fabric")
			.join("1.21.1")
			.join("0.15.0");
		fs::create_dir_all(&loader_dir).unwrap();
		fs::write(loader_dir.join("fabric-loader-0.15.0.jar"), "fake").unwrap();

		let status = mgr.status().unwrap();
		assert_eq!(status.jars.file_count, 0);
		assert_eq!(status.minecraft.file_count, 1);
		assert_eq!(status.loaders.file_count, 1);
		assert!(status.total_size() > 0);
	}

	#[test]
	fn test_cache_manager_clean_version_dirs() {
		let temp_dir = TempDir::new().unwrap();
		let mgr = CacheManager::new(temp_dir.path().to_path_buf());
		mgr.init().unwrap();

		let mc_120 = temp_dir.path().join("minecraft").join("1.20.4");
		fs::create_dir_all(&mc_120).unwrap();
		fs::write(mc_120.join("client.jar"), "a".repeat(1000)).unwrap();

		let mc_121 = temp_dir.path().join("minecraft").join("1.21.1");
		fs::create_dir_all(&mc_121).unwrap();
		fs::write(mc_121.join("client.jar"), "b".repeat(1000)).unwrap();

		let status = mgr.status().unwrap();
		assert_eq!(status.minecraft.file_count, 2);

		let removed = mgr.clean(1).unwrap();
		assert!(removed > 0);

		let status_after = mgr.status().unwrap();
		assert!(
			status_after.total_size() <= 1
				|| status_after.minecraft.file_count < 2
		);
	}

	#[test]
	fn test_cache_manager_obliterate() {
		let temp_dir = TempDir::new().unwrap();
		let mgr = CacheManager::new(temp_dir.path().to_path_buf());
		mgr.init().unwrap();

		let mc_dir = temp_dir.path().join("minecraft").join("1.21.1");
		fs::create_dir_all(&mc_dir).unwrap();
		fs::write(mc_dir.join("client.jar"), "fake").unwrap();

		mgr.obliterate().unwrap();
		assert!(!temp_dir.path().exists());
	}

	#[test]
	fn test_dir_stats_recursive() {
		let temp_dir = TempDir::new().unwrap();

		fs::create_dir_all(temp_dir.path().join("a/b")).unwrap();
		fs::write(temp_dir.path().join("a/file1.txt"), "hello").unwrap();
		fs::write(temp_dir.path().join("a/b/file2.txt"), "world").unwrap();

		let (count, size) = dir_stats_recursive(temp_dir.path());
		assert_eq!(count, 2);
		assert!(size > 0);
	}
}
