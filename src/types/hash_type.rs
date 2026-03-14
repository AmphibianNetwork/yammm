//! Hash types used for JAR integrity verification.
//!
//! Supports SHA-1, SHA-256, SHA-512, and MD5. Each variant can:
//! - Map from CurseForge algorithm IDs (`1` → SHA-1, `2` → MD5)
//! - Parse from human-readable names (`"sha512"`, `"sha-512"`, `"sha2_512"`)
//! - Compute hex-encoded digests for bytes or files
//!
//! The hash type also serves as a **prefix** in cache filenames
//! (e.g. `sha512_abc123.jar`) to avoid collisions between different
//! hash algorithms.

use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::io::Read;
use std::str::FromStr;

#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HashType {
	Sha1,
	Sha256,
	#[default]
	Sha512,
	Md5,
}

impl HashType {
	/// Maps a CurseForge algorithm ID to a `HashType`.
	///
	/// CurseForge uses numeric IDs: 1 = SHA-1, 2 = MD5.
	pub fn from_curseforge_algo(algo: i32) -> Option<Self> {
		// CurseForge algorithm number mapping: 1 → SHA-1, 2 → MD5
		match algo {
			1 => Some(HashType::Sha1),
			2 => Some(HashType::Md5),
			_ => None,
		}
	}

	/// Parses a hash algorithm name (e.g. `"sha256"`, `"sha-512"`, `"md5"`) into a `HashType`.
	fn from_name(s: &str) -> Option<Self> {
		match s.to_lowercase().as_str() {
			"sha1" | "sha-1" => Some(HashType::Sha1),
			"sha256" | "sha-256" | "sha2_256" => Some(HashType::Sha256),
			"sha512" | "sha-512" | "sha2_512" => Some(HashType::Sha512),
			"md5" => Some(HashType::Md5),
			_ => None,
		}
	}

	/// Returns the canonical lowercase name (e.g. `"sha512"`).
	///
	/// Also used as the file/directory prefix for cached JARs.
	pub fn as_str(&self) -> &'static str {
		match self {
			HashType::Sha1 => "sha1",
			HashType::Sha256 => "sha256",
			HashType::Sha512 => "sha512",
			HashType::Md5 => "md5",
		}
	}

	/// Returns the expected length of the hex-encoded digest for this hash type.
	pub fn hex_len(&self) -> usize {
		match self {
			HashType::Sha1 => 40,
			HashType::Sha256 => 64,
			HashType::Sha512 => 128,
			HashType::Md5 => 32,
		}
	}

	/// Computes the hex-encoded digest of the given byte slice.
	pub fn compute_for_bytes(
		&self,
		data: &[u8],
	) -> String {
		match self {
			HashType::Sha1 => compute_bytes_hash::<Sha1>(data),
			HashType::Sha256 => compute_bytes_hash::<Sha256>(data),
			HashType::Sha512 => compute_bytes_hash::<Sha512>(data),
			HashType::Md5 => compute_bytes_hash::<md5::Md5>(data),
		}
	}

	/// Computes the hex-encoded digest of a file, reading in 8 KB chunks.
	pub fn compute_for_file(
		&self,
		path: &std::path::Path,
	) -> anyhow::Result<String> {
		match self {
			HashType::Sha1 => compute_file_hash::<Sha1>(path),
			HashType::Sha256 => compute_file_hash::<Sha256>(path),
			HashType::Sha512 => compute_file_hash::<Sha512>(path),
			HashType::Md5 => compute_file_hash::<md5::Md5>(path),
		}
	}
}

fn compute_bytes_hash<D: Digest>(data: &[u8]) -> String {
	let mut hasher = D::new();
	hasher.update(data);
	hex::encode(hasher.finalize())
}

fn compute_file_hash<D: Digest>(
	path: &std::path::Path
) -> anyhow::Result<String> {
	let mut file = std::fs::File::open(path)?;
	let mut buf = [0u8; 8192];
	let mut hasher = D::new();
	loop {
		let n = file.read(&mut buf)?;
		if n == 0 {
			break;
		}
		hasher.update(&buf[..n]);
	}
	Ok(hex::encode(hasher.finalize()))
}

impl std::fmt::Display for HashType {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

#[derive(Debug, thiserror::Error)]
#[error("Unknown hash type: {0}")]
pub struct HashTypeParseError(pub String);

impl FromStr for HashType {
	type Err = HashTypeParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Self::from_name(s).ok_or_else(|| HashTypeParseError(s.to_string()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_from_name() {
		assert_eq!(HashType::from_name("sha256"), Some(HashType::Sha256));
		assert_eq!(HashType::from_name("sha-256"), Some(HashType::Sha256));
		assert_eq!(HashType::from_name("sha2_256"), Some(HashType::Sha256));
		assert_eq!(HashType::from_name("sha512"), Some(HashType::Sha512));
		assert_eq!(HashType::from_name("sha-512"), Some(HashType::Sha512));
		assert_eq!(HashType::from_name("md5"), Some(HashType::Md5));
		assert_eq!(HashType::from_name("unknown"), None);
	}

	#[test]
	fn test_from_curseforge_algo() {
		assert_eq!(HashType::from_curseforge_algo(1), Some(HashType::Sha1));
		assert_eq!(HashType::from_curseforge_algo(2), Some(HashType::Md5));
		assert_eq!(HashType::from_curseforge_algo(3), None);
	}
}
