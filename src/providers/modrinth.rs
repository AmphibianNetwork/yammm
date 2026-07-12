//! Modrinth source implementation.
//!
//! Delegates to `ModrinthClient` for HTTP calls and converts the API-specific
//! response types into the generic `ModInfo`/`ModVersion`/`SourceDependency`
//! types that the rest of the app uses.
//!
//! All dependencies returned by Modrinth are scoped to the Modrinth source —
//! they reference other Modrinth projects by slug/ID. Cross-source deps
//! would require the user to add them manually.

use std::sync::Arc;

use crate::api::ModrinthClient;
use crate::api::error::ApiError;
use crate::providers::error::{ProviderError, ProviderResult};
use crate::providers::provider::{ModSourceProvider, SearchFilters};
use crate::types::SourceDependency;
use crate::types::{
	DependencyKind, ModEnv, ModInfo, ModVersion, VersionFilters,
};

const SOURCE: &str = "modrinth";

#[derive(Clone, Debug)]
pub struct ModrinthSource {
	client: Arc<ModrinthClient>,
}

impl ModrinthSource {
	pub fn new(http_client: reqwest::Client) -> Self {
		Self {
			client: Arc::new(ModrinthClient::new().with_client(http_client)),
		}
	}
}

impl ModSourceProvider for ModrinthSource {
	fn name(&self) -> &str {
		"modrinth"
	}

	fn supports_search(&self) -> bool {
		true
	}

	fn get_mod_env(
		&self,
		mod_info: &ModInfo,
	) -> ModEnv {
		mod_env_from_modrinth_sides(
			mod_info.client_side.as_deref(),
			mod_info.server_side.as_deref(),
		)
	}

	async fn search(
		&self,
		query: &str,
		filters: &SearchFilters,
	) -> ProviderResult<Vec<ModInfo>> {
		let hits = self
			.client
			.search(query, filters.limit, filters.offset)
			.await
			.map_err(|e| ProviderError::from_api_error(e, SOURCE))?;

		let loader = filters.version.loader.map(|l| l.as_str());
		let mc_ver = filters.version.minecraft_version.as_deref();

		let results: Vec<ModInfo> = hits
			.into_iter()
			.filter(|h| matches_minecraft_version(h, mc_ver))
			.filter(|h| matches_loader(h, loader))
			.map(ModrinthClient::to_mod_info_from_hit)
			.collect();

		Ok(results)
	}

	async fn get_mod(
		&self,
		mod_id: &str,
	) -> ProviderResult<ModInfo> {
		let hit = self
			.client
			.get_mod(mod_id)
			.await
			.map_err(|e| map_modrinth_error_for_mod(e, mod_id))?;
		Ok(ModrinthClient::to_mod_info_from_hit(hit))
	}

	async fn get_versions(
		&self,
		mod_id: &str,
		filters: &VersionFilters,
	) -> ProviderResult<Vec<ModVersion>> {
		let loader_str = filters.loader.map(|l| l.as_str());
		let versions = self
			.client
			.get_versions(
				mod_id,
				filters.minecraft_version.as_deref(),
				loader_str,
			)
			.await
			.map_err(|e| map_modrinth_error_for_mod(e, mod_id))?;

		Ok(versions
			.into_iter()
			.map(ModrinthClient::to_mod_version)
			.collect())
	}

	async fn get_dependencies(
		&self,
		_mod_id: &str,
		version_id: &str,
	) -> ProviderResult<Vec<SourceDependency>> {
		let deps = self
			.client
			.get_dependencies(version_id)
			.await
			.map_err(|e| ProviderError::from_api_error(e, SOURCE))?;

		Ok(deps
			.into_iter()
			.filter_map(|d| {
				let mod_id = d.mod_id;
				let dep_type = match d.dependency_type.parse::<DependencyKind>()
				{
					Ok(kind) => kind,
					Err(e) => {
						tracing::warn!("Skipping dependency {}: {}", mod_id, e);
						return None;
					}
				};
				Some(SourceDependency {
					mod_id: mod_id.clone(),
					version_id: d.version_id,
					dep_type,
					source: Some(crate::types::ModSource::modrinth(mod_id)),
				})
			})
			.collect())
	}
}

