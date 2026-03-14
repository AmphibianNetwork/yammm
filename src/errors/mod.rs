//! Error classification for exit codes.
//!
//! Tries `YammmError` first (structured codes), then walks the chain
//! for `reqwest::Error` (network → 6). Default: 1.

mod kinds;

pub use kinds::YammmError;

/// Map an `anyhow::Error` to a process exit code.
pub fn exit_code(err: &anyhow::Error) -> i32 {
	if let Some(yammm_err) = err.downcast_ref::<YammmError>() {
		return yammm_err.exit_code();
	}
	// Fallback: detect network errors that weren't wrapped in YammmError
	for cause in err.chain() {
		if let Some(re) = cause.downcast_ref::<reqwest::Error>() {
			if re.is_timeout() || re.is_connect() {
				return 6;
			}
		}
	}
	1
}

#[cfg(test)]
mod tests {
	use super::*;

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
}
