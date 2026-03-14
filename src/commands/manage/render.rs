use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
	Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::Frame;

use crate::types::ModSource;

use super::app::{DetailField, Focus, ManageApp, Mode, StatusKind};

pub fn draw(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let size = f.area();
	f.render_widget(Clear, size);

	let status_height = if app.status_message.is_some() { 1 } else { 0 };

	let chunks = Layout::default()
		.direction(Direction::Vertical)
		.constraints([
			Constraint::Length(3),
			Constraint::Min(0),
			Constraint::Length(status_height),
			Constraint::Length(1),
		])
		.split(size);

	draw_header(f, app, chunks[0]);

	if app.mode == Mode::DepTree {
		draw_dep_tree(f, app, chunks[1]);
	} else {
		draw_dual_pane(f, app, chunks[1]);
	}

	if status_height > 0 {
		draw_status_bar(f, app, chunks[2]);
	}

	match app.mode {
		Mode::EditEnv => draw_env_popup(f, app),
		Mode::EditVersion => draw_version_popup(f, app),
		Mode::EditDescription => draw_description_popup(f, app),
		Mode::EditCategories => draw_categories_popup(f, app),
		Mode::RemoveConfirm => draw_remove_popup(f, app),
		Mode::UpdateCheck => draw_update_popup(f, app),
		_ => {}
	}

	draw_footer(f, app, chunks[3]);
}

fn draw_header(
	f: &mut Frame,
	app: &ManageApp,
	area: Rect,
) {
	let title = Span::styled(
		format!(
			" yammm manage — {} (MC {}, {}) — {} mods ",
			app.modpack_name,
			app.modpack_version,
			app.modpack_loader,
			app.mods.len()
		),
		Style::default()
			.fg(Color::Cyan)
			.add_modifier(Modifier::BOLD),
	);

	let header = Block::default()
		.title(title)
		.borders(Borders::ALL)
		.border_style(Style::default().fg(Color::DarkGray));

	let inner = header.inner(area);
	f.render_widget(header, area);

	let query_label =
		Span::styled("Search: ", Style::default().fg(Color::Yellow));
	let query_text = Span::styled(
		if app.query.is_empty() {
			"type to filter..."
		} else {
			&app.query
		},
		if app.query.is_empty() {
			Style::default().fg(Color::DarkGray)
		} else {
			Style::default().fg(Color::White)
		},
	);
	let cursor = Span::styled("█", Style::default().fg(Color::White));

	let cat_label = Span::styled(
		format!("  [Cat: {}]", app.category_filter_label()),
		Style::default().fg(Color::Magenta),
	);

	let line = Line::from(vec![query_label, query_text, cursor, cat_label]);
	let paragraph = Paragraph::new(line);
	f.render_widget(paragraph, inner);
}

fn draw_dual_pane(
	f: &mut Frame,
	app: &mut ManageApp,
	area: Rect,
) {
	let chunks = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
		.split(area);

	draw_mod_list(f, app, chunks[0]);
	draw_detail(f, app, chunks[1]);
}

