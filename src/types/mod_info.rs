//! Mod-related types: sources, metadata, and tracked mod records.

use super::{HashType, VersionReq};
use serde::{Deserialize, Serialize};

/// Identifies where a mod comes from.
///
/// Serialized as `{ type = "modrinth", id = "sodium" }` in RON/TOML.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ModSource {
	Modrinth { id: String },
	CurseForge { project_id: String },
	Url { url: String },
}

/// Unique identity for a mod within a specific source.
///
/// Used as a deduplication key during dependency resolution:
/// `modrinth:sodium` ≠ `curseforge:231093`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModIdentity {
	pub mod_id: String,
	pub source: ModSource,
}

impl std::fmt::Display for ModIdentity {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}:{}", self.source, self.mod_id)
	}
}

impl ModSource {
	pub fn modrinth(id: impl Into<String>) -> Self {
		ModSource::Modrinth { id: id.into() }
	}

	pub fn curseforge(project_id: impl Into<String>) -> Self {
		ModSource::CurseForge {
			project_id: project_id.into(),
		}
	}

	pub fn url(url: impl Into<String>) -> Self {
		ModSource::Url { url: url.into() }
	}

	pub fn as_str(&self) -> &str {
		match self {
			ModSource::Modrinth { .. } => "modrinth",
			ModSource::CurseForge { .. } => "curseforge",
			ModSource::Url { .. } => "url",
		}
	}

	pub fn source_id(&self) -> &str {
		match self {
			ModSource::Modrinth { id } => id,
			ModSource::CurseForge { project_id } => project_id,
			ModSource::Url { url } => url,
		}
	}

	/// Whether this source requires an API client. URL sources resolve locally.
	pub fn requires_api(&self) -> bool {
		matches!(
			self,
			ModSource::Modrinth { .. } | ModSource::CurseForge { .. }
		)
	}

	/// Check if an identifier looks like a URL or file path.
	pub fn is_url_like(id: &str) -> bool {
		id.starts_with("http://")
			|| id.starts_with("https://")
			|| id.starts_with("file://")
	}

	pub fn url_str(&self) -> Option<&str> {
		match self {
			ModSource::Url { url } => Some(url),
			_ => None,
		}
	}
}

/// Mod metadata returned from search queries
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

/// A specific mod version with download metadata
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModVersion {
	/// Source-specific version ID (Modrinth: UUID, CurseForge: file ID)
	pub version_id: Option<String>,
	/// The version string (e.g., "18.5.2")
	pub version: String,
	/// Minecraft version compatibility
	pub minecraft_versions: Vec<String>,
	/// Supported loaders
	pub loaders: Vec<String>,
	/// Download URL
	pub download_url: String,
	/// Hash of the JAR file
	pub hash: Option<String>,
	/// Algorithm used for the hash
	#[serde(default)]
	pub hash_type: HashType,
	/// File size in bytes
	pub file_size: u64,
	/// Release date
	pub release_date: String,
}

/// Persistent mod record stored in `mods/<slug>/mod.ron`.
///
/// Source of truth for what's installed. Created from provider's
/// `ModInfo` + `ModVersion`, dependencies populated after resolution.
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
	pub sha1: Option<String>,
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
			sha1: None,
			project_type: ProjectType::default(),
			env: ModEnv::default(),
			categories: Vec::new(),
			filename: None,
			unresolved: false,
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
			sha1: None,
			project_type,
			env,
			categories: Vec::new(),
			filename: None,
			unresolved: false,
		}
	}
}

/// Mod or pack type classification
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
	#[default]
	Mod,
	ResourcePack,
	Shader,
}

impl ProjectType {
	pub const VARIANTS: &[ProjectType] = &[
		ProjectType::Mod,
		ProjectType::ResourcePack,
		ProjectType::Shader,
	];

	pub const EXPORT_ENTRIES: &[(ProjectType, &str, &str)] = &[
		(ProjectType::Mod, "mods", ".jar"),
		(ProjectType::ResourcePack, "resourcepacks", ".zip"),
		(ProjectType::Shader, "shaderpacks", ".zip"),
	];

	pub fn as_str(&self) -> &'static str {
		match self {
			ProjectType::Mod => "mod",
			ProjectType::ResourcePack => "resourcepack",
			ProjectType::Shader => "shader",
		}
	}
}

impl std::str::FromStr for ProjectType {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"mod" => Ok(ProjectType::Mod),
			"resourcepack" | "resource_pack" => Ok(ProjectType::ResourcePack),
			"shader" | "shaderpack" | "shader_pack" => Ok(ProjectType::Shader),
			other => Err(format!("Unknown project type: {}", other)),
		}
	}
}

