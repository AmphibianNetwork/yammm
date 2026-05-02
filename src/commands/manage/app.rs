use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tui_textarea::TextArea;

use crate::storage::Storage;
use crate::types::{ModEnv, ModVersion, TrackedMod};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	Normal,
	EditEnv,
	EditVersion,
	EditDescription,
	EditCategories,
	DepTree,
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
#[allow(dead_code)]
pub struct DepNode {
	pub mod_id: String,
	pub name: String,
	pub version: String,
	pub kind: Option<crate::types::DependencyKind>,
	pub children: Vec<DepNode>,
	pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncOp {
	FetchVersions,
	CheckUpdate,
}

#[derive(Debug, Clone)]
pub enum AsyncResult {
	Versions(Result<Vec<ModVersion>, String>),
	UpdateCheck(Result<Option<crate::commands::update::ModUpdate>, String>),
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

	pub remove_dependents: Vec<(String, String)>,

	pub update_result: Option<crate::commands::update::ModUpdate>,
	pub update_loading: bool,
	pub update_error: Option<String>,

	pub pending_async: Option<AsyncOp>,

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
			remove_dependents: Vec::new(),
			update_result: None,
			update_loading: false,
			update_error: None,
			pending_async: None,
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

	pub fn detail_move_up(&mut self) {
		if self.detail_selected > 0 {
			self.detail_selected -= 1;
		}
	}

	pub fn detail_move_down(&mut self) {
		if self.detail_selected < DetailField::FIELDS.len() - 1 {
			self.detail_selected += 1;
		}
	}

