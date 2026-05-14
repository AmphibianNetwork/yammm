use serde::{Deserialize, Serialize};

use super::source::ModSource;
use crate::types::VersionReq;

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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::VersionReq;

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
}
