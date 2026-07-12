# YAMMM Usage Guide

This guide provides step-by-step instructions for using yammm (Yet Another Minecraft Modpack Maker).

## Prerequisites

### Install Rust Toolchain
```bash
rustup install stable
```

### Set up the environment (using Nix)
```bash
direnv allow
```

## Building the Project

```bash
# Build the project
cargo build

# Build in release mode
cargo build --release
```

## Getting Started

### 1. Initialize a New Modpack

Create a new modpack with interactive prompts:
```bash
yammm init
```

Create with custom settings (non-interactive):
```bash
yammm init --name "My Modpack" --minecraft-version 1.20.4 --loader fabric
```

Create with specific output directory:
```bash
yammm init --name "My Modpack" --minecraft-version 1.20.4 --loader fabric --output-dir ~/minecraft/modpacks/my-modpack
```

### 2. Set API Keys (Optional)

If you plan to use CurseForge, you'll need an API key:

```bash
# Set CurseForge API token (required for CurseForge)
yammm config set api_keys.curseforge your-curseforge-token
```

### 3. Add Mods to Your Modpack

yammm supports multiple mod sources. Use the `--source` flag or auto-detection for URLs:

| Source | Format | Example |
|--------|--------|---------|
| Modrinth (default) | `<slug>` | `sodium` |
| CurseForge | `<project-id>` with `--source curseforge` | `379952 --source curseforge` |
| GitHub | `https://github.com/<owner>/<repo>` | `https://github.com/IrisShaders/Iris` |
| HTTP URL | `https://...` | `https://example.com/mod.jar` |
| Local file | `file:///path/to/mod.jar` | `file:///path/to/mod.jar` |

URLs starting with `http://`, `https://`, or `file://` are auto-detected and use the URL source regardless of `--source`.

```bash
# Default source (Modrinth)
yammm add sodium
yammm add fabric-api -y

# Explicit source flag
yammm add 379952 --source curseforge
yammm add sodium --source modrinth

# URL auto-detection
yammm add https://github.com/IrisShaders/Iris
yammm add file:///path/to/mod.jar

# With specific version
yammm add sodium --version 1.0.0

# Skip confirmation prompts
yammm add fabric-api -y
```

**Note:** The `add` command does NOT download mod files. It only creates metadata. Files are downloaded during `launch` or `export`.

### 4. View Modpack Information

```bash
# Modpack overview
yammm info

# List all mods
yammm info list

# Verbose listing
yammm info list -v

# Detailed info about a specific mod
yammm info mod jei

# Dependency tree (flat list)
yammm info tree
```

### 5. Search for Mods

```bash
# Search Modrinth (default)
yammm search "optifine"

# Search a specific source
yammm search "optifine" -s modrinth
yammm search "optifine" -s curseforge

# Limit results
yammm search "fabric" -n 5

# Compact output
yammm search "sodium" -o compact
```

### 6. Remove Mods

Remove a mod by its folder name (slug):
```bash
yammm remove <mod-id>

# Skip confirmation
yammm remove <mod-id> -y

# Force remove (ignore dependents)
yammm remove <mod-id> --force
```

### 7. Export Your Modpack

```bash
# Export as MRPACK (default format, Modrinth-compatible)
yammm export -f mrpack

# Export as YMPK (yammm native format, includes configs)
yammm export -f ympk

# Export with output path
yammm export -f mrpack -o my-modpack.mrpack

# Skip confirmation
yammm export -y
```

The exported file will be saved in the modpack directory.

### 8. Import a Modpack

```bash
# Import MRPACK (adds mods to current profile)
yammm import ./modpack.mrpack

# Import YMPK (extracts as new profile directory)
yammm import ./backup.ympk -o ./my-modpack

# Skip confirmation
yammm import ./modpack.mrpack -y
```

### 9. Update Mods

```bash
# Check for updates
yammm update

# Auto-confirm all updates
yammm update -y
```

### 10. Organize Config Files

```bash
# Organize client configs (after launching client)
yammm organize client

# Organize server configs (after launching server)
yammm organize server
```

### 11. Launch Minecraft

```bash
# Launch client (offline mode)
yammm launch client --offline

# Launch client with custom memory
yammm launch client --offline --jvm-args="-Xmx4G"

# Launch client with specific Java
yammm launch client --offline --java /path/to/java

# Launch server
yammm launch server --port 25565 --eula --jvm-args="-Xmx4G"
```

Online-mode client launch requires a Microsoft account — see the `auth` section below.

