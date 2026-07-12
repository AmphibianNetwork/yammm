//! Version types for Minecraft and mods.
//!
//! Two related types:
//! - `Version`: an opaque version string (e.g. `"1.20.4"`)
//! - `VersionReq`: a constraint like `">=1.20"` or `"^1.0.0"` (semver-like)
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

/// Comparison strategy: try canonical semver first, fall back to the
/// digit-tuple shape we've used historically.
///
/// Why a hybrid:
///
/// - **Semver gives correct pre-release ordering.** `1.20.4-beta` is
///   *less than* `1.20.4`; `>=1.20.4` (and `^1.20.4`, `~1.20.4`) does
///   *not* match pre-release builds of `1.20.4`. This is the spec
///   behaviour every other modern ecosystem agrees on.
/// - **Minecraft versions aren't always semver.** `1.20` has only two
///   numeric components; `23w12a` is a snapshot. The digit-tuple
///   fallback keeps comparisons working for those strings instead of
///   silently failing.
///
/// The previous all-digit-tuple comparison stripped pre-release
/// suffixes (so `1.20.4-beta` and `1.20.4` compared equal), which is
/// useful for "is this Minecraft version supported" but wrong for mod
/// versioning. The hybrid keeps the looseness where strings can't be
/// parsed as semver, and tightens elsewhere.
fn try_semver_pair(
	version: &str,
	target: &str,
) -> Option<(semver::Version, semver::Version)> {
	let v = semver::Version::parse(version).ok()?;
	let t = semver::Version::parse(target).ok()?;
	Some((v, t))
}

fn version_cmp(
	version: &str,
	target: &str,
	op: impl Fn(std::cmp::Ordering) -> bool,
) -> bool {
	if let Some((v, t)) = try_semver_pair(version, target) {
		return op(v.cmp(&t));
	}
	let (Some(v), Some(t)) =
		(parse_version_parts(version), parse_version_parts(target))
	else {
		return false;
	};
	op(v.cmp(&t))
}

fn version_compare_gte(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |o| o.is_ge())
}

fn version_compare_lte(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |o| o.is_le())
}

fn version_compare_gt(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |o| o.is_gt())
}

fn version_compare_lt(
	version: &str,
	target: &str,
) -> bool {
	version_cmp(version, target, |o| o.is_lt())
}

