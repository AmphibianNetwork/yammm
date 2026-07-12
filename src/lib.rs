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
pub(crate) mod api;
pub(crate) mod app;
pub(crate) mod auth;
pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod config;
pub(crate) mod errors;
pub(crate) mod output;
pub(crate) mod providers;
pub(crate) mod services;
pub(crate) mod storage;
pub(crate) mod types;
pub(crate) mod utils;

#[cfg(test)]
pub(crate) mod test_util;

// Public crate surface — kept deliberately minimal. The binary at
// `src/bin/yammm.rs` is the only legitimate consumer; everything else stays
// crate-internal so refactors don't leak across the lib/bin boundary.
pub use cli::Cli;
pub use errors::exit_code;
pub use utils::print_error;

/// Initialize the tracing subscriber. Level: `RUST_LOG` > `--debug` > INFO.
pub(crate) fn init_logging(debug: bool) {
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
