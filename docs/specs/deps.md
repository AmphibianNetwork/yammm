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

The `DependencyResolver` in `services/resolver.rs` works as follows:

1. **Initialize** a queue with the root mod's dependencies
2. **Dequeue** the next dependency
3. **Skip** if the mod is already installed in the profile
4. **Skip** if the mod was already resolved in this session (avoid duplicates)
5. **Fetch** mod metadata and versions from the source provider
6. **Select** the latest compatible version (matching MC version + loader)
7. **Enqueue** the new mod's dependencies (required first, then optional)
8. **Repeat** until queue is empty

### Priority Order

Required dependencies are enqueued before optional ones, ensuring that:
- All required deps are resolved first
- Optional deps that fail to resolve are non-fatal (logged and skipped)

### Cycle Handling

If a mod appears in the resolution queue that was already visited, it is skipped. This naturally handles cycles since BFS visits each mod at most once.

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

`yammm info tree` displays the dependency tree as a flat list:

```
create
  requires: architectury-api, cloth-config
  optional: create-enchantment-improvements

architectury-api
  requires: fabric-api

cloth-config
  requires: fabric-api

fabric-api
  (no dependencies)
```

```

## Dependency Storage

Dependencies are stored in each mod's `mod.ron` file:

```ron
dependencies: [
    (
        mod_id: "fabric-api",
        source: Modrinth(id: "P7dR8mAK"),
        kind: required,
    ),
    (
        mod_id: "cloth-config",
        source: Modrinth(id: "9s6osm5g"),
        kind: optional,
    ),
],
```

Each dependency records:
- `mod_id` — the slug/ID used in the profile
- `source` — the upstream source reference
- `kind` — required, optional, incompatible, or embedded
- `version` — optional version constraint
- `required_by` — which mod introduced this dependency

---

## Reverse Dependency Check

When removing a mod, yammm scans all installed mods' dependency lists to find which mods depend on the target. This is used by `yammm remove` to warn about dependent mods before removal.
