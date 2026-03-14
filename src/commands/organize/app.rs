use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{OrphanConfig, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	ModSelect,
	DestSelect,
}

#[derive(Debug, Clone)]
pub struct DestOption {
	pub label: String,
	pub idx: usize,
}

pub struct OrganizeApp {
	pub orphan_configs: Vec<OrphanConfig>,
	pub mod_names: Vec<String>,
	pub side: Side,

	pub current_idx: usize,
	pub mode: Mode,
	pub query: String,
	pub mod_list_state: ModListState,
	pub dest_list_state: ListState,
	pub result: OrganizeResultInternal,

	#[cfg(feature = "syntax-highlight")]
	pub syntax_set: syntect::parsing::SyntaxSet,
	#[cfg(feature = "syntax-highlight")]
	pub theme_set: syntect::highlighting::ThemeSet,
}

#[derive(Debug, Clone)]
pub struct ModListState {
	pub filtered_indices: Vec<usize>,
	pub selected: usize,
	pub scroll_offset: usize,
}

#[derive(Debug, Clone)]
pub struct ListState {
	pub selected: usize,
}

#[derive(Debug, Clone, Default)]
pub struct OrganizeResultInternal {
	pub assigned: std::collections::HashMap<String, usize>,
	pub ignored_count: usize,
	pub skipped_count: usize,
	pub ignored_new: Option<Vec<String>>,
}

impl OrganizeApp {
	pub fn new(
		orphan_configs: Vec<OrphanConfig>,
		mod_names: Vec<String>,
		side: Side,
	) -> Self {
		let indices: Vec<usize> = (0..mod_names.len()).collect();
		Self {
			orphan_configs,
			mod_names,
			side,
			current_idx: 0,
			mode: Mode::ModSelect,
			query: String::new(),
			mod_list_state: ModListState {
				filtered_indices: indices,
				selected: 0,
				scroll_offset: 0,
			},
			dest_list_state: ListState { selected: 0 },
			result: OrganizeResultInternal::default(),
			#[cfg(feature = "syntax-highlight")]
			syntax_set: syntect::parsing::SyntaxSet::load_defaults_newlines(),
			#[cfg(feature = "syntax-highlight")]
			theme_set: syntect::highlighting::ThemeSet::load_defaults(),
		}
	}

	pub fn current_config(&self) -> Option<&OrphanConfig> {
		self.orphan_configs.get(self.current_idx)
	}

	pub fn total_configs(&self) -> usize {
		self.orphan_configs.len()
	}

	pub fn update_filter(&mut self) {
		if self.query.is_empty() {
			self.mod_list_state.filtered_indices =
				(0..self.mod_names.len()).collect();
			self.mod_list_state.selected = 0;
			self.mod_list_state.scroll_offset = 0;
			return;
		}

		let mut matcher = Matcher::new(Config::DEFAULT);
		let pattern = Pattern::parse(
			&self.query,
			CaseMatching::Ignore,
			Normalization::Smart,
		);

		let mut scored: Vec<(usize, u32)> = self
			.mod_names
			.iter()
			.enumerate()
			.filter_map(|(i, name)| {
				let mut buf = Vec::new();
				let haystack = Utf32Str::new(name, &mut buf);
				pattern.score(haystack, &mut matcher).map(|s| (i, s))
			})
			.collect();

		scored.sort_by_key(|b| std::cmp::Reverse(b.1));

		self.mod_list_state.filtered_indices =
			scored.into_iter().map(|(i, _)| i).collect();
		self.mod_list_state.selected = 0;
		self.mod_list_state.scroll_offset = 0;
	}

	pub fn dest_options(&self) -> Vec<DestOption> {
		let selected_mod = self.selected_mod_name();
		match self.side {
			Side::Client => vec![
				DestOption {
					label: format!("mods/{}/config/ (common)", selected_mod),
					idx: 0,
				},
				DestOption {
					label: format!(
						"mods/{}/client/config/ (client-only)",
						selected_mod
					),
					idx: 1,
				},
				DestOption {
					label: "config/ (fallback)".to_string(),
					idx: 2,
				},
				DestOption {
					label: "Ignore (add to ignore list)".to_string(),
					idx: 3,
				},
			],
			Side::Server => vec![
				DestOption {
					label: format!("mods/{}/config/ (common)", selected_mod),
					idx: 0,
				},
				DestOption {
					label: format!(
						"mods/{}/server/config/ (server-only)",
						selected_mod
					),
					idx: 1,
				},
				DestOption {
					label: "config/ (fallback)".to_string(),
					idx: 2,
				},
				DestOption {
					label: "Ignore (add to ignore list)".to_string(),
					idx: 3,
				},
			],
		}
	}

	pub fn selected_mod_name(&self) -> String {
		self.mod_list_state
			.filtered_indices
			.get(self.mod_list_state.selected)
			.map(|&i| self.mod_names[i].clone())
			.unwrap_or_default()
	}

	pub fn advance(&mut self) -> bool {
		self.current_idx += 1;
		self.mode = Mode::ModSelect;
		self.query.clear();
		self.update_filter();
		self.current_idx < self.orphan_configs.len()
	}

	pub fn select_mod(&mut self) {
		if self.mod_list_state.filtered_indices.is_empty() {
			return;
		}
		self.mode = Mode::DestSelect;
		self.dest_list_state.selected = 0;
	}

	pub fn cancel_dest(&mut self) {
		self.mode = Mode::ModSelect;
	}

	pub fn skip_config(&mut self) -> bool {
		self.result.skipped_count += 1;
		self.advance()
	}
}
