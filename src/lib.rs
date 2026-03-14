//! yammm - Yet Another Minecraft Modpack Maker
//!
//! ```text
//! CLI → Commands → AppContext
//!                     ↓
//!       Services ←→ Providers
//!                     ↓
//!       Storage  ←→ Config
//!                     ↓
//!       Types + Errors
//!                     ↓
//!       API Clients ←→ Auth
//!                     ↓
//!       Utils
//! ```
//!
pub mod api;
pub mod app;
pub mod auth;
pub mod cli;
pub mod commands;
pub mod config;
pub mod errors;
pub mod output;
pub mod providers;
pub mod services;
pub mod storage;
pub mod types;
pub mod utils;

#[cfg(test)]
pub mod test_util;

pub use cli::Cli;
pub use errors::exit_code;

/// Initialize the tracing subscriber. Level: `RUST_LOG` > `--debug` > INFO.
pub fn init_logging(debug: bool) {
	use tracing_subscriber::EnvFilter;

	let default_level = if debug { "debug" } else { "info" };
	let filter = EnvFilter::try_from_default_env()
		.unwrap_or_else(|_| EnvFilter::new(default_level));

	tracing_subscriber::fmt()
		.with_env_filter(filter)
		.with_target(false)
		.without_time()
		.init();
}
