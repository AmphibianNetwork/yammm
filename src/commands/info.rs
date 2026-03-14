use crate::app::AppContext;
use crate::config::ModpackManifest;
use crate::output;
use anyhow::Result;
use clap::Parser;
use comfy_table::Cell;

/// Display modpack information, list installed items, or show a dependency tree.
#[derive(Parser, Debug)]
pub struct InfoCommand {
	#[command(subcommand)]
	pub command: Option<InfoSubcommand>,
}

#[derive(Parser, Debug)]
pub enum InfoSubcommand {
	List {
		#[arg(short = 'v', long)]
		verbose: bool,
	},

	Mod {
		mod_id: String,
	},

	Tree,
}

impl InfoCommand {
	pub async fn run(
		self,
		ctx: AppContext,
	) -> Result<()> {
		let app = ctx.require_modpack()?;
		let modpack = &app.config;
		let storage = &app.storage;

		match self.command {
			None => self.display_overview(modpack, storage),
			Some(InfoSubcommand::List { verbose }) => {
				self.display_list(modpack, storage, verbose)
			}
			Some(InfoSubcommand::Mod { ref mod_id }) => {
				self.display_mod(storage, mod_id)?
			}
			Some(InfoSubcommand::Tree) => self.display_tree(storage)?,
		}

		Ok(())
	}

	/// Show an overview of the modpack (name, MC version, loader, item counts).
	fn display_overview(
		&self,
		modpack: &ModpackManifest,
		storage: &crate::storage::Storage,
	) {
		let mods = storage
			.list(crate::types::ProjectType::Mod)
			.unwrap_or_default();
		let resourcepacks = storage
			.list(crate::types::ProjectType::ResourcePack)
			.unwrap_or_default();
		let shaderpacks = storage
			.list(crate::types::ProjectType::Shader)
			.unwrap_or_default();

		output::heading(&modpack.name);
		output::blank_line();

		let mut table = output::new_table();
		table.set_header(vec![Cell::new("Property"), Cell::new("Value")]);

		table.add_row(vec![
			Cell::new("Minecraft"),
			Cell::new(&modpack.minecraft_version),
		]);
		table.add_row(vec![
			Cell::new("Loader"),
			Cell::new(format!(
				"{} {}",
				modpack
					.loader
					.loader
					.map(|l| l.to_string())
					.unwrap_or_default(),
				modpack.loader.version
			)),
		]);
		table.add_row(vec![
			Cell::new("Mods"),
			Cell::new(mods.len().to_string()),
		]);
		table.add_row(vec![
			Cell::new("Resource Packs"),
			Cell::new(resourcepacks.len().to_string()),
		]);
		table.add_row(vec![
			Cell::new("Shader Packs"),
			Cell::new(shaderpacks.len().to_string()),
		]);

		println!("{table}");

		if !mods.is_empty() {
			output::blank_line();
			output::heading(format!("Mods ({})", mods.len()));
			let mut mod_table = output::new_table();
			mod_table.set_header(vec![
				Cell::new("Name"),
				Cell::new("Version"),
				Cell::new("Env"),
				Cell::new("Source"),
				Cell::new("Categories"),
			]);

			for m in mods {
				let cats = if m.categories.is_empty() {
					String::new()
				} else {
					m.categories.join(", ")
				};
				mod_table.add_row(vec![
					Cell::new(&m.name)
						.add_attribute(comfy_table::Attribute::Bold),
					Cell::new(&m.version),
					Cell::new(m.env.as_str()),
					Cell::new(output::source_label(&m.source))
						.fg(output::source_color(&m.source)),
					Cell::new(&cats),
				]);
			}
			println!("{mod_table}");
		}
	}

	/// List all installed mods, resource packs, and shader packs.
	/// Pass `verbose` to include source tags.
	fn display_list(
		&self,
		modpack: &ModpackManifest,
		storage: &crate::storage::Storage,
		verbose: bool,
	) {
		let mods = storage
			.list(crate::types::ProjectType::Mod)
			.unwrap_or_default();
		let resourcepacks = storage
			.list(crate::types::ProjectType::ResourcePack)
			.unwrap_or_default();
		let shaderpacks = storage
			.list(crate::types::ProjectType::Shader)
			.unwrap_or_default();

		output::heading(format!(
			"{} - MC {} with {} {}",
			modpack.name,
			modpack.minecraft_version,
			modpack
				.loader
				.loader
				.map(|l| l.to_string())
				.unwrap_or_default(),
			modpack.loader.version
		));
		output::blank_line();

		display_items("Mods", &mods, verbose);
		output::blank_line();
		display_items("Resource Packs", &resourcepacks, verbose);
		output::blank_line();
		display_items("Shader Packs", &shaderpacks, verbose);
	}