### 11a. Authenticate (online launch)

```bash
# Sign in with your Microsoft account (device code flow)
yammm auth login

# Check current authentication state
yammm auth status

# Sign out and remove stored credentials
yammm auth logout
```

Tokens are stored at `~/.config/yammm/auth.json` with restricted permissions. The launch command automatically refreshes expired access tokens via the stored refresh token. See [microsoft-auth-setup.md](microsoft-auth-setup.md) if you're standing up a new Azure AD app.

### 11b. Interactive modpack manager (TUI)

```bash
# Browse, add, remove, update mods interactively
yammm manage
```

Only available when compiled with the `tui` feature (default). Backed by ratatui + crossterm.

### 12. Shell Completions

```bash
yammm completions bash
yammm completions zsh
yammm completions fish
yammm completions elvish
```

### 13. Cache Management

```bash
# Show cache status
yammm cache status

# Clean oldest JARs until cache is under threshold
yammm cache clean

# Remove all cached JAR files
yammm cache obliterate
```

### 14. Global Configuration

```bash
# Show current config
yammm config show

# Get a specific value
yammm config get cache_dir
yammm config get api_keys.curseforge

# Set a value
yammm config set cache_dir /tmp/yammm-cache
yammm config set api_keys.curseforge your-key

# Open config in editor
yammm config edit

# Reset to defaults
yammm config reset
```

### 15. Self-update

```bash
# Check for a newer release (no install)
yammm self-update --check

# Update in place, with confirmation
yammm self-update

# Update without confirmation
yammm self-update -y
```

Pulls the latest release archive from the GitHub Releases API and replaces
the running binary. Self-update is automatically disabled when running from
`/nix/store/` — update via `nix flake update` instead.

#### Verifying the downloaded binary

yammm's self-update path currently relies on HTTPS to GitHub for transport
integrity but does **not** automatically verify a detached signature or
SHA-256 checksum of the downloaded artifact. If you operate in a high-trust
environment and want belt-and-braces verification, perform it manually:

```bash
# Pin a version, download the archive + checksums.txt from the release page,
# then verify:
sha256sum -c checksums.txt --ignore-missing
```

Checksums and (where present) GPG signatures are published alongside every
GitHub release. Automatic verification is tracked as a follow-up — until
then, treat `yammm self-update` as equivalent to `curl | tar`: only as
trustworthy as your TLS chain to `github.com`.

---

## Command Reference

### `init` - Create a new modpack
```
yammm init [OPTIONS]

Options:
  -n, --name <NAME>                Name of the modpack
  -d, --description <TEXT>         Description of the modpack
  -m, --minecraft-version <VER>    Minecraft version
  -l, --loader <LOADER>            Loader [fabric|forge|neoforge|quilt]
  --loader-version <VER>           Loader version
  -o, --output-dir <DIR>           Output directory (default: current directory)
```

### `add` - Add a mod to the modpack
```
yammm add <QUERY> [OPTIONS]

Sources:
  <slug>                          Modrinth mod by slug (default)
  <project-id> -s curseforge      CurseForge mod by project ID
  https://github.com/<owner>/<repo>  GitHub repository (auto-detected)
  https://...                     Direct URL (auto-detected)
  file:///path/to/mod.jar         Local file (auto-detected)

Options:
  -s, --source <SOURCE>     Mod source [modrinth|curseforge]
  -v, --version <VERSION>   Specific version to download
  -l, --loader <LOADER>     Override loader for this mod
  -y, --yes                 Skip confirmation prompts
```

### `remove` - Remove a mod from the modpack
```
yammm remove <MOD_ID> [OPTIONS]

Arguments:
  <MOD_ID>  The folder name of the mod to remove

Options:
  -y, --yes   Skip confirmation prompts
  --force     Remove even if other mods depend on it
```

### `info` - Display modpack information
```
yammm info [SUBCOMMAND]

Subcommands:
  (none)     Show modpack overview
  list       List all mods [-v]
  mod <id>   Show info about a specific mod
  tree       Show dependency tree (flat list)
```

### `search` - Search for mods
```
yammm search <QUERY> [OPTIONS]

Options:
  -s, --source <SOURCE>   Search source [modrinth|curseforge]
  -n, --limit <N>         Max results [default: 20]
  -l, --loader <LOADER>   Filter by loader
  -o, --output <FORMAT>   Output format [table|compact]
```

### `export` - Export the modpack
```
yammm export [OPTIONS]

Options:
  -f, --format <FORMAT>   Format: mrpack, ympk [default: mrpack]
  -o, --output <PATH>     Output file path
  -y, --yes               Skip confirmation prompts
```

