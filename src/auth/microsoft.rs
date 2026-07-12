//! Microsoft OAuth2 device code flow for obtaining access and refresh tokens.
//!
//! Implements the device code grant type so users can authenticate in a browser
//! while the CLI polls Microsoft for the resulting token.

use anyhow::Result;
use serde::Deserialize;

use super::MS_CLIENT_ID;

const DEVICE_CODE_URL: &str =
	"https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str =
	"https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.SignIn XboxLive.offline_access";

/// Fallback lifetime used when Microsoft's token response omits `expires_in`.
/// Microsoft's documented default is 24 hours; refresh happens via the
/// refresh-token path before this matters in practice.
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 86400;

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
	device_code: String,
	user_code: String,
	verification_uri: String,
	expires_in: u64,
	interval: Option<u64>,
	message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
	error: String,
	error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenOrError {
	Error(ErrorResponse),
	Token(TokenResponse),
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
	access_token: String,
	refresh_token: Option<String>,
	expires_in: Option<u64>,
}

/// Initiates the OAuth2 device code flow, returning `(access_token, refresh_token, expires_in)`.
///
/// Prompts the user to visit a URL and enter a code, then polls the token endpoint
/// until the user authorises or the code expires.
pub async fn device_code_flow(
	http_client: &reqwest::Client
) -> Result<(String, String, u64)> {
	let resp = crate::api::retry::send_retried_request(
		&crate::api::retry::AUTH_RETRY_CONFIG,
		|| {
			let client = http_client.clone();
			async move {
				client
					.post(DEVICE_CODE_URL)
					.form(&[("client_id", MS_CLIENT_ID), ("scope", SCOPE)])
					.send()
					.await
					.map_err(|e| {
						crate::errors::YammmError::network_error(e.to_string())
					})
			}
		},
	)
	.await
	.map_err(|e| crate::errors::YammmError::network_error(e.to_string()))?;

	let body = resp.text().await?;

	if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
		return Err(crate::errors::YammmError::network_error(format!(
			"Device code request failed: {}",
			err.error_description.as_deref().unwrap_or(&err.error)
		))
		.into());
	}

	let resp: DeviceCodeResponse = serde_json::from_str(&body)?;

	let message = resp.message.clone().unwrap_or_else(|| {
		format!("To sign in, visit: {}", resp.verification_uri)
	});
	if crate::output::is_json_mode() {
		// The device-code prompt is critical: without it the user
		// can't authorize the session. Stdout in JSON mode is reserved
		// for the result document, so we surface the prompt on stderr
		// where scripts already route human-facing text.
		eprintln!("Microsoft Authentication");
		eprintln!("{message}");
		eprintln!("Enter code: {}", resp.user_code);
		eprintln!("Waiting for authorization...");
	} else {
		crate::output::heading("Microsoft Authentication");
		crate::output::blank_line();
		crate::output::bullet(&message);
		crate::output::bullet(format!("Enter code: {}", resp.user_code));
		crate::output::blank_line();
		crate::output::info("Waiting for authorization...");
	}

	let mut poll_interval =
		std::time::Duration::from_secs(resp.interval.unwrap_or(5));
	let deadline = std::time::Instant::now()
		+ std::time::Duration::from_secs(resp.expires_in);

	// Poll the token endpoint: wait for the user to complete login in the browser,
	// then keep requesting a token. Handle three error states per RFC 8628 §3.5:
	//   - authorization_pending: user hasn't approved yet, keep polling
	//   - slow_down: the interval MUST be increased by 5 seconds for this and
	//     all subsequent requests (not just sleep extra once)
	//   - expired_token: the device code has timed out, abort
	loop {
		tokio::time::sleep(poll_interval).await;
		if std::time::Instant::now() > deadline {
			return Err(crate::errors::YammmError::network_error(
				"Authentication timed out",
			)
			.into());
		}

		let token_body = http_client
			.post(TOKEN_URL)
			.form(&[
				("client_id", MS_CLIENT_ID),
				("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
				("device_code", &resp.device_code),
			])
			.send()
			.await?
			.text()
			.await?;

		// The token endpoint returns either an error or a token via the
		// untagged enum TokenOrError — serde tries ErrorResponse first, then TokenResponse.
		match serde_json::from_str::<TokenOrError>(&token_body)? {
			TokenOrError::Error(err) => {
				if err.error == "authorization_pending" {
					continue;
				}
				if err.error == "slow_down" {
					// RFC 8628 §3.5: bump the persistent polling interval, not
					// just a one-shot sleep. The previous implementation slept
					// an extra 5s once and immediately went back to the old
					// (too-aggressive) interval, which earns another slow_down.
					poll_interval = bump_poll_interval(poll_interval);
					continue;
				}
				if err.error == "expired_token" {
					return Err(crate::errors::YammmError::network_error(
						"Device code expired. Please try again.",
					)
					.into());
				}
				return Err(crate::errors::YammmError::network_error(format!(
					"Authentication failed: {}",
					err.error_description.as_deref().unwrap_or(&err.error)
				))
				.into());
			}
			TokenOrError::Token(token_resp) => {
				let refresh = token_resp.refresh_token.ok_or_else(|| {
					crate::errors::YammmError::network_error(
						"No refresh token received from Microsoft",
					)
				})?;
				let expires_in = token_resp
					.expires_in
					.unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);

				return Ok((token_resp.access_token, refresh, expires_in));
			}
		}
	}
}

