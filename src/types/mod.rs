//! Domain types shared across the application.
//!
//! Not tied to any specific storage format or API — `storage` and `api`
//! handle serialization/deserialization.

mod filters;
mod hash_type;
mod loader;
mod mod_info;
mod version;

pub use filters::VersionFilters;
pub use hash_type::{HashType, HashTypeParseError};
pub use loader::{LoaderError, LoaderType};
pub use mod_info::{
	Dependency, DependencyKind, DependencyKindError, ModEnv, ModIdentity,
	ModInfo, ModSource, ModVersion, ProjectType, SourceDependency, TrackedMod,
	TrackedModBuilder,
};
pub use version::{ComparableVersion, Version, VersionError, VersionReq};
