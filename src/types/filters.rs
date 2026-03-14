//! Version filter parameters for provider queries.
//!
//! Used when fetching versions from Modrinth/CurseForge to narrow results
//! to a specific Minecraft version and/or mod loader.

use serde::{Deserialize, Serialize};

use super::LoaderType;

/// Filters passed to `Provider::get_versions()` and `Provider::search()`.
///
/// Both fields are optional — `None` means "don't filter by this dimension".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VersionFilters {
	pub minecraft_version: Option<String>,
	pub loader: Option<LoaderType>,
}
