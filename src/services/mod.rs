//! Services layer — business logic between commands and providers/storage.
//!
//! - `resolver`: BFS dependency resolution with cycle detection
//! - `download`: JAR download with retry, hash verification, and concurrency
//! - `deps_install`: Shared dependency categorization, prompting, and installation

pub mod deps_install;
pub mod download;
pub mod resolver;

pub use download::{download_missing_mods, DownloadSummary};
pub use resolver::{DependencyResolver, ResolvedMod};
