//! Version types for Minecraft and mods.
//!
//! Three related types:
//! - `Version`: an opaque version string (e.g. `"1.20.4"`)
//! - `VersionReq`: a constraint like `">=1.20"` or `"^1.0.0"` (semver-like)
//! - `ComparableVersion`: a parsed `(major, minor, patch)` tuple for sorting
//!
//! These are **not** full semver — Minecraft versions don't follow semver.
//! Instead, we parse dot-separated numeric parts and compare them as tuples,
//! stripping pre-release suffixes (e.g. `"1.21.1-beta"` → `(1, 21, 1)`).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Error returned when parsing version strings fails
#[derive(Debug, thiserror::Error)]
#[error("Invalid version: {0}")]
pub struct VersionError(pub String);

/// A version string representing software versions (e.g., "1.20.4", "18.5.2")
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version(String);

impl Version {
	/// Creates a new Version from a string
	pub fn parse(s: impl Into<String>) -> Result<Self, VersionError> {
		let s = s.into();
		if s.is_empty() {
			return Err(VersionError("version cannot be empty".to_string()));
		}
		Ok(Version(s))
	}

	/// Returns the version as a string
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Creates a Version from a major.minor.patch tuple
	pub fn from_parts(
		major: u32,
		minor: u32,
		patch: u32,
	) -> Self {
		Version(format!("{}.{}.{}", major, minor, patch))
	}
}

impl FromStr for Version {
	type Err = VersionError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Self::parse(s)
	}
}

impl Serialize for Version {
	fn serialize<S>(
		&self,
		serializer: S,
	) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for Version {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		Version::parse(&s).map_err(serde::de::Error::custom)
	}
}

impl fmt::Display for Version {
	fn fmt(
		&self,
		f: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// Comparison operators for version requirements.
/// Follows semver/cargo conventions: `>=`, `<=`, `>`, `<`, `^` (caret), `~` (tilde).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
	Gte,
	Lte,
	Gt,
	Lt,
	Caret,
	Tilde,
	Exact,
	Any,
}

impl Operator {
	fn prefix(&self) -> &'static str {
		match self {
			Operator::Gte => ">=",
			Operator::Lte => "<=",
			Operator::Gt => ">",
			Operator::Lt => "<",
			Operator::Caret => "^",
			Operator::Tilde => "~",
			Operator::Exact => "",
			Operator::Any => "*",
		}
	}
}

/// A version requirement specifier (e.g., ">=1.20", "^1.0.0", "~1.2", "1.20.4")
/// Supports: * (any), >=, <=, >, <, ^ (caret), ~ (tilde/semver), and exact version matching
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionReq {
	operator: Operator,
	version: String,
}

impl Serialize for VersionReq {
	fn serialize<S>(
		&self,
		serializer: S,
	) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&self.display_str())
	}
}

impl<'de> Deserialize<'de> for VersionReq {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		VersionReq::parse(&s).map_err(serde::de::Error::custom)
	}
}

impl VersionReq {
	/// Parse a version requirement string
	/// Supports operators: * (any), >=, <=, >, <, ^ (caret), ~ (tilde/semver), exact
	///
	/// # Errors
	/// Returns an error if:
	/// - The string is empty
	/// - An operator is followed by a space before the version (e.g., ">= 1.20")
	/// - An operator is present but not followed by a version
	pub fn parse(s: impl Into<String>) -> Result<Self, VersionError> {
		let s = s.into();
		if s.is_empty() {
			return Err(VersionError(
				"version requirement cannot be empty".to_string(),
			));
		}

		const OPERATORS: &[(&str, Operator)] = &[
			(">=", Operator::Gte),
			("<=", Operator::Lte),
			(">", Operator::Gt),
			("<", Operator::Lt),
			("^", Operator::Caret),
			("~", Operator::Tilde),
		];

		for (prefix, op) in OPERATORS {
			if let Some(version_part) = s.strip_prefix(prefix) {
				if version_part.is_empty() {
					return Err(VersionError(format!(
						"version requirement '{}' has operator '{}' but no version",
						s, prefix
					)));
				}
				if version_part.starts_with(char::is_whitespace) {
					return Err(VersionError(format!(
						"version requirement '{}' has a space after operator; use '{}{}' instead",
						s,
						prefix,
						version_part.trim()
					)));
				}
				return Ok(VersionReq {
					operator: *op,
					version: version_part.to_string(),
				});
			}
		}

		if s == "*" {
			return Ok(VersionReq {
				operator: Operator::Any,
				version: String::new(),
			});
		}

		Ok(VersionReq {
			operator: Operator::Exact,
			version: s,
		})
	}

	/// Returns a wildcard requirement that matches any version
	pub fn any() -> Self {
		VersionReq {
			operator: Operator::Any,
			version: String::new(),
		}
	}

	/// Returns the requirement as a display string (handles Any as "*")
	pub fn display_str(&self) -> String {
		match self.operator {
			Operator::Any => "*".to_string(),
			_ => format!("{}{}", self.operator.prefix(), self.version),
		}
	}

