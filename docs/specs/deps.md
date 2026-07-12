# Dependency Resolution Specification

## Overview

yammm resolves mod dependencies using a **breadth-first search (BFS)** algorithm. Starting from a root mod, it traverses the dependency graph, fetching dependencies from each mod's source provider.

---

## Dependency Types

| Type | Action | Description |
|------|--------|-------------|
| `required` | Resolve | Must be installed for the mod to function |
| `optional` | Resolve | Nice to have, user is prompted |
| `incompatible` | Warn | Conflicts with another mod (currently skipped) |
| `embedded` | Skip | Bundled inside the mod JAR |

---

## API Mapping

### Modrinth

| Modrinth Type | yammm Type |
|---------------|------------|
| `required` | `required` |
| `optional` | `optional` |
| `incompatible` | `incompatible` |
| `embedded` | `embedded` |

### CurseForge

| CurseForge Type | yammm Type |
|-----------------|------------|
| `requiredDependency` | `required` |
| `optionalDependency` | `optional` |
| `inclusion` | `optional` |

### GitHub, URL, File

These sources return empty dependency lists since their APIs don't provide dependency metadata.

---

## BFS Resolution Algorithm

The `DependencyResolver` in `services/resolver.rs` uses **two queues** — `required` and `optional` — to keep priority O(1) per pop instead of an O(n) linear scan:

1. **Initialize**: push the root mod into `required` with empty `ancestors`.
2. **Pop**: from `required` first; only pop from `optional` when `required` is empty.
3. **Skip** if the mod is already in the resolved set (deduplication, including raw project ID ↔ slug aliasing).
4. **Cycle check**: if the mod key appears in the popped entry's `ancestors`, return `YammmError::CircularDependency`.
5. **Fetch** mod metadata and the latest version matching the active MC version + loader filters.
6. **Record** both `key` and `canonical_key` (slug-based) in the resolved set to avoid re-walking via aliases.
7. **Skip recursion** if the entry's kind is `Embedded` — embedded deps ship inside the parent JAR.
8. **Enqueue** each child dependency into `required` or `optional` (based on effective kind), carrying the ancestor set extended with the parent's key.

### Priority order

`required` drains fully before `optional` gets a turn. Consequence:

- All required deps resolve first; a failure short-circuits the whole call.
- Optional deps that fail are logged and dropped — the rest of the tree keeps going.

### Optional downgrade propagation

When the parent's resolved kind is `Optional`, all children are forced to `Optional` regardless of what the source said. This prevents an optional branch from secretly pulling in mandatory installs.

### Self-references and incompatibility

- A dependency whose `mod_id` matches the parent's `mod_id` or `source_id` is skipped at enqueue time (some APIs occasionally return these).
- `Incompatible` deps are skipped without warning — they're advisory metadata, not something we'd ever try to install.

### Connector compatibility fallback

When a Forge or NeoForge resolution fails and a Fabric–Forge connector mod is present in the modpack, the resolver retries the same mod under the Fabric loader. This is handled at the [`commands/import/resolve.rs`](../../src/commands/import/resolve.rs) and [`commands/add/mod.rs`](../../src/commands/add/mod.rs) layer, not inside the core resolver — see `should_try_connector`.

### Test coverage

`src/services/resolver.rs::tests` covers: linear chains, optional downgrade, mutual deps (direct + 3-cycle), diamond dependency, raw-project-id deduplication, required-fail propagation, optional-fail silent, embedded skip-without-recurse, incompatible skip, self-reference filter, and filter passthrough.

### Version Selection

For each dependency, the resolver:
1. Fetches available versions from the source
2. Filters by Minecraft version compatibility
3. Filters by loader compatibility
4. Applies any version constraints from the dependency
5. Selects the latest matching version

---

## Resolution During `add`

When a user runs `yammm add create`:

1. Fetch create's version list and select the latest compatible version
2. Fetch create's dependencies from the source API
3. Run BFS resolution on all dependencies
4. Present the full list of resolved dependencies to the user:
   - Required deps shown as "required"
   - Optional deps shown as "optional"
   - Already-installed deps shown as "already installed"
5. User confirms which optional deps to include
6. All selected mods are saved to the profile

### With `--yes`

All required and optional dependencies are automatically added without prompting.

---

## Dependency Tree Display

`yammm info tree` displays the dependency tree using ASCII branch characters:

```
├── Create v0.5.1+m both [Modrinth]
│   ├── architectury-api (required)
│   └── cloth-config (optional)
├── Sodium v0.6.0 both [Modrinth]
│   └── fabric-api (required)
└── Fabric API v0.92.0 both [Modrinth]
    (no dependencies)
```

Each top-level entry shows: name, version, environment, and source. Dependencies are indented with branch lines showing their ID and dependency kind.

---

## Dependency Storage

Dependencies are stored in each mod's `entry.ron` file:

```ron
dependencies: [
    (
        mod_id: "fabric-api",
        source: (type: "modrinth", id: "P7dR8mAK"),
        kind: required,
    ),
    (
        mod_id: "cloth-config",
        source: (type: "modrinth", id: "9s6osm5g"),
        kind: optional,
    ),
],
```

Each dependency records:
- `mod_id` — the slug/ID used in the profile
- `source` — the upstream source reference (internally-tagged)
- `kind` — required, optional, incompatible, or embedded
- `version` — optional version constraint (`VersionReq`)
- `required_by` — which mod introduced this dependency

Between the provider layer and the resolver, `SourceDependency` is used (with `version_id` and `dep_type` fields). The resolver converts these into stored `Dependency` structs.

---

## Reverse Dependency Check

When removing a mod, yammm scans all installed mods' dependency lists to find which mods depend on the target. This is used by `yammm remove` to warn about dependent mods before removal.