/// Like `ProviderError::from_api_error` but rewrites the not-found payload
/// to include the queried mod id (the API otherwise returns a generic
/// "not found").
fn map_modrinth_error_for_mod(
	err: ApiError,
	mod_id: &str,
) -> ProviderError {
	match err {
		ApiError::NotFound(_) | ApiError::Http { status: 404, .. } => {
			ProviderError::NotFound {
				provider: SOURCE,
				what: mod_id.to_string(),
			}
		}
		other => ProviderError::from_api_error(other, SOURCE),
	}
}

/// Whether a Modrinth search hit advertises support for the given Minecraft
/// version. Matches `1.20.4` to `1.20.4`, `1.20.4-pre1`, and `1.20.4+build.5`,
/// but **not** `1.20.42` (`starts_with` would lie) and **not** `1.20` when the
/// query is `1.20.4` (we treat MC versions as fully-qualified). When the query
/// is itself a prefix like `1.20`, the match passes for any `1.20.x`.
///
/// The previous implementation split `1.20.4` into segments `["1","20","4"]`
/// and accepted a query equal to *any* of those — so `mc_ver = "1"` matched
/// every 1.x release.
fn matches_minecraft_version(
	hit: &crate::api::ModrinthSearchHit,
	mc_ver: Option<&str>,
) -> bool {
	let Some(mc) = mc_ver else {
		return true;
	};
	hit.versions
		.iter()
		.any(|advertised| version_satisfies(advertised, mc))
}

/// Compare an advertised game version against a user query.
///
/// `advertised` is what Modrinth puts in the hit's `versions` array
/// (e.g. `"1.20.4"`, `"1.21-pre3"`, `"24w14a"`); `query` is whatever the user
/// asked for. The query matches if it equals the release portion or is a
/// dot-separated prefix of it (so `"1.20"` matches `"1.20.4"`).
fn version_satisfies(
	advertised: &str,
	query: &str,
) -> bool {
	let release = release_part(advertised);
	if release == query {
		return true;
	}
	// Treat the query as a dot-separated prefix only when each query segment
	// equals the corresponding release segment. This blocks `1.20.4` from
	// matching `1.20.42` (which `str::starts_with` would mishandle).
	let mut q = query.split('.');
	let mut r = release.split('.');
	loop {
		match (q.next(), r.next()) {
			(Some(qs), Some(rs)) if qs == rs => continue,
			(None, _) => return true,
			_ => return false,
		}
	}
}

/// Strip pre-release / build metadata so `1.20.4-pre1` and `1.20.4+build.5`
/// both reduce to `"1.20.4"`.
fn release_part(version: &str) -> &str {
	let cut = version.find(['-', '+']).unwrap_or(version.len());
	&version[..cut]
}

fn matches_loader(
	hit: &crate::api::ModrinthSearchHit,
	loader: Option<&str>,
) -> bool {
	let Some(ldr) = loader else {
		return true;
	};
	hit.categories
		.iter()
		.any(|c| c.to_lowercase() == ldr.to_lowercase())
}

