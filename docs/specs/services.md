# Services Specification

## Overview

yammm separates mod source interactions into two layers:

- **API clients** (`api/`): Raw HTTP wrappers for external services
- **Providers** (`providers/`): Implement the `ModSourceProvider` trait, translating API responses into domain types

Commands interact with the `Provider` enum, never with API clients directly.

---

## ModSourceProvider Trait

The core abstraction that unifies all 3 mod sources. Uses native async fn in trait (Rust 1.75+), not `async_trait`:

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

- **`api/retry.rs`**: Exponential backoff with `Retry-After` header support. Separate configs for general API calls (`API_RETRY_CONFIG`, max 3 retries) and auth flow calls (`AUTH_RETRY_CONFIG`, max 3 retries). `send_retried_request()` for API calls, separate auth retry via `send_retried_request` with auth config.
- **`api/installer/`**: Directory with 5 files: `mod.rs`, `libraries.rs`, `processors.rs`, `profile.rs`, `templates.rs`. Shared Forge/NeoForge installer logic.

---

## Business Logic Services

### Dependency Resolver (`services/resolver.rs`)

BFS-based resolver that traverses the dependency graph starting from a root mod. See [deps.md](deps.md) for details.

### Download Manager (`services/download.rs`)

Finds mods without cached JARs and downloads them concurrently using a tokio semaphore (default 8 concurrent tasks, configurable via `max_concurrent_downloads`). Includes hash verification and retry logic. Returns a `DownloadSummary` with success/failure counts.

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
