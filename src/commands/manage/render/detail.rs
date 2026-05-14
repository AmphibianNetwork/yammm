use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
	Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

use super::app::{DetailField, Focus, ManageApp, Mode};
use super::{
	action_row, centered_rect, field_row, read_only_row, source_label,
};

pub(super) fn draw_detail(
	f: &mut Frame,
	app: &mut ManageApp,
	area: Rect,
) {
	let detail_focused = app.focus == Focus::Detail && app.mode == Mode::Normal;
	let border_color = if detail_focused {
		Color::Yellow
	} else {
		Color::DarkGray
	};

	let lines = match app.selected_mod() {
		Some(m) => {
			let mut v = vec![
				Line::from(Span::styled(
					m.name.clone(),
					Style::default()
						.fg(Color::Cyan)
						.add_modifier(Modifier::BOLD),
				)),
				Line::raw(""),
			];

			v.push(read_only_row("ID", &m.id));
			v.push(field_row(
				DetailField::Version,
				&m.version,
				app.selected_field() == DetailField::Version && detail_focused,
			));
			v.push(read_only_row("Source", source_label(&m.source)));

			if m.unresolved {
				v.push(Line::from(vec![
					Span::styled(
						"  Unresolved: ",
						Style::default().fg(Color::Yellow),
					),
					Span::styled(
						"Yes",
						Style::default()
							.fg(Color::Yellow)
							.add_modifier(Modifier::BOLD),
					),
				]));
			}

			v.push(field_row(
				DetailField::Env,
				m.env.as_str(),
				app.selected_field() == DetailField::Env && detail_focused,
			));
			v.push(read_only_row("Project", m.project_type.as_str()));

			let desc_display = if m.description.is_empty() {
				"(empty)".to_string()
			} else {
				m.description.clone()
			};
			v.push(field_row(
				DetailField::Description,
				&desc_display,
				app.selected_field() == DetailField::Description
					&& detail_focused,
			));

			let cats = if m.categories.is_empty() {
				"(none)".to_string()
			} else {
				m.categories.join(", ")
			};
			v.push(field_row(
				DetailField::Categories,
				&cats,
				app.selected_field() == DetailField::Categories
					&& detail_focused,
			));

			if let Some(ref hash) = m.hash {
				let truncated: String = hash.chars().take(16).collect();
				v.push(read_only_row("Hash", &format!("{}…", truncated)));
			}

			if !m.download_url.is_empty() {
				v.push(read_only_row("Download", &m.download_url));
			}

			v.push(Line::raw(""));

			let dep_count = m.dependencies.len();
			let required = m
				.dependencies
				.iter()
				.filter(|d| d.kind.is_required())
				.count();
			let dep_label = if dep_count > 0 {
				format!("{} deps ({} required)", dep_count, required)
			} else {
				"No dependencies".to_string()
			};
			v.push(action_row(
				DetailField::DepTree,
				&dep_label,
				app.selected_field() == DetailField::DepTree && detail_focused,
			));

			v.push(action_row(
				DetailField::Remove,
				"Remove this mod",
				app.selected_field() == DetailField::Remove && detail_focused,
			));
			v.push(action_row(
				DetailField::UpdateCheck,
				"Check for updates",
				app.selected_field() == DetailField::UpdateCheck
					&& detail_focused,
			));

			v
		}
		None => vec![Line::from(Span::styled(
			"No mod selected",
			Style::default().fg(Color::DarkGray),
		))],
	};

	let title = if detail_focused {
		" Details (focused) "
	} else {
		" Details "
	};

	let paragraph = Paragraph::new(lines)
		.block(
			Block::default()
				.title(Span::styled(
					title,
					Style::default()
						.fg(Color::Yellow)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(border_color)),
		)
		.wrap(Wrap { trim: false });

	f.render_widget(paragraph, area);
}

pub(super) fn draw_env_popup(
	f: &mut Frame,
	app: &ManageApp,
) {
	let options = ["Both", "Client", "Server"];
	let items: Vec<ListItem> = options
		.iter()
		.enumerate()
		.map(|(i, opt)| {
			let style = if i == app.env_picker_selected {
				Style::default()
					.bg(Color::DarkGray)
					.add_modifier(Modifier::BOLD)
			} else {
				Style::default()
			};
			ListItem::new(Line::from(Span::styled(*opt, style)))
		})
		.collect();

	let area = centered_rect(40, 5, f.area());
	let list = List::new(items)
		.block(
			Block::default()
				.title(Span::styled(
					" Set Environment ",
					Style::default()
						.fg(Color::Yellow)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::Yellow)),
		)
		.highlight_symbol("▶ ");

	f.render_widget(Clear, area);
	let mut state = ListState::default();
	state.select(Some(app.env_picker_selected));
	f.render_stateful_widget(list, area, &mut state);
}

pub(super) fn draw_version_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let area = centered_rect(60, 15, f.area());
	f.render_widget(Clear, area);

	if app.version_loading {
		let paragraph = Paragraph::new("Loading versions...").block(
			Block::default()
				.title(" Select Version ")
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::Yellow)),
		);
		f.render_widget(paragraph, area);
		return;
	}

	let visible_count = area.height.saturating_sub(2) as usize;
	if app.version_list_selected < app.version_list_scroll {
		app.version_list_scroll = app.version_list_selected;
	}
	if app.version_list_selected >= app.version_list_scroll + visible_count {
		app.version_list_scroll = app.version_list_selected - visible_count + 1;
	}

	let items: Vec<ListItem> = app
		.version_list
		.iter()
		.skip(app.version_list_scroll)
		.take(visible_count)
		.enumerate()
		.map(|(i, v)| {
			let actual_idx = app.version_list_scroll + i;
			let style = if actual_idx == app.version_list_selected {
				Style::default()
					.bg(Color::DarkGray)
					.add_modifier(Modifier::BOLD)
			} else {
				Style::default()
			};
			ListItem::new(Line::from(Span::styled(
				format!("{} ({})", v.version, v.release_date),
				style,
			)))
		})
		.collect();

	let list = List::new(items)
		.block(
			Block::default()
				.title(Span::styled(
					" Select Version ",
					Style::default()
						.fg(Color::Yellow)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::Yellow)),
		)
		.highlight_symbol("▶ ");

	let mut state = ListState::default();
	state.select(Some(
		app.version_list_selected
			.saturating_sub(app.version_list_scroll),
	));
	f.render_stateful_widget(list, area, &mut state);
}