	pub fn activate_field(
		&mut self,
		field: DetailField,
	) {
		if let Some(idx) = DetailField::FIELDS.iter().position(|&f| f == field)
		{
			self.detail_selected = idx;
		}
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

	pub fn mod_list_move_up(&mut self) {
		if self.mod_list.selected > 0 {
			self.mod_list.selected -= 1;
		}
	}

	pub fn mod_list_move_down(&mut self) {
		if self.mod_list.selected
			< self.mod_list.filtered_indices.len().saturating_sub(1)
		{
			self.mod_list.selected += 1;
		}
	}

	pub fn search_push_char(
		&mut self,
		c: char,
	) {
		self.query.push(c);
		self.update_filter();
	}

	pub fn search_pop_char(&mut self) {
		self.query.pop();
		self.update_filter();
	}

	pub fn switch_to_detail(&mut self) {
		if self.selected_mod().is_some() {
			self.focus = Focus::Detail;
		}
	}

	pub fn switch_to_mod_list(&mut self) {
		self.focus = Focus::ModList;
	}

	pub fn start_edit_env(&mut self) {
		if let Some(m) = self.selected_mod() {
			self.env_picker_selected = match m.env {
				ModEnv::Both => 0,
				ModEnv::Client => 1,
				ModEnv::Server => 2,
			};
			self.mode = Mode::EditEnv;
		}
	}

	pub fn confirm_edit_env(
		&mut self,
		storage: &Storage,
	) {
		let new_env = match self.env_picker_selected {
			0 => ModEnv::Both,
			1 => ModEnv::Client,
			2 => ModEnv::Server,
			_ => ModEnv::Both,
		};
		if let Some(m) = self.selected_mod_mut() {
			m.env = new_env;
			let pt = m.project_type;
			let id = m.id.clone();
			if let Ok(mod_ron) = storage.load(pt, &id) {
				let mut updated = mod_ron;
				updated.env = new_env;
				if storage.save(pt, &id, &updated).is_ok() {
					self.set_status(
						StatusKind::Success,
						format!("Env updated to {}", new_env),
					);
				}
			}
		}
		self.mode = Mode::Normal;
	}

	pub fn start_edit_version(&mut self) {
		if let Some(m) = self.selected_mod() {
			if m.unresolved {
				self.set_status(
					StatusKind::Warning,
					"Cannot change version for unresolved mods".to_string(),
				);
			} else if m.source.requires_api() {
				self.version_list = Vec::new();
				self.version_list_selected = 0;
				self.version_list_scroll = 0;
				self.version_loading = true;
				self.pending_async = Some(AsyncOp::FetchVersions);
				self.mode = Mode::EditVersion;
			} else {
				self.set_status(
					StatusKind::Warning,
					"Cannot change version for URL-sourced mods".to_string(),
				);
			}
		}
	}

	pub fn confirm_edit_version(
		&mut self,
		storage: &Storage,
	) {
		let version =
			self.version_list.get(self.version_list_selected).cloned();
		if let Some(version) = version {
			let version_str = version.version.clone();
			if let Some(m) = self.selected_mod_mut() {
				m.version = version.version.clone();
				m.download_url = version.download_url.clone();
				m.hash = version.hash.clone();
				m.hash_type = version.hash_type;
				let pt = m.project_type;
				let id = m.id.clone();
				if let Ok(mod_ron) = storage.load(pt, &id) {
					let mut updated = mod_ron;
					updated.version = m.version.clone();
					updated.download_url = m.download_url.clone();
					updated.hash = m.hash.clone();
					updated.hash_type = m.hash_type;
					if storage.save(pt, &id, &updated).is_ok() {
						self.set_status(
							StatusKind::Success,
							format!("Version updated to {}", version_str),
						);
					}
				}
			}
		}
		self.mode = Mode::Normal;
		self.version_loading = false;
		self.pending_async = None;
	}

	pub fn start_edit_description(&mut self) {
		if let Some(m) = self.selected_mod() {
			let lines: Vec<String> = if m.description.is_empty() {
				vec![String::new()]
			} else {
				m.description.lines().map(String::from).collect()
			};
			let mut textarea = TextArea::new(lines);
			textarea.set_block(
				ratatui::widgets::Block::default()
					.title(ratatui::text::Span::styled(
						" Edit Description (Esc: save) ",
						ratatui::style::Style::default()
							.fg(ratatui::style::Color::Yellow),
					))
					.borders(ratatui::widgets::Borders::ALL)
					.border_style(
						ratatui::style::Style::default()
							.fg(ratatui::style::Color::Yellow),
					),
			);
			self.description_textarea = Some(textarea);
			self.mode = Mode::EditDescription;
		}
	}

	pub fn confirm_edit_description(
		&mut self,
		storage: &Storage,
	) {
		let new_desc = self
			.description_textarea
			.as_ref()
			.map(|t| t.lines().join("\n"))
			.unwrap_or_default();
		if let Some(m) = self.selected_mod_mut() {
			m.description = new_desc;
			let pt = m.project_type;
			let id = m.id.clone();
			if let Ok(mod_ron) = storage.load(pt, &id) {
				let mut updated = mod_ron;
				updated.description = m.description.clone();
				if storage.save(pt, &id, &updated).is_ok() {
					self.set_status(
						StatusKind::Success,
						"Description updated".to_string(),
					);
				}
			}
		}
		self.description_textarea = None;
		self.mode = Mode::Normal;
	}

	pub fn start_edit_categories(&mut self) {
		if self.selected_mod().is_some() {
			self.category_picker_selected = 0;
			self.category_picker_scroll = 0;
			self.category_new_input = String::new();
			self.category_new_mode = false;
			self.mode = Mode::EditCategories;
		}
	}

	pub fn confirm_edit_categories(
		&mut self,
		storage: &Storage,
	) {
		self.refresh_all_categories();
		if let Some(m) = self.selected_mod_mut() {
			let pt = m.project_type;
			let id = m.id.clone();
			if let Ok(mod_ron) = storage.load(pt, &id) {
				let mut updated = mod_ron;
				updated.categories = m.categories.clone();
				if storage.save(pt, &id, &updated).is_ok() {
					self.set_status(
						StatusKind::Success,
						"Categories updated".to_string(),
					);
				}
			}
		}
		self.mode = Mode::Normal;
	}

	pub fn toggle_category_for_selected(
		&mut self,
		category: &str,
	) {
		if let Some(m) = self.selected_mod_mut() {
			if let Some(pos) = m.categories.iter().position(|c| c == category) {
				m.categories.remove(pos);
			} else {
				m.categories.push(category.to_string());
				m.categories.sort();
			}
		}
	}

	pub fn add_new_category(
		&mut self,
		category: &str,
	) {
		let cat = category.trim().to_string();
		if cat.is_empty() {
			return;
		}
		if let Some(m) = self.selected_mod_mut() {
			if !m.categories.contains(&cat) {
				m.categories.push(cat.clone());
				m.categories.sort();
			}
		}
		if !self.all_categories.contains(&cat) {
			self.all_categories.push(cat.clone());
			self.all_categories.sort();
		}
	}

	fn refresh_all_categories(&mut self) {
		self.all_categories = self
			.mods
			.iter()
			.flat_map(|m| m.categories.iter().cloned())
			.collect();
		self.all_categories.sort();
		self.all_categories.dedup();
	}

	pub fn start_dep_tree(
		&mut self,
		storage: &Storage,
	) {
		if let Some(m) = self.selected_mod() {
			let mod_id = m.id.clone();
			let source = m.source.clone();
			self.dep_tree =
				super::deps::build_dep_tree(storage, &mod_id, &source, 5);
			self.dep_reverse =
				super::deps::find_reverse_deps(storage, &mod_id, &source);
			self.dep_tree_scroll = 0;
			self.mode = Mode::DepTree;
		}
	}

	pub fn start_remove(
		&mut self,
		storage: &Storage,
	) {
		if let Some(m) = self.selected_mod() {
			let mod_id = m.id.clone();
			let source = m.source.clone();
			self.remove_dependents =
				super::deps::find_reverse_deps(storage, &mod_id, &source);
			self.mode = Mode::RemoveConfirm;
		}
	}

	pub fn confirm_remove(
		&mut self,
		storage: &Storage,
	) -> bool {
		if let Some(m) = self.selected_mod() {
			let pt = m.project_type;
			let id = m.id.clone();
			let source = m.source.clone();
			let name = m.name.clone();
			if storage.remove(pt, &id).is_ok() {
				super::deps::cleanup_stale_deps_after_remove(
					storage, &id, &source,
				);
				self.mods.retain(|m| m.id != id);
				self.update_filter();
				self.set_status(
					StatusKind::Success,
					format!("Removed {}", name),
				);
				self.mode = Mode::Normal;
				self.focus = Focus::ModList;
				return true;
			}
		}
		self.set_status(StatusKind::Error, "Failed to remove mod".to_string());
		self.mode = Mode::Normal;
		false
	}

	pub fn start_update_check(&mut self) {
		if let Some(m) = self.selected_mod() {
			if m.unresolved {
				self.set_status(
					StatusKind::Warning,
					"Cannot check updates for unresolved mods".to_string(),
				);
			} else if m.source.requires_api() {
				self.update_loading = true;
				self.update_result = None;
				self.update_error = None;
				self.pending_async = Some(AsyncOp::CheckUpdate);
				self.mode = Mode::UpdateCheck;
			} else {
				self.set_status(
					StatusKind::Warning,
					"Cannot check updates for URL-sourced mods".to_string(),
				);
			}
		}
	}

	pub fn apply_update(
		&mut self,
		storage: &Storage,
	) {
		let update = self.update_result.take();
		if let Some(update) = update {
			let new_version = update.latest_version.clone();
			if let Some(m) = self.selected_mod_mut() {
				m.version = update.latest_version;
				m.download_url = update.download_url;
				m.hash = update.hash;
				m.hash_type = update.hash_type;
				let pt = m.project_type;
				let id = m.id.clone();
				if let Ok(mod_ron) = storage.load(pt, &id) {
					let mut updated = mod_ron;
					updated.version = m.version.clone();
					updated.download_url = m.download_url.clone();
					updated.hash = m.hash.clone();
					updated.hash_type = m.hash_type;
					if storage.save(pt, &id, &updated).is_ok() {
						self.set_status(
							StatusKind::Success,
							format!("Updated to v{}", new_version),
						);
					}
				}
			}
		}
		self.mode = Mode::Normal;
		self.update_loading = false;
		self.pending_async = None;
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
