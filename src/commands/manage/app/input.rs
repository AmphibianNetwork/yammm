use super::{Focus, ManageApp};

impl ManageApp {
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

	pub fn dep_move_up(&mut self) {
		if self.dep_selected > 0 {
			self.dep_selected -= 1;
		}
	}

	pub fn dep_move_down(&mut self) {
		if self.dep_selected < self.dep_entries.len().saturating_sub(1) {
			self.dep_selected += 1;
		}
	}

	pub fn dep_tree_move_up(&mut self) {
		if self.dep_tree_scroll > 0 {
			self.dep_tree_scroll -= 1;
		}
	}

	pub fn dep_tree_move_down(&mut self) {
		self.dep_tree_scroll += 1;
	}

	pub fn dep_toggle_mark(&mut self) {
		if let Some(entry) = self.dep_entries.get(self.dep_selected)
			&& !entry.installed
		{
			if self.dep_marked.contains(&entry.mod_id) {
				self.dep_marked.remove(&entry.mod_id);
			} else {
				self.dep_marked.insert(entry.mod_id.clone());
			}
		}
	}

	pub fn dep_toggle_all(&mut self) {
		let has_unmarked = self
			.dep_entries
			.iter()
			.any(|e| !e.installed && !self.dep_marked.contains(&e.mod_id));
		if has_unmarked {
			for entry in &self.dep_entries {
				if !entry.installed {
					self.dep_marked.insert(entry.mod_id.clone());
				}
			}
		} else {
			self.dep_marked.clear();
		}
	}
}
