use serde::{Deserialize, Serialize};

use super::project_type::ProjectType;
use super::source::ModSource;
use crate::types::HashType;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModInfo {
	pub id: String,
	pub name: String,
	pub description: String,
	pub source: ModSource,
	pub minecraft_versions: Vec<String>,
	#[serde(default)]
	pub loaders: Vec<String>,
	#[serde(default)]
	pub downloads: u64,
	pub url: String,
	#[serde(default)]
	pub project_type: Option<ProjectType>,
	#[serde(default)]
	pub client_side: Option<String>,
	#[serde(default)]
	pub server_side: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModVersion {
	pub version_id: Option<String>,
	pub version: String,
	pub minecraft_versions: Vec<String>,
	pub loaders: Vec<String>,
	pub download_url: String,
	pub hash: Option<String>,
	#[serde(default)]
	pub hash_type: HashType,
	pub file_size: u64,
	pub release_date: String,
}
