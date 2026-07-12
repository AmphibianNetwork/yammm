# Services Specification

## Overview

yammm separates mod source interactions into two layers:

- **API clients** (`api/`): Raw HTTP wrappers for external services
- **Providers** (`providers/`): Implement the `ModSourceProvider` trait, translating API responses into domain types

Commands interact with the `Provider` enum, never with API clients directly.

---

## ModSourceProvider Trait

The core abstraction that unifies all 3 mod sources. Uses native `async fn` in trait (stable since Rust 1.75; this crate's MSRV is 1.88), not the `async_trait` crate:

```rust
#[allow(async_fn_in_trait)]
pub trait ModSourceProvider {
    fn name(&self) -> &str;
    fn supports_search(&self) -> bool;
    fn get_mod_env(&self, mod_info: &ModInfo) -> ModEnv;

    async fn search(&self, query: &str, filters: &SearchFilters) -> Result<Vec<ModInfo>>;
    async fn get_mod(&self, mod_id: &str) -> Result<ModInfo>;
    async fn get_versions(&self, mod_id: &str, filters: &VersionFilters) -> Result<Vec<ModVersion>>;
    async fn get_dependencies(&self, mod_id: &str, version_id: &str) -> Result<Vec<SourceDependency>>;
}
```

### Provider Enum

`Provider` is a closed enum with manual dispatch via the `dispatch!` macro, instead of `dyn ModSourceProvider`:

```rust
pub enum Provider {
    Modrinth(ModrinthSource),
    CurseForge(CurseForgeSource),
    Url(UrlSource),
}
```

The `Provider` enum adds `get_latest_version()` which is not on the trait — it fetches all versions and picks the one with the most recent release date.

### Provider Implementations

| Provider | Search | Dependencies | Notes |
|----------|--------|-------------|-------|
| `ModrinthSource` | Yes | Yes | Full support, client-side MC version/loader filtering |
| `CurseForgeSource` | Yes | Yes | Requires API key, server-side filtering |
| `UrlSource` | No | Empty | Handles GitHub release URLs (https://github.com/owner/repo), file:// URLs for local files, and direct HTTP(S) download URLs. Internally delegates to GitHubClient for GitHub URLs and reads local files for file:// URLs. |

### Source Registry

`SourceRegistry` maps `SourceKey` enum variants to `Provider` instances. Built once during `AppContext::init()` from the global config.

---

## API Clients

Each API client handles raw HTTP communication with a specific service. They know about endpoints, headers, and response formats but nothing about yammm's domain types.

### Modrinth API

**Base URL:** `https://api.modrinth.com/v2`

| Endpoint | Method | Description |
|----------|--------|-------------|
| `GET /search` | GET | Search projects (facets for loader/MC version) |
| `GET /project/{id}` | GET | Get project details |
| `GET /project/{id}/version` | GET | List versions (query params for filtering) |
| `GET /version/{id}` | GET | Get version details |
| `GET /version/{id}/dependencies` | GET | Get version dependencies |
| `GET /version_file/{hash}` | GET | Lookup version by file hash |

### CurseForge API

**Base URL:** `https://api.curseforge.com`

**Authentication:** Requires `x-api-key` header (set via `api_keys.curseforge` in config).

| Endpoint | Method | Description |
|----------|--------|-------------|
| `GET /v1/mods/search` | GET | Search addons (gameid, modLoader, gameVersion params) |
| `GET /v1/mods/{id}` | GET | Get addon details |
| `GET /v1/mods/{id}/files` | GET | List files (versions) |
| `GET /v1/mods/{id}/files/{fileId}` | GET | Get file details |
| `GET /v1/mods/files/{fileId}/download-url` | GET | Get download URL |

### GitHub API

**Base URL:** `https://api.github.com`

| Endpoint | Method | Description |
|----------|--------|-------------|
| `GET /repos/{owner}/{repo}/releases` | GET | List releases |
| `GET /repos/{owner}/{repo}/releases/tags/{tag}` | GET | Get release by tag |

Finds the primary JAR asset from release assets (filters by `.jar` extension, prefers non-sources/non-javadoc).

> **Note:** GitHubClient is used internally by `UrlSource` to handle GitHub URLs. It is not directly exposed as a `ModSourceProvider`.

### Minecraft API (Mojang)

**Base URL:** `https://piston-meta.mojang.com`

| Endpoint | Method | Description |
|----------|--------|-------------|
| `GET /mc/game/version_manifest_v2.json` | GET | List all Minecraft versions |
| `GET {version_url}` | GET | Get version details (downloads, libraries) |

### Loader APIs

| Loader | Base URL | Purpose |
|--------|----------|---------|
| Fabric | `https://meta.fabricmc.net/v2` | Loader versions, profile JSON, library downloads |
| Quilt | `https://meta.quiltmc.org/v3` | Same pattern as Fabric |
| Forge | `https://maven.minecraftforge.net` | Version metadata XML, installer download |
| NeoForge | `https://maven.neoforged.net/releases` | Same pattern as Forge with MC version prefix mapping |

### Shared Infrastructure

- **`api/retry.rs`**: Exponential backoff with ±25% jitter and `Retry-After` header support. Two presets:
  - `API_RETRY_CONFIG`: 3 retries, 500ms initial — general API calls.
  - `AUTH_RETRY_CONFIG`: 2 retries, 1s initial — Microsoft OAuth steps.
  Retryable on 429 / 5xx / network errors; fast-fail on 4xx.
- **`api/streaming.rs`**: Chunked HTTP download into a temp file with hash verification; CPU-bound hashing runs inside `tokio::spawn_blocking`.
- **`api/installer/`**: Shared Forge/NeoForge installer pipeline (`mod.rs`, `libraries.rs`, `processors.rs`, `profile.rs`, `templates.rs`).

---

## Business Logic Services

### Dependency Resolver (`services/resolver.rs`)

BFS-based resolver that traverses the dependency graph starting from a root mod. Required deps are dequeued before optional ones (fail fast); optional deps that fail to resolve are logged and skipped. Cycles are detected via ancestor tracking; raw-project-ID vs slug references are deduplicated by canonical key. See [deps.md](deps.md) for details.

### Download Manager (`services/download.rs`)

Finds mods without cached JARs and downloads them concurrently using a tokio semaphore (default 8 concurrent tasks, configurable via `max_concurrent_downloads`). Streams to a temp file, hashes from disk, atomically renames into the cache only on hash match. Returns a `DownloadSummary` whose `.into_result()` aggregates failures while preserving the first error's variant for [exit code](errors.md) recovery.

---

## Rate Limiting

| Source | Unauthenticated | Authenticated |
|--------|----------------|---------------|
| Modrinth | ~5 req/sec | Same (no auth) |
| CurseForge | Very limited | ~5000 req/day |
| GitHub | 60 req/hr | 5000 req/hr |

**Strategy:**
- Exponential backoff on 429 responses (3 retries max)
- Cache API responses to reduce calls
- CurseForge requires API key for any meaningful use
