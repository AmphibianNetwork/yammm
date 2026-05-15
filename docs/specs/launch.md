# Launch Subsystem

The launch command takes a modpack and produces a running Minecraft process. It's the largest subsystem in yammm — roughly 2,500 LOC across `src/commands/launch/` — because Minecraft's launch contract is genuinely complicated: each loader has its own classpath rules, log4j has CVE patches across version ranges, native libraries get extracted per OS/arch, server-side Forge wants `@unix_args.txt` files, client-side macOS wants `-XstartOnFirstThread`, and so on. None of that is yammm's choice.

This doc explains the shape of the pipeline and the testable seams inside it. For the user-facing CLI, see [cli.md](cli.md).

---

## Subcommands

```
yammm launch                  # implicit client
yammm launch client [opts]
yammm launch server [opts]
```

`launch` with no subcommand defaults to `client`. Both modes share the bulk of the pipeline — version-info fetch, library download, loader install, Java resolution — and diverge only in argument assembly and which `downloads.{client,server}` entry from the Mojang manifest they consume.

---

## Module map

| File | Role |
|---|---|
| `commands/launch/mod.rs` | Subcommand dispatch, signal handlers, shared helpers (`build_classpath`, `resolve_jvm_args`, `extract_module_path_jars`, `build_java_command`, `resolve_classpath`) |
| `commands/launch/client.rs` | Client `run()` and the pure `build_client_java_args` |
| `commands/launch/server.rs` | Server `run()`, `launch_with_unix_args` / `launch_with_classpath`, pure `build_unix_args_java_args` / `build_classpath_java_args`, `parse_shim_jar_filenames` |
| `commands/launch/prepare.rs` | `prepare_launch` — orchestrates downloads, returns `LaunchContext`; loader install routing; `is_bundler_jar`, `deduplicate_classpath`, `merge_libraries` |
| `commands/launch/java.rs` | JDK discovery, version matching, native-image preflight; log4j helpers (`log4j_mitigation_args`, `needs_log4j_config_override`, `write_log4j_config`, `patch_log4j_config_in_jar`) |
| `commands/launch/libraries.rs` | MC library downloading, native extraction; OS/feature rule evaluation |
| `commands/launch/loader.rs` | Per-loader sanity warnings (e.g., "Fabric API is missing!") |
| `commands/launch/vfs.rs` | In-memory virtual file tree → realize as symlinks on disk |

---

## Pipeline overview

Both client and server follow the same broad shape:

```
        download_missing_mods (services::download)
               │
               ▼
        prepare_launch ──────────────────────────────────┐
        │                                                 │
        │ ├─ check_modloader_deps  (loader.rs)            │
        │ ├─ resolve_java          (java.rs)              │
        │ ├─ MinecraftClient::get_version_info            │
        │ ├─ MinecraftClient::download_jar (client/server)│  → mc_cache
        │ ├─ MinecraftClient::download_assets (client)    │  → mc_cache/assets
        │ ├─ download_mc_libraries (libraries.rs)          │  → mc_cache/{libs,natives}
        │ ├─ install_loader                                │
        │ │     ├── install_fabric_like  (Fabric/Quilt)   │  → loader_cache
        │ │     └── install_forge_like   (Forge/NeoForge) │  → loader_cache
        │ ├─ deduplicate_classpath                        │
        │ └─ resolve_mc_jvm_args                          │
        │                                                  │
        │  returns LaunchContext { java_path, classpath_jars,
        │                          loader_jvm_args, mc_jvm_args, ... }
        │
        ▼
        Build VFS (mods/, config/, resourcepacks/, ...) and realize as symlinks
               │
               ▼
        resolve_classpath  → ResolvedClasspath { classpath, main_class, ... }
               │
               ▼
        Side effects: write log4j config, fetch MSA token (online) ─┐
               │                                                     │
               ▼                                                     │
        PURE: build_*_java_args(inputs) → Vec<String>  ◄─────────────┘
               │
               ▼
        build_java_command + spawn_java_process + wait_for_child
```

---

## `LaunchContext`

