use anyhow::Result;
use clap::Parser;
use self_update::backends::github::Update;
use self_update::cargo_crate_version;

use crate::app::AppContext;
use crate::output;

const REPO_OWNER: &str = "AmphibianNetwork";
const REPO_NAME: &str = "yammm";

#[derive(Parser, Debug)]
pub struct SelfUpdateCommand {
	#[arg(short = 'c', long)]
	pub check: bool,

	#[arg(short = 'y', long)]
	pub yes: bool,
}

fn is_nix_install() -> bool {
	std::env::current_exe()
		.ok()
		.and_then(|p| p.canonicalize().ok())
		.is_some_and(|p| p.to_string_lossy().contains("/nix/store/"))
}

fn configure_update() -> Box<dyn self_update::update::ReleaseUpdate> {
	Update::configure()
		.repo_owner(REPO_OWNER)
		.repo_name(REPO_NAME)
		.current_version(cargo_crate_version!())
		.bin_name("yammm")
		.show_download_progress(true)
		.show_output(false)
		.no_confirm(true)
		.build()
		.expect("Failed to configure self_update")
}

impl SelfUpdateCommand {
	pub async fn run(
		self,
		_ctx: AppContext,
	) -> Result<()> {
		if is_nix_install() {
			output::warning(
				"Self-update is not available when yammm is installed via Nix.",
			);
			output::dim("  Update your flake inputs instead: nix flake update");
			return Ok(());
		}

		let current = cargo_crate_version!();
		output::info(format!("Current version: {}", current));

		if self.check {
			let release = tokio::task::spawn_blocking(|| {
				let updater = configure_update();
				updater.get_latest_release()
			})
			.await??;

			if release.version == current {
				output::success(format!("yammm is up to date (v{})", current));
			} else {
				output::heading(format!(
					"Update available: {} → {}",
					current, release.version
				));
				if let Some(body) = &release.body
					&& !body.is_empty()
				{
					output::blank_line();
					output::dim(body);
				}
			}
			return Ok(());
		}

		let release = tokio::task::spawn_blocking(|| {
			let updater = configure_update();
			updater.get_latest_release()
		})
		.await??;

		if release.version == current {
			output::success(format!("yammm is up to date (v{})", current));
			return Ok(());
		}

		output::heading(format!(
			"Update available: {} → {}",
			current, release.version
		));

		if !self.yes {
			let should_update = output::confirm(
				format!(
					"Update yammm from {} to {}?",
					current, release.version
				),
				true,
			)?;
			if !should_update {
				output::cancelled("Update");
				return Ok(());
			}
		}

		let status = tokio::task::spawn_blocking(|| {
			let updater = configure_update();
			updater.update()
		})
		.await??;

		match status {
			self_update::Status::UpToDate(ver) => {
				output::success(format!(
					"yammm is already up to date (v{})",
					ver
				));
			}
			self_update::Status::Updated(ver) => {
				output::success(format!("yammm updated to v{}", ver));
			}
		}

		Ok(())
	}
}
