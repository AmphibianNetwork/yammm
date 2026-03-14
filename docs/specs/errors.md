# Error Handling Specification

## Error Classification

yammm uses a typed `ErrorKind` enum attached to `anyhow::Error` for exit code determination. All internal error paths use `with_kind()` or `bail_with_kind!()` to classify errors. A minimal legacy fallback handles third-party `reqwest::Error` types that aren't wrapped by our code.

---

## ErrorKind Enum

| Code | Kind | Description |
|------|------|-------------|
| 1 | `General` | Catch-all for unexpected errors |
| 2 | `InvalidArgs` | Invalid CLI arguments or config parse failure |
| 3 | `ModNotFound` | Mod slug/ID not found on the source |
| 4 | `DownloadFailed` | Download or hash verification failure |
| 5 | `ConfigError` | Configuration file error |
| 6 | `NetworkError` | Network timeout, DNS failure, or HTTP 5xx |
| 7 | `StorageError` | I/O or storage failure |
| 8 | `DependencyError` | Dependency resolution failure |
| 9 | `VersionConflict` | No version satisfies constraints |
| 10 | `CircularDependency` | Circular dependency detected |

---

## Attaching ErrorKind

Errors are classified using `with_kind()` or the `bail_with_kind!` macro:

```rust
use crate::errors::{ErrorKind, with_kind};

return Err(with_kind(ErrorKind::ModNotFound, format!("Mod '{}' not found", id)));

// Or using the macro:
bail_with_kind!(ErrorKind::NetworkError, "Failed to fetch: HTTP {}", status);
```

---

## Legacy Exit Code Fallback

For errors without an attached `ErrorKind` (typically raw `reqwest::Error` from third-party code), the `legacy_exit_code()` function checks the error chain for:

1. `reqwest::Error` with `is_timeout()` or `is_connect()` → exit code 6 (network)
2. Everything else → exit code 1 (general)

The fragile string-matching heuristic has been removed. All yammm code paths use typed `ErrorKind`.

---

## User-Facing Error Display

Errors are displayed with chained context using `utils::print_error()`:

```
Error: Mod 'create' not found on Modrinth
```

Multi-line context for deeper errors:

```
Error: Failed to download mod 'fabric-api'
  Caused by: HTTP 403 Forbidden
  Caused by: CurseForge API key required
```

---

## Debug Output

When `--debug` is provided, `tracing` logs additional context:

```
[DEBUG] Querying Modrinth API...
[DEBUG] URL: https://api.modrinth.com/v2/project/fabric-api
[DEBUG] Response: 200 OK (123ms)
```

Controlled by `RUST_LOG` environment variable or `--debug` flag.

---

## Recovery Strategies

### Network Errors

1. Retry with exponential backoff (3 attempts max)
2. Respect `Retry-After` header on 429 responses
3. Fail with `NetworkError` if all retries exhausted

### Download Failures

1. Verify hash after download
2. If mismatch, retry up to 3 times with fresh download
3. If persistent failure, remove partial file and return `DownloadFailed`

### API Rate Limiting

1. Check `Retry-After` header
2. Wait and retry
3. Suggest using API key (CurseForge) if repeatedly rate-limited
