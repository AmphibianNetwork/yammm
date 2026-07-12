//! Error classification for exit codes.
//!
//! Walks the full `anyhow` error chain looking for a structured
//! `YammmError` (or bare `ApiError`) and returns its exit code. Falls
//! back to a `reqwest::Error` (network → 6). Default: 1.
//!
//! Walking the chain matters because providers add human-readable context
//! with `.with_context()` on top of a structured error — without chain
//! traversal, every `add nonexistent-mod` would return exit code 1
//! instead of the documented 3.

mod kinds;

pub use kinds::YammmError;

/// Map an `anyhow::Error` to a process exit code by walking the cause chain.
pub fn exit_code(err: &anyhow::Error) -> i32 {
	for cause in err.chain() {
		if let Some(yammm_err) = cause.downcast_ref::<YammmError>() {
			return yammm_err.exit_code();
		}
		if let Some(api_err) =
			cause.downcast_ref::<crate::api::error::ApiError>()
		{
			return api_err.exit_code();
		}
	}
	for cause in err.chain() {
		if let Some(re) = cause.downcast_ref::<reqwest::Error>()
			&& (re.is_timeout() || re.is_connect())
		{
			return 6;
		}
	}
	1
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::api::error::ApiError;

	#[test]
	fn test_exit_code_yammm_error() {
		let err: anyhow::Error = YammmError::mod_not_found("test").into();
		assert_eq!(exit_code(&err), 3);
	}

	#[test]
	fn test_exit_code_generic_error() {
		let err = anyhow::anyhow!("something went wrong");
		assert_eq!(exit_code(&err), 1);
	}

	#[test]
	fn test_exit_code_wrapped_yammm_error() {
		let inner: anyhow::Error = YammmError::network_error("timeout").into();
		let wrapped = inner.context("additional context");
		assert_eq!(exit_code(&wrapped), 6);
	}

	#[test]
	fn test_exit_code_walks_chain_for_yammm_not_found() {
		// Provider adds context on top — the structured error is buried.
		let inner: anyhow::Error = YammmError::mod_not_found("jei").into();
		let wrapped = inner.context("Failed to fetch mod jei");
		let wrapped = wrapped.context("while resolving dependencies");
		assert_eq!(exit_code(&wrapped), 3);
	}

	#[test]
	fn test_exit_code_walks_chain_for_bare_api_error() {
		let inner: anyhow::Error = ApiError::NotFound("jei".into()).into();
		let wrapped = inner.context("contextual message");
		assert_eq!(exit_code(&wrapped), 3);
	}
}
