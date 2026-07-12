use crate::config::ModpackManifest;
use crate::output;
use crate::types::{HashType, ModEnv, ProjectType, TrackedMod};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
	pub format_version: i32,
	pub game: String,
	pub version_id: String,
	pub name: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub summary: Option<String>,
	pub files: Vec<MrpackFile>,
	pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
	pub path: String,
	pub hashes: MrpackHashes,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub env: Option<MrpackEnv>,
	pub downloads: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub file_size: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub loaders: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackHashes {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sha1: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sha256: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sha512: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackEnv {
	pub client: String,
	pub server: String,
}

impl MrpackIndex {
	pub fn from_modpack(
		modpack: &ModpackManifest,
		storage: &crate::storage::Storage,
		cache: &crate::storage::JarCache,
	) -> anyhow::Result<Self> {
		let mut dependencies = HashMap::new();
		let minecraft_version = if modpack.minecraft_version.is_empty() {
			return Err(crate::errors::YammmError::config_error(
				"Minecraft version is not set. Run `yammm config` or set it in modpack.toml before exporting.",
			).into());
		} else {
			modpack.minecraft_version.clone()
		};
		dependencies.insert("minecraft".to_string(), minecraft_version);

		let loader_type = modpack.loader.loader_or_default();
		let loader_key = format!("{}-loader", loader_type);
		if modpack.loader.version.is_empty() {
			return Err(crate::errors::YammmError::config_error(
				"Loader version is not set. Run `yammm config` or set it in modpack.toml before exporting.",
			).into());
		}
		let loader_version = modpack.loader.version.clone();
		dependencies.insert(loader_key, loader_version);

		let mut files = Vec::new();
		for (project_type, _, ext) in ProjectType::EXPORT_ENTRIES {
			let items = storage.list(*project_type)?;
			let dir_prefix = mrpack_dir_prefix(*project_type);
			files.extend(collect_mrpack_files(&items, cache, dir_prefix, ext));
		}

		Ok(Self {
			format_version: 1,
			game: "minecraft".to_string(),
			version_id: if modpack.version.is_empty() {
				"1.0.0".to_string()
			} else {
				modpack.version.clone()
			},
			name: modpack.name.clone(),
			summary: if modpack.description.is_empty() {
				None
			} else {
				Some(modpack.description.clone())
			},
			files,
			dependencies,
		})
	}
}

fn mrpack_dir_prefix(project_type: ProjectType) -> &'static str {
	match project_type {
		ProjectType::Mod => "mods",
		ProjectType::ResourcePack => "resourcepacks",
		ProjectType::Shader => "shaderpacks",
	}
}

/// Convert a list of mods/resource packs/shader packs into mrpack file entries.
fn collect_mrpack_files(
	items: &[TrackedMod],
	cache: &crate::storage::JarCache,
	dir_prefix: &str,
	ext: &str,
) -> Vec<MrpackFile> {
	items
		.iter()
		.map(|m| {
			let path = match &m.filename {
				Some(fname) => format!("{}/{}", dir_prefix, fname),
				None => {
					let slug = crate::utils::slugify(&m.name);
					format!("{}/{}{}", dir_prefix, slug, ext)
				}
			};
			let file_size = m
				.hash
				.as_ref()
				.map(|h| get_jar_file_size(cache, m.hash_type, h));

			MrpackFile {
				path,
				hashes: build_mrpack_hashes(m, cache),
				env: Some(mod_env_to_mrpack(&m.env)),
				downloads: if m.download_url.is_empty() {
					Vec::new()
				} else {
					vec![m.download_url.clone()]
				},
				file_size,
				loaders: if m.connector_compat {
					Some(vec!["fabric".to_string()])
				} else {
					None
				},
			}
		})
		.collect()
}

fn build_mrpack_hashes(
	m: &TrackedMod,
	cache: &crate::storage::JarCache,
) -> MrpackHashes {
	let sha1 = if m.hash_type == HashType::Sha1 {
		m.hash.clone()
	} else {
		m.hash.as_ref().and_then(|h| {
			let path = cache.get(m.hash_type, h)?;
			HashType::Sha1
				.compute_for_file(&path)
				.map_err(|e| {
					tracing::warn!(
						"Failed to compute SHA1 for {}: {e}",
						path.display()
					);
					e
				})
				.ok()
		})
	};

	match m.hash_type {
		HashType::Sha1 => MrpackHashes {
			sha1: m.hash.clone(),
			sha256: None,
			sha512: None,
		},
		HashType::Sha256 => MrpackHashes {
			sha1,
			sha256: m.hash.clone(),
			sha512: None,
		},
		HashType::Sha512 => MrpackHashes {
			sha1,
			sha256: None,
			sha512: m.hash.clone(),
		},
		HashType::Md5 => MrpackHashes {
			sha1,
			sha256: None,
			sha512: None,
		},
	}
}

