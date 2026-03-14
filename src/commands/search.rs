use anyhow::Result;
use clap::{Parser, ValueEnum};
use comfy_table::Cell;

use super::CliSource;
use crate::app::AppContext;
use crate::output;
use crate::providers::{SearchFilters, SourceKey};
use crate::types::VersionFilters;
use crate::types::{LoaderType, ModInfo};
use crate::utils::truncate_str;

/// Search for mods on Modrinth or CurseForge.
#[derive(Parser, Debug)]
pub struct SearchCommand {
	pub query: String,

	#[arg(short = 'l', long)]
	pub loader: Option<String>,

	#[arg(short = 's', long, default_value = "modrinth")]
	pub source: CliSource,

	#[arg(short = 'o', long, default_value = "table")]
	pub output: SearchFormat,

	#[arg(short = 'n', long, default_value = "20")]
	pub limit: usize,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq)]
pub enum SearchFormat {
	Table,
	Compact,
}

impl SearchCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let loader_filter = if let Some(ref loader_str) = self.loader {
			Some(loader_str.parse::<LoaderType>().map_err(|e| {
				crate::errors::YammmError::invalid_args(e.to_string())
			})?)
		} else {
			None
		};

		let mc_version = ctx.modpack.as_ref().and_then(|app| {
			if app.config.minecraft_version.is_empty() {
				None
			} else {
				Some(app.config.minecraft_version.clone())
			}
		});

		let mut results: Vec<ModInfo> = Vec::new();

		let source_key = match self.source {
			CliSource::Modrinth => SourceKey::Modrinth,
			CliSource::CurseForge => SourceKey::CurseForge,
		};

		if let Some(provider) = ctx.registry.get_by_key(&source_key) {
			let version_filters = VersionFilters {
				minecraft_version: mc_version.clone(),
				loader: loader_filter,
			};
			let filters = SearchFilters::new(version_filters, Some(self.limit));

			match provider.search(&self.query, &filters).await {
				Ok(provider_results) => results.extend(provider_results),
				Err(e) => {
					output::warning(format!(
						"Search on {:?} failed: {}",
						source_key, e
					));
				}
			}
		} else {
			output::error(format!(
				"Provider for {:?} is not registered",
				source_key
			));
		}

		results.truncate(self.limit);

		if results.is_empty() {
			output::warning(format!("No mods found matching '{}'", self.query));
			return Ok(());
		}

		match self.output {
			SearchFormat::Table => self.display_table(&results),
			SearchFormat::Compact => self.display_compact(&results),
		}

		Ok(())
	}

	fn display_table(
		&self,
		results: &[ModInfo],
	) {
		let mut table = output::new_table();

		table.set_header(vec![
			Cell::new("Source"),
			Cell::new("Name"),
			Cell::new("Slug"),
			Cell::new("Description"),
		]);

		for result in results {
			let slug = result.source.source_id();

			let description = result
				.description
				.lines()
				.next()
				.unwrap_or("No description")
				.to_string();

			let truncated_desc = truncate_str(&description, 57, "...");

			table.add_row(vec![
				Cell::new(output::source_label(&result.source))
					.fg(output::source_color(&result.source)),
				Cell::new(&result.name)
					.add_attribute(comfy_table::Attribute::Bold),
				Cell::new(slug),
				Cell::new(truncated_desc),
			]);
		}

		println!("{table}");
	}

	fn display_compact(
		&self,
		results: &[ModInfo],
	) {
		for result in results {
			let slug = result.source.source_id();

			let desc = result
				.description
				.lines()
				.next()
				.unwrap_or("No description");

			let source_str =
				output::source_tag(output::source_label(&result.source));
			let name_str = console::Style::new().bold().apply_to(&result.name);
			println!("[{}] {} ({}) - {}", source_str, name_str, slug, desc);
		}
	}
}