	/// Show detailed info for a single mod by slug or ID.
	fn display_mod(
		&self,
		storage: &crate::storage::Storage,
		mod_id: &str,
	) -> Result<()> {
		let (_, mod_ron) = storage.find_any(mod_id).map_err(|_| {
			crate::errors::YammmError::mod_not_found(format!(
				"Mod '{}' not found in mods, resourcepacks, or shaderpacks",
				mod_id
			))
		})?;

		let mut table = output::new_table();
		table.set_header(vec![Cell::new("Property"), Cell::new("Value")]);

		table.add_row(vec![Cell::new("ID"), Cell::new(&mod_ron.id)]);
		table.add_row(vec![Cell::new("Name"), Cell::new(&mod_ron.name)]);
		table.add_row(vec![Cell::new("Version"), Cell::new(&mod_ron.version)]);
		table.add_row(vec![Cell::new("Env"), Cell::new(mod_ron.env.as_str())]);
		table.add_row(vec![
			Cell::new("Source"),
			Cell::new(output::source_label(&mod_ron.source)),
		]);
		if !mod_ron.description.is_empty() {
			table.add_row(vec![
				Cell::new("Description"),
				Cell::new(&mod_ron.description),
			]);
		}
		if !mod_ron.categories.is_empty() {
			table.add_row(vec![
				Cell::new("Categories"),
				Cell::new(mod_ron.categories.join(", ")),
			]);
		}
		if !mod_ron.download_url.is_empty() {
			table.add_row(vec![
				Cell::new("Download URL"),
				Cell::new(&mod_ron.download_url),
			]);
		}
		if mod_ron.hash.is_some() {
			let hash_display = mod_ron
				.hash
				.as_deref()
				.map(|h| {
					let chars: String = h.chars().take(32).collect();
					if h.chars().count() > 32 {
						format!("{}…", chars)
					} else {
						chars
					}
				})
				.unwrap_or_default();
			table.add_row(vec![Cell::new("Hash"), Cell::new(&hash_display)]);
		}

		println!("{table}");
		Ok(())
	}

	/// Display a tree view of all mods and their declared dependencies.
	fn display_tree(
		&self,
		storage: &crate::storage::Storage,
	) -> Result<()> {
		let mods = storage
			.list(crate::types::ProjectType::Mod)
			.unwrap_or_default();

		output::heading("Dependency Tree");
		output::blank_line();

		if mods.is_empty() {
			output::dim("  (no mods installed)");
			return Ok(());
		}

		for (i, m) in mods.iter().enumerate() {
			let is_last = i == mods.len() - 1;
			let prefix = if is_last { "└── " } else { "├── " };
			let tag_str = output::source_label(&m.source);
			let tag = output::source_tag(tag_str);
			let name_str = console::Style::new().bold().apply_to(&m.name);
			let ver_str = console::Style::new()
				.dim()
				.apply_to(format!("v{}", m.version));
			let env_str = console::Style::new().cyan().apply_to(m.env.as_str());
			if m.categories.is_empty() {
				println!(
					"{}{} {} {} [{}]",
					prefix, name_str, ver_str, env_str, tag
				);
			} else {
				let cats = console::Style::new()
					.magenta()
					.apply_to(format!("[{}]", m.categories.join(", ")));
				println!(
					"{}{} {} {} [{}] {}",
					prefix, name_str, ver_str, env_str, tag, cats
				);
			}

			let dep_count = m.dependencies.len();
			for (j, dep) in m.dependencies.iter().enumerate() {
				let dep_is_last = j == dep_count - 1;
				let dep_prefix = if is_last { "    " } else { "│   " };
				let dep_branch = if dep_is_last {
					"└── "
				} else {
					"├── "
				};
				let dep_id_str =
					console::Style::new().dim().apply_to(&dep.mod_id);
				println!(
					"{}{}{} ({})",
					dep_prefix,
					dep_branch,
					dep_id_str,
					crate::output::dependency_kind_styled(&dep.kind)
				);
			}
		}

		Ok(())
	}
}

fn display_items(
	heading: &str,
	items: &[crate::types::TrackedMod],
	verbose: bool,
) {
	output::heading(format!("{} ({})", heading, items.len()));
	for m in items {
		if verbose {
			let ver = console::Style::new()
				.dim()
				.apply_to(format!("v{}", m.version));
			let env = console::Style::new().cyan().apply_to(m.env.as_str());
			let src = console::Style::new()
				.blue()
				.apply_to(format!("[{}]", output::source_label(&m.source)));
			let name_str = console::Style::new().bold().apply_to(&m.name);
			if m.categories.is_empty() {
				println!("  ✓ {} {} {} {}", name_str, ver, env, src);
			} else {
				let cats = console::Style::new()
					.magenta()
					.apply_to(format!("[{}]", m.categories.join(", ")));
				println!("  ✓ {} {} {} {} {}", name_str, ver, env, src, cats);
			}
		} else {
			output::bullet(format!("{} v{}", m.name, m.version));
		}
	}
}
