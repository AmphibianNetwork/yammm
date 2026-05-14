use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ModSource {
	Modrinth { id: String },
	CurseForge { project_id: String },
	Url { url: String },
}

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

	pub fn requires_api(&self) -> bool {
		matches!(
			self,
			ModSource::Modrinth { .. } | ModSource::CurseForge { .. }
		)
	}

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

#[derive(Debug, thiserror::Error)]
pub enum ModSourceParseError {
	#[error(
		"Invalid format: expected 'source:id' (e.g., 'modrinth:jei'), got '{0}'"
	)]
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
}
