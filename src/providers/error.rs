//! Typed error for the `ModSourceProvider` trait.
//!
//! Callers in `services/` need to distinguish "this mod doesn't exist on the
//! source" from "the source is rate-limited" from "the network is broken"
//! without grepping error message strings. `ProviderError` exposes those
//! cases as discriminants; everything else falls through `Other` and is
//! still wrappable into `anyhow::Error` for ergonomic `?` propagation.

use crate::api::error::ApiError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
	/// The mod or version was not found on the source.
	#[error("{provider} has no entry for {what}")]
	NotFound { provider: &'static str, what: String },

	/// The provider returned HTTP 429.
	#[error("{provider} rate-limited: {message}")]
	RateLimited {
		provider: &'static str,
		message: String,
	},

	/// Network or transport-level failure talking to the provider.
	#[error("{provider} network error: {message}")]
	Network {
		provider: &'static str,
		message: String,
	},

	/// Provider returned data we couldn't parse or that violated our
	/// expectations.
	#[error("{provider} returned bad data: {message}")]
	BadResponse {
		provider: &'static str,
		message: String,
	},

	/// Catch-all for anything that doesn't fit the discriminants above.
	#[error(transparent)]
	Other(#[from] anyhow::Error),
}

impl ProviderError {
	pub fn is_not_found(&self) -> bool {
		matches!(self, Self::NotFound { .. })
	}

	pub fn is_rate_limited(&self) -> bool {
		matches!(self, Self::RateLimited { .. })
	}

	pub fn is_retryable(&self) -> bool {
		match self {
			Self::RateLimited { .. } | Self::Network { .. } => true,
			Self::Other(e) => e
				.downcast_ref::<ApiError>()
				.is_some_and(ApiError::is_retryable),
			_ => false,
		}
	}

	/// Convert an `ApiError` into a `ProviderError`, tagging it with the
	/// source name so error messages identify which provider failed.
	pub fn from_api_error(err: ApiError, provider: &'static str) -> Self {
		match err {
			ApiError::NotFound(what) => Self::NotFound { provider, what },
			ApiError::Http {
				status: 404,
				message,
			} => Self::NotFound {
				provider,
				what: message,
			},
			ApiError::Http {
				status: 429,
				message,
			} => Self::RateLimited { provider, message },
			ApiError::Http { status, message }
				if (500..=599).contains(&status) =>
			{
				Self::Network {
					provider,
					message: format!("HTTP {}: {}", status, message),
				}
			}
			ApiError::Network(msg) => Self::Network {
				provider,
				message: msg,
			},
			ApiError::Request(re) if re.is_timeout() || re.is_connect() => {
				Self::Network {
					provider,
					message: re.to_string(),
				}
			}
			ApiError::Json(e) => Self::BadResponse {
				provider,
				message: e.to_string(),
			},
			other => Self::Other(other.into()),
		}
	}
}

pub type ProviderResult<T> = std::result::Result<T, ProviderError>;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn maps_not_found_directly() {
		let err = ApiError::not_found("jei");
		let mapped = ProviderError::from_api_error(err, "modrinth");
		assert!(mapped.is_not_found());
	}

	#[test]
	fn maps_http_404_to_not_found() {
		let err = ApiError::http(404, "no such mod");
		let mapped = ProviderError::from_api_error(err, "curseforge");
		assert!(mapped.is_not_found());
	}

	#[test]
	fn maps_http_429_to_rate_limited() {
		let err = ApiError::http(429, "slow down");
		let mapped = ProviderError::from_api_error(err, "modrinth");
		assert!(mapped.is_rate_limited());
		assert!(mapped.is_retryable());
	}

	#[test]
	fn maps_5xx_to_network() {
		let err = ApiError::http(503, "service unavailable");
		let mapped = ProviderError::from_api_error(err, "modrinth");
		assert!(mapped.is_retryable());
		assert!(!mapped.is_not_found());
	}

	#[test]
	fn other_is_not_retryable_unless_inner_is() {
		let err = ProviderError::Other(anyhow::anyhow!("oops"));
		assert!(!err.is_retryable());
	}
}