/// Derive a `ModEnv` from Modrinth's `client_side` / `server_side` values.
///
/// Modrinth uses `"required"`, `"optional"`, `"unsupported"` for each side.
/// This maps those semantics to our `ModEnv` enum.
pub(crate) fn mod_env_from_modrinth_sides(
	client_side: Option<&str>,
	server_side: Option<&str>,
) -> ModEnv {
	let client = client_side.unwrap_or("required");
	let server = server_side.unwrap_or("required");
	match (client, server) {
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

#[cfg(test)]
mod tests {
	use super::*;

	fn make_hit(
		versions: Vec<&str>,
		categories: Vec<&str>,
	) -> crate::api::ModrinthSearchHit {
		crate::api::ModrinthSearchHit {
			project_id: String::new(),
			project_type: String::new(),
			slug: String::new(),
			author: String::new(),
			title: String::new(),
			description: String::new(),
			categories: categories.into_iter().map(String::from).collect(),
			display_categories: Vec::new(),
			versions: versions.into_iter().map(String::from).collect(),
			downloads: 0,
			follows: 0,
			icon_url: None,
			date_created: String::new(),
			date_modified: String::new(),
			latest_version: None,
			license: String::new(),
			client_side: None,
			server_side: None,
		}
	}

	#[test]
	fn test_mod_env_both_required() {
		assert_eq!(
			mod_env_from_modrinth_sides(Some("required"), Some("required")),
			ModEnv::Both
		);
	}

	#[test]
	fn test_mod_env_client_only() {
		assert_eq!(
			mod_env_from_modrinth_sides(Some("required"), Some("unsupported")),
			ModEnv::Client
		);
		assert_eq!(
			mod_env_from_modrinth_sides(Some("required"), Some("optional")),
			ModEnv::Client
		);
	}

	#[test]
	fn test_mod_env_server_only() {
		assert_eq!(
			mod_env_from_modrinth_sides(Some("unsupported"), Some("required")),
			ModEnv::Server
		);
		assert_eq!(
			mod_env_from_modrinth_sides(Some("optional"), Some("required")),
			ModEnv::Server
		);
	}

	#[test]
	fn test_mod_env_defaults_to_both() {
		assert_eq!(mod_env_from_modrinth_sides(None, None), ModEnv::Both);
		assert_eq!(
			mod_env_from_modrinth_sides(Some("optional"), Some("optional")),
			ModEnv::Both
		);
		assert_eq!(
			mod_env_from_modrinth_sides(
				Some("unsupported"),
				Some("unsupported")
			),
			ModEnv::Both
		);
	}

	#[test]
	fn test_matches_minecraft_version_none_passes() {
		let hit = make_hit(vec!["1.20.4"], vec![]);
		assert!(matches_minecraft_version(&hit, None));
	}

	#[test]
	fn test_matches_minecraft_version_exact() {
		let hit = make_hit(vec!["1.20.4"], vec![]);
		assert!(matches_minecraft_version(&hit, Some("1.20.4")));
		assert!(!matches_minecraft_version(&hit, Some("1.21")));
	}

	#[test]
	fn test_matches_minecraft_version_strips_build_and_prerelease() {
		let with_build = make_hit(vec!["1.20.4+build.1"], vec![]);
		assert!(matches_minecraft_version(&with_build, Some("1.20.4")));
		assert!(matches_minecraft_version(&with_build, Some("1.20")));
		assert!(matches_minecraft_version(&with_build, Some("1")));
		assert!(!matches_minecraft_version(&with_build, Some("1.21")));
		assert!(!matches_minecraft_version(&with_build, Some("4")));

		let prerelease = make_hit(vec!["1.21-pre3"], vec![]);
		assert!(matches_minecraft_version(&prerelease, Some("1.21")));
		assert!(!matches_minecraft_version(&prerelease, Some("1.20")));
	}

	#[test]
	fn test_matches_minecraft_version_prefix_does_not_match_numeric_suffix() {
		// `1.20.42` (a hypothetical patch release) must not match a query
		// for `1.20.4` — a previous prefix-by-segment bug accepted it.
		let hit = make_hit(vec!["1.20.42"], vec![]);
		assert!(!matches_minecraft_version(&hit, Some("1.20.4")));
		// But a true prefix like `1.20` should still match `1.20.42`.
		assert!(matches_minecraft_version(&hit, Some("1.20")));
	}

	#[test]
	fn test_matches_minecraft_version_query_longer_than_release_rejected() {
		// User asked for `1.20.4` but the hit only advertises `1.20`.
		let hit = make_hit(vec!["1.20"], vec![]);
		assert!(!matches_minecraft_version(&hit, Some("1.20.4")));
		assert!(matches_minecraft_version(&hit, Some("1.20")));
	}

	#[test]
	fn test_matches_minecraft_version_snapshot_does_not_match_release() {
		// Modrinth includes snapshot identifiers like `24w14a`; querying
		// `1.20.4` must not match them.
		let hit = make_hit(vec!["24w14a"], vec![]);
		assert!(!matches_minecraft_version(&hit, Some("1.20.4")));
		// Exact snapshot match still works.
		assert!(matches_minecraft_version(&hit, Some("24w14a")));
	}

	#[test]
	fn test_matches_loader_case_insensitive() {
		let hit = make_hit(vec![], vec!["fabric"]);
		assert!(matches_loader(&hit, Some("fabric")));
		assert!(matches_loader(&hit, Some("Fabric")));
		assert!(!matches_loader(&hit, Some("forge")));
	}

	#[test]
	fn test_matches_loader_none_passes() {
		let hit = make_hit(vec![], vec![]);
		assert!(matches_loader(&hit, None));
	}
}
