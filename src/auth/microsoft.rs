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

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
	device_code: String,
	user_code: String,
	verification_uri: String,
	expires_in: u64,
	interval: Option<u64>,
	#[allow(dead_code)]
	message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
	error: String,
	#[allow(dead_code)]
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
	let resp = http_client
		.post(DEVICE_CODE_URL)
		.form(&[("client_id", MS_CLIENT_ID), ("scope", SCOPE)])
		.send()
		.await?;

	let body = resp.text().await?;

	if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
		return Err(crate::errors::YammmError::network_error(format!(
			"Device code request failed: {}",
			err.error_description.as_deref().unwrap_or(&err.error)
		))
		.into());
	}

	let resp: DeviceCodeResponse = serde_json::from_str(&body)?;

	crate::output::heading("Microsoft Authentication");
	crate::output::blank_line();
	crate::output::bullet(format!(
		"To sign in, visit: {}",
		resp.verification_uri
	));
	crate::output::bullet(format!("Enter code: {}", resp.user_code));
	crate::output::blank_line();
	crate::output::info("Waiting for authorization...");

	let poll_interval =
		std::time::Duration::from_secs(resp.interval.unwrap_or(5));
	let deadline = std::time::Instant::now()
		+ std::time::Duration::from_secs(resp.expires_in);

	// Poll the token endpoint: wait for the user to complete login in the browser,
	// then keep requesting a token. Handle three error states:
	//   - authorization_pending: user hasn't approved yet, keep polling
	//   - slow_down: back off an extra 5 seconds before retrying
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
					tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
				let expires_in = token_resp.expires_in.unwrap_or(86400);

				return Ok((token_resp.access_token, refresh, expires_in));
			}
		}
	}
}

/// Refreshes an expired access token using a stored refresh token.
///
/// Returns `(new_access_token, new_refresh_token, expires_in)`. Falls back to the old
/// refresh token if Microsoft does not return a new one.
pub async fn refresh_access_token(
	http_client: &reqwest::Client,
	refresh_token: &str,
) -> Result<(String, String, u64)> {
	let body = http_client
		.post(TOKEN_URL)
		.form(&[
			("client_id", MS_CLIENT_ID),
			("grant_type", "refresh_token"),
			("refresh_token", refresh_token),
			("scope", SCOPE),
		])
		.send()
		.await?
		.text()
		.await?;

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
			let expires_in = resp.expires_in.unwrap_or(86400);
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
