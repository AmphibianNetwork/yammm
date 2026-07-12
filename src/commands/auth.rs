use crate::app::AppContext;
use crate::output;
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct AuthCommand {
	#[command(subcommand)]
	pub command: AuthSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum AuthSubcommand {
	/// Sign in with your Microsoft account
	Login,

	/// Sign out and remove stored credentials
	Logout,

	/// Show current authentication status
	Status,
}

impl AuthCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		match self.command {
			AuthSubcommand::Login => {
				// Login is a long device-code OAuth flow. In JSON mode
				// the prompt (URL + code) is routed to stderr by the
				// device-code subroutine so scripts can read it; the
				// final auth result still lands as a JSON document on
				// stdout once the user completes the browser step.
				if !output::is_json_mode() {
					output::heading("Microsoft Account Login");
					output::blank_line();
				}

				let token = crate::auth::login(ctx.http_client()).await?;

				if output::is_json_mode() {
					output::emit_json(&serde_json::json!({
						"command": "auth login",
						"status": "logged_in",
						"username": token.username,
						"uuid": token.uuid,
						"expires_at": token.expires_at,
					}))?;
					return Ok(());
				}

				output::blank_line();
				output::success(format!(
					"Logged in as {} (UUID: {})",
					token.username, token.uuid
				));
			}
			AuthSubcommand::Logout => {
				crate::auth::delete_token()?;
				if output::is_json_mode() {
					output::emit_json(&serde_json::json!({
						"command": "auth logout",
						"status": "logged_out",
					}))?;
					return Ok(());
				}
				output::success("Logged out. Stored credentials removed.");
			}
			AuthSubcommand::Status => match crate::auth::load_token()? {
				Some(token) => {
					if output::is_json_mode() {
						output::emit_json(&serde_json::json!({
							"command": "auth status",
							"logged_in": true,
							"username": token.username,
							"uuid": token.uuid,
							"expires_at": token.expires_at,
							"expired": token.is_expired(),
						}))?;
						return Ok(());
					}
					output::heading("Authentication Status");
					output::blank_line();
					output::bullet(format!("Username: {}", token.username));
					output::bullet(format!("UUID: {}", token.uuid));
					if token.is_expired() {
						output::warning(
							"Token expired. Will refresh on next launch."
								.to_string(),
						);
					} else {
						output::bullet(format!(
							"Token expires: {}",
							crate::utils::system_time_to_date(
								std::time::SystemTime::UNIX_EPOCH
									+ std::time::Duration::from_secs(
										token.expires_at
									)
							)
						));
					}
				}
				None => {
					if output::is_json_mode() {
						output::emit_json(&serde_json::json!({
							"command": "auth status",
							"logged_in": false,
						}))?;
						return Ok(());
					}
					output::info(
						"Not logged in. Run `yammm auth login` to sign in.",
					);
				}
			},
		}

		Ok(())
	}
}
