use serde::{Deserialize, Serialize};

use super::dependency::Dependency;
use super::env::ModEnv;
use super::info::{ModInfo, ModVersion};
use super::project_type::ProjectType;
use super::source::ModSource;
use crate::types::HashType;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedMod {
	pub id: String,
	pub name: String,
	pub description: String,
	pub version: String,
	pub source: ModSource,
	#[serde(default)]
	pub dependencies: Vec<Dependency>,
	pub url: String,
	pub download_url: String,
	#[serde(default)]
	pub hash: Option<String>,
	#[serde(default)]
	pub hash_type: HashType,
	#[serde(default)]
	pub project_type: ProjectType,
	#[serde(default)]
	pub env: ModEnv,
	#[serde(default)]
	pub categories: Vec<String>,
	#[serde(default)]
	pub filename: Option<String>,
	#[serde(default)]
	pub unresolved: bool,
	#[serde(default)]
	pub connector_compat: bool,
}

impl TrackedMod {
	pub fn builder(
		id: impl Into<String>,
		source: ModSource,
	) -> TrackedModBuilder {
		TrackedModBuilder {
			id: id.into(),
			name: String::new(),
			description: String::new(),
			version: String::new(),
			source,
			url: String::new(),
			download_url: String::new(),
			hash: None,
			hash_type: HashType::default(),
			project_type: ProjectType::default(),
			env: ModEnv::default(),
			categories: Vec::new(),
			filename: None,
			unresolved: false,
			connector_compat: false,
		}
	}

	pub fn from_mod_info(
		mod_info: &ModInfo,
		version: &ModVersion,
		slug: impl Into<String>,
		project_type: ProjectType,
		env: ModEnv,
	) -> Self {
		Self {
			id: slug.into(),
			name: mod_info.name.clone(),
			description: mod_info.description.clone(),
			version: version.version.clone(),
			source: mod_info.source.clone(),
			dependencies: Vec::new(),
			url: mod_info.url.clone(),
			download_url: version.download_url.clone(),
			hash: version.hash.clone(),
			hash_type: version.hash_type,
			project_type,
			env,
			categories: Vec::new(),
			filename: None,
			unresolved: false,
			connector_compat: false,
		}
	}
}

#[must_use = "call .build() to construct the TrackedMod"]
pub struct TrackedModBuilder {
	id: String,
	name: String,
	description: String,
	version: String,
	source: ModSource,
	url: String,
	download_url: String,
	hash: Option<String>,
	hash_type: HashType,
	project_type: ProjectType,
	env: ModEnv,
	categories: Vec<String>,
	filename: Option<String>,
	unresolved: bool,
	connector_compat: bool,
}

impl TrackedModBuilder {
	pub fn name(
		mut self,
		name: impl Into<String>,
	) -> Self {
		self.name = name.into();
		self
	}

	pub fn description(
		mut self,
		description: impl Into<String>,
	) -> Self {
		self.description = description.into();
		self
	}

	pub fn version(
		mut self,
		version: impl Into<String>,
	) -> Self {
		self.version = version.into();
		self
	}

	pub fn url(
		mut self,
		url: impl Into<String>,
	) -> Self {
		self.url = url.into();
		self
	}

	pub fn download_url(
		mut self,
		url: impl Into<String>,
	) -> Self {
		self.download_url = url.into();
		self
	}

	pub fn hash(
		mut self,
		hash: Option<String>,
	) -> Self {
		self.hash = hash;
		self
	}

	pub fn hash_type(
		mut self,
		hash_type: HashType,
	) -> Self {
		self.hash_type = hash_type;
		self
	}

	pub fn project_type(
		mut self,
		project_type: ProjectType,
	) -> Self {
		self.project_type = project_type;
		self
	}

	pub fn env(
		mut self,
		env: ModEnv,
	) -> Self {
		self.env = env;
		self
	}

	pub fn categories(
		mut self,
		categories: Vec<String>,
	) -> Self {
		self.categories = categories;
		self
	}

	pub fn filename(
		mut self,
		filename: impl Into<Option<String>>,
	) -> Self {
		self.filename = filename.into();
		self
	}

	pub fn unresolved(
		mut self,
		unresolved: bool,
	) -> Self {
		self.unresolved = unresolved;
		self
	}

	pub fn connector_compat(
		mut self,
		connector_compat: bool,
	) -> Self {
		self.connector_compat = connector_compat;
		self
	}

	pub fn build(self) -> TrackedMod {
		TrackedMod {
			id: self.id,
			name: self.name,
			description: self.description,
			version: self.version,
			source: self.source,
			dependencies: Vec::new(),
			url: self.url,
			download_url: self.download_url,
			hash: self.hash,
			hash_type: self.hash_type,
			project_type: self.project_type,
			env: self.env,
			categories: self.categories,
			filename: self.filename,
			unresolved: self.unresolved,
			connector_compat: self.connector_compat,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::HashType;

	#[test]
	fn test_tracked_mod_builder() {
		let mod_ron = TrackedMod::builder("jei", ModSource::modrinth("jei"))
			.name("Just Enough Items")
			.description("JEI mod")
			.version("1.0.0")
			.url("https://modrinth.com/mod/jei")
			.download_url("https://cdn.modrinth.com/xxx.jar")
			.hash(Some("a".repeat(128)))
			.hash_type(HashType::Sha512)
			.project_type(ProjectType::Mod)
			.build();
		assert_eq!(mod_ron.id, "jei");
		assert_eq!(mod_ron.name, "Just Enough Items");
		assert!(mod_ron.dependencies.is_empty());
		assert!(mod_ron.categories.is_empty());
	}

	#[test]
	fn test_tracked_mod_builder_with_categories() {
		let mod_ron =
			TrackedMod::builder("sodium", ModSource::modrinth("sodium"))
				.name("Sodium")
				.version("0.6.0")
				.categories(vec![
					"optimization".to_string(),
					"performance".to_string(),
				])
				.build();
		assert_eq!(mod_ron.categories.len(), 2);
		assert_eq!(mod_ron.categories[0], "optimization");
	}
}
