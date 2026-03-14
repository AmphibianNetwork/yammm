# Architecture Specification

## Overview

yammm is a Rust CLI for managing Minecraft modpacks. It uses a layered architecture where high-level commands delegate to focused modules that handle specific responsibilities.

---

## Design Principles

- **Separation of concerns**: Each module handles one domain (API clients, providers, storage, caching)
- **Abstraction over sources**: `ModSourceProvider` trait unifies all 3 mod sources behind a common interface
- **Cache transparency**: `JarCache` handles all JAR storage and deduplication; commands don't manage files directly
- **Graceful failures**: No panics, proper error propagation with typed `ErrorKind` + exit codes

---

## Layered Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      CLI Layer (clap)                       │
│  cli.rs — parses arguments, dispatches to commands          │
├─────────────────────────────────────────────────────────────┤
│                 AppContext (global state)                    │
│  GlobalConfig │ SourceRegistry │ App │ JarCache │ HTTP      │
├─────────────────────────────────────────────────────────────┤
│                    Command Layer                             │
│  13 commands: init, add, remove, search, info, update,      │
│  export, import, launch, cache, config, organize,           │
│  completions                                                │
├─────────────────────────────────────────────────────────────┤
│                   Provider Layer                             │
│  ModSourceProvider trait + 3 implementations:               │
│  ModrinthSource │ CurseForgeSource │ UrlSource               │
├─────────────────────────────────────────────────────────────┤
│                     API Layer                                │
│  Raw HTTP clients: ModrinthClient, CurseForgeClient,        │
│  MinecraftClient, FabricClient, QuiltClient,                │
│  ForgeClient, NeoForgeClient                                │
├─────────────────────────────────────────────────────────────┤
│          Domain / Storage / Cache / Config                   │
│  Types (ModSource, ModRon, Version, HashType, ...)          │
│  ModStore (RON) │ ModpackConfig (TOML) │ JarCache           │
│  GlobalConfig (TOML) │ Storage (facade)                     │
└─────────────────────────────────────────────────────────────┘
```

---

## Module Map

| Module       | Responsibility                                                                 |
| ------------ | ------------------------------------------------------------------------------ |
| `cli.rs`     | Argument parsing (clap), command dispatch                                      |
| `app.rs`     | `App` (loaded modpack), `AppContext` (global CLI state)                        |
| `commands/`  | 13 command implementations                                                     |
| `providers/` | `ModSourceProvider` trait + 3 source implementations + registry                |
| `api/`       | Raw HTTP clients for external services                                         |
| `services/`  | Business logic: dependency resolver, download manager                          |
| `storage/`   | Persistence: `ModStore` (RON), `JarCache`, `ModpackStore`, `Storage` facade    |
| `config/`    | Global and modpack configuration (TOML-based)                                  |
| `types/`     | Domain types: `ModSource`, `ModRon`, `Version`, `VersionReq`, `HashType`, etc. |
| `errors.rs`  | Typed error classification (`ErrorKind`) with exit-code mapping                |
| `output.rs`  | Terminal output formatting (colors, progress bars, tables)                     |
| `utils/`     | Helpers: `slugify`, `format_size`, `print_error`, etc.                         |

---

## AppContext and App

### AppContext

Global state shared across all commands, created once at startup:

- `global: GlobalConfig` — user preferences from `~/.config/yammm/config.toml`
- `modpack: Option<App>` — current modpack (None if not in a modpack directory)
- `cwd: PathBuf` — current working directory
- `registry: Arc<SourceRegistry>` — provider registry (maps source keys to providers)
- `http_client: reqwest::Client` — shared HTTP client
- `insecure: bool` — SSL verification disabled flag

Initialization flow:
1. Load global config from disk (or use defaults)
2. Build HTTP client (with optional insecure mode)
3. Initialize `JarCache`
4. Find and load modpack if `modpack.toml` exists in CWD or `--config` path
5. Build `SourceRegistry` from config (CurseForge provider requires API key)

### App

Represents a loaded modpack:

- `root_dir: PathBuf` — modpack root directory
- `config: ModpackConfig` — parsed `modpack.toml`
- `storage: Storage` — unified storage facade
- `cache: JarCache` — global JAR cache

Three construction paths:
- `App::new()` — explicit config
- `App::load()` — from existing `modpack.toml`
- `App::create()` — default/empty config (for `init`)

---

## Provider vs API Layer

The key architectural split is between **API clients** (`api/`) and **providers** (`providers/`):

- **API clients** are thin HTTP wrappers that handle request/response serialization for specific services. They know about HTTP headers, endpoints, and response formats but nothing about yammm's domain.

- **Providers** implement the `ModSourceProvider` trait and translate API client responses into yammm's domain types (`ModInfo`, `ModVersion`, `Dependency`). They also add business logic like version filtering, loader matching, and hash type mapping.

This separation means adding a new mod source requires:
1. An API client in `api/` (HTTP concerns)
2. A provider in `providers/` (domain translation)
3. Registration in `SourceRegistry`

---

## Command Flow

1. **Parse CLI** via clap in `cli.rs`
2. **Build AppContext**: load config, init cache, find/load modpack, build registry
3. **Execute command**: each command accesses `AppContext` for state
4. **Delegate to providers/services**: commands call `ModSourceProvider` methods or `DependencyResolver` / download manager
5. **Persist changes**: `ModStore` saves `.ron` files, `ModpackConfig` saves `modpack.toml`
6. **Return exit code** via `errors::exit_code()` — typed `ErrorKind` with legacy fallback

---

## Spec Files

| File                       | Description                           |
| -------------------------- | ------------------------------------- |
| [cli.md](cli.md)           | CLI commands and options              |
| [config.md](config.md)     | Configuration (global + modpack)      |
| [storage.md](storage.md)   | Local file structure and RON format   |
| [services.md](services.md) | Provider trait, API clients, registry |
| [deps.md](deps.md)         | Dependency resolution algorithm       |
| [caching.md](caching.md)   | JAR file caching                      |
| [errors.md](errors.md)     | Error types and exit codes            |
