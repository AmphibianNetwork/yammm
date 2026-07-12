# Architecture

yammm is a Rust CLI for managing Minecraft modpacks. It's organized as a layered, acyclic architecture: high-level commands delegate to services, which call providers, which call API clients. Storage, config, and caching sit underneath as primitives.

For the conventions that hold this architecture together (output channel, `AppContext` accessors, pure-builder extractions), read [conventions.md](conventions.md) before contributing.

---

## Design principles

- **Separation of concerns.** Each layer owns one kind of concern: HTTP shape (api), domain translation (providers), business logic (services), orchestration (commands), persistence (storage), state (config), bytes-on-disk (cache).
- **Closed-set providers.** `ModSourceProvider` is implemented by a fixed enum of 3 variants (Modrinth, CurseForge, URL), dispatched manually via a macro. Avoids async-trait boxing without sacrificing polymorphism.
- **Typed errors with exit codes.** Library layers return `thiserror` enums; the CLI surface wraps them in `anyhow` for ergonomic `?` propagation. `errors::exit_code` walks the cause chain to recover the structured exit code. See [errors.md](errors.md).
- **Cache transparency.** Mods, MC JARs, libraries, and loader installs all live in the global cache, deduplicated by content hash. Commands don't manage files directly.
- **Bounded async.** Concurrent downloads use a semaphore. HTTP retry has jitter and respects `Retry-After`. No unbounded `join_all`.

---

## Layered architecture

```
┌───────────────────────────────────────────────────────────────┐
│                       CLI Layer (clap)                        │
│  cli.rs — argument parsing, dispatch                          │
├───────────────────────────────────────────────────────────────┤
│                  AppContext (shared state)                    │
│  global / modpack / registry / http_client / jar_cache /      │
│  cache_dir — all behind accessor methods (see conventions.md) │
├───────────────────────────────────────────────────────────────┤
│                       Command Layer                           │
│  16 commands: init, add, remove, search, info, update,        │
│  export, import, launch (largest — own subsystem), organize,  │
│  manage, cache, config, auth, self-update, completions        │
├───────────────────────────────────────────────────────────────┤
│                      Service Layer                            │
│  resolver (BFS dep resolver) │ download (semaphore-bounded)   │
│  mod_install / deps_install / connector / dep_graph           │
├───────────────────────────────────────────────────────────────┤
│                      Provider Layer                           │
│  Provider enum + ModSourceProvider trait                      │
│  ModrinthSource │ CurseForgeSource │ UrlSource                │
├───────────────────────────────────────────────────────────────┤
│                        API Layer                              │
│  ModrinthClient │ CurseForgeClient │ MinecraftClient │        │
│  FabricClient │ QuiltClient │ ForgeClient │ NeoForgeClient │  │
│  GitHubClient │ AdoptiumClient │ streaming │ retry            │
├───────────────────────────────────────────────────────────────┤
│             Domain / Storage / Cache / Config                 │
│  types │ EntryStore (RON) │ ManifestStore (TOML) │ JarCache   │
│  CacheManager │ GlobalConfig │ Storage facade                 │
└───────────────────────────────────────────────────────────────┘
```

Dependencies flow strictly downward. No layer imports from a layer above it; the test suite would not compile if that rule were broken.

---

## Module map

