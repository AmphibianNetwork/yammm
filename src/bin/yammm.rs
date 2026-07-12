//! Binary entry point for yammm.
//!
//! Parses CLI arguments, sets up the tokio runtime, and delegates to `Cli::exec()`.
//! On error, prints the error chain and exits with the mapped exit code.

use clap::Parser;
use yammm::Cli;

fn main() {
	// human-panic provides friendly crash reports for unexpected panics
	human_panic::setup_panic!();

	let cli = Cli::parse();

	// Create the tokio runtime manually so we can control the error handling.
	// Using `#[tokio::main]` would swallow the exit code on error.
	let rt =
		tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
	let result = rt.block_on(cli.exec());
	if let Err(e) = result {
		yammm::print_error(&e);
		std::process::exit(yammm::exit_code(&e));
	}
}