	/// Checks if a version satisfies this requirement
	pub fn satisfies(
		&self,
		version: &Version,
	) -> bool {
		self.matches(&version.0)
	}

	/// Check if a version string satisfies this requirement
	/// Implements: * (any), >= (gte), <= (lte), > (gt), < (lt), ^ (caret), ~ (tilde), exact match
	pub fn matches(
		&self,
		version: &str,
	) -> bool {
		match self.operator {
			Operator::Gte => version_compare_gte(version, &self.version),
			Operator::Lte => version_compare_lte(version, &self.version),
			Operator::Gt => version_compare_gt(version, &self.version),
			Operator::Lt => version_compare_lt(version, &self.version),
			Operator::Caret => version_compare_caret(version, &self.version),
			Operator::Tilde => version_compare_tilde(version, &self.version),
			Operator::Exact => version == self.version,
			Operator::Any => true,
		}
	}
}

/// A parsed semver-like version that implements `Ord` for sorting.
///
/// Strips non-digit suffixes (e.g., "1.21.1-beta" → "1.21.1").
/// Falls back to `(0, 0, 0)` if parsing fails, so unparseable versions
/// sort below all valid ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparableVersion((u64, u64, u64));

impl ComparableVersion {
	pub fn parse(s: &str) -> Self {
		match parse_version_parts(s) {
			Some(parts) => Self(parts),
			None => {
				tracing::warn!(
					"Could not parse version '{}', treating as (0, 0, 0)",
					s
				);
				Self((0, 0, 0))
			}
		}
	}
}

impl Ord for ComparableVersion {
	fn cmp(
		&self,
		other: &Self,
	) -> std::cmp::Ordering {
		self.0.cmp(&other.0)
	}
}

impl PartialOrd for ComparableVersion {
	fn partial_cmp(
		&self,
		other: &Self,
	) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl fmt::Display for ComparableVersion {
	fn fmt(
		&self,
		f: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		write!(f, "{}.{}.{}", (self.0).0, (self.0).1, (self.0).2)
	}
}

/// Parse a version string into a (major, minor, patch) tuple.
/// Strips non-digit suffixes (e.g., "1.21.1-beta" → "1.21.1").
/// Returns `None` if the first numeric part cannot be parsed (e.g., "abc").
pub fn parse_version_parts(s: &str) -> Option<(u64, u64, u64)> {
	let mut parts = s.split('.').map(|p| {
		p.chars()
			.take_while(|c| c.is_ascii_digit())
			.collect::<String>()
			.parse::<u64>()
			.ok()
	});
	let major = parts.next().flatten()?;
	let minor = parts.next().unwrap_or(Some(0)).unwrap_or(0);
	let patch = parts.next().unwrap_or(Some(0)).unwrap_or(0);
	Some((major, minor, patch))
}

fn version_cmp(
	version: &str,
	target: &str,
	op: impl Fn((u64, u64, u64), (u64, u64, u64)) -> bool,
) -> bool {
	let (Some(v), Some(t)) =
		(parse_version_parts(version), parse_version_parts(target))
	else {
		return false;
	};
	op(v, t)
}

fn version_compare_gte(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |v, t| v >= t)
}

fn version_compare_lte(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |v, t| v <= t)
}

fn version_compare_gt(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |v, t| v > t)
}

fn version_compare_lt(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |v, t| v < t)
}

fn version_compare_caret(
	version: &str,
	target: &str,
) -> bool {
	let (Some(v), Some(t)) =
		(parse_version_parts(version), parse_version_parts(target))
	else {
		return false;
	};
	if v < t {
		return false;
	}
	if t.0 != 0 {
		v.0 == t.0
	} else if t.1 != 0 {
		v.0 == t.0 && v.1 == t.1
	} else {
		v.0 == t.0 && v.1 == t.1 && v.2 == t.2
	}
}

/// Tilde (~) version comparison (semver-compatible range).
///
/// Matches versions with the same major version, and:
/// - If minor is 0, allows any minor (e.g., `~1.0` matches `1.0`, `1.5`, but not `2.0`)
/// - If minor is non-zero, requires same minor (e.g., `~1.2` matches `1.2.0`, `1.2.5`, but not `1.3.0`)
/// - Always requires the version to be >= the target
///
/// Used internally by [`VersionReq::matches`] when the `~` prefix is present.
fn version_compare_tilde(
	version: &str,
	target: &str,
) -> bool {
	let (Some(v), Some(t)) =
		(parse_version_parts(version), parse_version_parts(target))
	else {
		return false;
	};
	v.0 == t.0 && (t.1 == 0 || v.1 == t.1) && v >= t
}

impl FromStr for VersionReq {
	type Err = VersionError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Self::parse(s)
	}
}

