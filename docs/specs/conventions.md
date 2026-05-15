# Code Conventions

Patterns that aren't enforced by the compiler but are load-bearing for how this codebase stays coherent. New code should follow them; existing code that doesn't is a candidate for follow-up cleanup, not a license to write more like it.

---

## 1. User output channel

There are **two output channels**, and they're not interchangeable.

| Channel            | What it's for                                                                                      | Module                             |
| ------------------ | -------------------------------------------------------------------------------------------------- | ---------------------------------- |
| User-facing output | The result of a command. Tables, search hits, config values, status lines, prompts, progress bars. | `crate::output`                    |
| Diagnostic logging | What happened internally, for debugging. URL hits, retry attempts, cache decisions.                | `tracing::{debug,info,warn,error}` |

A user running `yammm search sodium` expects the table on stdout. A user running with `--debug` or `RUST_LOG=debug` expects the diagnostic stream — separate. Crossing the streams is wrong: a `println!` for a debug message is hidden from the user's chosen log filter; a `tracing::info!` for a search result is hidden from default runs.

### Picking the right helper

```rust
// Result of the command (the user asked for this).
output::raw_line(format!("[{}] {} ({}) - {}", src, name, slug, desc));
output::raw_block(&table);  // pre-formatted multi-line content (tables, TOML dumps, JSON)

// Status / progress / feedback during the command.
output::success("Mod added.");
output::info("Resolving dependencies...");
output::warning("Optional dep skipped: not on Modrinth");
output::error("Hash mismatch — refusing to install");
output::heading("Importing modpack");
output::bullet(format!("{} v{}", m.name, m.version));
output::blank_line();

// Progress UI.
let pb = output::download_progress(total);
let sp = output::spinner("Fetching version info...");

// Confirmation.
let proceed = output::confirm("Apply 4 updates?", false)?;

// Debug / diagnostic only — never the primary output of a command.
tracing::debug!("Querying Modrinth /v2/project/{}", id);
tracing::warn!("Failed to fetch dependencies for {}: {}", mod_id, err);
```

`raw_line` / `raw_block` are the escape hatch for "this is the data the user asked for, please don't style it for me." Use them for command results that should be greppable, pipeable, or rendered verbatim (tables, dumps, dependency lists). Everything else uses the styled helpers.

### Why not `println!` directly?

Three reasons:

1. **Capture machinery.** `output::*` checks a thread-local capture flag; when the TUI in `commands/manage/tui.rs` runs a download, it captures status lines instead of letting them scroll the terminal. Bypassing `output::*` breaks that.
2. **Color control.** `output::set_colors_enabled(false)` (driven by `[output] color = false` in global config) only works for code that goes through `output::*`. Direct `println!` ignores it.
3. **One channel = one place to add behavior.** Future concerns (`--quiet`, `--json`, structured stdout) land in `output.rs`, not scattered across every command.

### Why not `tracing` for user output?

`tracing` is designed around filters — `RUST_LOG=error` would silence a search result table. The user did not ask to silence their search results. User output is unconditional; diagnostic logging is filterable. They have different audiences.

---

## 2. AppContext access

`AppContext` carries the shared state every command needs: global config, HTTP client, source registry, jar cache, the optional loaded modpack. Its fields are **private** and accessed through methods.

```rust
// Read access.
let key = ctx.global().api_keys.curseforge.as_deref();
let client = ctx.http_client();      // &reqwest::Client (cheap to clone)
let registry = ctx.registry();        // &Arc<SourceRegistry>
let cache = ctx.jar_cache();          // &JarCache
let cache_dir = ctx.cache_dir();      // &Path

// Optional modpack — None if not invoked inside a pack.
if let Some(app) = ctx.modpack() {
    ...
}

// Modpack required — errors out (exit code 2) if not in a pack.
let app = ctx.require_modpack()?;

// Mutation — only the `config` command should reach for this.
set_config_value(ctx.global_mut(), &key, &value)?;
ctx.global().save()?;
```

### Why not public fields?

