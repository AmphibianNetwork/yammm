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
				output::heading("Microsoft Account Login");
				output::blank_line();

				let token = crate::auth::login(&ctx.http_client).await?;

				output::blank_line();
				output::success(format!(
					"Logged in as {} (UUID: {})",
					token.username, token.uuid
				));
			}
			AuthSubcommand::Logout => {
				crate::auth::delete_token()?;
				output::success("Logged out. Stored credentials removed.");
			}
			AuthSubcommand::Status => match crate::auth::load_token()? {
				Some(token) => {
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
					output::info(
						"Not logged in. Run `yammm auth login` to sign in.",
					);
				}
			},
		}

		Ok(())
	}
}
