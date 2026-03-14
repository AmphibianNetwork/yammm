//! Microsoft / Mojang authentication for online-mode Minecraft launch.
//!
//! Chain: MS OAuth (device code) → Xbox Live → XSTS → MC login → MC profile.
//! Tokens persisted in `~/.config/yammm/auth.json` (0o600 on Unix).
//! `get_valid_token()` handles: load cached → check expiry → refresh → full re-login.

pub mod microsoft;
pub mod minecraft;
pub mod xbox;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Microsoft OAuth client ID (public, non-sensitive).
pub const MS_CLIENT_ID: &str = "31c26fc2-ce20-4fa9-95ca-21ecb8fd231b";

fn epoch_secs() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

/// Persisted Minecraft auth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthToken {
	pub access_token: String,
	pub refresh_token: String,
	pub username: String,
	pub uuid: String,
	pub expires_at: u64,
}

impl AuthToken {
	pub fn is_expired(&self) -> bool {
		epoch_secs() >= self.expires_at
	}
}

fn auth_path() -> Option<std::path::PathBuf> {
	dirs::config_dir().map(|dir| dir.join("yammm").join("auth.json"))
}

pub fn load_token() -> Result<Option<AuthToken>> {
	let path = match auth_path() {
		Some(p) => p,
		None => return Ok(None),
	};

	if !path.exists() {
		return Ok(None);
	}

	let contents =
		std::fs::read_to_string(&path).context("Failed to read auth.json")?;
	let token: AuthToken =
		serde_json::from_str(&contents).context("Failed to parse auth.json")?;
	Ok(Some(token))
}

pub fn save_token(token: &AuthToken) -> Result<()> {
	let path = auth_path().context("Config directory not found")?;

	let contents = serde_json::to_string_pretty(token)
		.context("Failed to serialize auth token")?;

	crate::utils::write_secret_file(&path, &contents)
}

pub fn delete_token() -> Result<()> {
	let path = match auth_path() {
		Some(p) if p.exists() => p,
		_ => return Ok(()),
	};

	std::fs::remove_file(&path).context("Failed to remove auth.json")?;
	Ok(())
}

/// Intermediate result after exchanging a Microsoft access token through
/// the Xbox Live + Minecraft auth chain.
struct ExchangedTokens {
	mc_access_token: String,
	username: String,
	uuid: String,
	expires_at: u64,
}

/// Exchange a Microsoft access token through Xbox → Minecraft chain.
async fn exchange_tokens(
	ms_access_token: &str,
	http_client: &reqwest::Client,
	ms_expires_in: u64,
) -> Result<ExchangedTokens> {
	let (xbl_token, xbl_uhs) =
		xbox::authenticate_xbl(http_client, ms_access_token).await?;
	let (xsts_token, xsts_uhs) =
		xbox::authenticate_xsts(http_client, &xbl_token, &xbl_uhs).await?;
	let mc_access_token =
		minecraft::login_with_xbox(http_client, &xsts_token, &xsts_uhs).await?;
	let profile = minecraft::get_profile(http_client, &mc_access_token).await?;
	Ok(ExchangedTokens {
		mc_access_token,
		username: profile.name,
		uuid: profile.id,
		expires_at: epoch_secs() + ms_expires_in,
	})
}

/// Full login flow: Microsoft device code → Xbox → Minecraft.
pub async fn login(http_client: &reqwest::Client) -> Result<AuthToken> {
	let (ms_access_token, ms_refresh_token, ms_expires_in) =
		microsoft::device_code_flow(http_client).await?;

	let exchanged =
		exchange_tokens(&ms_access_token, http_client, ms_expires_in).await?;

	let token = AuthToken {
		access_token: exchanged.mc_access_token,
		refresh_token: ms_refresh_token,
		username: exchanged.username,
		uuid: exchanged.uuid,
		expires_at: exchanged.expires_at,
	};

	save_token(&token)?;
	Ok(token)
}

/// Get a valid (non-expired) Minecraft auth token.
/// Tries: cached → refresh → full re-login.
pub async fn get_valid_token(
	http_client: &reqwest::Client
) -> Result<AuthToken> {
	let token = load_token()?;
	let token: AuthToken = match token {
		Some(t) => t,
		None => return login(http_client).await,
	};

	if !token.is_expired() {
		return Ok(token);
	}

	let refresh_token = token.refresh_token.clone();
	match microsoft::refresh_access_token(http_client, &refresh_token).await {
		Ok((new_ms_access, new_ms_refresh, ms_expires_in)) => {
			let exchanged =
				exchange_tokens(&new_ms_access, http_client, ms_expires_in)
					.await?;

			let new_token = AuthToken {
				access_token: exchanged.mc_access_token,
				refresh_token: new_ms_refresh,
				username: exchanged.username,
				uuid: exchanged.uuid,
				expires_at: exchanged.expires_at,
			};
			save_token(&new_token)?;
			Ok(new_token)
		}
		// If refresh fails (e.g. revoked consent), fall back to full login
		Err(_) => login(http_client).await,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_auth_token_serialization_roundtrip() {
		let token = AuthToken {
			access_token: "mc-access".to_string(),
			refresh_token: "ms-refresh".to_string(),
			username: "Player1".to_string(),
			uuid: "abc-123".to_string(),
			expires_at: 9999999999,
		};
		let json = serde_json::to_string(&token).unwrap();
		let loaded: AuthToken = serde_json::from_str(&json).unwrap();
		assert_eq!(loaded.access_token, "mc-access");
		assert_eq!(loaded.refresh_token, "ms-refresh");
		assert_eq!(loaded.username, "Player1");
		assert_eq!(loaded.uuid, "abc-123");
		assert_eq!(loaded.expires_at, 9999999999);
	}

	#[test]
	fn test_auth_token_is_expired_future() {
		let token = AuthToken {
			access_token: String::new(),
			refresh_token: String::new(),
			username: String::new(),
			uuid: String::new(),
			expires_at: u64::MAX,
		};
		assert!(!token.is_expired());
	}

	#[test]
	fn test_auth_token_is_expired_past() {
		let token = AuthToken {
			access_token: String::new(),
			refresh_token: String::new(),
			username: String::new(),
			uuid: String::new(),
			expires_at: 0,
		};
		assert!(token.is_expired());
	}
}
