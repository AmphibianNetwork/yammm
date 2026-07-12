//! Source abstraction layer — the bridge between domain logic and external APIs.
//!
//! Defines the `Provider` enum (closed-set dispatch) and the `ModSourceProvider`
//! trait that all sources implement. The `SourceRegistry` maps `ModSource`
//! variants to their `Provider` implementations.
//!
//! ## Adding a new source
//!
//! 1. Create a new file (e.g. `github.rs`) implementing `ModSourceProvider`
//! 2. Add a variant to `Provider` and update the `dispatch!` macro in `provider.rs`
//! 3. Add a `SourceKey` variant and wire it in `SourceRegistry::from_config`
//! 4. Add a `ModSource` variant and `FromStr` parsing support
//! 5. (Optional) Add a `CliSource` variant in `commands/mod.rs`

pub mod curseforge;
pub mod error;
pub mod modrinth;
pub mod provider;
pub mod registry;
pub mod url;

#[cfg(test)]
pub mod mock;

pub use provider::{Provider, SearchFilters};
pub use registry::{SourceKey, SourceRegistry};
