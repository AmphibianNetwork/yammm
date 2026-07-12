# Storage Specification

## Directory Layout

```
modpack/
├── modpack.toml              # Modpack metadata
├── .yammm/
│   └── ignored_configs.ron   # Ignored config file paths
├── mods/
│   └── <mod-id>/
│       ├── entry.ron         # Mod metadata
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
│       └── entry.ron
└── shaderpacks/             # Per-shader-pack directories
    └── <pack-id>/
        └── entry.ron
```

**Note:** Downloaded files (JARs, ZIPs) are stored in the **global cache** (see [caching.md](caching.md)), not in the modpack directory.

---

## Mod Storage

### entry.ron

Per-mod metadata in `mods/{mod-id}/entry.ron` (RON format for readability):

```ron
(
    id: "create",
    name: "Create",
    description: "Building Tools and Aesthetic Technology",
    version: "0.5.1+m",
    source: (type: "modrinth", id: "DHiOCvQa"),
    dependencies: [],
    url: "https://modrinth.com/mod/create",
    download_url: "https://cdn.modrinth.com/data/DHiOCvQa/versions/...",
    hash: Some("abc123...456"),
    hash_type: Sha512,
    project_type: Mod,
    env: Both,
    categories: ["technology", "aesthetic"],
    filename: Some("create-0.5.1+m.jar"),
    unresolved: false,
    connector_compat: false,
)
```

### Source Types in RON

`ModSource` uses internally-tagged serialization (`#[serde(tag = "type", rename_all = "lowercase")]`):

**Modrinth:**
```ron
source: (type: "modrinth", id: "DHiOCvQa"),
```

**CurseForge:**
```ron
source: (type: "curseforge", project_id: "238222"),
```

**Url:**
```ron
source: (type: "url", url: "https://example.com/mod.jar"),
```

GitHub repos use the `Url` variant:
```ron
source: (type: "url", url: "https://github.com/IrisShaders/Iris"),
```

Local files use the `Url` variant with a `file://` scheme:
```ron
source: (type: "url", url: "file:///home/user/mods/my-mod.jar"),
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | String | Unique mod identifier (slug) |
| `name` | String | Human-readable mod name |
| `description` | String | Short description |
| `version` | String | Version number |
| `source` | ModSource | Source and upstream ID (internally-tagged) |
| `dependencies` | Vec\<Dependency\> | List of dependencies |
| `url` | String | URL to the mod page |
| `download_url` | String | Direct download URL |
| `hash` | Option\<String\> | SHA-512 hash of JAR file |
| `hash_type` | HashType | Hash algorithm (Sha512 default) |
| `project_type` | ProjectType | `Mod`, `ResourcePack`, or `Shader` |
| `env` | ModEnv | `Both`, `Client`, or `Server` |
| `categories` | Vec\<String\> | Category tags from the source |
| `filename` | Option\<String\> | Original filename of the downloaded JAR |
| `unresolved` | bool | Whether this mod's dependencies still need resolution |
| `connector_compat` | bool | Compatibility flag for Fabric-Quilt connector |

---

## Storage Facade

`Storage` in `storage/mod.rs` is the unified interface for all persistence operations:

- Holds paths to `mods/`, `resourcepacks/`, and `shaderpacks/` directories
- Dispatches load/save/remove/list to the appropriate `EntryStore` based on `ProjectType`
- `find_any()` searches across all three stores
- `list_all()` lists items across all project types
- Delegates modpack config I/O to `ManifestStore`

### EntryStore

Each store manages `entry.ron` files in per-item subdirectories:

- `exists(id)` — check if an entry is installed
- `load(id)` — read and parse `entry.ron`
- `save(id, tracked_mod)` — serialize and write `entry.ron` (atomic write via .tmp + rename)
- `remove(id)` — delete the entire entry directory
- `list()` — scan for subdirectories containing `entry.ron`

All `Storage` methods require a `ProjectType` parameter for dispatch:

- `exists(project_type, id)`
- `load(project_type, id)`
- `save(project_type, id, tracked_mod)`
- `remove(project_type, id)`
- `list(project_type)`

---

## Concurrency Model

yammm assumes **a single writer per modpack at a time**. The CLI is short-lived
and operates from one process, so this is the realistic case. The pieces line
up with that assumption:

- **`EntryStore`** writes `entry.ron` files atomically via `.tmp` + `rename`,
  but does *not* hold an OS-level lock. Concurrent `yammm add` processes
  against the same modpack are undefined behavior — last writer wins on each
  individual `entry.ron`, and the manifest tail can interleave.
- **`ManifestStore`** writes `modpack.toml` the same way: atomic file
  replacement, no lock. A second concurrent writer can clobber the first.
- **`JarCache`** is the exception — its in-memory LRU manifest sits behind
  an `Arc<Mutex<>>` and handles poisoning. It's the only storage component
  designed for shared, multi-threaded access (the launcher and downloader
  read it from the same async runtime). The on-disk manifest still uses
  atomic write, so a second yammm process can't corrupt it, but the two
  processes will overwrite each other's LRU bookkeeping.

If you ever need true multi-process safety (e.g., a long-running daemon),
add a `flock`-style lock file at the modpack root and acquire it in
`App::load` / `App::create`. Until then: don't run two `yammm` writes
against the same modpack at once.

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
