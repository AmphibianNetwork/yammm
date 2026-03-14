# yammm Documentation

Welcome to the yammm (Yet Another Minecraft Modpack Maker) documentation. This guide covers everything you need to know about the CLI tool for managing Minecraft modpacks.

## Quick Start

```bash
# Initialize a new modpack
yammm init --name "My Pack" --minecraft-version 1.20.4 --loader fabric

# Search and add a mod
yammm search jei
yammm add jei

# Launch Minecraft (downloads missing mods automatically)
yammm launch client

# Export as MRPACK
yammm export -f mrpack
```

---

## User Guide

See [USAGE.md](USAGE.md) for a step-by-step walkthrough of every command.

### CLI Reference

| Command | Description |
|---------|-------------|
| `yammm init` | Initialize a new modpack |
| `yammm search <query>` | Search for mods |
| `yammm add <query>` | Add mods to profile |
| `yammm remove <mod>` | Remove mods from profile |
| `yammm info` | Show modpack/mod information |
| `yammm update` | Check for mod updates |
| `yammm launch <client\|server>` | Launch Minecraft |
| `yammm export` | Export modpack |
| `yammm import <file>` | Import a modpack |
| `yammm organize <client\|server>` | Sort orphan configs |
| `yammm cache <subcmd>` | Manage global cache |
| `yammm config <subcmd>` | Manage global config |
| `yammm completions <shell>` | Generate shell completions |

For detailed command options and examples, see [USAGE.md](USAGE.md) and the [CLI Specification](specs/cli.md).

---

## Architecture Documentation

For developers contributing to yammm.

### Core Architecture

| Document | Description |
|----------|-------------|
| [architecture.md](specs/architecture.md) | System overview, module structure, data flow |
| [services.md](specs/services.md) | Provider trait, API clients, source registry |
| [config.md](specs/config.md) | Global and modpack configuration |
| [storage.md](specs/storage.md) | Mod file storage and metadata |
| [caching.md](specs/caching.md) | JAR file caching and deduplication |
| [deps.md](specs/deps.md) | Dependency resolution algorithm |
| [errors.md](specs/errors.md) | Error types and exit codes |

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      CLI Layer (clap)                       │
├─────────────────────────────────────────────────────────────┤
│                 AppContext (global state)                    │
│           GlobalConfig │ SourceRegistry │ App               │
├─────────────────────────────────────────────────────────────┤
│                    Command Layer                             │
│  init │ add │ remove │ search │ info │ update │ ...         │
├─────────────────────────────────────────────────────────────┤
│                   Provider Layer                             │
│  ModrinthSource │ CurseForgeSource │ UrlSource               │
├─────────────────────────────────────────────────────────────┤
│                     API Layer                                │
│  ModrinthClient │ CurseForgeClient │ ...                     │
├─────────────────────────────────────────────────────────────┤
│              Domain / Storage / Cache                        │
│  Types │ ModStore (RON) │ ModpackConfig (TOML) │ JarCache  │
└─────────────────────────────────────────────────────────────┘
```

---

## Development

```bash
# Build the project
cargo build

# Run tests
cargo test

# Run with debug output
cargo run -- --debug search jei
```

### Documentation Structure

```
docs/
├── README.md                       # This file — main documentation index
├── USAGE.md                        # Step-by-step usage guide
├── microsoft-auth-setup.md         # Azure AD app setup for auth
└── specs/                          # Detailed specifications
    ├── architecture.md             # System architecture overview
    ├── caching.md                  # JAR file caching
    ├── cli.md                      # Full CLI specification
    ├── config.md                   # Configuration schema
    ├── deps.md                     # Dependency resolution
    ├── errors.md                   # Error types and exit codes
    ├── services.md                 # API integrations and provider trait
    └── storage.md                  # Storage and metadata formats
```

---

## License

yammm is released under the MIT License. See [LICENSE](../LICENSE) for details.
