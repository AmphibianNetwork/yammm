# JSON Output Reference

`yammm` supports machine-readable JSON output via the global `--json`
flag. This document catalogs the payload schemas for every command that
opts in.

## Conventions

- **Single document per command.** Each invocation writes exactly one
  JSON document to stdout. Combine with `--quiet` to silence any other
  diagnostic noise: `yammm --quiet --json info`.
- **Status lines go to stderr.** Errors, warnings, and `--debug`
  tracing still print to stderr even in JSON mode; they never
  contaminate stdout.
- **Non-interactive.** JSON mode implies `--yes` for any command that
  would otherwise prompt (`add` deps, `remove` confirmation, `update`
  apply prompt, `export` confirmation, `import` overwrite). Interactive
  commands without a non-interactive path (`auth login`, `config edit`,
  `config reset`, `cache obliterate`) reject `--json` with an error.
- **Stable fields, additive evolution.** Existing fields keep their
  names and types; new fields may be added at any time. Removing or
  renaming a field is a breaking change and gets a major-version bump.
- **Exit codes preserved.** A non-zero exit code on failure is kept
  even in JSON mode, so scripts can short-circuit on success without
  parsing the document.

## Commands rejecting `--json`

These commands return a non-zero exit with stderr message
`command 'X' does not yet support --json output`:
`launch`, `self-update`, `completions`, `organize`, `manage`,
`config edit`, `config reset`, `cache obliterate`, `cache clean`.

---

## `info`

Default invocation: pack overview.

```json
{
  "name": "my-pack",
  "version": "1.0.0",
  "description": "...",
  "minecraft_version": "1.20.4",
  "loader": { "name": "fabric", "version": "0.15.11" },
  "counts": {
    "mods": 5,
    "resource_packs": 1,
    "shader_packs": 0
  },
  "mods": [TrackedMod, ...],
  "resource_packs": [TrackedMod, ...],
  "shader_packs": [TrackedMod, ...]
}
```

`info list` emits the same shape.

`info mod <id>` emits a single `TrackedMod` directly.

`info tree` emits a flat node/edge dependency graph (intentionally not
nested — handles cycles, shared deps, and scripted graph operations
without recursion):

```json
{
  "nodes": [
    {
      "id": "fabric-api",
      "name": "Fabric API",
      "version": "0.92.0",
      "env": "both",
      "source": "modrinth",
      "project_type": "mod"
    }
  ],
  "edges": [
    { "from": "iris", "to": "sodium", "kind": "required" }
  ]
}
```

`kind` is one of `required`, `optional`, `incompatible`, `embedded`.

---

## `search`

```json
{
  "query": "sodium",
  "count": 3,
  "hits": [
    {
      "source": "modrinth",
      "id": "AANobbMI",
      "name": "Sodium",
      "description": "...",
      "url": "https://modrinth.com/mod/sodium",
      "minecraft_versions": ["1.20.4", "1.20.3"],
      "loaders": ["fabric"],
      "downloads": 12345678
    }
  ]
}
```

`source` is one of `modrinth` / `curseforge` / `url`. `id` is the
upstream identifier (Modrinth project ID, CurseForge numeric id, or
the URL itself).

---

## `cache status`

```json
{
  "root": "/home/u/.cache/yammm",
  "jars":      { "file_count": 42, "total_size": 12345678 },
  "minecraft": { "file_count":  3, "total_size":  1234567 },
  "loaders":   { "file_count":  2, "total_size":   234567 },
  "http_meta": { "file_count": 17, "total_size":     8910 },
  "total":     { "file_count": 64, "total_size": 13822722 }
}
```

`total_size` is in bytes. `total.file_count` and `total.total_size`
include `http_meta`.

---

## `cache clear-http-meta`

Without flags: wipes everything.

```json
{
  "command": "cache clear-http-meta",
  "status": "cleared"
}
```

With `--stale` or `--max-age <duration>`: removes only entries past the
threshold.

```json
{
  "command": "cache clear-http-meta",
  "status": "pruned",
  "threshold_secs": 86400,
  "removed": 7
}
```

`threshold_secs` reports the resolved threshold actually applied:
86400 for the default `--stale` window, otherwise whatever
`--max-age` was parsed to (see *Duration syntax* below).

---

## `add`

```json
{
  "command": "add",
  "added": {
    "id": "fabric-api",
    "project_type": "mod",
    "source": "modrinth"
  },
  "dependencies_installed": [
    { "id": "cloth-config", "project_type": "mod" }
  ]
}
```

`dependencies_installed` is computed by diffing the installed-slug set
before and after the add. It only includes mods that were not already
present.

---

## `remove`

