# Error Handling Specification

## Error Classification

yammm uses a typed `YammmError` enum attached to `anyhow::Error` for exit code determination. All internal error paths use convenience constructors (e.g. `YammmError::mod_not_found()`) to classify errors. A minimal legacy fallback handles third-party `reqwest::Error` types that aren't wrapped by our code.

---

## YammmError Enum

| Code | Variant | Description |
|------|---------|-------------|
| 1 | `General(String)` | Catch-all for unexpected errors |
| 2 | `InvalidArgs(String)` | Invalid CLI arguments or config parse failure |
| 3 | `ModNotFound(String)` | Mod slug/ID not found on the source |
| 4 | `DownloadFailed(String)` | Download failure |
| 4 | `HashMismatch { name, expected, actual }` | Hash verification failure |
| 5 | `ConfigError(String)` | Configuration file error |
| 6 | `NetworkError(String)` | Network timeout, DNS failure, or HTTP 5xx |
| 6 | `NetworkRequest(reqwest::Error)` | Raw reqwest error (auto-converted via `#[from]`) |
| 7 | `IoError(std::io::Error)` | I/O or storage failure (auto-converted via `#[from]`) |
| 3–8 | `Api(ApiError)` | API-layer error (delegates to `ApiError::exit_code()`) |
| 9 | `VersionConflict(String)` | No version satisfies constraints |
| 10 | `CircularDependency { mod_id, chain }` | Circular dependency detected |

The `Api` variant wraps `ApiError` from `api/error.rs`. Its exit code depends on the inner variant:

| ApiError variant | Exit code |
|-----------------|-----------|
| `NotFound` / `Http { status: 404 }` | 3 |
| `HashMismatch` | 4 |
| `Network` / `Request` | 6 |
| `Io` | 7 |
| All others (`Http`, `Json`, `Url`, `Install`) | 8 |

---

## Attaching YammmError

Errors are classified using convenience constructors:

```rust
use crate::errors::YammmError;

return Err(YammmError::mod_not_found(format!("Mod '{}' not found", id)).into());

return Err(YammmError::network_error("Failed to fetch: HTTP 500").into());

return Err(YammmError::hash_mismatch("mod.jar", "abc123", "def456").into());
```

Available constructors: `invalid_args()`, `mod_not_found()`, `download_failed()`, `config_error()`, `network_error()`, `version_conflict()`, `circular_dep()`, `general()`, `hash_mismatch()`.

> The `HashMismatch` variant is most often constructed indirectly via `From<ApiError>` (the download pipeline lifts an `ApiError::HashMismatch` into a `YammmError`). The `YammmError::hash_mismatch(...)` factory is reserved for direct callers and tests.

---

## Retryable Errors

`YammmError::is_retryable()` returns `true` for:
- `NetworkError` — always retryable
- `HashMismatch` — re-downloading may fix the mismatch
- `NetworkRequest` — only if `is_timeout()`, `is_connect()`, or `is_request()`
- `Api(api_err)` — delegates to `ApiError::is_retryable()`, which returns `true` for HTTP 429/5xx, network errors, and hash mismatches

---

## Legacy Exit Code Fallback

For errors without an attached `YammmError` (typically raw `reqwest::Error` from third-party code), the `exit_code()` function in `errors/mod.rs` checks the error chain for:

1. `reqwest::Error` with `is_timeout()` or `is_connect()` → exit code 6 (network)
2. Everything else → exit code 1 (general)

All yammm code paths use typed `YammmError`.

---

## User-Facing Error Display

Errors are displayed with chained context using `utils::print_error()` (re-exported at the crate root as `yammm::print_error`):

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
3. If persistent failure, remove partial file and return `DownloadFailed` or `HashMismatch`

### API Rate Limiting

1. Check `Retry-After` header
2. Wait and retry
3. Suggest using API key (CurseForge) if repeatedly rate-limited