/// Caret comparison: defer to `semver` when both sides parse as
/// canonical semver — that picks up pre-release exclusion automatically
/// — and fall back to the historic tuple logic otherwise.
fn version_compare_caret(
	version: &str,
	target: &str,
) -> bool {
	if let Some((v, _)) = try_semver_pair(version, target)
		&& let Ok(req) = semver::VersionReq::parse(&format!("^{target}"))
	{
		return req.matches(&v);
	}
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
/// When both sides are valid semver we delegate to `semver::VersionReq`
/// so pre-release pinning matches the spec. The legacy fallback covers
/// 2-component Minecraft versions (`~1.20`) and snapshot strings.
fn version_compare_tilde(
	version: &str,
	target: &str,
) -> bool {
	if let Some((v, _)) = try_semver_pair(version, target)
		&& let Ok(req) = semver::VersionReq::parse(&format!("~{target}"))
	{
		return req.matches(&v);
	}
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

	#[test]
	fn semver_prerelease_excluded_from_gte_when_both_parse() {
		// Spec behaviour: `>=1.20.4` does NOT match a pre-release build
		// of 1.20.4. Both strings parse as semver, so the canonical
		// comparison kicks in. Previously the digit-tuple stripped the
		// suffix and reported them equal.
		let req = VersionReq::parse(">=1.20.4").unwrap();
		assert!(!req.matches("1.20.4-beta"));
		assert!(req.matches("1.20.4"));
		assert!(req.matches("1.20.5"));
	}

	#[test]
	fn semver_caret_excludes_prerelease_on_newer_version() {
		// `^1.20.4` should match 1.20.5, but not 1.21.0-rc.1 — the
		// pre-release attaches to a *newer* version and gets filtered
		// per the semver caret rules.
		let req = VersionReq::parse("^1.20.4").unwrap();
		assert!(req.matches("1.20.5"));
		assert!(req.matches("1.20.4"));
		assert!(!req.matches("1.21.0-rc.1"));
		assert!(!req.matches("2.0.0"));
	}

	#[test]
	fn legacy_fallback_still_handles_two_component_minecraft_versions() {
		// `1.20` isn't valid semver (semver wants three parts), so the
		// digit-tuple fallback must still answer.
		let req = VersionReq::parse(">=1.20").unwrap();
		assert!(req.matches("1.20"));
		assert!(req.matches("1.21"));
		assert!(!req.matches("1.19"));
	}

	#[test]
	fn legacy_fallback_strips_snapshot_style_suffixes() {
		// Minecraft snapshot suffixes like `1.21.1-SNAPSHOT` are
		// pre-release under semver, *but* `>=1.21` is itself not valid
		// semver — so both sides fall back to the digit-tuple path and
		// the suffix gets stripped, matching historical behaviour.
		let req = VersionReq::parse(">=1.21").unwrap();
		assert!(req.matches("1.21.1-SNAPSHOT"));
	}
}

#[cfg(test)]
mod proptests {
	//! Property tests for [`VersionReq`] semantics.
	//!
	//! These exercise the algebraic relationships between operators —
	//! invariants the example-based tests can only sample, not prove.
	//! When an invariant fails, proptest shrinks to the minimal input
	//! that triggers it, which is more useful than a random
	//! counter-example.

	use super::*;
	use proptest::prelude::*;

	/// Generate a version triple `"major.minor.patch"` with bounded
	/// components. The bounds keep the search space small enough to
	/// converge quickly while still covering interesting carries
	/// (10 vs 100, 0 vs non-zero).
	fn version_triple() -> impl Strategy<Value = (u32, u32, u32)> {
		(0u32..100, 0u32..100, 0u32..100)
	}

	fn version_string() -> impl Strategy<Value = String> {
		version_triple().prop_map(|(a, b, c)| format!("{a}.{b}.{c}"))
	}

	proptest! {
		#![proptest_config(ProptestConfig {
			cases: 256,
			.. ProptestConfig::default()
		})]

		/// Any well-formed `VersionReq`'s `display_str` round-trips
		/// through `parse` back to itself.
		#[test]
		fn version_req_display_roundtrips(
			(op_idx, ver) in (0usize..=7, version_string())
		) {
			let prefixes = [">=", "<=", ">", "<", "^", "~", "", "*"];
			let raw = match prefixes[op_idx] {
				"*" => "*".to_string(),
				"" => ver.clone(),
				prefix => format!("{prefix}{ver}"),
			};
			let req = VersionReq::parse(&raw).unwrap();
			let displayed = req.display_str();
			let reparsed = VersionReq::parse(&displayed).unwrap();
			prop_assert_eq!(req, reparsed);
		}

		/// `*` matches every well-formed version string.
		#[test]
		fn any_matches_every_version(v in version_string()) {
			let req = VersionReq::any();
			prop_assert!(req.matches(&v));
		}

		/// Exact match is symmetric: `x` matches exactly `x` and
		/// nothing else within the same prefix tier.
		#[test]
		fn exact_matches_only_self(v in version_string()) {
			let req = VersionReq::parse(&v).unwrap();
			prop_assert!(req.matches(&v));
		}

		/// `>= x` is the reflexive closure of `> x`: if `> x` matches
		/// then `>= x` does too, and `>= x` matches `x` itself.
		#[test]
		fn gte_is_gt_or_equal(
			v in version_string(),
			t in version_string(),
		) {
			let gte = VersionReq::parse(format!(">={t}")).unwrap();
			let gt = VersionReq::parse(format!(">{t}")).unwrap();
			if gt.matches(&v) {
				prop_assert!(gte.matches(&v),
					"`>= {}` should match {} since `> {}` does", t, v, t);
			}
			// Reflexivity at the boundary.
			prop_assert!(gte.matches(&t));
		}

		/// `<= x` symmetry with `<`.
		#[test]
		fn lte_is_lt_or_equal(
			v in version_string(),
			t in version_string(),
		) {
			let lte = VersionReq::parse(format!("<={t}")).unwrap();
			let lt = VersionReq::parse(format!("<{t}")).unwrap();
			if lt.matches(&v) {
				prop_assert!(lte.matches(&v),
					"`<= {}` should match {} since `< {}` does", t, v, t);
			}
			prop_assert!(lte.matches(&t));
		}

		/// `> x` and `<= x` are complementary on parseable versions:
		/// every version satisfies exactly one (never both, never
		/// neither when both sides parse).
		#[test]
		fn gt_and_lte_are_complementary(
			v in version_string(),
			t in version_string(),
		) {
			let gt = VersionReq::parse(format!(">{t}")).unwrap();
			let lte = VersionReq::parse(format!("<={t}")).unwrap();
			prop_assert_ne!(gt.matches(&v), lte.matches(&v),
				"`> {}` and `<= {}` must partition the version space at {}",
				t, t, v);
		}

		/// Caret `^x.y.z` (for major > 0) implies the matched version
		/// is `>= x.y.z` AND `< (x+1).0.0`.
		#[test]
		fn caret_bounds_match_implication(
			(major, minor, patch) in (1u32..50, 0u32..50, 0u32..50),
			v in version_string(),
		) {
			let t = format!("{major}.{minor}.{patch}");
			let caret = VersionReq::parse(format!("^{t}")).unwrap();
			if caret.matches(&v) {
				let gte = VersionReq::parse(format!(">={t}")).unwrap();
				let upper =
					VersionReq::parse(format!("<{}.0.0", major + 1))
						.unwrap();
				prop_assert!(gte.matches(&v),
					"^{} matched {} but >={} did not", t, v, t);
				prop_assert!(upper.matches(&v),
					"^{} matched {} but <{}.0.0 did not", t, v, major + 1);
			}
		}

		/// Tilde `~x.y` (for minor > 0) implies the matched version
		/// is `>= x.y` AND `< x.(y+1).0`.
		#[test]
		fn tilde_bounds_match_implication(
			(major, minor, patch) in (0u32..50, 1u32..50, 0u32..50),
			v in version_string(),
		) {
			let t = format!("{major}.{minor}.{patch}");
			let tilde = VersionReq::parse(format!("~{t}")).unwrap();
			if tilde.matches(&v) {
				let gte = VersionReq::parse(format!(">={t}")).unwrap();
				let upper =
					VersionReq::parse(format!("<{}.{}.0", major, minor + 1))
						.unwrap();
				prop_assert!(gte.matches(&v),
					"~{} matched {} but >={} did not", t, v, t);
				prop_assert!(upper.matches(&v),
					"~{} matched {} but <{}.{}.0 did not", t, v, major, minor + 1);
			}
		}

		/// `parse_version_parts` is total over our generator: every
		/// `"a.b.c"` with small integers should parse to `Some(...)`.
		#[test]
		fn version_parts_parses_well_formed(
			(major, minor, patch) in version_triple()
		) {
			let v = format!("{major}.{minor}.{patch}");
			let parsed = parse_version_parts(&v);
			prop_assert_eq!(parsed,
				Some((major as u64, minor as u64, patch as u64)));
		}

		/// Pre-release suffixes get stripped: `"1.2.3-beta.5"` parses
		/// to `(1, 2, 3)`.
		#[test]
		fn pre_release_suffix_is_stripped(
			(major, minor, patch) in version_triple(),
			suffix in "[a-z]{1,8}",
		) {
			let v = format!("{major}.{minor}.{patch}-{suffix}");
			let parsed = parse_version_parts(&v);
			prop_assert_eq!(parsed,
				Some((major as u64, minor as u64, patch as u64)));
		}
	}
}
