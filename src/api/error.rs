//! Shared error type for all API clients.
//!
//! | Retryable | Conditions |
//! |-----------|-----------|
//! | Yes | HTTP 429/5xx, network errors, hash mismatches |
//! | No | 4xx (except 429), not-found, parse errors |
//!
//! Exit codes: NotFound → 3, HashMismatch → 4, Network → 6, Io → 7, other → 8

use std::fmt;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
	#[error("HTTP {status}: {message}")]
	Http { status: u16, message: String },

	#[error(transparent)]
	Request(#[from] reqwest::Error),

	#[error(transparent)]
	Json(#[from] serde_json::Error),

	#[error(transparent)]
	Io(#[from] std::io::Error),

	#[error("not found: {0}")]
	NotFound(String),

	#[error("invalid URL: {0}")]
	Url(String),

	#[error("hash mismatch for {name}: expected {expected}, got {actual}")]
	HashMismatch {
		name: String,
		expected: String,
		actual: String,
	},

	#[error("install error: {0}")]
	Install(String),

	#[error("network error: {0}")]
	Network(String),
}

impl ApiError {
	pub fn not_found(msg: impl fmt::Display) -> Self {
		ApiError::NotFound(msg.to_string())
	}

	pub fn http(
		status: u16,
		message: impl fmt::Display,
	) -> Self {
		ApiError::Http {
			status,
			message: message.to_string(),
		}
	}

	pub fn install_error(msg: impl fmt::Display) -> Self {
		ApiError::Install(msg.to_string())
	}

	pub fn url_error(msg: impl fmt::Display) -> Self {
		ApiError::Url(msg.to_string())
	}

	pub fn network_error(msg: impl fmt::Display) -> Self {
		ApiError::Network(msg.to_string())
	}

	pub fn is_rate_limited(&self) -> bool {
		matches!(self, ApiError::Http { status: 429, .. })
	}

	pub fn is_retryable(&self) -> bool {
		match self {
			ApiError::Http { status: 429, .. }
			| ApiError::Http {
				status: 500..=599, ..
			} => true,
			ApiError::Network(_) | ApiError::HashMismatch { .. } => true,
			ApiError::Request(re) => {
				re.is_timeout() || re.is_connect() || re.is_request()
			}
			_ => false,
		}
	}

	pub fn exit_code(&self) -> i32 {
		match self {
			ApiError::NotFound(_) => 3,
			ApiError::HashMismatch { .. } => 4,
			ApiError::Http { status: 404, .. } => 3,
			ApiError::Network(_) | ApiError::Request(_) => 6,
			ApiError::Io(_) => 7,
			_ => 8,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_is_rate_limited() {
		let err = ApiError::Http {
			status: 429,
			message: "rate limited".to_string(),
		};
		assert!(err.is_rate_limited());
	}

	#[test]
	fn test_is_not_rate_limited() {
		let err200 = ApiError::Http {
			status: 200,
			message: String::new(),
		};
		let err404 = ApiError::Http {
			status: 404,
			message: String::new(),
		};
		let err500 = ApiError::Http {
			status: 500,
			message: String::new(),
		};
		assert!(!err200.is_rate_limited());
		assert!(!err404.is_rate_limited());
		assert!(!err500.is_rate_limited());
	}

	#[test]
	fn test_api_error_convenience_constructors() {
		let not_found = ApiError::not_found("missing");
		match not_found {
			ApiError::NotFound(msg) => assert_eq!(msg, "missing"),
			_ => panic!("Expected NotFound"),
		}

		let http = ApiError::http(500, "server error");
		match http {
			ApiError::Http { status, message } => {
				assert_eq!(status, 500);
				assert_eq!(message, "server error");
			}
			_ => panic!("Expected Http"),
		}

		let install = ApiError::install_error("install failed");
		match install {
			ApiError::Install(msg) => assert_eq!(msg, "install failed"),
			_ => panic!("Expected Install"),
		}

		let url = ApiError::url_error("bad url");
		match url {
			ApiError::Url(msg) => assert_eq!(msg, "bad url"),
			_ => panic!("Expected Url"),
		}
	}
}