The original layout exposed every field as `pub`. The accessor wrapping isn't ceremony — it's the place future cross-cutting concerns land. If we add per-command rate-limit counters, telemetry, or an offline-mode flag, those live behind the accessors without changing every call site.

It also stops a recurring class of mistake: code reaching into `ctx.modpack.as_ref().unwrap()` instead of `ctx.require_modpack()?`, which correctly returns `YammmError::InvalidArgs` (exit code 2) when the user invoked a modpack-only command from outside a pack.

### Don't add a new public field. Add an accessor.

If you find yourself wanting `pub something: T` on `AppContext`, the answer is `something(&self) -> &T` instead. Tests and callers should never know whether a value is computed, cached, or borrowed.

---

## 3. Pure-builder extraction pattern

Used in the launch subsystem ([specs/launch.md](launch.md)). The pattern: when a command's `run()` function contains a long imperative block that's interleaved with I/O, extract the pure data-transformation half into a named function the I/O half calls.

### Shape

```rust
pub async fn run(args: SomeArgs, ctx: AppContext) -> Result<()> {
    // I/O: download, read files, contact APIs.
    let raw_inputs = fetch_inputs(...).await?;

    // I/O: side effects that *produce* data the builder needs.
    let extra = if needs_extra(...) {
        compute_extra_side_effect(...).unwrap_or_default()
    } else {
        Vec::new()
    };

    // PURE: assemble the result from inputs + extra. No I/O, no globals.
    let result = build_thing(BuildInputs {
        raw_inputs: &raw_inputs,
        extra: &extra,
        // ... only the slices the builder reads, not whole context objects
    });

    // I/O: act on the result.
    spawn_or_write_or_send(result)?;
    Ok(())
}

fn build_thing(inputs: BuildInputs<'_>) -> Vec<String> {
    // Pure. Tested in isolation.
}
```

### Concrete example

`commands/launch/client.rs::run` does ten things: download mods, prepare the version JSON, fetch loader libs, resolve Java, build the VFS tree, realize it on disk, compute classpath, write log4j config, fetch an MSA token, **assemble ~95 lines of `java_args.push(...)` decisions**, and finally `spawn`. The assembly block is the one part that's worth testing: it has branches for offline/online, macOS, log4j-affected versions, `resolved_jvm_args` vs raw, user-supplied `--jvm-args`, asset index, game args. The rest is I/O orchestration.

`build_client_java_args(ClientLaunchInputs<'_>) -> Vec<String>` pulls that block out. The inputs struct borrows only what the builder reads — `natives_dir: &Path`, `mc_jvm_args: &[String]`, etc. — never the full `LaunchContext`, so tests can stack-allocate the bits they care about. Auth flows in via a small `ClientAuth { Offline | Online{...} }` enum, not the full `auth::AuthToken` (which carries refresh tokens, expiry, etc. that the launcher arg vector has no business knowing about).

### Rules of thumb

- **Inputs are the slices the builder reads, not whole context objects.** `mc_jvm_args: &[String]` beats `&LaunchContext` if the builder only touches that field — tests don't have to construct a `VersionInfo` they don't care about.
- **Side-effect-producing helpers move out, their results move in.** Log4j config writing is a side effect; the path it writes to is an input to the builder. The builder doesn't write the file.
- **Don't trait-ify the I/O for testability.** Tempting, but a `FsLike` + `ProcessSpawnLike` pair is more surface area than it's worth. Extract the pure half; leave the I/O as direct calls.
- **Test the builder, not the runner.** Tests assert on the produced `Vec<String>`: ordering, presence/absence of specific flags, conditional branches. `run()` itself is integration territory.

### When not to use this

If a `run()` is genuinely just orchestration — five `.await?` calls in sequence with no in-between data shaping — there's nothing pure to extract. Don't manufacture a builder for the sake of a test.

---

## Related

- [output.rs](../../src/output.rs) — the user-output channel
- [app.rs](../../src/app.rs) — `AppContext` and its accessors
- [commands/launch/client.rs](../../src/commands/launch/client.rs) and [commands/launch/server.rs](../../src/commands/launch/server.rs) — pure-builder examples
- [errors.md](errors.md) — typed errors that flow through the accessors and back out
