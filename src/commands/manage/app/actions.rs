use crate::storage::Storage;
use crate::types::ModEnv;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};
use tui_textarea::TextArea;

use super::{DetailField, Focus, ManageApp, Mode, StatusKind};

impl ManageApp {
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
				match storage.save(pt, &id, &updated) {
					Ok(()) => {
						self.set_status(
							StatusKind::Success,
							format!("Env updated to {}", new_env),
						);
					}
					Err(e) => {
						self.set_status(
							StatusKind::Error,
							format!("Failed to save: {e}"),
						);
					}
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
				self.pending_async = Some(super::AsyncOp::FetchVersions);
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
					match storage.save(pt, &id, &updated) {
						Ok(()) => {
							self.set_status(
								StatusKind::Success,
								format!("Version updated to {}", version_str),
							);
						}
						Err(e) => {
							self.set_status(
								StatusKind::Error,
								format!("Failed to save: {e}"),
							);
						}
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
				Block::default()
					.title(Span::styled(
						" Edit Description (Esc: save) ",
						Style::default().fg(Color::Yellow),
					))
					.borders(Borders::ALL)
					.border_style(Style::default().fg(Color::Yellow)),
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
				match storage.save(pt, &id, &updated) {
					Ok(()) => {
						self.set_status(
							StatusKind::Success,
							"Description updated".to_string(),
						);
					}
					Err(e) => {
						self.set_status(
							StatusKind::Error,
							format!("Failed to save: {e}"),
						);
					}
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
				match storage.save(pt, &id, &updated) {
					Ok(()) => {
						self.set_status(
							StatusKind::Success,
							"Categories updated".to_string(),
						);
					}
					Err(e) => {
						self.set_status(
							StatusKind::Error,
							format!("Failed to save: {e}"),
						);
					}
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
		if let Some(m) = self.selected_mod_mut()
			&& !m.categories.contains(&cat)
		{
			m.categories.push(cat.clone());
			m.categories.sort();
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
			self.dep_tree = super::super::deps::build_dep_tree(
				storage, &mod_id, &source, 5,
			);
			self.dep_reverse = super::super::deps::find_reverse_dependents(
				storage, &mod_id, &source,
			);
			self.dep_entries =
				super::super::deps::flatten_dep_tree(&self.dep_tree, 0);
			self.dep_selected = 0;
			self.dep_scroll = 0;
			self.dep_marked.clear();
			self.dep_tree_scroll = 0;
			self.mode = Mode::DepTree;
		}
	}

	pub fn start_install_deps(&mut self) {
		self.mode = Mode::InstallDeps;
		self.dep_selected = 0;
		self.dep_scroll = 0;
		self.dep_marked.clear();
	}

	pub fn start_remove(
		&mut self,
		storage: &Storage,
	) {
		if let Some(m) = self.selected_mod() {
			let mod_id = m.id.clone();
			let source = m.source.clone();
			self.remove_dependents =
				super::super::deps::find_reverse_dependents(
					storage, &mod_id, &source,
				);
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
				super::super::deps::cleanup_stale_deps_after_remove(
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
				self.pending_async = Some(super::AsyncOp::CheckUpdate);
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
					match storage.save(pt, &id, &updated) {
						Ok(()) => {
							self.set_status(
								StatusKind::Success,
								format!("Updated to v{}", new_version),
							);
						}
						Err(e) => {
							self.set_status(
								StatusKind::Error,
								format!("Failed to save: {e}"),
							);
						}
					}
				}
			}
		}
		self.mode = Mode::Normal;
		self.update_loading = false;
		self.pending_async = None;
	}
}
