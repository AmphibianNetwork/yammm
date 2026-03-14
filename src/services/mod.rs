//! Services layer — business logic between commands and providers/storage.
//!
//! - `resolver`: BFS dependency resolution with cycle detection
//! - `download`: JAR download with retry, hash verification, and concurrency

pub mod download;
pub mod resolver;

pub use download::{download_missing_mods, DownloadSummary};
pub use resolver::{DependencyResolver, ResolvedMod};
