//! Configuration management.
//!
//! - `GlobalConfig` (`~/.config/yammm/config.toml`) — user-wide preferences
//! - `ModpackManifest` (`modpack.toml`) — per-modpack metadata

mod global;
mod modpack;

pub use global::{GlobalConfig, OutputFormat};
pub use modpack::{LoaderConfig, ModpackManifest};
