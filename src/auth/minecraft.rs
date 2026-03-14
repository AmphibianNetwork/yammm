//! Minecraft authentication via Xbox tokens.
//!
//! Exchanges an XSTS token + user hash for a Minecraft access token,
//! then fetches the player profile to confirm game ownership.

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct McLoginResponse {
	access_token: String,
}

#[derive(Debug, Deserialize)]
pub struct McProfile {
	pub id: String,
	pub name: String,
}

/// Exchanges an XSTS token and user hash for a Minecraft access token.
pub async fn login_with_xbox(
	http_client: &reqwest::Client,
	xsts_token: &str,
	xsts_uhs: &str,
) -> Result<String> {
	let resp: McLoginResponse = http_client
		.post(
			"https://api.minecraftservices.com/authentication/login_with_xbox",
		)
		.header("Content-Type", "application/json")
		.json(&serde_json::json!({
			"identityToken": format!("XBL3.0 x={};{}", xsts_uhs, xsts_token),
		}))
		.send()
		.await?
		.json()
		.await?;

	Ok(resp.access_token)
}

/// Fetches the Minecraft player profile using an access token.
///
/// A non-success status is treated as the user not owning the game,
/// since the Minecraft API returns 404/403 for accounts without a licence.
pub async fn get_profile(
	http_client: &reqwest::Client,
	mc_access_token: &str,
) -> Result<McProfile> {
	let resp = http_client
		.get("https://api.minecraftservices.com/minecraft/profile")
		.header("Authorization", format!("Bearer {}", mc_access_token))
		.send()
		.await?;

	if !resp.status().is_success() {
		return Err(crate::errors::YammmError::network_error(
			"Minecraft profile not found. Do you own the game?",
		)
		.into());
	}

	let profile: McProfile = resp.json().await?;
	Ok(profile)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_mc_login_response_deserialization() {
		let json = r#"{ "access_token": "MC_AT_123" }"#;
		let resp: McLoginResponse = serde_json::from_str(json).unwrap();
		assert_eq!(resp.access_token, "MC_AT_123");
	}

	#[test]
	fn test_mc_profile_deserialization() {
		let json = r##"{
			"id": "abc-123-def",
			"name": "Player1"
		}"##;
		let profile: McProfile = serde_json::from_str(json).unwrap();
		assert_eq!(profile.id, "abc-123-def");
		assert_eq!(profile.name, "Player1");
	}
}
