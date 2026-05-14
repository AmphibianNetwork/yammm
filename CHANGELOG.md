# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- Fix zip-slip vulnerability in Adoptium JDK extraction — validates all paths stay within the target directory
- Fix race condition in `write_secret_file` — creates files with `0o600` mode from the start

### Changed

- Atomic file writes for manifests, entry stores, and cache files (write `.tmp` + rename) to prevent corruption
- Replace atime-based LRU cache eviction with manifest-based approach (`cache_manifest.json` tracks access timestamps)
- RON entry files renamed from `mod.ron` to `entry.ron`
- Retry support added to all Microsoft authentication HTTP requests
- Auth token expiry now has a 60-second buffer to prevent edge-case failures
- CurseForge provider excluded from registry when no API key is configured
- `UrlSource::search` returns `Ok(vec![])` instead of `Err`, matching `MockSource` pattern
- Modrinth provider uses `network_error` for API failures, reserves `mod_not_found` for genuine 404s
- Blocking filesystem I/O in async contexts now uses `spawn_blocking` (asset downloads, JAR writes, cache writes)
- Installer temp directory gets RAII cleanup on drop
- `DepNode` now carries a `source` field — removed `infer_source()` that always defaulted to Modrinth
- Retry backoff now includes ±25% jitter (xorshift PRNG) to avoid thundering herd
- `maven_url()` and free `filename()` delegate to `MavenCoords::filename()` respecting classifier and extension

### Removed

- `EntryStore::new()` footgun removed (always required `mods_dir`, was never called correctly)
- Architecture violation: `services/deps_install` no longer imports from `commands` layer

### Added

- `services::mod_install` module — core non-interactive mod installation logic
- `services::dep_graph` module — shared `find_reverse_deps()` and `cleanup_stale_deps()`
- `resolve_args_array()` in `api/minecraft.rs` — eliminates ~60 lines of duplicated rule-evaluation logic
- Typed error structs: `ProjectTypeParseError`, `ModEnvParseError`, `OutputFormatParseError`, `ModSourceParseError`
- `check_for_updates` now takes `max_concurrent` param from caller config
- `JarCache::write_bytes()` uses atomic writes + existence check, matching `put()`
- `sha1` field on `TrackedMod` documented as secondary hash for MRPACK export compatibility
- 21 new unit tests for Modrinth, CurseForge, URL, and dep_graph provider logic

### Refactored

- `types/mod_info.rs` (829 lines) split into 7 modules under `types/mod_info/`
- `commands/import.rs` (913 lines) split into 5 modules under `commands/import/`
- `commands/manage/app.rs` (845 lines) split into 3 modules under `commands/manage/app/`
- `commands/manage/render.rs` (1194 lines) split into 5 modules under `commands/manage/render/`

## [0.1.0] - 2026-05-01

### Added

- **14 CLI commands**: init, add, remove, search, info, export, import, launch, auth, cache, config, update, organize, completions
- **3 mod source providers**: Modrinth, CurseForge, URL (GitHub repos and local files)
- **BFS dependency resolver** with cycle detection and optional-dependency propagation
- **Export formats**: MRPACK (Modrinth-compatible) and YMPK (native)
- **Minecraft launcher**: client and server with VFS symlinks, offline and online modes
- **Microsoft/Mojang authentication**: device code flow, token refresh, profile
- **Config organizer TUI**: interactive sorting of orphan config files into mod directories
- **Global JAR cache**: content-addressed, hash-based deduplication across modpacks
- **Shell completions**: bash, zsh, fish, elvish
- **Windows support**: signal handling, win_args.txt, directory symlinks, platform-aware paths
- **Typed error handling**: `YammmError` enum with exit codes and retryable classification
- **Retry logic**: exponential backoff with `Retry-After` header support
- **Concurrent downloads**: semaphore-limited with progress bars

### Security

- Secret files (auth tokens, API keys) written with restrictive permissions (0o600 on Unix)
- `human-panic` for friendly crash reports
