# Contributing to yammm

Thanks for your interest in contributing! Here's how to get started.

## Setup

1. Clone the repository
2. Install Rust stable (`rustup install stable`)
3. Build: `cargo build`
4. Run tests: `cargo test --lib`
5. (Optional) Enter the Nix dev shell: `nix develop`

## Code Style

- Format with `cargo fmt` before committing
- No warnings from `cargo clippy --all-targets --all-features -- -D warnings`
- No `TODO`/`FIXME` markers, file issues instead
- Avoid `unwrap()` in production code; use `?`, `context()`, or `expect("reason")`
- No comments unless they explain *why*, not *what*

## Making Changes

1. Create a feature branch from `main`
2. Make your changes with tests
3. Run the full check: `just check` (or `cargo fmt && cargo clippy && cargo test --lib`)
4. Commit with a clear message
5. Open a pull request

## Testing

- **Unit tests**: `cargo test --lib`
- **Integration tests (offline)**: `cargo test --test integration_tests -- --test-threads=1`
- **Integration tests (network)**: `cargo test --test integration_tests -- --ignored --test-threads=1`
- **E2E launch tests**: `cargo run -p yammm-e2e`

## Architecture

See [docs/specs/architecture.md](docs/specs/architecture.md) for the system design and module structure.

## Reporting Issues

- Use [GitHub Issues](https://github.com/Conquerix/yammm/issues)
- Include: yammm version, OS, steps to reproduce, expected vs actual behavior
