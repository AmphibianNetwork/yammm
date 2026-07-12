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

	/// Skip the first N results — pair with --limit to page through.
	#[arg(long, default_value = "0")]
	pub offset: usize,
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

		let mc_version = ctx.modpack().and_then(|app| {
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

		if let Some(provider) = ctx.registry().get_by_key(&source_key) {
			let version_filters = VersionFilters {
				minecraft_version: mc_version.clone(),
				loader: loader_filter,
			};
			let filters = SearchFilters::new(version_filters, Some(self.limit))
				.with_offset(if self.offset > 0 {
					Some(self.offset)
				} else {
					None
				});

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

		if output::is_json_mode() {
			return self.emit_json(&results);
		}

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

	fn emit_json(
		&self,
		results: &[ModInfo],
	) -> Result<()> {
		let hits: Vec<_> = results
			.iter()
			.map(|r| {
				serde_json::json!({
					"source": r.source.as_str(),
					"id": r.source.source_id(),
					"name": r.name,
					"description": r.description,
					"url": r.url,
					"minecraft_versions": r.minecraft_versions,
					"loaders": r.loaders,
					"downloads": r.downloads,
				})
			})
			.collect();
		output::emit_json(&serde_json::json!({
			"query": self.query,
			"count": hits.len(),
			"hits": hits,
		}))
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

		output::raw_block(&table);
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
			output::raw_line(format!(
				"[{}] {} ({}) - {}",
				source_str, name_str, slug, desc
			));
		}
	}
}
