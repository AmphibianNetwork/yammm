# yammm

Yet Another Minecraft Modpack Maker — a Rust CLI for developing Minecraft modpacks.

## Features

- **Multiple mod sources**: Modrinth, CurseForge, URL (GitHub repos and local files via https:// and file:// URLs)
- **Dependency resolution**: Automatic BFS-based transitive dependency handling
- **Multiple loaders**: Fabric, Forge, NeoForge, Quilt
- **Export formats**: MRPACK (Modrinth-compatible) and YMPK (native)
- **Minecraft launcher**: Built-in client and server launch with VFS symlinks
- **Config organizer**: Interactive TUI to sort orphan config files into mod directories
- **Global JAR cache**: Hash-based deduplication across modpacks
- **Shell completions**: bash, zsh, fish, elvish

## Quick Start

```bash
# Initialize a modpack
yammm init --name "My Pack" --minecraft-version 1.20.4 --loader fabric

# Add mods
yammm add fabric-api
yammm add sodium -y

# Search for mods
yammm search "create"

# Launch Minecraft
yammm launch client --offline

# Export
yammm export -f mrpack
```

## Installation

### Pre-built Binaries (Recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/AmphibianNetwork/yammm/releases).

#### One-liner Install (Linux/macOS)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/AmphibianNetwork/yammm/releases/latest/download/yammm-installer.sh | sh
```

#### One-liner Install (Windows PowerShell)

```powershell
irm https://github.com/AmphibianNetwork/yammm/releases/latest/download/yammm-installer.ps1 | iex
```

#### Homebrew (macOS/Linux)

```bash
brew install AmphibianNetwork/tap/yammm
```

#### Windows MSI

Download the `.msi` installer from the [latest release](https://github.com/AmphibianNetwork/yammm/releases) — it adds `yammm` to your PATH automatically.

#### Linux Packages

Debian/Ubuntu:
```bash
sudo dpkg -i yammm_*_amd64.deb
```

Fedora/RHEL:
```bash
sudo rpm -i yammm-*.x86_64.rpm
```

### From Source

```bash
cargo install --path .
```

### With Nix

```bash
nix profile install github:AmphibianNetwork/yammm    # Install directly
nix run github:AmphibianNetwork/yammm                 # Run without installing
nix develop                                    # Enter dev shell (from repo)
nix build                                      # Build release binary (from repo)
```

## Platform Support

yammm runs on **Linux**, **macOS**, and **Windows**. Platform-specific notes:

- **Unix (Linux/macOS)**: Full support including signal forwarding (SIGINT/SIGTERM) to child processes
- **Windows**: Supported with graceful Ctrl+C handling. Secret files rely on per-user ACLs in `%APPDATA%`. Directory symlinks use junction points or copy fallbacks when `SeCreateSymbolicLinkPrivilege` is not available

## Documentation

Full documentation is in [docs/](docs/):

- [Usage Guide](docs/USAGE.md) — Step-by-step walkthrough
- [CLI Reference](docs/specs/cli.md) — Complete command documentation
- [Architecture](docs/specs/architecture.md) — System design and module structure

## Development

```bash
cargo build           # Build
cargo test            # Run tests
cargo clippy --all-targets  # Lint
just list             # List available tasks
```

### Requirements

- Rust stable toolchain
- Nix with Flakes enabled (optional, for reproducible dev environment)
- [direnv](https://direnv.net/) (optional, recommended for VSCode)

---

MIT License
