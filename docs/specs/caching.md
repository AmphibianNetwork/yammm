# Caching Specification

## Overview

yammm caches downloaded files in a **global cache** to:
- Enable offline operation (mods only need to be downloaded once)
- Avoid re-downloading the same mods across modpacks
- Save bandwidth and disk space through hash-based deduplication

All three cache subdirectories are managed by the `CacheManager`:

| Subdirectory | Content | Eviction strategy |
|---|---|---|
| `jars/` | Mod JARs (hash-based) | LRU by manifest-recorded access time |
| `minecraft/` | MC version JARs, libraries, assets | LRU by version directory modification time |
| `loaders/` | Fabric/Quilt/Forge/NeoForge libraries | LRU by version directory modification time |

---

## Global Cache Directory

| Platform | Path |
|----------|------|
| Linux | `~/.cache/yammm/` |
| macOS | `~/Library/Caches/yammm/` |
| Windows | `%LOCALAPPDATA%/yammm/` |

Override with `YAMMM_CACHE_DIR` environment variable or `cache_dir` in global config.

---

## Cache Structure

```
~/.cache/yammm/
├── jars/               # Mod JARs (managed by JarCache)
│   ├── sha512_abc123...456.jar
│   ├── sha512_def789...012.jar
│   └── ...
├── minecraft/          # Minecraft client/server JARs (managed by launch code)
│   └── {version}/
│       ├── client.jar
│       ├── server.jar
│       └── libraries/
└── loaders/            # Mod loader files (managed by launch code)
    ├── fabric/
    │   └── {version}/
    ├── forge/
    │   └── {version}/
    ├── neoforge/
    │   └── {version}/
    └── quilt/
        └── {version}/
```

**Note:** The `minecraft/` and `loaders/` subdirectories are managed by `CacheManager` at the version-directory level (entire versions are evicted together to avoid inconsistent state).

---

## JAR File Naming

JAR files are named by their hash to enable deduplication:

```
~/.cache/yammm/jars/
├── sha512_abc123...456.jar
├── sha512_def789...012.jar
└── ...
```

The default hash algorithm is **SHA-512**. Other supported algorithms (SHA-1, SHA-256, MD5) use their respective prefix (e.g., `sha1_`, `sha256_`, `md5_`).

This allows sharing mod files across all modpacks without duplication.

---

## Cache Lookup

Before downloading a mod:

1. Get the hash from the mod source API or from the stored `entry.ron`
2. Check if the corresponding file exists in `~/.cache/yammm/jars/`
3. If yes, use the cached file (symlink or reference it)
4. If no, download and save to global cache

No path is stored in `entry.ron` — only the hash is needed to locate the file.

---

## Download with Retry

The download manager in `services/download.rs` includes retry logic:

1. Download file to a temporary path
2. Verify hash after download
3. If hash mismatch, retry up to 3 times with exponential backoff
4. On success, write to cache via `JarCache::write_bytes()` or `JarCache::put()`
5. On persistent failure, remove partial file and return error

The `JarCache` itself does not have a `download()` method — it provides `get()`, `put()`, `write_bytes()`, `remove()`, `contains()`, `count()`, `size()`, `clear()`, and `cleanup()`.

---

## Cache Management

### `yammm cache status`

Shows total file count and size for all three subdirectories (`jars/`, `minecraft/`, `loaders/`) and the grand total.

### `yammm cache clean`

Removes oldest files across all subdirectories until the total cache size is under the configured threshold (`cache_max_size_mb` in global config, default 5000 MB). Eviction order: JARs first (by manifest-recorded access time), then Minecraft versions (by directory modification time), then loader versions (by directory modification time).

**LRU Mechanism:** The `JarCache` maintains a `cache_manifest.json` that records last-access timestamps. This is used instead of filesystem `atime` because atime is unreliable (noatime mounts, etc.). Minecraft and loader version directories use `mtime` (modification time) for ordering — also because atime is unreliable.

### `yammm cache obliterate`

Removes the entire cache directory. Prompts for confirmation.

---

## Extended Cache Structure

### Minecraft JARs

```
~/.cache/yammm/minecraft/{version}/
├── client.jar
├── server.jar
└── libraries/
    └── {lib-name}/{version}/
        └── {lib-name}-{version}.jar
```

Downloaded by the launch code from Mojang's piston-meta API. Cached by version.

### Mod Loaders

```
~/.cache/yammm/loaders/
├── fabric/{version}/
│   ├── launcher-install.jar
│   ├── loader.jar
│   └── server.jar
├── forge/{version}/
│   └── installer.jar
├── neoforge/{version}/
│   └── installer.jar
└── quilt/{version}/
    ├── launcher-install.jar
    ├── loader.jar
    └── server.jar
```

Downloaded by the launch code from loader-specific APIs. Cached by loader type and version.

---

## Configuration

```toml
# ~/.config/yammm/config.toml
cache_dir = "/custom/cache"     # Custom cache directory (None = platform default)
cache_max_size_mb = 5000        # Max cache size in MB (None = 5000 default, 0 = unlimited)
```

The size limit is checked by `yammm cache clean` but is not enforced automatically. Use `yammm cache clean` or `yammm cache obliterate` to manage cache size.

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `YAMMM_CACHE_DIR` | Override cache directory (takes precedence over config) |