Returned by `prepare_launch`. Carries everything the downstream stages need:

| Field | Source |
|---|---|
| `java_path: PathBuf` | `java::resolve_java` (env / config / bundled-JDK install) |
| `mc_cache: PathBuf` | `<cache_dir>/minecraft` |
| `loader_lib_dir: PathBuf` | Loader-specific subdir under `<cache_dir>/loaders/<loader>/<mc>/<ver>/` |
| `natives_dir: PathBuf` | `<mc_cache>/<mc_version>/natives` |
| `version_info: VersionInfo` | Mojang piston-meta `version.json` |
| `main_class: String` | Loader profile (Fabric/Quilt) or installer (Forge/NeoForge) |
| `classpath_jars: Vec<PathBuf>` | MC JAR + libs + loader libs, after dedup |
| `loader_jvm_args: Vec<String>` | Forge/NeoForge module-path args; empty for Fabric/Quilt |
| `loader_game_args: Vec<String>` | Forge/NeoForge game args; empty for Fabric/Quilt |
| `mc_jvm_args: Vec<String>` | Mojang manifest JVM args, resolved (substituted natives_dir, etc.) |
| `skip_merge_libs: bool` | True when bundler JAR + Fabric-like server (libs are inside the jar) |

The crucial split is **Forge/NeoForge load `resolved_jvm_args`** (a module-path-driven launch) while **Fabric/Quilt load a classic classpath**. Downstream code branches on `resolved.resolved_jvm_args.is_some()`.

---

## Client launch

`commands/launch/client.rs::run` does:

1. **Download missing mods** via `services::download_missing_mods` (concurrency 4 by default for the launch path).
2. **`prepare_launch(side: Client)`** — full pipeline above.
3. **Build the client VFS** — `build_mod_vfs` populates `mods/` from the JAR cache, configs from each mod's `mods/<id>/{,client/}config/` plus the global `config/`, plus `resources/client/`. Client adds `resourcepacks/` and `shaderpacks/`, then extracts an icon entry from the MC jar.
4. **Realize VFS** as symlinks under `<modpack>/client/`.
5. **Resolve classpath** → `ResolvedClasspath { classpath, resolved_jvm_args, main_class, game_args }`.
6. **Write log4j config** (side effect) if `needs_log4j_config_override(mc_version)`.
7. **Pick auth** — `ClientAuth::Offline` (`--offline`) or `ClientAuth::Online { username, access_token, uuid }` from `auth::get_valid_token` (which silently refreshes expired tokens via the stored refresh token; falls back to interactive device-code login).
8. **`build_client_java_args(...)`** — the pure builder, see below.
9. **`build_java_command` + `spawn_java_process` + `wait_for_child`** — spawn under the resolved Java, record the PID for signal forwarding, poll-wait.

### `build_client_java_args` (pure)

Defined in `client.rs`. Takes a `ClientLaunchInputs<'_>` carrying only what it reads (no full `LaunchContext`, no `auth::AuthToken`) so tests can stack-allocate fixture inputs. Produces a `Vec<String>` in a Minecraft-sensitive order:

```
[macOS only] -XstartOnFirstThread
-Djava.library.path=<natives>
<log4j mitigation args>           # CVE-2021-44228 family
<log4j config args>               # if needs_log4j_config_override
<resolved_jvm_args> + ADD_OPENS_ARG + filtered mc_jvm_args  ← Forge/NeoForge path
   OR
<mc_jvm_args minus -Djava.library.path=*>                   ← Fabric/Quilt path
<user --jvm-args>                 # space-split
-cp <classpath> <main_class>      # -cp MUST immediately precede main_class
<resolved.game_args>
--gameDir <client_dir>
--assetsDir <mc_cache>/assets
[--assetIndex <id>]               # only when version_info.asset_index is Some
--version <mc_version>
<auth>:
  Offline:  --username Player  --accessToken 0  --uuid 0…0
  Online:   --username <u>     --accessToken <t> --uuid <id>
--versionType yammm
```