```json
{
  "command": "remove",
  "removed": [
    { "id": "alpha", "name": "Alpha Mod", "project_type": "mod" }
  ]
}
```

`--json` implies non-interactive *and* `--force` (no removal of
dependents). If you want dependents removed in scripting, run a
separate `remove` for each.

---

## `update`

Default behavior (apply available updates):

```json
{
  "command": "update",
  "updated": [
    {
      "id": "fabric-api",
      "name": "Fabric API",
      "from": "0.91.0",
      "to":   "0.92.0"
    }
  ],
  "failed": [
    { "id": "x", "name": "X", "error": "404 Not Found" }
  ],
  "checks_failed": [
    { "name": "Y", "error": "timeout" }
  ]
}
```

Process exit is non-zero if `failed` is non-empty.

`--check-only` short-circuits before any write:

```json
{
  "command": "update",
  "check_only": true,
  "updates_available": [
    {
      "id": "fabric-api",
      "name": "Fabric API",
      "current_version": "0.91.0",
      "latest_version":  "0.92.0"
    }
  ],
  "checks_failed": [...]
}
```

Process exit is always zero in `--check-only` mode regardless of drift.

---

## `export`

```json
{
  "command": "export",
  "format": "mrpack",
  "output_path": "/tmp/my-pack-1700000000.mrpack",
  "size_bytes": 1234567,
  "downloads": {
    "completed": 5,
    "failed_count": 0
  }
}
```

`format` is one of `mrpack` / `ympk`. Process exit is non-zero if any
JAR download failed; `downloads.failed_count` lets you distinguish
"some downloads failed" from "archive write failed".

---

## `import`

```json
{
  "command": "import",
  "format": "mrpack",
  "output_dir": "/tmp/imported",
  "added": 5,
  "skipped": 1,
  "unresolved": 0
}
```

For YMPK imports the `unresolved` field is omitted (YMPK carries the
TrackedMod entries directly; nothing needs Modrinth-side resolution).

---

## `init`

```json
{
  "command": "init",
  "output_dir": "/tmp/pack",
  "name": "my-pack",
  "version": "1.0.0",
  "minecraft_version": "1.20.4",
  "loader": { "name": "fabric", "version": "" },
  "created": ["mods", "config", "...", "modpack.toml"],
  "modpack_toml_created": true
}
```

`created` lists newly-created paths; `modpack_toml_created` is
`false` when an existing `modpack.toml` was preserved (idempotent
re-init).

---

## `auth status`

When logged in:

```json
{
  "command": "auth status",
  "logged_in": true,
  "username": "Player",
  "uuid": "abc-def-...",
  "expires_at": 1700001234,
  "expired": false
}
```

When not:

```json
{
  "command": "auth status",
  "logged_in": false
}
```

`expires_at` is a Unix epoch second (UTC). Clients should compare
against their own clock rather than trusting `expired` for fresh
decisions.

## `auth logout`

```json
{
  "command": "auth logout",
  "status": "logged_out"
}
```

## `auth login`

A long, two-phase flow:

1. **Prompt phase.** While waiting on the user to approve the device
   code in their browser, the verification URL and code are printed to
   **stderr** so a script can surface them to a human without polluting
   stdout. No JSON is emitted during this phase.
2. **Result phase.** Once authentication completes (or fails) the final
   document lands on stdout:

```json
{
  "command": "auth login",
  "status": "logged_in",
  "username": "Player",
  "uuid": "abc-def-...",
  "expires_at": 1700001234
}
```

Failures (timeout, user denial, network) come back as a normal
non-zero exit with the error on stderr.

---

## `config show`

Emits the entire `GlobalConfig` as JSON. The `api_keys.curseforge`
value is redacted (`XXXX***YYYY`) if present.

```json
{
  "default_modpack_dir": null,
  "cache_dir": null,
  "cache_max_size_mb": 5000,
  "max_concurrent_downloads": 8,
  "api_keys": { "curseforge": "1234***7890" },
  "output": { "format": "table", "color": true }
}
```

## `config get`

```json
{
  "key": "ApiKeysCurseforge",
  "value": "1234***7890"
}
```

## `config set`

```json
{
  "command": "config set",
  "key": "CacheMaxSizeMb",
  "value": "10000",
  "status": "updated"
}
```

---

## Duration syntax

`cache clear-http-meta --max-age` accepts a positive integer with an
optional unit suffix:

| Input  | Resolved to |
|--------|-------------|
| `30s`  | 30 seconds  |
| `5m`   | 300 seconds |
| `2h`   | 7,200 seconds |
| `7d`   | 604,800 seconds |
| `90`   | 90 seconds (bare integer) |

Whitespace around the value is tolerated. Unknown suffixes and
overflow are reported as parse errors.
