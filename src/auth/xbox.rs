//! Xbox Live authentication chain: MS access token → XBL token → XSTS token.
//!
//! The Microsoft access token is first exchanged for an Xbox Live (XBL) token
//! and user hash (UHS), then the XBL token is exchanged for an XSTS token
//! which is required by the Minecraft authentication endpoint.

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct XblResponse {
	Token: String,
	DisplayClaims: XblDisplayClaims,
}

#[derive(Debug, Deserialize)]
struct XblDisplayClaims {
	xui: Vec<XblUserInfo>,
}

#[derive(Debug, Deserialize)]
struct XblUserInfo {
	uhs: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct XstsResponse {
	Token: String,
	DisplayClaims: XstsDisplayClaims,
}

#[derive(Debug, Deserialize)]
struct XstsDisplayClaims {
	xui: Vec<XblUserInfo>,
}

/// Exchanges a Microsoft access token for an Xbox Live token and user hash.
///
/// Returns `(xbl_token, uhs)`.
pub async fn authenticate_xbl(
	http_client: &reqwest::Client,
	ms_access_token: &str,
) -> Result<(String, String)> {
	let ms_token_owned = ms_access_token.to_string();
	let resp: XblResponse = crate::api::retry::send_retried_request(
		&crate::api::retry::AUTH_RETRY_CONFIG,
		|| {
			let client = http_client.clone();
			let token = ms_token_owned.clone();
			async move {
				client
					.post("https://user.auth.xboxlive.com/user/authenticate")
					.header("Content-Type", "application/json")
					.header("Accept", "application/json")
					.json(&serde_json::json!({
						"Properties": {
							"AuthMethod": "RPS",
							"SiteName": "user.auth.xboxlive.com",
							"RpsTicket": format!("d={}", token),
						},
						"RelyingParty": "http://auth.xboxlive.com",
						"TokenType": "JWT",
					}))
					.send()
					.await
					.map_err(|e| {
						crate::errors::YammmError::network_error(e.to_string())
					})
			}
		},
	)
	.await
	.map_err(|e| crate::errors::YammmError::network_error(e.to_string()))?
	.json()
	.await?;

	let uhs = resp
		.DisplayClaims
		.xui
		.first()
		.ok_or_else(|| {
			crate::errors::YammmError::network_error(
				"No user hash in Xbox Live response",
			)
		})?
		.uhs
		.clone();

	Ok((resp.Token, uhs))
}

/// Exchanges an XBL token for an XSTS token, validating the user hash matches.
///
/// Returns `(xsts_token, uhs)`.
pub async fn authenticate_xsts(
	http_client: &reqwest::Client,
	xbl_token: &str,
	xbl_uhs: &str,
) -> Result<(String, String)> {
	let xbl_token_owned = xbl_token.to_string();
	let resp: XstsResponse = crate::api::retry::send_retried_request(
		&crate::api::retry::AUTH_RETRY_CONFIG,
		|| {
			let client = http_client.clone();
			let token = xbl_token_owned.clone();
			async move {
				client
					.post("https://xsts.auth.xboxlive.com/xsts/authorize")
					.header("Content-Type", "application/json")
					.header("Accept", "application/json")
					.json(&serde_json::json!({
						"Properties": {
							"SandboxId": "RETAIL",
							"UserTokens": [token],
						},
						"RelyingParty": "rp://api.minecraftservices.com/",
						"TokenType": "JWT",
					}))
					.send()
					.await
					.map_err(|e| {
						crate::errors::YammmError::network_error(e.to_string())
					})
			}
		},
	)
	.await
	.map_err(|e| crate::errors::YammmError::network_error(e.to_string()))?
	.json()
	.await?;

	let uhs = resp
		.DisplayClaims
		.xui
		.first()
		.ok_or_else(|| {
			crate::errors::YammmError::network_error(
				"No user hash in XSTS response",
			)
		})?
		.uhs
		.clone();

	// The UHS from the XSTS response must match the one from the XBL response;
	// a mismatch would indicate something went wrong in the auth chain.
	if uhs != xbl_uhs {
		return Err(crate::errors::YammmError::network_error(
			"XSTS user hash mismatch",
		)
		.into());
	}

	Ok((resp.Token, uhs))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_xbl_response_deserialization() {
		let json = r##"{
			"Token": "XBL_TOKEN_123",
			"DisplayClaims": {
				"xui": [{"uhs": "UHS_456"}]
			}
		}"##;
		let resp: XblResponse = serde_json::from_str(json).unwrap();
		assert_eq!(resp.Token, "XBL_TOKEN_123");
		assert_eq!(resp.DisplayClaims.xui.len(), 1);
		assert_eq!(resp.DisplayClaims.xui[0].uhs, "UHS_456");
	}

	#[test]
	fn test_xsts_response_deserialization() {
		let json = r##"{
			"Token": "XSTS_TOKEN_789",
			"DisplayClaims": {
				"xui": [{"uhs": "UHS_456"}]
			}
		}"##;
		let resp: XstsResponse = serde_json::from_str(json).unwrap();
		assert_eq!(resp.Token, "XSTS_TOKEN_789");
		assert_eq!(resp.DisplayClaims.xui[0].uhs, "UHS_456");
	}
}
