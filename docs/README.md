# yammm Documentation

This directory holds yammm's user guide and developer specs. The repo-root [README.md](../README.md) is the marketing surface; this is where the depth lives.

---

## For users

| Document | Description |
|---|---|
| [USAGE.md](USAGE.md) | Step-by-step walkthrough of every command, with examples |
| [specs/cli.md](specs/cli.md) | Reference for every command flag, exit code, and environment variable |
| [microsoft-auth-setup.md](microsoft-auth-setup.md) | Setting up a custom Azure AD app for Microsoft authentication (only needed if you're forking) |

---

## For contributors

Read in this order:

1. [../CONTRIBUTING.md](../CONTRIBUTING.md) — setup, day-to-day commands, navigation
2. [specs/architecture.md](specs/architecture.md) — layered architecture, module map, `AppContext`
3. [specs/conventions.md](specs/conventions.md) — output channel, `AppContext` access, pure-builder pattern

Then dig into whichever subsystem you're touching:

| Subsystem | Spec |
|---|---|
| Provider trait + API clients | [specs/services.md](specs/services.md) |
| Dependency resolution (BFS) | [specs/deps.md](specs/deps.md) |
| The launcher (client + server) | [specs/launch.md](specs/launch.md) |
| On-disk layout and RON files | [specs/storage.md](specs/storage.md) |
| Global cache and eviction | [specs/caching.md](specs/caching.md) |
| Configuration schemas | [specs/config.md](specs/config.md) |
| Error model and exit codes | [specs/errors.md](specs/errors.md) |
| JSON output schemas for every `--json` command | [specs/json-output.md](specs/json-output.md) |

---

## Doc tree

```
docs/
├── README.md                       # this file
├── USAGE.md                        # user walkthrough
├── microsoft-auth-setup.md         # Azure AD setup for forks
└── specs/
    ├── architecture.md             # system architecture
    ├── caching.md                  # JAR + MC + loader caching
    ├── cli.md                      # CLI reference
    ├── config.md                   # config schemas
    ├── conventions.md              # code conventions
    ├── deps.md                     # dependency resolver
    ├── errors.md                   # error types and exit codes
    ├── json-output.md              # JSON payload schemas per command
    ├── launch.md                   # launch subsystem
    ├── services.md                 # providers and API clients
    └── storage.md                  # on-disk layout
```

---

## License

yammm is released under the MIT License. See [LICENSE](../LICENSE).
