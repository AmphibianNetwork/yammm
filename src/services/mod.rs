//! Services layer — business logic between commands and providers/storage.
//!
//! - `resolver`: BFS dependency resolution with cycle detection
//! - `download`: JAR download with retry, hash verification, and concurrency
//! - `deps_install`: Shared dependency categorization, prompting, and installation
//! - `mod_install`: Core non-interactive mod installation logic
//! - `dep_graph`: Reverse-dependency lookup and stale-dep cleanup
//! - `connector`: Sinytra Connector detection for Fabric-on-Forge compat

pub mod connector;
pub mod dep_graph;
pub mod deps_install;
pub mod download;
pub mod mod_install;
pub mod resolver;

pub use connector::is_connector_installed;
pub use dep_graph::{cleanup_stale_deps, find_reverse_deps};
pub use download::{DownloadSummary, download_missing_mods};
pub use resolver::{DependencyResolver, ResolvedMod};
