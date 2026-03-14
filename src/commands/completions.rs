use anyhow::Result;
use clap::CommandFactory;
use clap::Parser;
use clap_complete::Shell;

use crate::app::AppContext;

#[derive(Parser, Debug)]
pub struct CompletionsCommand {
	pub shell: Shell,
}

impl CompletionsCommand {
	pub async fn run(
		self,
		_ctx: AppContext,
	) -> Result<()> {
		let mut cmd = crate::cli::Cli::command();
		let name = cmd.get_name().to_string();
		clap_complete::generate(
			self.shell,
			&mut cmd,
			name,
			&mut std::io::stdout(),
		);
		Ok(())
	}
}