/// Copy cached JARs for a project type into the temp staging directory.
fn copy_project_files_to_dir(
	items: &[TrackedMod],
	cache: &crate::storage::JarCache,
	dest_dir: &Path,
	ext: &str,
) -> anyhow::Result<()> {
	for item in items {
		if let Some(jar_path) = item
			.hash
			.as_ref()
			.and_then(|h| cache.get(item.hash_type, h))
		{
			let dest_name = match &item.filename {
				Some(fname) => fname.clone(),
				None => format!("{}{}", crate::utils::slugify(&item.name), ext),
			};
			let dest_path = dest_dir.join(&dest_name);
			fs::copy(&jar_path, &dest_path)?;
			output::bullet(format!("Copied: {}", item.name));
		}
	}
	Ok(())
}

fn get_jar_file_size(
	cache: &crate::storage::JarCache,
	hash_type: crate::types::HashType,
	hash: &str,
) -> u64 {
	cache
		.get(hash_type, hash)
		.and_then(|p| std::fs::metadata(p).ok())
		.map(|m| m.len())
		.unwrap_or(0)
}

fn mod_env_to_mrpack(env: &ModEnv) -> MrpackEnv {
	match env {
		ModEnv::Both => MrpackEnv {
			client: "required".to_string(),
			server: "required".to_string(),
		},
		ModEnv::Client => MrpackEnv {
			client: "required".to_string(),
			server: "unsupported".to_string(),
		},
		ModEnv::Server => MrpackEnv {
			client: "unsupported".to_string(),
			server: "required".to_string(),
		},
	}
}

pub fn mrpack_env_to_mod_env(env: &MrpackEnv) -> ModEnv {
	match (env.client.as_str(), env.server.as_str()) {
		("required", "required") => ModEnv::Both,
		("required", "unsupported") | ("required", "optional") => {
			ModEnv::Client
		}
		("unsupported", "required") | ("optional", "required") => {
			ModEnv::Server
		}
		_ => ModEnv::Both,
	}
}

pub fn export_to_mrpack(
	modpack: &ModpackManifest,
	storage: &crate::storage::Storage,
	cache: &crate::storage::JarCache,
	root_dir: &Path,
	output_path: &Path,
) -> anyhow::Result<()> {
	let temp_dir = std::env::temp_dir()
		.join(format!("mrpack_export_{}", std::process::id()));
	fs::create_dir_all(&temp_dir)?;

	let index = MrpackIndex::from_modpack(modpack, storage, cache)?;
	let json_content = serde_json::to_string_pretty(&index)?;
	let index_path = temp_dir.join("modrinth.index.json");
	fs::write(&index_path, json_content)?;

	output::bullet("Created modrinth.index.json");

	for (project_type, _, ext) in ProjectType::EXPORT_ENTRIES {
		let items = storage.list(*project_type)?;
		let dir = temp_dir.join(mrpack_dir_prefix(*project_type));
		fs::create_dir_all(&dir)?;
		copy_project_files_to_dir(&items, cache, &dir, ext)?;
	}

	let config_dir = root_dir.join("config");
	if config_dir.exists() {
		let overrides_dir = temp_dir.join("overrides").join("config");
		fs::create_dir_all(&overrides_dir)?;
		copy_dir_recursive(&config_dir, &overrides_dir)?;
		output::bullet("Copied: config/ (overrides)");
	}

	let final_path = if output_path.extension().is_some() {
		output_path.to_path_buf()
	} else {
		output_path.with_extension("mrpack")
	};

	create_archive(&temp_dir, &final_path)?;

	fs::remove_dir_all(&temp_dir)?;

	Ok(())
}

fn create_archive(
	source_dir: &Path,
	output_path: &Path,
) -> anyhow::Result<()> {
	use zip::write::SimpleFileOptions;

	let file = fs::File::create(output_path)?;
	let mut zip = zip::ZipWriter::new(file);

	let index_path = source_dir.join("modrinth.index.json");
	if index_path.exists() {
		let content = fs::read(&index_path)?;
		zip.start_file::<_, ()>(
			"modrinth.index.json",
			SimpleFileOptions::default(),
		)?;
		zip.write_all(&content)?;
	}

	for dir_name in &["mods", "resourcepacks", "shaderpacks", "overrides"] {
		let dir = source_dir.join(dir_name);
		if dir.exists() {
			super::add_dir_to_zip(&mut zip, &dir, dir_name)?;
		}
	}

	zip.finish()?;

	Ok(())
}

