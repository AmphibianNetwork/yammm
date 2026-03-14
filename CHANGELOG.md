# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