fn draw_mod_list(
	f: &mut Frame,
	app: &mut ManageApp,
	area: Rect,
) {
	let visible_count = area.height.saturating_sub(2) as usize;
	let list_height = visible_count.max(1);

	if app.mod_list.selected < app.mod_list.scroll_offset {
		app.mod_list.scroll_offset = app.mod_list.selected;
	}
	if app.mod_list.selected >= app.mod_list.scroll_offset + list_height {
		app.mod_list.scroll_offset = app.mod_list.selected - list_height + 1;
	}

	let list_focused = app.focus == Focus::ModList && app.mode == Mode::Normal;
	let border_color = if list_focused {
		Color::Green
	} else {
		Color::DarkGray
	};

	let items: Vec<ListItem> = app
		.mod_list
		.filtered_indices
		.iter()
		.skip(app.mod_list.scroll_offset)
		.take(list_height)
		.enumerate()
		.map(|(i, &idx)| {
			let m = &app.mods[idx];
			let is_selected =
				app.mod_list.scroll_offset + i == app.mod_list.selected;
			let style = if is_selected && list_focused {
				Style::default()
					.bg(Color::DarkGray)
					.add_modifier(Modifier::BOLD)
			} else if is_selected {
				Style::default()
					.fg(Color::Cyan)
					.add_modifier(Modifier::BOLD)
			} else {
				Style::default()
			};
			let line = Line::from(vec![
				Span::styled(m.name.clone(), style),
				Span::raw(" "),
				Span::styled(
					format!("[{}]", source_short_tag(&m.source)),
					style.fg(source_color(&m.source)),
				),
			]);
			ListItem::new(line)
		})
		.collect();

	let list = List::new(items)
		.block(
			Block::default()
				.title(Span::styled(
					format!(
						" Mods ({}/{}) ",
						app.mod_list.filtered_indices.len(),
						app.mods.len()
					),
					Style::default()
						.fg(Color::Green)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(border_color)),
		)
		.highlight_style(
			Style::default()
				.bg(Color::DarkGray)
				.add_modifier(Modifier::BOLD),
		)
		.highlight_symbol("▶ ");

	let mut state = ListState::default();
	state.select(Some(
		app.mod_list
			.selected
			.saturating_sub(app.mod_list.scroll_offset),
	));
	f.render_stateful_widget(list, area, &mut state);
}

fn draw_detail(
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

fn read_only_row(
	label: &str,
	value: &str,
) -> Line<'static> {
	Line::from(vec![
		Span::styled(
			format!("  {}: ", label),
			Style::default().fg(Color::DarkGray),
		),
		Span::raw(value.to_string()),
	])
}

fn field_row(
	field: DetailField,
	value: &str,
	highlighted: bool,
) -> Line<'static> {
	let label_style = if highlighted {
		Style::default()
			.fg(Color::Yellow)
			.add_modifier(Modifier::BOLD)
	} else {
		Style::default().fg(Color::Yellow)
	};
	let value_style = if highlighted {
		Style::default()
			.fg(Color::White)
			.add_modifier(Modifier::BOLD)
	} else {
		Style::default()
	};
	let marker = if highlighted { "▶ " } else { "  " };
	let shortcut = format!(" [{}]", field.shortcut());
	let shortcut_style = if highlighted {
		Style::default().fg(Color::Cyan)
	} else {
		Style::default().fg(Color::DarkGray)
	};

	Line::from(vec![
		Span::styled(marker, Style::default().fg(Color::Cyan)),
		Span::styled(format!("{}: ", field.label()), label_style),
		Span::styled(value.to_string(), value_style),
		Span::styled(shortcut, shortcut_style),
	])
}

fn action_row(
	field: DetailField,
	label: &str,
	highlighted: bool,
) -> Line<'static> {
	let style = if highlighted {
		Style::default()
			.fg(Color::White)
			.add_modifier(Modifier::BOLD)
	} else {
		Style::default().fg(Color::Gray)
	};
	let marker = if highlighted { "▶ " } else { "  " };
	let shortcut = format!(" [{}]", field.shortcut());
	let shortcut_style = if highlighted {
		Style::default().fg(Color::Cyan)
	} else {
		Style::default().fg(Color::DarkGray)
	};

	Line::from(vec![
		Span::styled(marker, Style::default().fg(Color::Cyan)),
		Span::styled(label.to_string(), style),
		Span::styled(shortcut, shortcut_style),
	])
}

fn draw_env_popup(
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

fn draw_version_popup(
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

fn draw_description_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let area = centered_rect(70, 14, f.area());
	f.render_widget(Clear, area);

	if let Some(ref textarea) = app.description_textarea {
		f.render_widget(textarea, area);
	}
}

fn draw_categories_popup(
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

fn draw_dep_tree(
	f: &mut Frame,
	app: &mut ManageApp,
	area: Rect,
) {
	let mut lines = Vec::new();

	if let Some(m) = app.selected_mod() {
		lines.push(Line::from(Span::styled(
			format!("Dependencies of {}", m.name),
			Style::default()
				.fg(Color::Cyan)
				.add_modifier(Modifier::BOLD),
		)));
		lines.push(Line::raw(""));

		for node in &app.dep_tree {
			draw_dep_node(&mut lines, node, "");
		}

		if !app.dep_reverse.is_empty() {
			lines.push(Line::raw(""));
			lines.push(Line::from(Span::styled(
				"Dependents (reverse):",
				Style::default()
					.fg(Color::Yellow)
					.add_modifier(Modifier::BOLD),
			)));
			for (id, name) in &app.dep_reverse {
				lines.push(Line::from(Span::styled(
					format!("  • {} ({})", name, id),
					Style::default().fg(Color::Gray),
				)));
			}
		}
	}

	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(
		"Press Esc to go back",
		Style::default().fg(Color::DarkGray),
	)));

	let paragraph = Paragraph::new(lines)
		.block(
			Block::default()
				.title(Span::styled(
					" Dependency Tree ",
					Style::default()
						.fg(Color::Blue)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::DarkGray)),
		)
		.wrap(Wrap { trim: false });

	f.render_widget(paragraph, area);
}

