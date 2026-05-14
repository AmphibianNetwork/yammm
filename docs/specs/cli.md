# CLI Specification

## Global Options

```
-d, --debug            Enable debug mode (verbose logging)
-C, --config <PATH>    Path to modpack directory or modpack.toml
--insecure             Disable SSL verification for HTTP requests
--help                 Print help
--version              Print version
```

**Warning:** `--insecure` disables SSL certificate verification. Only use with trusted local servers.

---

## Commands

### `init`

Initialize a new modpack workspace.

```
yammm init [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--name` | `-n` | Interactive | Modpack name |
| `--description` | | Interactive | Modpack description |
| `--minecraft-version` | `-V` | Interactive | Minecraft version |
| `--loader` | `-L` | Interactive | Loader: `fabric`, `forge`, `neoforge`, `quilt` |
| `--loader-version` | | `""` | Loader version |
| `--output-dir` | `-o` | Current directory | Output directory |
| `--interactive` | | `false` | Force interactive mode even when other flags are provided |

When no flags are given, the command runs interactively (prompts for name, version, loader). When any flag is provided, it runs non-interactively with defaults for missing values.

---

### `add <IDENTIFIER>`

Add a mod to the current profile.

```
yammm add <IDENTIFIER> [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--source` | `-s` | `modrinth` | Source: `modrinth`, `curseforge` |
| `--version` | `-v` | `None` | Specific mod version (omit for latest) |
| `--loader` | `-l` | Profile default | Override loader: `fabric`, `forge`, `neoforge`, `quilt` |
| `--yes` | `-y` | `false` | Skip confirmation prompts |
| `--force` | `-f` | `false` | Force add without confirmation |
| `--env` | | `None` | Side: `client`, `server`, `both` (defaults to `both` when omitted) |
| `--project-type` | | `None` | Project type: `mod`, `resourcepack`, `shader` (defaults to `mod` when omitted) |
| `--categories` | `-t` | `[]` | Category tags (comma-separated) |

**URL auto-detection:** When `<IDENTIFIER>` starts with `http://`, `https://`, or `file://`, the URL source is used automatically regardless of `--source`.

**Behavior:**
- Automatically resolves dependencies via BFS traversal
- Prompts user whether to add each dependency
- If exact mod not found on Modrinth, enters search mode
- Does NOT download files — download happens during `launch` or `export`

---

### `remove <IDENTIFIER>`

Remove a mod from the current profile.

```
yammm remove <IDENTIFIER> [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--yes` | `-y` | `false` | Skip confirmation prompts |
| `--force` | | `false` | Remove without checking for dependent mods |

**Behavior:**
- By default, checks for mods that depend on the target and warns the user
- With `--force`, removes immediately without dependency checks
- Removes the mod directory (`mods/{mod-id}/`) including `entry.ron`
- Does NOT remove the JAR from global cache

---

### `info [SUBCOMMAND]`

Show information about the modpack or individual mods.

#### `info` (no subcommand)

Show modpack overview.

#### `info list [-v]`

List all mods, resource packs, and shader packs.

| Option | Short | Description |
|--------|-------|-------------|
| `--verbose` | `-v` | Show detailed information |

#### `info mod <ID>`

Show detailed information about a specific mod.

#### `info tree`

Show dependency tree.

---

### `search <QUERY>`

Search for mods on a single source.

```
yammm search <QUERY> [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--source` | `-s` | `modrinth` | Source: `modrinth`, `curseforge` |
| `--limit` | `-n` | `20` | Max results |
| `--loader` | `-l` | Profile default | Filter by loader |
| `--output` | `-o` | `table` | Output format: `table`, `compact` |

**Note:** Minecraft version is taken from `modpack.toml`. URL source does not support search.

---

### `update`

Check for updates to installed mods.

```
yammm update [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--yes` | `-y` | `false` | Auto-confirm all updates |

Queries each mod's source API for newer versions compatible with the profile's Minecraft version and loader.

---

### `export [OPTIONS]`

Export the current modpack.

```
yammm export [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--format` | `-f` | `mrpack` | Format: `mrpack`, `ympk` |
| `--output` | `-o` | `{name}-{timestamp}.{ext}` | Output file path |
| `--yes` | `-y` | `false` | Skip confirmation prompts |

**Formats:**
- `mrpack`: Modrinth modpack format — contains `modrinth.index.json` + mod JARs
- `ympk`: yammm native format — contains `modpack.toml` + `entry.ron` files + config files

Downloads missing mods from upstream before packaging.

---

### `import <FILE>`

Import a modpack from MRPACK or YMPK format.

