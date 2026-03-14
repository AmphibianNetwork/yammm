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
| `--description` | `-d` | `""` | Modpack description |
| `--minecraft-version` | `-m` | Interactive | Minecraft version |
| `--loader` | `-l` | Interactive | Loader: `fabric`, `forge`, `neoforge`, `quilt` |
| `--loader-version` | | `""` | Loader version |
| `--output-dir` | `-o` | Current directory | Output directory |

When `--name` is provided, non-interactive mode is used (no prompts).

---

### `add <QUERY>`

Add a mod to the current profile.

```
yammm add <QUERY> [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--source` | `-s` | `modrinth` | Source: `modrinth`, `curseforge` |
| `--version` | `-v` | `latest` | Specific mod version |
| `--loader` | `-l` | Profile default | Override loader: `fabric`, `forge`, `neoforge`, `quilt` |
| `--yes` | `-y` | `false` | Skip confirmation prompts |
| `--force` | `-f` | `false` | Force add without confirmation |
| `--env` | | `both` | Side: `client`, `server`, `both` |
| `--project-type` | | `mod` | Project type: `mod`, `resourcepack`, `shader` |

**URL auto-detection:** When `<QUERY>` starts with `http://`, `https://`, or `file://`, the URL source is used automatically regardless of `--source`.

**Behavior:**
- Automatically resolves dependencies via BFS traversal
- Prompts user whether to add each dependency
- If exact mod not found on Modrinth, enters search mode
- Does NOT download files — download happens during `launch` or `export`

---

### `remove <MOD_ID>`

Remove a mod from the current profile.

```
yammm remove <MOD_ID> [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--yes` | `-y` | `false` | Skip confirmation prompts |
| `--force` | | `false` | Remove without checking for dependent mods |

**Behavior:**
- By default, checks for mods that depend on the target and warns the user
- With `--force`, removes immediately without dependency checks
- Removes the mod directory (`mods/{mod-id}/`) including `mod.ron`
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

Show dependency tree (flat list format).

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
| `--output` | `-o` | `{modpack-name}.{ext}` | Output file path |
| `--yes` | `-y` | `false` | Skip confirmation prompts |

**Formats:**
- `mrpack`: Modrinth modpack format — contains `modrinth.index.json` + mod JARs
- `ympk`: yammm native format — contains `modpack.toml` + `mod.ron` files + config files

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

The `cache clean` command evicts files using LRU: JARs by file access time, Minecraft/loader versions by directory access time (entire versions are removed together).

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

### `completions <SHELL>`

Generate shell completion scripts.

```
yammm completions <SHELL>
```

Shells: `bash`, `zsh`, `fish`, `elvish`

---

## Exit Codes

| Code | Kind | Meaning |
| ---- | ---- | ------- |
| 0 | — | Success |
| 1 | `General` | General error |
| 2 | `InvalidArgs` | Invalid arguments or config parse error |
| 3 | `ModNotFound` | Mod slug/ID not found |
| 4 | `DownloadFailed` | Download or hash verification failure |
| 5 | `ConfigError` | Configuration file error |
| 6 | `NetworkError` | Network timeout, DNS, or 5xx error |
| 7 | `StorageError` | I/O or storage error |
| 8 | `DependencyError` | Dependency resolution failure |
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