pub(super) fn draw_description_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let area = centered_rect(70, 14, f.area());
	f.render_widget(Clear, area);

	if let Some(ref textarea) = app.description_textarea {
		f.render_widget(textarea, area);
	}
}

pub(super) fn draw_categories_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let area = centered_rect(50, 14, f.area());
	f.render_widget(Clear, area);

	let current = app
		.selected_mod()
		.map(|m| m.categories.clone())
		.unwrap_or_default();

	if app.category_new_mode {
		let items = vec![ListItem::new(Line::from(vec![
			Span::styled("New category: ", Style::default().fg(Color::Yellow)),
			Span::raw(&app.category_new_input),
		]))];
		let list = List::new(items)
			.block(
				Block::default()
					.title(Span::styled(
						" Edit Categories ",
						Style::default()
							.fg(Color::Yellow)
							.add_modifier(Modifier::BOLD),
					))
					.borders(Borders::ALL)
					.border_style(Style::default().fg(Color::Yellow)),
			)
			.highlight_symbol("▶ ");
		f.render_widget(list, area);
		return;
	}

	let visible_count = area.height.saturating_sub(2) as usize;
	if app.category_picker_selected < app.category_picker_scroll {
		app.category_picker_scroll = app.category_picker_selected;
	}
	if app.category_picker_selected
		>= app.category_picker_scroll + visible_count
	{
		app.category_picker_scroll =
			app.category_picker_selected - visible_count + 1;
	}

	let total_items = app.all_categories.len() + 1;
	let items: Vec<ListItem> = app
		.all_categories
		.iter()
		.enumerate()
		.skip(app.category_picker_scroll.min(app.all_categories.len()))
		.take(visible_count)
		.map(|(i, cat)| {
			let is_selected = i == app.category_picker_selected;
			let is_applied = current.contains(cat);
			let marker = if is_applied { "[x] " } else { "[ ] " };
			let style = if is_selected {
				Style::default()
					.bg(Color::DarkGray)
					.add_modifier(Modifier::BOLD)
			} else {
				Style::default()
			};
			ListItem::new(Line::from(Span::styled(
				format!("{}{}", marker, cat),
				style,
			)))
		})
		.chain(
			if app.category_picker_scroll + visible_count >= total_items {
				std::iter::once(ListItem::new(Line::from(Span::styled(
					"[+] Add new category...",
					Style::default().fg(Color::Green),
				))))
			} else {
				std::iter::once(ListItem::new(Line::raw("")))
			},
		)
		.collect();

	let list = List::new(items)
		.block(
			Block::default()
				.title(Span::styled(
					" Edit Categories ",
					Style::default()
						.fg(Color::Yellow)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::Yellow)),
		)
		.highlight_symbol("▶ ");

	f.render_widget(list, area);
}