```
yammm import <FILE> [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--output` | `-o` | | Extract destination directory (YMPK only) |
| `--yes` | `-y` | `false` | Skip confirmation prompts |

**Behavior by format:**
- **MRPACK**: Adds mods to the current profile using upstream references
- **YMPK**: Extracts as a complete profile directory

---

### `launch <SUBCOMMAND>`

Launch Minecraft client or server with the current profile.

#### `client`

| Option | Description |
|--------|-------------|
| `--offline` | Launch in offline mode (no authentication) |
| `--jvm-args <ARGS>` | Additional JVM arguments (e.g., `-Xmx4G`) |
| `--java <PATH>` | Path to Java executable |

Downloads missing mods, Minecraft JAR, and loader. Symlinks content to `./client/` directory.

#### `server`

| Option | Default | Description |
|--------|---------|-------------|
| `--port <PORT>` | `25565` | Server port |
| `--jvm-args <ARGS>` | | Additional JVM arguments |
| `--eula` | `false` | Auto-accept EULA |
| `--java <PATH>` | | Path to Java executable |

Downloads missing mods, Minecraft server JAR, and loader. Sets up `./server/` directory.

---

### `organize <SUBCOMMAND>`

Sort discovered config files into the appropriate mod directories.

| Subcommand | Description |
|------------|-------------|
| `client` | Organize configs from `./client/config/` |
| `server` | Organize configs from `./server/config/` |

Interactive TUI with fuzzy search, syntax-highlighted preview, and destination selection.

---

### `manage`

Interactive modpack management TUI.

```
yammm manage
```

Provides a full-screen TUI for browsing, adding, removing, and updating mods. Only available when compiled with the `tui` feature.

---

### `auth <SUBCOMMAND>`

Manage Microsoft/Mojang authentication for online-mode launch.

| Subcommand | Description |
|------------|-------------|
| `login` | Sign in with your Microsoft account (device code flow) |
| `logout` | Sign out and remove stored credentials |
| `status` | Show current authentication state (username, UUID, token expiry) |

Tokens are stored at `~/.config/yammm/auth.json` with restricted permissions. The launch command automatically uses stored credentials when available.

---

### `cache <SUBCOMMAND>`

Manage the global cache.

| Subcommand | Description |
|------------|-------------|
| `status` | Show cache statistics for all subdirectories (jars, minecraft, loaders) |
| `clean` | Remove oldest files across all subdirectories until under threshold |
| `obliterate` | Remove all cached files (prompts for confirmation) |

The `cache clean` command evicts files using LRU: JARs by manifest-recorded access time, Minecraft/loader versions by directory modification time (entire versions are removed together).

---

### `config <SUBCOMMAND>`

Manage global configuration.

| Subcommand | Description |
|------------|-------------|
| `edit` | Open config in `$EDITOR` |
| `show` | Display current configuration |
| `get <KEY>` | Get a value (dot-notation: `api_keys.curseforge`) |
| `set <KEY> <VALUE>` | Set a value |
| `reset` | Reset all values to defaults |

---

### `self-update [OPTIONS]`

Update yammm to the latest version.

```
yammm self-update [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--check` | `-c` | `false` | Only check for updates, don't install |
| `--yes` | `-y` | `false` | Skip confirmation prompts |

---

### `completions <SHELL>`

Generate shell completion scripts.

```
yammm completions <SHELL>
```

Shells: `bash`, `zsh`, `fish`, `elvish`

---

## Exit Codes

| Code | Variant | Meaning |
| ---- | ------- | ------- |
| 0 | — | Success |
| 1 | `General` | General error |
| 2 | `InvalidArgs` | Invalid arguments or config parse error |
| 3 | `ModNotFound` / `Api(NotFound)` | Mod slug/ID not found |
| 4 | `DownloadFailed` / `HashMismatch` | Download or hash verification failure |
| 5 | `ConfigError` | Configuration file error |
| 6 | `NetworkError` / `NetworkRequest` | Network timeout, DNS, or 5xx error |
| 7 | `IoError` | I/O or storage error |
| 8 | `Api(other)` | Other API error |
| 9 | `VersionConflict` | No version satisfies constraints |
| 10 | `CircularDependency` | Circular dependency detected |

---

## Environment Variables

| Variable | Description |
| -------- | ----------- |
| `YAMMM_DEBUG` | Enable debug mode |
| `YAMMM_CONFIG` | Path to global config file |
| `YAMMM_CACHE_DIR` | Override cache directory |
| `CURSEFORGE_API_TOKEN` | CurseForge API key |