The order matters: vanilla Minecraft parses positionally for some args, and Forge module-path launches require `ADD_OPENS_ARG` between resolved JVM args and the classpath. The unit tests in `client.rs::tests` pin most of these orderings down.

---

## Server launch

`server::run` does steps 1–3 as well, plus writes `eula.txt` if `--eula`, then branches:

| Branch | When | Builder |
|---|---|---|
| `launch_with_unix_args` | A `unix_args.txt` (or `win_args.txt`) exists in `launch_ctx.loader_lib_dir` — Forge/NeoForge installer style | `build_unix_args_java_args` |
| `launch_with_classpath` | Otherwise — vanilla / Fabric / Quilt server | `build_classpath_java_args` |

### `build_unix_args_java_args` (pure)

```
<log4j mitigation args>
<log4j config args>
<filtered mc_jvm_args>             # --add-opens / --add-exports only
@user_jvm_args.txt                 # caller writes this file with defaults + user --jvm-args
@<unix_args.txt>                   # contains Forge/NeoForge launch args
--port <port>
nogui
```

The `@filename` syntax tells Java to read flags from a file. `user_jvm_args.txt` is written by `launch_with_unix_args` itself (side effect — defaults to `-Xmx2G` plus any user `--jvm-args`). The Forge/NeoForge installer pre-stages `unix_args.txt` (and possibly `win_args.txt`) in the loader library directory.

### `build_classpath_java_args` (pure)

Mirrors the client arg-builder but writes server args:

- Same log4j/mitigation/resolved-vs-raw-jvm-args branching as client.
- The classpath has a quirk: when `needs_log4j_config_override(mc_version)`, the server jar is **copied** to `<server_dir>/server.jar` and patched in-place (`patch_log4j_config_in_jar`). The builder rewrites the classpath string to substitute that patched path for the original server JAR.
- Tail is always `--port <port> nogui`.

### `parse_shim_jar_filenames` (pure)

Pulled out of `link_shim_jars` for tests. Reads the body of `unix_args.txt` and returns the bare `.jar` filenames that need to be symlinked into the server dir. Skips comments, blanks, flag-like tokens, and absolute paths; returns only the file-name component.

---

## VFS layer

`vfs.rs` defines an in-memory `VfsTree` of `VfsEntry::{ Dir { children }, File { source } }`. Commands build the tree, then call `realize_vfs(&tree, &target)`, which:

1. Creates each directory under `target`.
2. For each file entry, creates a symlink from the virtual path to the canonical source path on disk (typically a JAR in the global cache, or a config file in the modpack).

Symlinks (not copies) are what make the modpack workspace and the launched-process working directory share state. Editing `mods/sodium/config/sodium-options.json` in the workspace immediately affects the next launch.

Tests cover the tree-building API (add file at root / nested / overwrite / mirror from real disk) in isolation from filesystem realization.

---

## Java resolution

`java::resolve_java(cache_dir, mc_version, loader, http_client, java_override)` returns `(path_to_java, major_version)`. Resolution order:

1. `--java <path>` override if given (validate by running `java -version`).
2. `JAVA_HOME` env if set and version-compatible.
3. `which java` on PATH if version-compatible.
4. Auto-download a JDK from Adoptium (`api/adoptium.rs`) matching the required major version.

The required major version is derived from `(mc_version, loader)`:

- MC ≥ 1.20.5 → Java 21
- MC 1.17 – 1.20.4 → Java 17
- MC 1.16 – 1.16.5 → Java 8 (Forge legacy quirks add wrinkles)

`java.rs::tests` covers the version-matching logic for every loader on each MC version range.

---

## Log4j handling