impl std::fmt::Display for ProjectType {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

/// Mod environment — specifies where a mod should be present
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ModEnv {
	#[default]
	Both,
	Client,
	Server,
}

impl ModEnv {
	pub fn as_str(&self) -> &'static str {
		match self {
			ModEnv::Both => "both",
			ModEnv::Client => "client",
			ModEnv::Server => "server",
		}
	}
}

impl std::str::FromStr for ModEnv {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"both" => Ok(ModEnv::Both),
			"client" => Ok(ModEnv::Client),
			"server" => Ok(ModEnv::Server),
			other => Err(format!("Unknown env: {}", other)),
		}
	}
}

impl std::fmt::Display for ModEnv {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

/// A dependency on another mod
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dependency {
	pub mod_id: String,
	pub source: ModSource,
	pub kind: DependencyKind,
	#[serde(default)]
	pub version: Option<VersionReq>,
	#[serde(default)]
	pub required_by: Option<String>,
}

/// A dependency returned by a source before normalization.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceDependency {
	pub mod_id: String,
	pub version_id: Option<String>,
	pub dep_type: DependencyKind,
	pub source: Option<ModSource>,
}

impl Dependency {
	pub fn new(
		mod_id: impl Into<String>,
		source: ModSource,
		kind: DependencyKind,
	) -> Self {
		Self {
			mod_id: mod_id.into(),
			source,
			kind,
			version: None,
			required_by: None,
		}
	}

	pub fn with_version(
		mut self,
		version: VersionReq,
	) -> Self {
		self.version = Some(version);
		self
	}

	pub fn with_required_by(
		mut self,
		required_by: impl Into<String>,
	) -> Self {
		self.required_by = Some(required_by.into());
		self
	}
}

/// Dependency kind (matches Modrinth specification).
///
/// `#[non_exhaustive]` allows providers to add new kinds — unknown kinds
/// fail to parse and are skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum DependencyKind {
	Required,
	Optional,
	Incompatible,
	Embedded,
}

impl DependencyKind {
	pub fn as_str(&self) -> &'static str {
		match self {
			DependencyKind::Required => "required",
			DependencyKind::Optional => "optional",
			DependencyKind::Incompatible => "incompatible",
			DependencyKind::Embedded => "embedded",
		}
	}

	pub fn is_required(&self) -> bool {
		matches!(self, DependencyKind::Required)
	}
}

impl std::fmt::Display for DependencyKind {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

/// Parse error for unknown dependency kinds
#[derive(Debug, thiserror::Error)]
#[error("Unknown dependency kind: {0}")]
pub struct DependencyKindError(pub String);

impl std::str::FromStr for DependencyKind {
	type Err = DependencyKindError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"required" => Ok(DependencyKind::Required),
			"optional" => Ok(DependencyKind::Optional),
			"incompatible" => Ok(DependencyKind::Incompatible),
			"embedded" => Ok(DependencyKind::Embedded),
			other => Err(DependencyKindError(other.to_string())),
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
	sha1: Option<String>,
	project_type: ProjectType,
	env: ModEnv,
	categories: Vec<String>,
	filename: Option<String>,
	unresolved: bool,
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

	pub fn sha1(
		mut self,
		sha1: impl Into<Option<String>>,
	) -> Self {
		self.sha1 = sha1.into();
		self
	}

	pub fn unresolved(
		mut self,
		unresolved: bool,
	) -> Self {
		self.unresolved = unresolved;
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
			sha1: self.sha1,
			project_type: self.project_type,
			env: self.env,
			categories: self.categories,
			filename: self.filename,
			unresolved: self.unresolved,
		}
	}
}

/// Parse errors for `ModSource` strings like `"modrinth:sodium"` or `"cf:12345"`.
#[derive(Debug, thiserror::Error)]
pub enum ModSourceParseError {
	#[error("Invalid format: expected 'source:id' (e.g., 'modrinth:jei'), got '{0}'")]
	InvalidFormat(String),

	#[error(
		"Unknown source type: '{0}'. Valid sources: modrinth, curseforge, url"
	)]
	UnknownSource(String),
}

impl std::str::FromStr for ModSource {
	type Err = ModSourceParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if Self::is_url_like(s) {
			return Ok(ModSource::url(s));
		}

		// Split "source:id" format (e.g. "modrinth:sodium", "cf:12345")
		let parts: Vec<&str> = s.splitn(2, ':').collect();
		if parts.len() != 2 {
			return Err(ModSourceParseError::InvalidFormat(s.to_string()));
		}

		let source_type = parts[0].to_lowercase();
		let id = parts[1].to_string();

		Ok(match source_type.as_str() {
			"modrinth" | "mr" => ModSource::modrinth(id),
			"curseforge" | "cf" => ModSource::curseforge(id),
			"url" => ModSource::url(id),
			_ => return Err(ModSourceParseError::UnknownSource(source_type)),
		})
	}
}