/// Apply the RFC 8628 §3.5 rule: on `slow_down`, the polling interval MUST be
/// increased by 5 seconds. We also cap at 60s so a misbehaving server can't
/// stall auth forever.
fn bump_poll_interval(current: std::time::Duration) -> std::time::Duration {
	const SLOW_DOWN_INCREMENT: std::time::Duration =
		std::time::Duration::from_secs(5);
	const MAX_INTERVAL: std::time::Duration =
		std::time::Duration::from_secs(60);
	std::cmp::min(current + SLOW_DOWN_INCREMENT, MAX_INTERVAL)
}

/// Refreshes an expired access token using a stored refresh token.
///
/// Returns `(new_access_token, new_refresh_token, expires_in)`. Falls back to the old
/// refresh token if Microsoft does not return a new one.
pub async fn refresh_access_token(
	http_client: &reqwest::Client,
	refresh_token: &str,
) -> Result<(String, String, u64)> {
	let refresh_token_owned = refresh_token.to_string();
	let resp = crate::api::retry::send_retried_request(
		&crate::api::retry::AUTH_RETRY_CONFIG,
		|| {
			let client = http_client.clone();
			let rt = refresh_token_owned.clone();
			async move {
				client
					.post(TOKEN_URL)
					.form(&[
						("client_id", MS_CLIENT_ID),
						("grant_type", "refresh_token"),
						("refresh_token", &rt),
						("scope", SCOPE),
					])
					.send()
					.await
					.map_err(|e| {
						crate::errors::YammmError::network_error(e.to_string())
					})
			}
		},
	)
	.await
	.map_err(|e| crate::errors::YammmError::network_error(e.to_string()))?;

	let body = resp.text().await?;

	match serde_json::from_str::<TokenOrError>(&body)? {
		TokenOrError::Error(err) => {
			Err(crate::errors::YammmError::network_error(format!(
				"Token refresh failed: {}",
				err.error_description.as_deref().unwrap_or(&err.error)
			))
			.into())
		}
		TokenOrError::Token(resp) => {
			let new_refresh = resp
				.refresh_token
				.unwrap_or_else(|| refresh_token.to_string());
			let expires_in =
				resp.expires_in.unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);
			Ok((resp.access_token, new_refresh, expires_in))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_device_code_response_deserialization() {
		let json = r##"{
			"device_code": "DC123",
			"user_code": "ABCD-EFGH",
			"verification_uri": "https://microsoft.com/link",
			"expires_in": 900,
			"interval": 5,
			"message": "To sign in, visit"
		}"##;
		let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
		assert_eq!(resp.device_code, "DC123");
		assert_eq!(resp.user_code, "ABCD-EFGH");
		assert_eq!(resp.expires_in, 900);
		assert_eq!(resp.interval, Some(5));
	}

	#[test]
	fn test_error_response_deserialization() {
		let json = r##"{
			"error": "authorization_pending",
			"error_description": "User has not yet completed the flow"
		}"##;
		let resp: ErrorResponse = serde_json::from_str(json).unwrap();
		assert_eq!(resp.error, "authorization_pending");
		assert!(resp.error_description.is_some());
	}

	#[test]
	fn test_token_or_error_token() {
		let json = r##"{
			"access_token": "AT123",
			"refresh_token": "RT456",
			"expires_in": 3600
		}"##;
		let result: TokenOrError = serde_json::from_str(json).unwrap();
		match result {
			TokenOrError::Token(t) => {
				assert_eq!(t.access_token, "AT123");
				assert_eq!(t.refresh_token, Some("RT456".to_string()));
				assert_eq!(t.expires_in, Some(3600));
			}
			TokenOrError::Error(_) => panic!("Expected Token"),
		}
	}

	#[test]
	fn test_bump_poll_interval_adds_five_seconds() {
		let bumped = bump_poll_interval(std::time::Duration::from_secs(5));
		assert_eq!(bumped, std::time::Duration::from_secs(10));
		let bumped_again = bump_poll_interval(bumped);
		assert_eq!(bumped_again, std::time::Duration::from_secs(15));
	}

	#[test]
	fn test_bump_poll_interval_caps_at_60s() {
		let bumped = bump_poll_interval(std::time::Duration::from_secs(58));
		assert_eq!(bumped, std::time::Duration::from_secs(60));
		// Already at the cap — stays at the cap.
		let still_capped =
			bump_poll_interval(std::time::Duration::from_secs(60));
		assert_eq!(still_capped, std::time::Duration::from_secs(60));
	}

	#[test]
	fn test_token_or_error_error() {
		let json = r##"{
			"error": "expired_token",
			"error_description": "Code expired"
		}"##;
		let result: TokenOrError = serde_json::from_str(json).unwrap();
		match result {
			TokenOrError::Error(e) => {
				assert_eq!(e.error, "expired_token");
			}
			TokenOrError::Token(_) => panic!("Expected Error"),
		}
	}
}