Two related concerns for the [CVE-2021-44228](https://nvd.nist.gov/vuln/detail/CVE-2021-44228) family:

| Helper | What it does | When |
|---|---|---|
| `log4j_mitigation_args` | Returns `["-Dlog4j2.formatMsgNoLookups=true"]` as a flag the launched JVM picks up. | MC < 1.18.1 *and* MC ≤ 1.20 (the flag was the upstream patch before they shipped a config-based fix; modern versions don't need it) |
| `needs_log4j_config_override` | Triggers writing a hardened `log4j2.xml` and pointing the launched JVM at it. | MC < 1.18.1 (the flag alone isn't enough — needs the config replacement) |
| `write_log4j_config(dir)` | Writes the bundled `log4j_compat.xml` into `dir/`, returns the JVM args to point at it. | Called by client/server runners when `needs_log4j_config_override` is true |
| `patch_log4j_config_in_jar` | Replaces the embedded `log4j2.xml` inside a copied server JAR. | Server vanilla path only, for versions where the bundled config is the vulnerable one |

The exact version ranges are encoded in `java.rs` and unit-tested.

---

## Signal handling

Defined in `commands/launch/mod.rs`. The runtime has to forward Ctrl-C and SIGTERM to the Minecraft child — otherwise Ctrl-C kills yammm but leaves Minecraft orphaned, and SIGTERM during shutdown doesn't reach the server.

### Unix path

- **SIGINT (Ctrl-C)**: `ctrlc::set_handler` (runs on a dedicated thread via self-pipe — so the handler body can freely call non async-signal-safe code: atomic stores, `libc::kill`). Sets `INTERRUPTED.store(true, Release)` and `kill(CHILD_PID, SIGINT)`.
- **SIGTERM**: `sigaction(2)` with a real signal handler (not `signal(3)` — its reset-to-default semantics differ across systems). The handler **must be async-signal-safe**: only `libc::kill`, atomic load, `libc::_exit(143)`. It forwards SIGTERM to the child and then exits with the conventional 128+SIGTERM exit code.

Each `unsafe` block has a multi-line `// SAFETY:` comment explaining the invariant. None of the signal-handler code is reached without the corresponding atomic having been set after a successful spawn, so PID-zero / never-spawned races are not a concern.

### Windows path

`ctrlc::set_handler` invokes `taskkill /PID <pid> /T /F` via `Command::spawn` from its own thread (not from the actual signal context — `ctrlc` shields us from that). `/T` propagates the kill to the child tree, `/F` is force.

### Wait loop

`wait_for_child` polls `try_wait` every 100ms. When `INTERRUPTED` becomes true, it sets a 10-second grace deadline; if the child doesn't exit by then, it issues `child.kill()`. This is intentionally simple — no async cancellation tokens, no `tokio::select!`. The launch tree runs synchronously in tokio's `block_on` (see `bin/yammm.rs`); a polled loop with atomic flags is the right shape here.

---

## Testing strategy

The launch tree has ~75 tests (across `vfs`, `libraries`, `java`, `prepare`, `client`, `server`, and `mod`) that follow the [pure-builder pattern](conventions.md#3-pure-builder-extraction-pattern). What's tested:

- All four pure `build_*_java_args` orderings, conditional branches, filter rules
- VFS tree manipulation and on-disk realization (via `tempfile`)
- MC library OS/feature rule evaluation
- Java version requirements for every loader × MC version range
- `is_bundler_jar` (via tempdirs + `zip::ZipWriter`)
- `parse_shim_jar_filenames`, `deduplicate_classpath`, `resolve_jvm_args`, `extract_module_path_jars`, `build_java_command`

What's **not** tested in unit tests (by design):

- `cmd.spawn()` and `wait_for_child`'s polling loop — libc territory, manual / E2E only.
- Signal handlers — unsafe libc with no clean way to drive them in process.
- The async `run()` orchestrators end-to-end — they'd need a wholesale DI refactor (filesystem trait, process trait, HTTP trait) for a margin of testability we don't need. The `api/*` modules already cover HTTP via `mockito`; the pure builders cover the data shaping.

---

## Related

- [services.md](services.md) — `MinecraftClient`, `FabricClient`, `QuiltClient`, `ForgeClient`, `NeoForgeClient`, `AdoptiumClient`
- [caching.md](caching.md) — the `<cache_dir>/{minecraft,loaders}/<ver>/` layout
- [storage.md](storage.md) — the `config/`, `resources/`, `mods/<id>/config/` layout that becomes the VFS
- [conventions.md](conventions.md) — pure-builder pattern, output channel