impl fmt::Display for VersionReq {
	fn fmt(
		&self,
		f: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		write!(f, "{}{}", self.operator.prefix(), self.version)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_version_parse() {
		let v = Version::parse("1.20.4").unwrap();
		assert_eq!(v.as_str(), "1.20.4");
	}

	#[test]
	fn test_version_empty() {
		assert!(Version::parse("").is_err());
	}

	#[test]
	fn test_version_from_parts() {
		let v = Version::from_parts(1, 20, 4);
		assert_eq!(v.as_str(), "1.20.4");
	}

	#[test]
	fn test_version_with_suffix() {
		let v = Version::parse("1.20.4-SNAPSHOT").unwrap();
		assert_eq!(v.as_str(), "1.20.4-SNAPSHOT");
	}

	#[test]
	fn test_version_req_parse() {
		let req = VersionReq::parse(">=1.20").unwrap();
		assert_eq!(req.display_str(), ">=1.20");
	}

	#[test]
	fn test_version_req_empty() {
		assert!(VersionReq::parse("").is_err());
	}

	#[test]
	fn test_version_req_matches_gte() {
		let req = VersionReq::parse(">=1.20").unwrap();
		assert!(req.matches("1.20"));
		assert!(req.matches("1.20.4"));
		assert!(!req.matches("1.19"));
	}

	#[test]
	fn test_version_req_matches_caret() {
		let req = VersionReq::parse("^1.20").unwrap();
		assert!(req.matches("1.20"));
		assert!(req.matches("1.20.4"));
		assert!(req.matches("1.21.0"));
		assert!(!req.matches("2.0.0"));
	}

	#[test]
	fn test_version_req_matches_caret_zero() {
		let req = VersionReq::parse("^0.5").unwrap();
		assert!(req.matches("0.5.0"));
		assert!(req.matches("0.5.8"));
		assert!(!req.matches("0.6.0"));
		assert!(!req.matches("1.0.0"));
	}

	#[test]
	fn test_version_req_matches_caret_zero_zero() {
		let req = VersionReq::parse("^0.0.3").unwrap();
		assert!(req.matches("0.0.3"));
		assert!(!req.matches("0.0.4"));
		assert!(!req.matches("0.0.2"));
		assert!(!req.matches("0.1.0"));
	}

	#[test]
	fn test_version_req_matches_wildcard() {
		let req = VersionReq::parse("*").unwrap();
		assert!(req.matches("1.0.0"));
		assert!(req.matches("2.0.0"));
		assert!(req.matches("0.0.1"));
		assert!(req.matches("0.0.0"));
	}

	#[test]
	fn test_version_req_any() {
		let req = VersionReq::any();
		assert!(req.matches("1.0.0"));
		assert!(req.matches("99.99.99"));
	}

	#[test]
	fn test_version_req_matches_tilde() {
		let req = VersionReq::parse("~1.2").unwrap();
		assert!(req.matches("1.2.0"));
		assert!(req.matches("1.2.5"));
		assert!(!req.matches("1.3.0"));
		assert!(!req.matches("1.10.0"));
	}

	#[test]
	fn test_version_req_matches_tilde_zero_minor() {
		let req = VersionReq::parse("~1.0").unwrap();
		assert!(req.matches("1.0.0"));
		assert!(req.matches("1.5.0"));
		assert!(!req.matches("2.0.0"));
	}

	#[test]
	fn test_version_req_matches_exact() {
		let req = VersionReq::parse("1.20.4").unwrap();
		assert!(req.matches("1.20.4"));
		assert!(!req.matches("1.20.3"));
	}

	#[test]
	fn test_version_req_rejects_space_after_operator() {
		assert!(VersionReq::parse(">= 1.20").is_err());
		assert!(VersionReq::parse("> 1.0").is_err());
		assert!(VersionReq::parse("< 2.0").is_err());
		assert!(VersionReq::parse("<= 3.0").is_err());
		assert!(VersionReq::parse("^ 1.0").is_err());
		assert!(VersionReq::parse("~ 1.0").is_err());
	}

	#[test]
	fn test_version_req_rejects_operator_without_version() {
		assert!(VersionReq::parse(">=").is_err());
		assert!(VersionReq::parse("^").is_err());
		assert!(VersionReq::parse("~").is_err());
	}

	#[test]
	fn test_parse_version_parts_invalid() {
		assert!(parse_version_parts("abc").is_none());
		assert!(parse_version_parts("").is_none());
		assert_eq!(parse_version_parts("1.20.4"), Some((1, 20, 4)));
		assert_eq!(parse_version_parts("0.5.0"), Some((0, 5, 0)));
	}

	#[test]
	fn test_version_non_numeric_fails_comparison() {
		let req = VersionReq::parse(">=1.20").unwrap();
		assert!(!req.matches("abc"));
	}

	#[test]
	fn test_version_display() {
		let v = Version::from_parts(1, 20, 4);
		assert_eq!(format!("{}", v), "1.20.4");
	}

	#[test]
	fn test_version_from_str() {
		let v: Version = "1.20.4".parse().unwrap();
		assert_eq!(v.as_str(), "1.20.4");
	}

	#[test]
	fn test_version_req_from_str() {
		let req: VersionReq = ">=1.20".parse().unwrap();
		assert_eq!(req.display_str(), ">=1.20");
	}
}
