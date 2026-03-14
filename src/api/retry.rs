//! Retry and rate-limiting utilities with exponential backoff.
//!
//! Honors `Retry-After` headers for HTTP 429. Retries 429, 5xx, and
//! network errors. Returns immediately on 4xx client errors.

use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const INITIAL_RETRY_DELAY_MS: u64 = 500;

/// Error returned when a retried request ultimately fails.
/// Carries HTTP status (0 for non-HTTP errors) and message.
#[derive(Debug, Clone, thiserror::Error)]
pub struct RetryError {
	pub status: u16,
	pub message: String,
}

impl std::fmt::Display for RetryError {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		if self.status > 0 {
			write!(f, "HTTP {}: {}", self.status, self.message)
		} else {
			write!(f, "{}", self.message)
		}
	}
}

/// Response metadata for retry-after header inspection.
#[derive(Debug, Clone)]
pub struct ResponseMeta {
	pub status: u16,
	pub retry_after: Option<u64>,
}

impl std::fmt::Display for ResponseMeta {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(
			f,
			"HTTP {} (retry-after: {:?})",
			self.status, self.retry_after
		)
	}
}

impl ResponseMeta {
	pub fn from_response(resp: &reqwest::Response) -> Self {
		let status = resp.status().as_u16();
		let retry_after = resp
			.headers()
			.get("retry-after")
			.and_then(|v| v.to_str().ok())
			.and_then(|s| s.parse::<u64>().ok());
		Self {
			status,
			retry_after,
		}
	}
}

impl RetryError {
	fn from_anyhow(e: &anyhow::Error) -> Self {
		if let Some(meta) = e.downcast_ref::<ResponseMeta>() {
			Self {
				status: meta.status,
				message: meta.to_string(),
			}
		} else {
			Self {
				status: 0,
				message: format!("{:#}", e),
			}
		}
	}
}

pub struct RetryConfig {
	pub max_retries: u32,
	pub initial_delay_ms: u64,
}

impl Default for RetryConfig {
	fn default() -> Self {
		Self {
			max_retries: MAX_RETRIES,
			initial_delay_ms: INITIAL_RETRY_DELAY_MS,
		}
	}
}

/// Execute an async operation with exponential backoff.
/// Retries on 429, 5xx, and network errors. Returns immediately on other errors.
pub async fn retry_request<F, Fut, T>(
	config: &RetryConfig,
	mut f: F,
) -> Result<T, anyhow::Error>
where
	F: FnMut() -> Fut,
	Fut: std::future::Future<Output = Result<T, anyhow::Error>>,
{
	let mut last_err: Option<anyhow::Error> = None;

	for attempt in 0..=config.max_retries {
		match f().await {
			Ok(result) => return Ok(result),
			Err(e) => {
				let should_retry = is_retryable(&e);

				if !should_retry {
					return Err(e);
				}

				if attempt < config.max_retries {
					let delay =
						retry_delay(&e, attempt, config.initial_delay_ms);
					tracing::warn!(
						"Retry {}/{} in {}ms: {}",
						attempt + 1,
						config.max_retries,
						delay.as_millis(),
						e
					);
					tokio::time::sleep(delay).await;
				}

				last_err = Some(e);
			}
		}
	}

	Err(last_err.unwrap_or_else(|| {
		crate::errors::YammmError::network_error(format!(
			"Request failed after {} retries",
			config.max_retries
		))
		.into()
	}))
}

/// Check if an error is retryable (network, rate-limit, hash mismatch).
fn is_retryable(err: &anyhow::Error) -> bool {
	if err
		.downcast_ref::<reqwest::Error>()
		.map(|re| re.is_timeout() || re.is_connect() || re.is_request())
		.unwrap_or(false)
	{
		return true;
	}

	if let Some(yammm_err) = err.downcast_ref::<crate::errors::YammmError>() {
		return yammm_err.is_retryable();
	}

	false
}

/// Calculate retry delay. Honors `Retry-After` for 429, otherwise exponential backoff.
fn retry_delay(
	err: &anyhow::Error,
	attempt: u32,
	initial_delay_ms: u64,
) -> Duration {
	if let Some(meta) = err.downcast_ref::<ResponseMeta>() {
		if meta.status == 429 {
			if let Some(secs) = meta.retry_after {
				return Duration::from_secs(secs);
			}
		}
	}

	Duration::from_millis(initial_delay_ms * 2u64.pow(attempt))
}

/// Send a GET request with retry, mapping error via closure.
pub async fn send_retried_mapped<E>(
	client: &reqwest::Client,
	url: &str,
	headers: Vec<(String, String)>,
	map_err: impl Fn(RetryError) -> E,
) -> Result<reqwest::Response, E> {
	match send_retried(client, url, headers).await {
		Ok(resp) => Ok(resp),
		Err(retry_err) => Err(map_err(retry_err)),
	}
}

