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
