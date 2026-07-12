//! Typed domain error enum with exit-code mapping.
//!
//! | Code | Variant |
//! |------|---------|
//! | 1 | General |
//! | 2 | InvalidArgs |
//! | 3 | ModNotFound |
//! | 4 | DownloadFailed / HashMismatch |
//! | 5 | ConfigError |
//! | 6 | NetworkError / NetworkRequest |
//! | 7 | IoError |
//! | 3-8 | Api (delegates to ApiError::exit_code()) |
//! | 9 | VersionConflict |
//! | 10 | CircularDependency |

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum YammmError {
	#[error("invalid arguments: {0}")]
	InvalidArgs(String),

	#[error("mod not found: {0}")]
	ModNotFound(String),

	#[error("download failed: {0}")]
	DownloadFailed(String),

	#[error("hash mismatch for {name}: expected {expected}, got {actual}")]
	HashMismatch {
		name: String,
		expected: String,
		actual: String,
	},

	#[error("config error: {0}")]
	ConfigError(String),

	#[error("network error: {0}")]
	NetworkError(String),

	#[error(transparent)]
	NetworkRequest(#[from] reqwest::Error),

	#[error("I/O error: {0}")]
	IoError(#[from] std::io::Error),

	#[error(transparent)]
	Api(#[from] crate::api::error::ApiError),

	#[error("version conflict: {0}")]
	VersionConflict(String),

	#[error("circular dependency: {mod_id} -> {chain}")]
	CircularDependency { mod_id: String, chain: String },

	#[error("error: {0}")]
	General(String),
}

impl YammmError {
	pub fn exit_code(&self) -> i32 {
		match self {
			Self::General(_) => 1,
			Self::InvalidArgs(_) => 2,
			Self::ModNotFound(_) => 3,
			Self::DownloadFailed(_) | Self::HashMismatch { .. } => 4,
			Self::ConfigError(_) => 5,
			Self::NetworkError(_) | Self::NetworkRequest(_) => 6,
			Self::IoError(_) => 7,
			Self::Api(api_err) => api_err.exit_code(),
			Self::VersionConflict(_) => 9,
			Self::CircularDependency { .. } => 10,
		}
	}

	/// Whether this error is worth retrying (network errors + hash mismatches).
	pub fn is_retryable(&self) -> bool {
		match self {
			Self::NetworkError(_) | Self::HashMismatch { .. } => true,
			Self::NetworkRequest(re) => {
				re.is_timeout() || re.is_connect() || re.is_request()
			}
			Self::Api(api_err) => api_err.is_retryable(),
			_ => false,
		}
	}
}

/// Convenience constructors for inline error creation.
impl YammmError {
	pub fn invalid_args(msg: impl Into<String>) -> Self {
		YammmError::InvalidArgs(msg.into())
	}

	pub fn mod_not_found(msg: impl Into<String>) -> Self {
		YammmError::ModNotFound(msg.into())
	}

	pub fn download_failed(msg: impl Into<String>) -> Self {
		YammmError::DownloadFailed(msg.into())
	}

	pub fn config_error(msg: impl Into<String>) -> Self {
		YammmError::ConfigError(msg.into())
	}

	pub fn network_error(msg: impl Into<String>) -> Self {
		YammmError::NetworkError(msg.into())
	}

	pub fn version_conflict(msg: impl Into<String>) -> Self {
		YammmError::VersionConflict(msg.into())
	}

	pub fn circular_dep(
		mod_id: impl Into<String>,
		chain: impl Into<String>,
	) -> Self {
		YammmError::CircularDependency {
			mod_id: mod_id.into(),
			chain: chain.into(),
		}
	}

	pub fn general(msg: impl Into<String>) -> Self {
		YammmError::General(msg.into())
	}

	#[allow(dead_code)] // Variant is constructed via From<ApiError>; this factory is reserved for direct callers and tests.
	pub fn hash_mismatch(
		name: impl Into<String>,
		expected: impl Into<String>,
		actual: impl Into<String>,
	) -> Self {
		YammmError::HashMismatch {
			name: name.into(),
			expected: expected.into(),
			actual: actual.into(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::api::error::ApiError;

	#[test]
	fn test_exit_code_mapping() {
		assert_eq!(YammmError::General("msg".into()).exit_code(), 1);
		assert_eq!(YammmError::InvalidArgs("msg".into()).exit_code(), 2);
		assert_eq!(YammmError::ModNotFound("x".into()).exit_code(), 3);
		assert_eq!(YammmError::DownloadFailed("msg".into()).exit_code(), 4);
		assert_eq!(
			YammmError::HashMismatch {
				name: "n".into(),
				expected: "e".into(),
				actual: "a".into()
			}
			.exit_code(),
			4
		);
		assert_eq!(YammmError::ConfigError("msg".into()).exit_code(), 5);
		assert_eq!(YammmError::NetworkError("msg".into()).exit_code(), 6);
		assert_eq!(
			YammmError::IoError(std::io::Error::new(
				std::io::ErrorKind::NotFound,
				"file"
			))
			.exit_code(),
			7
		);
		assert_eq!(
			YammmError::Api(ApiError::NotFound("x".into())).exit_code(),
			3
		);
		assert_eq!(
			YammmError::Api(ApiError::http(500, "server error")).exit_code(),
			8
		);
		assert_eq!(YammmError::VersionConflict("msg".into()).exit_code(), 9);
		assert_eq!(
			YammmError::CircularDependency {
				mod_id: "m".into(),
				chain: "c".into()
			}
			.exit_code(),
			10
		);
	}

	#[test]
	fn test_is_retryable() {
		assert!(!YammmError::General("msg".into()).is_retryable());
		assert!(!YammmError::InvalidArgs("msg".into()).is_retryable());
		assert!(!YammmError::ModNotFound("x".into()).is_retryable());
		assert!(!YammmError::DownloadFailed("msg".into()).is_retryable());
		assert!(!YammmError::ConfigError("msg".into()).is_retryable());
		assert!(YammmError::NetworkError("msg".into()).is_retryable());
		assert!(
			YammmError::HashMismatch {
				name: "n".into(),
				expected: "e".into(),
				actual: "a".into()
			}
			.is_retryable()
		);
		assert!(
			YammmError::Api(ApiError::http(429, "rate limited")).is_retryable()
		);
		assert!(
			YammmError::Api(ApiError::http(503, "unavailable")).is_retryable()
		);
		assert!(
			!YammmError::Api(ApiError::http(404, "not found")).is_retryable()
		);
		assert!(!YammmError::VersionConflict("msg".into()).is_retryable());
		assert!(
			!YammmError::CircularDependency {
				mod_id: "m".into(),
				chain: "c".into()
			}
			.is_retryable()
		);
	}

	#[tokio::test]
	async fn test_is_retryable_network_request() {
		let client = reqwest::Client::new();
		let result = client.get("http://0.0.0.0:1").send().await;
		if let Err(req_err) = result {
			assert!(YammmError::NetworkRequest(req_err).is_retryable());
		}
	}

	#[test]
	fn test_from_api_error_not_found() {
		let api_err = ApiError::NotFound("sodium".into());
		let yammm_err: YammmError = api_err.into();
		assert!(
			matches!(yammm_err, YammmError::Api(ApiError::NotFound(id)) if id == "sodium")
		);
	}

	#[test]
	fn test_from_api_error_hash_mismatch() {
		let api_err = ApiError::HashMismatch {
			name: "mod.jar".into(),
			expected: "abc".into(),
			actual: "def".into(),
		};
		let yammm_err: YammmError = api_err.into();
		assert!(matches!(
			yammm_err,
			YammmError::Api(ApiError::HashMismatch { name, expected, actual })
			if name == "mod.jar" && expected == "abc" && actual == "def"
		));
	}

	#[test]
	fn test_from_api_error_http() {
		let api_err = ApiError::Http {
			status: 500,
			message: "Internal Server Error".into(),
		};
		let yammm_err: YammmError = api_err.into();
		assert!(matches!(yammm_err, YammmError::Api(_)));
	}

	#[test]
	fn test_from_api_error_url() {
		let api_err = ApiError::Url("bad url".into());
		let yammm_err: YammmError = api_err.into();
		assert!(matches!(yammm_err, YammmError::Api(_)));
	}

	#[test]
	fn test_from_api_error_install() {
		let api_err = ApiError::Install("install failed".into());
		let yammm_err: YammmError = api_err.into();
		assert!(matches!(yammm_err, YammmError::Api(_)));
	}

	#[test]
	fn test_convenience_constructors() {
		assert!(matches!(
			YammmError::invalid_args("bad"),
			YammmError::InvalidArgs(s) if s == "bad"
		));
		assert!(matches!(
			YammmError::mod_not_found("x"),
			YammmError::ModNotFound(s) if s == "x"
		));
		assert!(matches!(
			YammmError::download_failed("fail"),
			YammmError::DownloadFailed(s) if s == "fail"
		));
		assert!(matches!(
			YammmError::config_error("cfg"),
			YammmError::ConfigError(s) if s == "cfg"
		));
		assert!(matches!(
			YammmError::network_error("net"),
			YammmError::NetworkError(s) if s == "net"
		));
		assert!(matches!(
			YammmError::version_conflict("vc"),
			YammmError::VersionConflict(s) if s == "vc"
		));
		assert!(matches!(
			YammmError::circular_dep("m", "c"),
			YammmError::CircularDependency { mod_id, chain }
			if mod_id == "m" && chain == "c"
		));
		assert!(matches!(
			YammmError::general("gen"),
			YammmError::General(s) if s == "gen"
		));
		assert!(matches!(
			YammmError::hash_mismatch("n", "e", "a"),
			YammmError::HashMismatch { name, expected, actual }
			if name == "n" && expected == "e" && actual == "a"
		));
	}
}