fn copy_dir_recursive(
	src: &Path,
	dst: &Path,
) -> anyhow::Result<()> {
	fs::create_dir_all(dst)?;
	for entry in fs::read_dir(src)? {
		let entry = entry?;
		let src_path = entry.path();
		let dst_path = dst.join(entry.file_name());
		if src_path.is_dir() {
			copy_dir_recursive(&src_path, &dst_path)?;
		} else {
			fs::copy(&src_path, &dst_path)?;
		}
	}
	Ok(())
}

#[cfg(test)]
mod golden_tests {
	use super::*;
	use crate::config::LoaderConfig;
	use crate::storage::{JarCache, Storage};
	use crate::types::{
		HashType, LoaderType, ModEnv, ModSource, ProjectType, TrackedMod,
	};

	// Golden file for the MRPACK `modrinth.index.json` shape. If you change the
	// `MrpackIndex` schema or how `from_modpack` populates it, update this
	// string. The test exists to catch *accidental* drift — Modrinth's spec is
	// loose enough that two valid implementations can still be mutually
	// incompatible.
	// Key order is alphabetical because we canonicalize through
	// `serde_json::Value` (which uses a BTreeMap). `fileSize: 0` reflects
	// the real exporter behavior when the cache has no entry for the mod
	// — file_size falls back to 0 — and is captured here so accidental
	// changes to that fallback surface in CI.
	const EXPECTED_INDEX_JSON: &str = r#"{
  "dependencies": {
    "fabric-loader": "0.16.5",
    "minecraft": "1.20.4"
  },
  "files": [
    {
      "downloads": [
        "https://example.com/sodium.jar"
      ],
      "env": {
        "client": "required",
        "server": "unsupported"
      },
      "fileSize": 0,
      "hashes": {
        "sha1": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      "path": "mods/sodium.jar"
    }
  ],
  "formatVersion": 1,
  "game": "minecraft",
  "name": "Golden Pack",
  "summary": "Fixture for golden test",
  "versionId": "1.0.0"
}"#;

	/// Build a TrackedMod that looks like what `add` would produce after
	/// resolving a Modrinth mod, downloading the JAR, and writing the entry.
	fn fixture_tracked_mod() -> TrackedMod {
		TrackedMod {
			id: "sodium".to_string(),
			name: "Sodium".to_string(),
			description: String::new(),
			version: "0.5.0".to_string(),
			source: ModSource::modrinth("sodium"),
			dependencies: Vec::new(),
			url: String::new(),
			download_url: "https://example.com/sodium.jar".to_string(),
			hash: Some("a".repeat(40)),
			hash_type: HashType::Sha1,
			project_type: ProjectType::Mod,
			env: ModEnv::Client,
			categories: Vec::new(),
			filename: Some("sodium.jar".to_string()),
			unresolved: false,
			connector_compat: false,
		}
	}

	fn fixture_manifest() -> ModpackManifest {
		ModpackManifest {
			name: "Golden Pack".to_string(),
			description: "Fixture for golden test".to_string(),
			version: "1.0.0".to_string(),
			minecraft_version: "1.20.4".to_string(),
			loader: LoaderConfig {
				loader: Some(LoaderType::Fabric),
				version: "0.16.5".to_string(),
			},
			mod_path: None,
			resource_pack_path: None,
			shader_pack_path: None,
		}
	}

	#[test]
	fn mrpack_index_matches_golden_for_fixture_modpack() {
		// Build a fully in-memory modpack: one Fabric mod for MC 1.20.4 with a
		// known SHA-1 hash, client-only, no resource packs or shaders.
		let tmp = tempfile::TempDir::new().unwrap();
		let root = tmp.path();

		let manifest = fixture_manifest();
		let storage = Storage::new(root, &manifest);
		std::fs::create_dir_all(&storage.mods_dir).unwrap();
		std::fs::create_dir_all(&storage.resourcepacks_dir).unwrap();
		std::fs::create_dir_all(&storage.shaderpacks_dir).unwrap();

		let tracked = fixture_tracked_mod();
		storage.save(ProjectType::Mod, "sodium", &tracked).unwrap();

		let cache = JarCache::new(root.join("cache"));

		let index =
			MrpackIndex::from_modpack(&manifest, &storage, &cache).unwrap();

		// Canonicalize: serde_json::Value emits maps in sorted key order, so
		// going through Value before `to_string_pretty` neutralizes HashMap
		// iteration order in the `dependencies` field.
		let value: serde_json::Value = serde_json::to_value(&index).unwrap();
		let actual = serde_json::to_string_pretty(&value).unwrap();

		assert_eq!(
			actual, EXPECTED_INDEX_JSON,
			"MRPACK index drifted from golden — update EXPECTED_INDEX_JSON if intentional"
		);
	}