/// Send a GET request with retry. Returns `Err(RetryError)` on failure.
pub async fn send_retried(
	client: &reqwest::Client,
	url: &str,
	headers: Vec<(String, String)>,
) -> Result<reqwest::Response, RetryError> {
	let config = RetryConfig::default();
	let client = client.clone();
	let url = url.to_string();

	let result = retry_request(&config, || {
		let client = client.clone();
		let url = url.clone();
		let headers = headers.clone();
		async move {
			let mut req = client.get(&url);
			for (key, value) in &headers {
				req = req.header(key.as_str(), value.as_str());
			}
			let resp = req.send().await.map_err(|e| {
				crate::errors::YammmError::network_error(format!("{}", e))
			})?;
			let status = resp.status().as_u16();
			if status == 429 || (500..=599).contains(&status) {
				let meta = ResponseMeta::from_response(&resp);
				return Err(crate::errors::YammmError::network_error(format!(
					"{}",
					meta
				))
				.into());
			}
			Ok(resp)
		}
	})
	.await;

	match result {
		Ok(resp) => Ok(resp),
		Err(e) => Err(RetryError::from_anyhow(&e)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_retry_error_display_http() {
		let err = RetryError {
			status: 429,
			message: "rate limited".to_string(),
		};
		assert_eq!(format!("{}", err), "HTTP 429: rate limited");
	}

	#[test]
	fn test_retry_error_display_network() {
		let err = RetryError {
			status: 0,
			message: "connection refused".to_string(),
		};
		assert_eq!(format!("{}", err), "connection refused");
	}

	#[test]
	fn test_retry_error_from_anyhow_with_meta() {
		let meta = ResponseMeta {
			status: 503,
			retry_after: Some(30),
		};
		let err: anyhow::Error = anyhow::anyhow!("upstream").context(meta);
		let retry_err = RetryError::from_anyhow(&err);
		assert_eq!(retry_err.status, 503);
		assert!(retry_err.message.contains("503"));
	}

	#[test]
	fn test_retry_error_from_anyhow_without_meta() {
		let err: anyhow::Error = anyhow::anyhow!("connection refused");
		let retry_err = RetryError::from_anyhow(&err);
		assert_eq!(retry_err.status, 0);
		assert!(retry_err.message.contains("connection refused"));
	}

	#[test]
	fn test_response_meta_from_response() {
		let meta = ResponseMeta {
			status: 429,
			retry_after: Some(60),
		};
		assert_eq!(meta.status, 429);
		assert_eq!(meta.retry_after, Some(60));
	}

	#[test]
	fn test_response_meta_display() {
		let meta = ResponseMeta {
			status: 429,
			retry_after: Some(60),
		};
		let display = format!("{}", meta);
		assert!(display.contains("429"));
		assert!(display.contains("60"));
	}

	#[test]
	fn test_retry_config_default() {
		let config = RetryConfig::default();
		assert_eq!(config.max_retries, MAX_RETRIES);
		assert_eq!(config.initial_delay_ms, INITIAL_RETRY_DELAY_MS);
	}

	#[test]
	fn test_is_retryable_yammm_network_error() {
		let err: anyhow::Error =
			crate::errors::YammmError::network_error("timeout").into();
		assert!(is_retryable(&err));
	}

	#[test]
	fn test_is_retryable_yammm_hash_mismatch() {
		let err: anyhow::Error =
			crate::errors::YammmError::hash_mismatch("mod.jar", "abc", "def")
				.into();
		assert!(is_retryable(&err));
	}

	#[test]
	fn test_is_retryable_non_retryable() {
		let err: anyhow::Error =
			crate::errors::YammmError::invalid_args("bad input").into();
		assert!(!is_retryable(&err));
	}

	#[test]
	fn test_is_retryable_generic_error() {
		let err: anyhow::Error = anyhow::anyhow!("something went wrong");
		assert!(!is_retryable(&err));
	}

	#[tokio::test]
	async fn test_retry_request_succeeds_immediately() {
		let config = RetryConfig {
			max_retries: 3,
			initial_delay_ms: 0,
		};
		let mut attempts = 0;
		let result = retry_request(&config, || {
			attempts += 1;
			async move { Ok::<i32, anyhow::Error>(42) }
		})
		.await
		.unwrap();
		assert_eq!(result, 42);
		assert_eq!(attempts, 1);
	}

	#[tokio::test]
	async fn test_retry_request_succeeds_after_retry() {
		let config = RetryConfig {
			max_retries: 3,
			initial_delay_ms: 0,
		};
		let mut attempts = 0;
		let result = retry_request(&config, || {
			attempts += 1;
			async move {
				if attempts < 3 {
					Err(crate::errors::YammmError::network_error("fail").into())
				} else {
					Ok(99)
				}
			}
		})
		.await
		.unwrap();
		assert_eq!(result, 99);
	}

	#[tokio::test]
	async fn test_retry_request_fails_all_attempts() {
		let config = RetryConfig {
			max_retries: 2,
			initial_delay_ms: 0,
		};
		let result: Result<i32, anyhow::Error> =
			retry_request(&config, || async {
				Err(crate::errors::YammmError::network_error("always fail")
					.into())
			})
			.await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_retry_request_non_retryable_fails_fast() {
		let config = RetryConfig {
			max_retries: 3,
			initial_delay_ms: 0,
		};
		let mut attempts = 0;
		let result: Result<i32, anyhow::Error> = retry_request(&config, || {
			attempts += 1;
			async { Err(crate::errors::YammmError::invalid_args("bad").into()) }
		})
		.await;
		assert!(result.is_err());
		assert_eq!(attempts, 1);
	}

	#[test]
	fn test_retry_delay_with_retry_after() {
		let meta = ResponseMeta {
			status: 429,
			retry_after: Some(10),
		};
		let err: anyhow::Error = anyhow::anyhow!("rate limited").context(meta);
		let delay = retry_delay(&err, 0, 500);
		assert_eq!(delay, Duration::from_secs(10));
	}

	#[test]
	fn test_retry_delay_exponential_backoff() {
		let err: anyhow::Error =
			crate::errors::YammmError::network_error("timeout").into();
		assert_eq!(retry_delay(&err, 0, 500), Duration::from_millis(500));
		assert_eq!(retry_delay(&err, 1, 500), Duration::from_millis(1000));
		assert_eq!(retry_delay(&err, 2, 500), Duration::from_millis(2000));
	}
}
