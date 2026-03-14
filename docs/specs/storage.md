# Storage Specification

## Directory Layout

```
modpack/
├── modpack.toml              # Modpack metadata
├── .yammm/
│   └── ignored_configs.ron   # Ignored config file paths
├── mods/
│   └── <mod-id>/
│       ├── mod.ron           # Mod metadata
│       ├── config/           # Common configs (client + server)
│       ├── client/
│       │   └── config/       # Client-only configs
│       └── server/
│           └── config/       # Server-only configs
├── config/                   # Fallback for unknown mod ownership
├── resources/
│   ├── client/               # Client-specific global files
│   └── server/               # Server-specific global files
├── resourcepacks/           # Per-resource-pack directories
│   └── <pack-id>/
│       └── pack.ron
└── shaderpacks/             # Per-shader-pack directories
    └── <pack-id>/
        └── pack.ron
```

**Note:** Downloaded files (JARs, ZIPs) are stored in the **global cache** (see [caching.md](caching.md)), not in the modpack directory.

---

## Mod Storage

### mod.ron

Per-mod metadata in `mods/{mod-id}/mod.ron` (RON format for readability):

```ron
(
    id: "create",
    name: "Create",
    description: "Building Tools and Aesthetic Technology",
    version: "0.5.1+m",
    source: Modrinth(id: "DHiOCvQa"),
    dependencies: [
        (
            mod_id: "fabric-api",
            source: Modrinth(id: "P7dR8mAK"),
            kind: required,
        ),
    ],
    url: "https://modrinth.com/mod/create",
    download_url: "https://cdn.modrinth.com/data/DHiOCvQa/versions/...",
    hash: Some("abc123...456"),
    hash_type: Sha512,
    project_type: Mod,
    env: Both,
)
```

### Source Types in RON

**Modrinth:**
```ron
source: Modrinth(id: "DHiOCvQa"),
```

**CurseForge:**
```ron
source: CurseForge(project_id: "238222"),
```

**Url:**
```ron
source: Url(url: "https://example.com/mod.jar"),
```

GitHub repos use the `Url` variant:
```ron
source: Url(url: "https://github.com/IrisShaders/Iris"),
```

Local files use the `Url` variant with a `file://` scheme:
```ron
source: Url(url: "file:///home/user/mods/my-mod.jar"),
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | String | Unique mod identifier (slug) |
| `name` | String | Human-readable mod name |
| `description` | String | Short description |
| `version` | String | Version number |
| `source` | ModSource | Source and upstream ID |
| `dependencies` | Array | List of dependencies |
| `url` | String | URL to the mod page |
| `download_url` | String | Direct download URL |
| `hash` | Option<String> | SHA-512 hash of JAR file |
| `hash_type` | HashType | Hash algorithm (Sha512 default) |
| `project_type` | ProjectType | `Mod`, `ResourcePack`, or `Shader` |
| `env` | ModEnv | `Both`, `Client`, or `Server` |

---

## Storage Facade

`Storage` in `storage/mod.rs` is the unified interface for all persistence operations:

- Holds paths to `mods/`, `resourcepacks/`, and `shaderpacks/` directories
- Dispatches load/save/remove/list to the appropriate `ModStore` based on `ProjectType`
- `find_any()` searches across all three stores
- Delegates modpack config I/O to `ModpackConfig`

### ModStore

Each store manages `.ron` files in per-item subdirectories:

- `exists(slug)` — check if a mod is installed
- `load(slug)` — read and parse `mod.ron`
- `save(mod_ron)` — serialize and write `mod.ron`
- `remove(slug)` — delete the entire mod directory
- `list()` — scan for subdirectories containing `mod.ron`

---

## Config Storage

Config files are stored in multiple locations based on scope and ownership:

| Location | Priority | Description | Symlinked To |
|----------|----------|-------------|-------------|
| `mods/<id>/client/config/` | 1 (highest) | Client-specific configs | `./client/config/` |
| `mods/<id>/server/config/` | 1 (highest) | Server-specific configs | `./server/config/` |
| `mods/<id>/config/` | 2 | Common configs (both) | Both `./client/config/` and `./server/config/` |
| `config/` | 3 (lowest) | Fallback/unknown ownership | Both `./client/config/` and `./server/config/` |

During launch, config files are **symlinked** from workspace to launch directories. Changes to config files in the workspace are immediately reflected during launch.

### Ignored Configs

Ignored configs are tracked in `.yammm/ignored_configs.ron`:

```ron
(
    client: [
        "logs/access.log",
        "options.txt.backup",
    ],
    server: [
        "logs/server.log",
        "auto_save.dat",
    ],
)
```

Paths are relative to the launch directory. Files in the ignore list are skipped during `yammm organize`.

---

## Resources Storage

The `resources/` directory stores modpack-level files that don't belong to specific mods:

```
resources/
├── client/           # Client-specific global files (symlinked to ./client/)
│   ├── options.txt
│   └── lang/
└── server/           # Server-specific global files (symlinked to ./server/)
    ├── server.properties
    ├── eula.txt
    └── ops.json
```

Files in `resources/client/` are symlinked to `./client/` root during launch. Files in `resources/server/` are symlinked to `./server/` root.

---

## Organize Command

The `yammm organize` command sorts orphan config files into the proper directories:

1. **Scan** for orphan configs in `./client/config/` or `./server/config/`
2. **Filter out** configs already tracked (symlinked from workspace)
3. **Filter out** configs in the ignore list
4. **For each orphan**, display interactive TUI prompt with:
   - File name and relative path
   - Syntax-highlighted content preview
   - Fuzzy search to find the corresponding mod
   - Destination selection (common, client-only, server-only, fallback, or ignore)
5. **Copy** file to selected location
6. **Update** ignore list for ignored files