	/// Full export → re-parse roundtrip. Catches "we wrote a field that we
	/// can't read back" and "the zip layout drifted from what import expects."
	///
	/// We don't drive the `add` or `import` commands directly because both
	/// involve network calls (provider lookups, JAR downloads) that would
	/// need a mockito server and a real cached JAR to make the export step
	/// succeed end-to-end. The path we *do* cover is the lossless one: the
	/// `TrackedMod` that `add` would persist, after `export`, after pulling
	/// the index back out of the zip, still parses into an `MrpackIndex`
	/// with the same shape — which is precisely what the importer reads.
	#[test]
	fn export_to_mrpack_roundtrips_through_reparse() {
		use std::io::Read;

		let tmp = tempfile::TempDir::new().unwrap();
		let root = tmp.path();

		let manifest = fixture_manifest();
		let storage = Storage::new(root, &manifest);
		std::fs::create_dir_all(&storage.mods_dir).unwrap();
		std::fs::create_dir_all(&storage.resourcepacks_dir).unwrap();
		std::fs::create_dir_all(&storage.shaderpacks_dir).unwrap();

		let tracked = fixture_tracked_mod();
		storage.save(ProjectType::Mod, "sodium", &tracked).unwrap();

		// Cache contains the JAR bytes keyed by sha1, so the exporter has
		// something to copy into the zip's mods/ directory.
		let cache_dir = root.join("cache");
		std::fs::create_dir_all(&cache_dir).unwrap();
		let jar_bytes = b"PK\x03\x04 fake jar bytes for roundtrip test";
		let sha1 = HashType::Sha1.compute_for_bytes(jar_bytes);
		let jar_path = cache_dir.join(format!("sha1_{}.jar", sha1));
		std::fs::write(&jar_path, jar_bytes).unwrap();

		// Re-save the tracked mod with the cache-aligned hash so the exporter
		// can find the JAR. Everything else from the fixture is preserved.
		let mut tracked = tracked;
		tracked.hash = Some(sha1.clone());
		storage.save(ProjectType::Mod, "sodium", &tracked).unwrap();

		let cache = JarCache::new(cache_dir);

		let output_path = root.join("out.mrpack");
		export_to_mrpack(&manifest, &storage, &cache, root, &output_path)
			.expect("export should succeed");

		// Read the produced zip and pull modrinth.index.json back out.
		let file = std::fs::File::open(&output_path).unwrap();
		let mut archive = zip::ZipArchive::new(file).unwrap();

		let mut index_bytes = Vec::new();
		archive
			.by_name("modrinth.index.json")
			.expect("zip must contain modrinth.index.json")
			.read_to_end(&mut index_bytes)
			.unwrap();
		let reparsed: MrpackIndex = serde_json::from_slice(&index_bytes)
			.expect("index must parse back into MrpackIndex");

		assert_eq!(reparsed.name, "Golden Pack");
		assert_eq!(reparsed.version_id, "1.0.0");
		assert_eq!(
			reparsed.dependencies.get("minecraft").map(String::as_str),
			Some("1.20.4")
		);
		assert_eq!(
			reparsed
				.dependencies
				.get("fabric-loader")
				.map(String::as_str),
			Some("0.16.5"),
		);
		assert_eq!(reparsed.files.len(), 1);

		let file = &reparsed.files[0];
		assert_eq!(file.path, "mods/sodium.jar");
		assert_eq!(file.hashes.sha1.as_deref(), Some(sha1.as_str()));
		assert_eq!(
			file.downloads,
			vec!["https://example.com/sodium.jar".to_string()],
		);
		let env = file.env.as_ref().expect("env must roundtrip");
		assert_eq!(env.client, "required");
		assert_eq!(env.server, "unsupported");

		// And the importer's env-conversion function agrees on the inverse.
		assert_eq!(mrpack_env_to_mod_env(env), ModEnv::Client);

		// The mods/ directory should contain the JAR bytes we staged.
		let mut jar_in_zip = Vec::new();
		archive
			.by_name("mods/sodium.jar")
			.expect("zip must contain the staged JAR")
			.read_to_end(&mut jar_in_zip)
			.unwrap();
		assert_eq!(jar_in_zip, jar_bytes);
	}
}
