default:
    @just --list

# Run pre-commit hooks on all files, including autoformatting
pre-commit-all:
    pre-commit run --all-files

# Run 'cargo run' on the project
run *ARGS:
    cargo run {{ARGS}}

# Run 'bacon' to run the project (auto-recompiles)
watch *ARGS:
	bacon --job run -- -- {{ ARGS }}

# Run the e2e launch test suite
e2e *ARGS:
	cargo run -p yammm-e2e -- {{ ARGS }}

# Run all unit tests
test:
    cargo test --lib

# Run all tests including integration
test-all:
    cargo test --all -- --test-threads=1

# Run clippy lints
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Auto-format code
fmt:
    cargo fmt --all

# Run full CI check (fmt + lint + test)
check: fmt-check lint test
