use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tui_textarea::TextArea;

use crate::types::{ModVersion, TrackedMod};

mod actions;
mod input;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	Normal,
	EditEnv,
	EditVersion,
	EditDescription,
	EditCategories,
	DepTree,
	InstallDeps,
	InstallProgress,
	RemoveConfirm,
	UpdateCheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
	ModList,
	Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailField {
	Version,
	Env,
	Description,
	Categories,
	DepTree,
	Remove,
	UpdateCheck,
}

impl DetailField {
	pub const FIELDS: &[DetailField] = &[
		DetailField::Version,
		DetailField::Env,
		DetailField::Description,
		DetailField::Categories,
		DetailField::DepTree,
		DetailField::Remove,
		DetailField::UpdateCheck,
	];

	pub fn label(self) -> &'static str {
		match self {
			DetailField::Version => "Version",
			DetailField::Env => "Env",
			DetailField::Description => "Description",
			DetailField::Categories => "Categories",
			DetailField::DepTree => "Dependencies",
			DetailField::Remove => "Remove",
			DetailField::UpdateCheck => "Check update",
		}
	}

	pub fn shortcut(self) -> char {
		match self {
			DetailField::Version => 'v',
			DetailField::Env => 'e',
			DetailField::Description => 'd',
			DetailField::Categories => 'c',
			DetailField::DepTree => 't',
			DetailField::Remove => 'r',
			DetailField::UpdateCheck => 'u',
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryFilter {
	All,
	Category(usize),
}

#[derive(Debug, Clone)]
pub struct ModListState {
	pub filtered_indices: Vec<usize>,
	pub selected: usize,
	pub scroll_offset: usize,
}

#[derive(Debug, Clone)]
pub struct DepNode {
	pub mod_id: String,
	pub name: String,
	pub version: String,
	pub kind: Option<crate::types::DependencyKind>,
	pub installed: bool,
	pub source: crate::types::ModSource,
	pub children: Vec<DepNode>,
}

#[derive(Debug, Clone)]
pub struct DepEntry {
	pub mod_id: String,
	pub name: String,
	pub version: String,
	pub kind: Option<crate::types::DependencyKind>,
	pub installed: bool,
	pub source: crate::types::ModSource,
	pub indent: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncOp {
	FetchVersions,
	CheckUpdate,
	InstallDeps,
}

#[derive(Debug, Clone)]
pub enum AsyncResult {
	Versions(Result<Vec<ModVersion>, String>),
	UpdateCheck(Result<Option<crate::commands::update::ModUpdate>, String>),
	InstallDeps(Result<Vec<String>, String>),
}

pub struct ManageApp {
	pub mods: Vec<TrackedMod>,
	pub all_categories: Vec<String>,
	pub mode: Mode,
	pub focus: Focus,
	pub category_filter: CategoryFilter,
	pub query: String,
	pub mod_list: ModListState,
	pub modpack_name: String,
	pub modpack_version: String,
	pub modpack_loader: String,

	pub detail_selected: usize,

	pub env_picker_selected: usize,

	pub version_list: Vec<ModVersion>,
	pub version_list_selected: usize,
	pub version_list_scroll: usize,
	pub version_loading: bool,

	pub description_textarea: Option<TextArea<'static>>,

	pub category_picker_selected: usize,
	pub category_picker_scroll: usize,
	pub category_new_input: String,
	pub category_new_mode: bool,

	pub dep_tree: Vec<DepNode>,
	pub dep_reverse: Vec<(String, String)>,
	pub dep_tree_scroll: usize,

	pub dep_entries: Vec<DepEntry>,
	pub dep_selected: usize,
	pub dep_scroll: usize,
	pub dep_marked: std::collections::HashSet<String>,
	pub dep_installing: bool,
	pub dep_output: Vec<String>,
	pub dep_output_scroll: usize,

	pub remove_dependents: Vec<(String, String)>,

	pub update_result: Option<crate::commands::update::ModUpdate>,
	pub update_loading: bool,
	pub update_error: Option<String>,

	pub pending_async: Option<AsyncOp>,

	pub show_help: bool,
	pub should_quit: bool,
	pub status_message: Option<(StatusKind, String)>,
	pub status_ticks: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
	Success,
	Warning,
	Error,
}

impl ManageApp {
	pub fn new(
		mods: Vec<TrackedMod>,
		modpack_name: String,
		modpack_version: String,
		modpack_loader: String,
	) -> Self {
		let mut all_categories: Vec<String> = mods
			.iter()
			.flat_map(|m| m.categories.iter().cloned())
			.collect();
		all_categories.sort();
		all_categories.dedup();

		let indices: Vec<usize> = (0..mods.len()).collect();
		Self {
			mods,
			all_categories,
			mode: Mode::Normal,
			focus: Focus::ModList,
			category_filter: CategoryFilter::All,
			query: String::new(),
			mod_list: ModListState {
				filtered_indices: indices,
				selected: 0,
				scroll_offset: 0,
			},
			modpack_name,
			modpack_version,
			modpack_loader,
			detail_selected: 0,
			env_picker_selected: 0,
			version_list: Vec::new(),
			version_list_selected: 0,
			version_list_scroll: 0,
			version_loading: false,
			description_textarea: None,
			category_picker_selected: 0,
			category_picker_scroll: 0,
			category_new_input: String::new(),
			category_new_mode: false,
			dep_tree: Vec::new(),
			dep_reverse: Vec::new(),
			dep_tree_scroll: 0,
			dep_entries: Vec::new(),
			dep_selected: 0,
			dep_scroll: 0,
			dep_marked: std::collections::HashSet::new(),
			dep_installing: false,
			dep_output: Vec::new(),
			dep_output_scroll: 0,
			remove_dependents: Vec::new(),
			update_result: None,
			update_loading: false,
			update_error: None,
			pending_async: None,
			show_help: false,
			should_quit: false,
			status_message: None,
			status_ticks: 0,
		}
	}

	pub fn selected_mod(&self) -> Option<&TrackedMod> {
		self.mod_list
			.filtered_indices
			.get(self.mod_list.selected)
			.map(|&i| &self.mods[i])
	}

	pub fn selected_mod_mut(&mut self) -> Option<&mut TrackedMod> {
		let idx = self
			.mod_list
			.filtered_indices
			.get(self.mod_list.selected)
			.copied();
		if let Some(i) = idx {
			Some(&mut self.mods[i])
		} else {
			None
		}
	}

	pub fn selected_field(&self) -> DetailField {
		DetailField::FIELDS
			.get(self.detail_selected)
			.copied()
			.unwrap_or(DetailField::Version)
	}

	pub fn update_filter(&mut self) {
		let mut matcher = Matcher::new(Config::DEFAULT);
		let pattern = Pattern::parse(
			&self.query,
			CaseMatching::Ignore,
			Normalization::Smart,
		);

		let mut scored: Vec<(usize, u32)> = if self.query.is_empty() {
			self.mods.iter().enumerate().map(|(i, _)| (i, 0)).collect()
		} else {
			self.mods
				.iter()
				.enumerate()
				.filter_map(|(i, m)| {
					let mut buf = Vec::new();
					let haystack = Utf32Str::new(&m.name, &mut buf);
					pattern.score(haystack, &mut matcher).map(|s| (i, s))
				})
				.collect()
		};

		if self.query.is_empty() {
			scored.sort_by(|a, b| {
				self.mods[a.0]
					.name
					.to_lowercase()
					.cmp(&self.mods[b.0].name.to_lowercase())
			});
		} else {
			scored.sort_by_key(|b| std::cmp::Reverse(b.1));
		}

		self.mod_list.filtered_indices =
			scored.into_iter().map(|(i, _)| i).collect();

		self.apply_category_filter();

		if self.mod_list.selected >= self.mod_list.filtered_indices.len() {
			self.mod_list.selected =
				self.mod_list.filtered_indices.len().saturating_sub(1);
		}
		self.mod_list.scroll_offset = 0;
	}

	fn apply_category_filter(&mut self) {
		match self.category_filter {
			CategoryFilter::All => {}
			CategoryFilter::Category(cat_idx) => {
				let cat = match self.all_categories.get(cat_idx) {
					Some(c) => c.clone(),
					None => return,
				};
				self.mod_list
					.filtered_indices
					.retain(|&i| self.mods[i].categories.contains(&cat));
			}
		}
	}

	pub fn cycle_category_filter(&mut self) {
		self.category_filter = match self.category_filter {
			CategoryFilter::All => {
				if self.all_categories.is_empty() {
					CategoryFilter::All
				} else {
					CategoryFilter::Category(0)
				}
			}
			CategoryFilter::Category(idx) => {
				if idx + 1 >= self.all_categories.len() {
					CategoryFilter::All
				} else {
					CategoryFilter::Category(idx + 1)
				}
			}
		};
		self.update_filter();
	}

	pub fn category_filter_label(&self) -> String {
		match &self.category_filter {
			CategoryFilter::All => "all".to_string(),
			CategoryFilter::Category(idx) => self
				.all_categories
				.get(*idx)
				.cloned()
				.unwrap_or_else(|| "all".to_string()),
		}
	}

	pub fn set_status(
		&mut self,
		kind: StatusKind,
		msg: String,
	) {
		self.status_message = Some((kind, msg));
		self.status_ticks = 60;
	}

	pub fn tick_status(&mut self) {
		if self.status_ticks > 0 {
			self.status_ticks -= 1;
			if self.status_ticks == 0 {
				self.status_message = None;
			}
		}
	}
}
