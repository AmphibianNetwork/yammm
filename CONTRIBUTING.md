# Contributing to yammm

Thanks for your interest. This page gets you set up and points you at the docs that explain how the codebase is organized.

---

## Setup

```bash
git clone https://github.com/AmphibianNetwork/yammm
cd yammm
rustup install stable         # MSRV is 1.88, edition 2024
cargo build
cargo test --lib              # 428+ tests, runs in seconds
```

Optional:

- **Nix dev shell** — `nix develop` (or just `direnv allow` if you have direnv). Sets up rustfmt, clippy, just, bacon, pre-commit, etc.
- **bacon** — `just watch` for auto-recompile while editing.

---

## Day-to-day

Everything goes through `just`:

| Command | What it does |
|---|---|
| `just check` | The gate: fmt-check + clippy `-D warnings` + unit tests. Run this before pushing. |
| `just fmt` | Auto-format. |
| `just lint` | Clippy with denials. |
| `just test` | Unit tests only. |
| `just test-all` | All tests including integration (single-threaded). |
| `just e2e` | The launch-pipeline E2E suite (separate `yammm-e2e` workspace member). |
| `just run -- <args>` | `cargo run` shorthand. |
| `just watch -- <args>` | Auto-recompile loop via bacon. |

CI runs `just check` and integration tests on every PR; the `[lints.clippy] all = warn` in `Cargo.toml` is escalated to `-D warnings` by CI.

---

## Getting your bearings

Start here:

1. **[docs/specs/architecture.md](docs/specs/architecture.md)** — the layered architecture: CLI → commands → services → providers → API clients → storage/cache.
2. **[docs/specs/conventions.md](docs/specs/conventions.md)** — the three patterns this codebase leans on: the user-output channel, `AppContext` accessors, and the pure-builder extraction pattern. Read this before writing your first PR.
3. **[docs/README.md](docs/README.md)** — the doc index. Spec files cover each subsystem.

Then dig into whichever subsystem you're touching:

| You're working on... | Read |
|---|---|
| A new mod source (Modrinth-style provider) | [services.md](docs/specs/services.md) |
| Dependency resolution | [deps.md](docs/specs/deps.md), `src/services/resolver.rs` |
| Caching / disk layout | [caching.md](docs/specs/caching.md), [storage.md](docs/specs/storage.md) |
| The launcher (client/server) | [launch.md](docs/specs/launch.md) |
| Errors / exit codes | [errors.md](docs/specs/errors.md) |
| The CLI surface itself | [cli.md](docs/specs/cli.md) |

---

## Code style

The compiler-enforceable parts are in `rustfmt.toml` and clippy. The judgment-call parts:

- **No `unwrap()` in production code.** Use `?`, `.context("...")`, or `.expect("invariant that justifies the panic")`. Tests can `.unwrap()` freely.
- **No `TODO` / `FIXME` markers.** File an issue or just don't ship the half-finished code. The codebase has zero of these on purpose.
- **Comments explain *why*, not *what*.** A well-named function doesn't need a doc-comment paraphrasing its name. A subtle invariant, a CVE patch, a workaround for an API quirk — those deserve a comment.
- **No `println!` for user output.** Route through [`crate::output`](src/output.rs). See the [output channel section](docs/specs/conventions.md#1-user-output-channel) of conventions.md.
- **No new public fields on `AppContext`.** Add an accessor. See [conventions.md §2](docs/specs/conventions.md#2-appcontext-access).
- **Extract pure logic from `run()` functions.** See [conventions.md §3](docs/specs/conventions.md#3-pure-builder-extraction-pattern). The launch subsystem is the canonical example.

---

## Testing

| Tier | Command | What runs |
|---|---|---|
| Unit | `cargo test --lib` | All `#[test]` and `#[tokio::test]` inside `src/` |
| Integration (offline) | `cargo test --test integration_tests -- --test-threads=1` | Black-box CLI tests under `tests/` |
| Integration (network) | `cargo test --test integration_tests -- --ignored --test-threads=1` | The same suite plus `#[ignore]`-gated tests that hit real APIs |
| E2E | `just e2e` | The `yammm-e2e` workspace member — drives real launch scenarios |

HTTP-touching code is unit-testable via `mockito` (used in `api/streaming.rs`, `api/modrinth.rs`, `api/minecraft.rs`, `services/download.rs`). The launch subsystem extracts pure args-builders so they're testable without spawning processes — see [launch.md § Testing strategy](docs/specs/launch.md#testing-strategy).

---

## How to add a new mod source

A new mod source is a textbook walk through the architecture. You'll touch four files plus the registry:

1. **API client** in `src/api/<name>.rs` — raw HTTP wrapper. Knows endpoints, headers, response formats. Add it to `src/api/mod.rs`.
2. **Provider** in `src/providers/<name>.rs` — implements the `ModSourceProvider` trait. Translates API responses into yammm domain types (`ModInfo`, `ModVersion`, `SourceDependency`).
3. **Provider enum variant** in `src/providers/provider.rs` — extend `Provider` with the new arm, update the dispatch macro.
4. **Registry registration** in `src/providers/registry.rs` — wire up `SourceRegistry::from_config` to build your provider from `GlobalConfig` (API keys, etc.).
5. **`ModSource` variant** in `src/types/mod_info/source.rs` — add the serialization tag (used in `entry.ron`).

Tests go alongside the provider (use `providers::mock::MockSource` as a template) and the API client (use `mockito` for HTTP fixtures).

See [services.md](docs/specs/services.md) for the trait shape.

---

## How to add a new command

1. Add a file in `src/commands/<name>.rs` (or a module if it has subcommands).
2. Wire it into `src/cli.rs` as a clap subcommand variant.
3. Implement `pub async fn run(args: <YourArgs>, ctx: AppContext) -> Result<()>`.
4. Access state through [`AppContext` accessors](docs/specs/conventions.md#2-appcontext-access). Route user output through [`output::*`](docs/specs/conventions.md#1-user-output-channel).
5. Tests: extract any non-trivial data shaping into pure helpers next to `run()` and test those. See `commands/launch/client.rs` for the pattern.

Update [docs/specs/cli.md](docs/specs/cli.md) and [docs/USAGE.md](docs/USAGE.md) with the user-visible surface.

---

## Pull requests

- Keep PRs scoped. "Refactor + new feature + bug fix" lands as three PRs in this codebase.
- `just check` must pass.
- Update the relevant doc when the user-visible CLI or the public-ish surface changes (`docs/specs/cli.md`, `docs/USAGE.md`).
- A passing test for the change is expected for anything non-trivial. The bar isn't 100% coverage; it's "if I broke this, would a test catch it?"

---

## Reporting issues

- [GitHub Issues](https://github.com/AmphibianNetwork/yammm/issues)
- Include: yammm version (`yammm --version`), OS + arch, the modpack's MC version + loader, exact command you ran, exit code, and what you expected vs. what happened.
- For launch failures, attach the `--debug` log and the contents of `<modpack>/{client,server}/logs/latest.log` if present.

---

## License

By contributing you agree your changes will be licensed under the MIT license (see [LICENSE](LICENSE)).