### `import` - Import a modpack
```
yammm import <FILE> [OPTIONS]

Arguments:
  <FILE>    Path to MRPACK or YMPK file

Options:
  -o, --output <DIR>   Extract destination directory (YMPK)
  -y, --yes             Skip confirmation prompts
```

### `update` - Check for mod updates
```
yammm update [OPTIONS]

Options:
  -y, --yes   Auto-confirm all updates
```

### `organize` - Sort config files into mod directories
```
yammm organize <SUBCOMMAND>

Subcommands:
  client    Organize client configs
  server    Organize server configs
```

### `launch` - Launch Minecraft
```
yammm launch <SUBCOMMAND> [OPTIONS]

Subcommands:
  client [--offline] [--jvm-args <ARGS>] [--java <PATH>]
  server [--port <PORT>] [--eula] [--jvm-args <ARGS>] [--java <PATH>]
```

Defaults to `client` when no subcommand is given.

### `auth` - Microsoft / Mojang authentication
```
yammm auth <SUBCOMMAND>

Subcommands:
  login     Sign in via Microsoft device code flow
  logout    Remove stored credentials
  status    Show current auth state
```

### `manage` - Interactive TUI modpack manager
```
yammm manage
```

Requires the `tui` feature (default).

### `cache` - Manage the global cache
```
yammm cache <SUBCOMMAND>

Subcommands:
  status      Show cache statistics
  clean       Remove oldest JARs until under threshold
  obliterate  Remove all cached JAR files
```

### `config` - Manage global configuration
```
yammm config <SUBCOMMAND>

Subcommands:
  edit              Open config in editor
  show              Display current configuration
  get <KEY>         Get a specific value (dot-notation)
  set <KEY> <VALUE> Set a configuration value
  reset             Reset to defaults
```

### `completions` - Generate shell completions
```
yammm completions <SHELL>

Shells:
  bash, zsh, fish, elvish
```

### `self-update` - Update the yammm binary in place
```
yammm self-update [OPTIONS]

Options:
  -c, --check   Check for updates without installing
  -y, --yes     Skip confirmation prompts
```

---

## Project Structure

After running `init`, your modpack will have this structure:
```
my-modpack/
├── modpack.toml          # Modpack configuration (name, version, loader)
├── .yammm/
│   └── ignored_configs.ron  # Ignored config file paths
├── mods/                 # Mod metadata directories
│   └── {mod-id}/
│       └── entry.ron     # Per-mod metadata (source, version, hash, dependencies)
├── resourcepacks/       # Resource pack metadata directories
│   └── {pack-id}/
│       └── entry.ron
├── shaderpacks/         # Shader pack metadata directories
│   └── {pack-id}/
│       └── entry.ron
├── config/               # Fallback config files
├── resources/
│   ├── client/           # Client-side resource overrides
│   └── server/           # Server-side resource overrides
├── .gitignore
└── README.md
```

Per-item metadata lives in `entry.ron` files inside per-id subdirectories. The `modpack.toml` file only contains modpack-wide settings. Downloaded JARs are stored in the global cache, not in the modpack directory.

## Configuration

### Global Config (~/.config/yammm/config.toml)
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

### Modpack Config (modpack.toml)
```toml
name = "My Modpack"
description = "A cool modpack"
version = "1.0.0"
minecraft_version = "1.20.4"

[loader]
loader = "fabric"
version = "0.15.11"
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `YAMMM_CONFIG` | Override global config file path |
| `YAMMM_CACHE_DIR` | Override cache directory |
| `YAMMM_DEBUG` | Enable debug logging |
| `CURSEFORGE_API_TOKEN` | CurseForge API key |

---

## Troubleshooting

### API Rate Limiting
- Use API keys to increase rate limits
- The app has built-in caching to reduce API calls

### Download Failures
- Check your internet connection
- Verify the mod slug/ID is correct
- CurseForge requires an API key

### Mod Dependencies
- yammm automatically resolves dependencies (BFS traversal)
- GitHub, URL, and File sources do not provide dependency metadata

---

## Testing

Run the test suite:
```bash
cargo test
```

Run with clippy:
```bash
cargo clippy --all-targets
```

---

## Next Steps

After using yammm, you can:
1. Import the MRPACK file into Modrinth App or Prism Launcher
2. Share the YMPK file with other yammm users
3. Use `yammm launch client --offline` to test locally