impl std::fmt::Display for ModSource {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		match self {
			ModSource::Modrinth { id } => write!(f, "modrinth:{}", id),
			ModSource::CurseForge { project_id } => {
				write!(f, "curseforge:{}", project_id)
			}
			ModSource::Url { url } => write!(f, "url:{}", url),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_mod_source_modrinth() {
		let source = ModSource::modrinth("jei");
		assert_eq!(source.as_str(), "modrinth");
		assert_eq!(source.source_id(), "jei");
	}

	#[test]
	fn test_mod_source_curseforge() {
		let source = ModSource::curseforge("238222");
		assert_eq!(source.as_str(), "curseforge");
		assert_eq!(source.source_id(), "238222");
	}

	#[test]
	fn test_mod_source_url() {
		let source = ModSource::url("https://example.com/mod.jar");
		assert_eq!(source.as_str(), "url");
		assert_eq!(source.source_id(), "https://example.com/mod.jar");
	}

	#[test]
	fn test_mod_source_from_str_roundtrip() {
		let mr = ModSource::modrinth("jei");
		let s = mr.to_string();
		let parsed: ModSource = s.parse().unwrap();
		assert_eq!(mr, parsed);

		let cf = ModSource::curseforge("238222");
		let s = cf.to_string();
		let parsed: ModSource = s.parse().unwrap();
		assert_eq!(cf, parsed);

		let url = ModSource::url("https://example.com/mod.jar");
		let s = url.to_string();
		let parsed: ModSource = s.parse().unwrap();
		assert_eq!(url, parsed);
	}

	#[test]
	fn test_mod_source_url_github() {
		let source = ModSource::url("https://github.com/owner/repo");
		assert_eq!(source.as_str(), "url");
		assert_eq!(source.source_id(), "https://github.com/owner/repo");
	}

	#[test]
	fn test_requires_api() {
		assert!(ModSource::modrinth("x").requires_api());
		assert!(ModSource::curseforge("x").requires_api());
		assert!(!ModSource::url("x").requires_api());
	}

	#[test]
	fn test_from_str_aliases() {
		let mr: ModSource = "mr:jei".parse().unwrap();
		assert_eq!(mr, ModSource::modrinth("jei"));

		let cf: ModSource = "cf:12345".parse().unwrap();
		assert_eq!(cf, ModSource::curseforge("12345"));
	}

	#[test]
	fn test_from_str_errors() {
		assert!("no-colon".parse::<ModSource>().is_err());
		assert!("unknown:id".parse::<ModSource>().is_err());
	}

	#[test]
	fn test_display() {
		assert_eq!(ModSource::modrinth("jei").to_string(), "modrinth:jei");
		assert_eq!(
			ModSource::curseforge("12345").to_string(),
			"curseforge:12345"
		);
		assert_eq!(
			ModSource::url("https://github.com/owner/repo").to_string(),
			"url:https://github.com/owner/repo"
		);
	}

	#[test]
	fn test_dependency_kind_display() {
		assert_eq!(DependencyKind::Required.to_string(), "required");
		assert_eq!(DependencyKind::Optional.to_string(), "optional");
		assert_eq!(DependencyKind::Incompatible.to_string(), "incompatible");
		assert_eq!(DependencyKind::Embedded.to_string(), "embedded");
	}

	#[test]
	fn test_dependency_kind_from_str() {
		assert_eq!(
			"required".parse::<DependencyKind>().unwrap(),
			DependencyKind::Required
		);
		assert_eq!(
			"optional".parse::<DependencyKind>().unwrap(),
			DependencyKind::Optional
		);
		assert_eq!(
			"incompatible".parse::<DependencyKind>().unwrap(),
			DependencyKind::Incompatible
		);
		assert_eq!(
			"embedded".parse::<DependencyKind>().unwrap(),
			DependencyKind::Embedded
		);
		assert!("unknown".parse::<DependencyKind>().is_err());
	}

	#[test]
	fn test_dependency_kind_is_required() {
		assert!(DependencyKind::Required.is_required());
		assert!(!DependencyKind::Optional.is_required());
		assert!(!DependencyKind::Incompatible.is_required());
		assert!(!DependencyKind::Embedded.is_required());
	}

	#[test]
	fn test_dependency_new() {
		let dep = Dependency::new(
			"jei",
			ModSource::modrinth("jei"),
			DependencyKind::Required,
		);
		assert_eq!(dep.mod_id, "jei");
		assert!(dep.version.is_none());
		assert_eq!(dep.kind, DependencyKind::Required);
	}

	#[test]
	fn test_dependency_with_version() {
		let dep = Dependency::new(
			"jei",
			ModSource::modrinth("jei"),
			DependencyKind::Required,
		)
		.with_version(VersionReq::any());
		assert!(dep.version.is_some());
		assert_eq!(dep.kind, DependencyKind::Required);
	}

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