pub(super) fn draw_remove_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let mut lines = Vec::new();

	if let Some(m) = app.selected_mod() {
		lines.push(Line::from(Span::styled(
			format!("Remove {}?", m.name),
			Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
		)));

		if !app.remove_dependents.is_empty() {
			lines.push(Line::raw(""));
			lines.push(Line::from(Span::styled(
				format!(
					"{} mod(s) depend on this:",
					app.remove_dependents.len()
				),
				Style::default().fg(Color::Yellow),
			)));
			for (id, name) in &app.remove_dependents {
				lines.push(Line::from(Span::styled(
					format!("  • {} ({})", name, id),
					Style::default().fg(Color::Gray),
				)));
			}
			lines.push(Line::raw(""));
			lines.push(Line::from(Span::styled(
				"This may break those mods!",
				Style::default().fg(Color::Red),
			)));
		}

		lines.push(Line::raw(""));
		lines.push(Line::from(Span::styled(
			"Enter: confirm  Esc: cancel",
			Style::default().fg(Color::DarkGray),
		)));
	}

	let area = centered_rect(50, 12, f.area());
	f.render_widget(Clear, area);

	let paragraph = Paragraph::new(lines).block(
		Block::default()
			.borders(Borders::ALL)
			.border_style(Style::default().fg(Color::Red)),
	);

	f.render_widget(paragraph, area);
}

pub(super) fn draw_update_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let area = centered_rect(50, 8, f.area());
	f.render_widget(Clear, area);

	let lines = if app.update_loading {
		vec![Line::from("Checking for updates...")]
	} else if let Some(ref err) = app.update_error {
		vec![
			Line::from(Span::styled(
				"Update check failed",
				Style::default().fg(Color::Red),
			)),
			Line::from(Span::styled(
				err.clone(),
				Style::default().fg(Color::Gray),
			)),
		]
	} else if let Some(ref update) = app.update_result {
		vec![
			Line::from(Span::styled(
				"Update available!",
				Style::default().fg(Color::Green),
			)),
			Line::from(Span::raw(format!(
				"{} → {}",
				update.current_version, update.latest_version
			))),
			Line::raw(""),
			Line::from(Span::styled(
				"Enter: apply  Esc: cancel",
				Style::default().fg(Color::DarkGray),
			)),
		]
	} else {
		vec![Line::from(Span::styled(
			"Already up to date",
			Style::default().fg(Color::Green),
		))]
	};

	let paragraph = Paragraph::new(lines).block(
		Block::default()
			.title(" Update Check ")
			.borders(Borders::ALL)
			.border_style(Style::default().fg(Color::Green)),
	);

	f.render_widget(paragraph, area);
}
