# Configuration Specification

## Overview

yammm uses two configuration files:
1. **Global config** (`~/.config/yammm/config.toml`): User preferences and API keys
2. **Modpack config** (`modpack.toml`): Minecraft version, loader, modpack metadata

Mods, resource packs, and shader packs are tracked in individual `.ron` files, not in `modpack.toml`. See [storage.md](storage.md).

---

## Global Configuration

### Location

| Platform | Path |
|----------|------|
| Linux | `~/.config/yammm/config.toml` |
| macOS | `~/Library/Application Support/yammm/config.toml` |
| Windows | `%APPDATA%/yammm/config.toml` |

Override with `YAMMM_CONFIG` environment variable.

### Schema

```toml
default_modpack_dir = ""
cache_dir = ""
cache_max_size_mb = 5000

[api_keys]
curseforge = ""

[output]
format = "table"
color = true
```

### Fields

#### Root Level

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_modpack_dir` | string | `""` | Default directory for modpack operations |
| `cache_dir` | string | `""` | Custom cache directory (platform default if empty) |
| `cache_max_size_mb` | integer | `5000` | Maximum cache size in MB (0 = unlimited) |

#### `[api_keys]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `curseforge` | string | `""` | CurseForge API key (required for CurseForge operations) |

#### `[output]` Section

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `format` | string | `"table"` | Output format: `table`, `compact`, `json` |
| `color` | boolean | `true` | Enable colored output |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `YAMMM_CONFIG` | Override global config file path |
| `YAMMM_CACHE_DIR` | Override cache directory (takes precedence over `cache_dir` in config) |
| `YAMMM_DEBUG` | Enable debug logging |
| `CURSEFORGE_API_TOKEN` | CurseForge API key (alternative to config file) |

---

## Modpack Configuration

### Location

`modpack.toml` in the modpack root directory.

### Schema

```toml
name = "My Modpack"
description = "A custom modpack"
version = "1.0.0"
minecraft_version = "1.20.4"

[loader]
loader = "fabric"
version = "0.15.11"
```

### Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | No | `""` | Modpack display name |
| `description` | string | No | `""` | Modpack description |
| `version` | string | No | `""` | Modpack version (used during export) |
| `minecraft_version` | string | No | `""` | Target Minecraft version (e.g., `1.20.4`) |
| `mod_path` | string | No | `None` | Custom mods directory path (defaults to `mods/`) |

#### `[loader]` Section

| Field | Type | Required | Default | Description |
|-------|------|---------|---------|-------------|
| `loader` | string | No | `""` | Loader type: `fabric`, `forge`, `neoforge`, `quilt` |
| `version` | string | No | `""` | Loader version string |

### Notes

- `modpack.toml` only contains **modpack-wide** settings. Individual mods, resource packs, and shader packs are tracked in their respective `.ron` files.
- `minecraft_version` is a top-level field (not nested under `[minecraft]`).
- The `loader` field inside `[loader]` uses the key name `loader` (not `type`).
- Downloaded JARs are stored in the **global cache**, not in the modpack directory.

---

## Modpack Directory Structure

After running `yammm init`, the modpack directory contains:

```
my-modpack/
├── modpack.toml          # Modpack configuration
├── .yammm/
│   └── ignored_configs.ron  # Ignored config file paths
├── mods/                 # Mod metadata directories
│   └── {mod-id}/
│       └── mod.ron       # Per-mod metadata (source, version, hash, dependencies)
├── resourcepacks/       # Resource pack metadata directories
│   └── {pack-id}/
│       └── pack.ron
├── shaderpacks/         # Shader pack metadata directories
│   └── {pack-id}/
│       └── pack.ron
├── config/               # Fallback config files
├── resources/
│   ├── client/           # Client-specific global files (options.txt, etc.)
│   └── server/           # Server-specific global files (server.properties, etc.)
├── .gitignore            # Ignores build artifacts
└── README.md             # Basic README with next steps
```