fn draw_dep_node(
	lines: &mut Vec<Line<'static>>,
	node: &super::app::DepNode,
	prefix: &str,
) {
	let kind_str = node
		.kind
		.map(|k| format!(" ({})", k.as_str()))
		.unwrap_or_default();

	let line = Line::from(vec![
		Span::raw(prefix.to_string()),
		Span::styled(node.name.clone(), Style::default().fg(Color::White)),
		Span::styled(
			format!(" v{}", node.version),
			Style::default().fg(Color::DarkGray),
		),
		Span::styled(kind_str, Style::default().fg(Color::Blue)),
	]);
	lines.push(line);

	for child in &node.children {
		draw_dep_node(lines, child, &format!("{}  ", prefix));
	}
}

fn draw_remove_popup(
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

fn draw_update_popup(
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

fn draw_status_bar(
	f: &mut Frame,
	app: &ManageApp,
	area: Rect,
) {
	if let Some((kind, msg)) = &app.status_message {
		let color = match kind {
			StatusKind::Success => Color::Green,
			StatusKind::Warning => Color::Yellow,
			StatusKind::Error => Color::Red,
		};
		let icon = match kind {
			StatusKind::Success => "✓",
			StatusKind::Warning => "⚠",
			StatusKind::Error => "✗",
		};
		let line = Line::from(vec![
			Span::styled(
				format!(" {} ", icon),
				Style::default().fg(color).add_modifier(Modifier::BOLD),
			),
			Span::styled(msg.clone(), Style::default().fg(color)),
		]);
		let paragraph = Paragraph::new(line);
		f.render_widget(paragraph, area);
	}
}

fn draw_footer(
	f: &mut Frame,
	app: &ManageApp,
	area: Rect,
) {
	let help = match app.mode {
		Mode::Normal => match app.focus {
			Focus::ModList => "↑↓ navigate │ type to filter │ Tab: category │ Enter: detail │ Esc: quit",
			Focus::Detail => "↑↓/Enter: fields │ Tab: back to list │ v/e/d/c/t/r/u: shortcuts",
		},
		Mode::EditEnv => "↑↓ select │ Enter: confirm │ Esc: cancel",
		Mode::EditVersion => "↑↓ select │ Enter: confirm │ Esc: cancel",
		Mode::EditDescription => "Type to edit │ Esc: save │ Ctrl+A: select all │ Ctrl+C/V/X: copy/paste/cut",
		Mode::EditCategories => "↑↓/Enter: toggle │ n: new │ Esc: done",
		Mode::DepTree => "Esc: back",
		Mode::RemoveConfirm => "Enter: confirm │ Esc: cancel",
		Mode::UpdateCheck => "Enter: apply │ Esc: back",
	};

	let style = Style::default().fg(Color::DarkGray);
	let paragraph = Paragraph::new(Span::styled(help, style));
	f.render_widget(paragraph, area);
}

fn centered_rect(
	percent_x: u16,
	height: u16,
	r: Rect,
) -> Rect {
	let popup_width = r.width * percent_x / 100;
	let x = (r.width.saturating_sub(popup_width)) / 2;
	let y = (r.height.saturating_sub(height)) / 2;
	Rect::new(
		r.x + x,
		r.y + y,
		popup_width.min(r.width),
		height.min(r.height),
	)
}

fn source_label(source: &ModSource) -> &'static str {
	crate::output::source_label(source)
}

fn source_color(source: &ModSource) -> Color {
	match source {
		ModSource::Modrinth { .. } => Color::Green,
		ModSource::CurseForge { .. } => Color::Magenta,
		ModSource::Url { .. } => Color::DarkGray,
	}
}

fn source_short_tag(source: &ModSource) -> &'static str {
	match source {
		ModSource::Modrinth { .. } => "mr",
		ModSource::CurseForge { .. } => "cf",
		ModSource::Url { .. } => "url",
	}
}