| Module             | Responsibility                                                                                                                                                       |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cli.rs`           | Top-level clap parsing, dispatch, `init_logging`                                                                                                                     |
| `app.rs`           | `App` (loaded modpack) and `AppContext` (shared CLI state) — see [conventions.md §2](conventions.md#2-appcontext-access) for the accessor contract                   |
| `commands/`        | 16 command implementations. `manage` and parts of `organize` are gated behind the `tui` feature; `organize`'s syntax highlighting is gated behind `syntax-highlight` |
| `commands/launch/` | The launch subsystem (client + server), large enough to warrant its own doc — see [launch.md](launch.md)                                                             |
| `providers/`       | `ModSourceProvider` trait, `Provider` enum, three source implementations, `SourceRegistry`, `MockSource` for tests                                                   |
| `api/`             | Raw HTTP clients (no domain types), shared retry/streaming infrastructure, Forge-style installer helpers                                                             |
| `services/`        | Business logic: dependency resolution, download orchestration, mod install / dep install pipelines                                                                   |
| `storage/`         | `EntryStore` (RON), `ManifestStore` (TOML), `JarCache` + `CacheManager`, `Storage` facade                                                                            |
| `config/`          | `GlobalConfig` (`~/.config/yammm/config.toml`), `ModpackManifest` (`modpack.toml`)                                                                                   |
| `types/`           | Domain types: `ModSource`, `TrackedMod`, `Version`, `VersionReq`, `HashType`, `LoaderType`, `ModInfo`, etc.                                                          |
| `errors/`          | `YammmError` enum with exit-code mapping; `exit_code()` chain walker                                                                                                 |
| `output.rs`        | The single user-output channel — styled helpers, capture machinery, progress bars. See [conventions.md §1](conventions.md#1-user-output-channel)                     |
| `utils/`           | Helpers: `slugify`, `format_size`, `print_error`, `current_os_name`, maven coord parsing, etc.                                                                       |
| `auth/`            | Microsoft OAuth2 device-code flow for online-mode launch                                                                                                             |

---

## `AppContext`

Built once at startup; carried through every command. Fields are private — access via methods. See [conventions.md §2](conventions.md#2-appcontext-access).

| Accessor            | Returns                | Notes                                                  |
| ------------------- | ---------------------- | ------------------------------------------------------ |
| `global()`          | `&GlobalConfig`        | Read access                                            |
| `global_mut()`      | `&mut GlobalConfig`    | Only the `config` command should reach for this        |
| `modpack()`         | `Option<&App>`         | None when invoked outside a pack                       |
| `require_modpack()` | `Result<&App>`         | Errors with `InvalidArgs` (exit 2) when not in a pack  |
| `in_modpack()`      | `bool`                 | Convenience for branchy commands                       |
| `registry()`        | `&Arc<SourceRegistry>` | Shared, cloneable for handing into tasks               |
| `http_client()`     | `&reqwest::Client`     | Shared connection pool — clone instead of constructing |
| `jar_cache()`       | `&JarCache`            | Global hash-addressed JAR store                        |
| `cache_dir()`       | `&Path`                | Resolved cache root                                    |

### Build flow

1. Load `GlobalConfig` from disk (or defaults).
2. Resolve `cwd` and build the shared `reqwest::Client` (with optional `--insecure`).
3. Resolve cache directory: `--cache-dir` arg → `YAMMM_CACHE_DIR` env → `cache_dir` config → platform default.
4. Initialize `JarCache`.
5. Walk upward from `cwd` looking for `modpack.toml` (like `git` finds `.git`); load `App` if found.
6. Build `SourceRegistry` from config (CurseForge provider activates only if an API key is present).

---

## `App`

Represents a loaded modpack:

| Field      | Type              | Source                                  |
| ---------- | ----------------- | --------------------------------------- |
| `root_dir` | `PathBuf`         | The directory containing `modpack.toml` |
| `config`   | `ModpackManifest` | Parsed `modpack.toml`                   |
| `storage`  | `Storage`         | Unified persistence facade              |
| `cache`    | `JarCache`        | Reference to the global cache           |

Three construction paths:

- `App::load(root_dir, cache)` — parse existing `modpack.toml`.
- `App::create(root_dir, cache)` — default empty config, for `init`.
- `App::from_parts(root_dir, config, cache)` — explicit, used by import.

---

## Provider vs. API layer

The cleanest seam in the codebase, and the one to study first.

**API clients** in `src/api/` are thin HTTP wrappers. They know URLs, headers, and JSON shapes; they don't know what a "mod" is. `ModrinthClient` deserializes a `ModrinthProject`, period.

**Providers** in `src/providers/` implement `ModSourceProvider`. They call API clients, translate the responses into yammm's domain types (`ModInfo`, `ModVersion`, `SourceDependency`), and add business rules — version filtering by MC version + loader, hash-type mapping, environment classification, etc.

Dispatch is via the `Provider` enum with a manual macro instead of `Box<dyn ModSourceProvider>`:

```rust
pub enum Provider {
    Modrinth(ModrinthSource),
    CurseForge(CurseForgeSource),
    Url(UrlSource),
}
```

This avoids `Pin<Box<dyn Future>>` allocations on every call without giving up the trait abstraction inside the implementations. Adding a new source requires editing the enum (closed-set, on purpose — see [services.md](services.md) for the full walkthrough).

---

## Command flow

1. **Parse** CLI args via clap in `cli.rs`.
2. **Build `AppContext`**: load config, init cache, find/load modpack, build registry.
3. **Dispatch** to the matching command's `run(args, ctx)`.
4. The command **reads through accessors** (`ctx.registry()`, `ctx.require_modpack()?`, etc.), calls **services** (`DependencyResolver`, `download_missing_mods`, ...), which call **providers**, which call **API clients**.
5. **Persist** via `EntryStore` (`entry.ron` files) and `ManifestStore` (`modpack.toml`).
6. **Emit output** through `output::*` ([conventions.md §1](conventions.md#1-user-output-channel)).
7. **Return** an `anyhow::Result<()>` from `run`; the bin in `src/bin/yammm.rs` calls `errors::exit_code(&err)` to map to a process exit code.

---

## Library surface

`src/lib.rs` is deliberately narrow. Only three symbols are public:

```rust
pub use cli::Cli;
pub use errors::exit_code;
pub use utils::print_error;
```

Everything else is `pub(crate)`. The binary at `src/bin/yammm.rs` is the only legitimate consumer of the library; tests live inside `src/` so they don't need the surface widened. New `pub` items shouldn't appear without a reason that survives a code review.

---

## Spec files

| File                             | Description                                                         |
| -------------------------------- | ------------------------------------------------------------------- |
| [cli.md](cli.md)                 | CLI commands, flags, and exit codes                                 |
| [conventions.md](conventions.md) | Code conventions (output channel, AppContext access, pure builders) |
| [config.md](config.md)           | Global and modpack configuration schemas                            |
| [storage.md](storage.md)         | On-disk layout, RON format, config file ownership                   |
| [caching.md](caching.md)         | Global cache layout and eviction                                    |
| [services.md](services.md)       | Provider trait, API clients, registry                               |
| [deps.md](deps.md)               | Dependency resolution algorithm                                     |
| [launch.md](launch.md)           | The launch subsystem (client + server)                              |
| [errors.md](errors.md)           | Error types and exit codes                                          |
