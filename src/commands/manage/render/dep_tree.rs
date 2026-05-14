use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::app::{DepNode, ManageApp};
use super::centered_rect;

pub(super) fn draw_dep_tree(
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
		lines.push(Line::from(vec![
			Span::styled(" ● installed   ", Style::default().fg(Color::Green)),
			Span::styled("○ missing", Style::default().fg(Color::Red)),
		]));
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

	let missing = app.dep_entries.iter().filter(|e| !e.installed).count();
	if missing > 0 {
		lines.push(Line::raw(""));
		lines.push(Line::from(Span::styled(
			format!("{} missing dep(s) — press 'i' to install", missing),
			Style::default().fg(Color::Yellow),
		)));
	}
	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(
		"↑↓ scroll │ i: install missing │ Esc: back",
		Style::default().fg(Color::DarkGray),
	)));

	let visible_height = area.height.saturating_sub(2) as usize;
	let max_scroll = lines.len().saturating_sub(visible_height);
	if app.dep_tree_scroll > max_scroll {
		app.dep_tree_scroll = max_scroll;
	}

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
		.scroll((app.dep_tree_scroll as u16, 0))
		.wrap(Wrap { trim: false });

	f.render_widget(paragraph, area);
}

fn draw_dep_node(
	lines: &mut Vec<Line<'static>>,
	node: &DepNode,
	prefix: &str,
) {
	let kind_str = node
		.kind
		.map(|k| format!(" ({})", k.as_str()))
		.unwrap_or_default();

	let status = if node.installed {
		Span::styled(" ●".to_string(), Style::default().fg(Color::Green))
	} else {
		Span::styled(" ○".to_string(), Style::default().fg(Color::Red))
	};

	let name_style = if node.installed {
		Style::default().fg(Color::White)
	} else {
		Style::default().fg(Color::Gray)
	};

	let version_span = if node.version.is_empty() {
		Span::raw(String::new())
	} else {
		Span::styled(
			format!(" v{}", node.version),
			Style::default().fg(Color::DarkGray),
		)
	};

	let line = Line::from(vec![
		Span::raw(prefix.to_string()),
		Span::styled(node.name.clone(), name_style),
		version_span,
		Span::styled(kind_str, Style::default().fg(Color::Blue)),
		status,
	]);
	lines.push(line);

	for child in &node.children {
		draw_dep_node(lines, child, &format!("{}  ", prefix));
	}
}

pub(super) fn draw_install_deps(
	f: &mut Frame,
	app: &mut ManageApp,
	area: Rect,
) {
	let visible_count = area.height.saturating_sub(2) as usize;
	if app.dep_selected < app.dep_scroll {
		app.dep_scroll = app.dep_selected;
	}
	if app.dep_selected >= app.dep_scroll + visible_count {
		app.dep_scroll = app.dep_selected - visible_count + 1;
	}

	let mut lines = Vec::new();

	if let Some(m) = app.selected_mod() {
		lines.push(Line::from(Span::styled(
			format!("Install dependencies for {}", m.name),
			Style::default()
				.fg(Color::Cyan)
				.add_modifier(Modifier::BOLD),
		)));
		lines.push(Line::from(vec![
			Span::styled("[x] ", Style::default().fg(Color::Green)),
			Span::raw("install  "),
			Span::styled("[ ] ", Style::default().fg(Color::DarkGray)),
			Span::raw("skip  "),
			Span::styled("● ", Style::default().fg(Color::Green)),
			Span::raw("installed  "),
			Span::styled("○ ", Style::default().fg(Color::Red)),
			Span::raw("missing"),
		]));
		lines.push(Line::raw(""));
	}

	for (i, entry) in app.dep_entries.iter().enumerate() {
		if i < app.dep_scroll || i >= app.dep_scroll + visible_count {
			continue;
		}

		let is_selected = i == app.dep_selected;
		let bg = if is_selected {
			Style::default().bg(Color::DarkGray)
		} else {
			Style::default()
		};
		let indent = "  ".repeat(entry.indent);
		let marker = if entry.installed {
			""
		} else if app.dep_marked.contains(&entry.mod_id) {
			"[x] "
		} else {
			"[ ] "
		};

		let status = if entry.installed {
			Span::styled(" ●", Style::default().fg(Color::Green).patch(bg))
		} else {
			Span::styled(" ○", Style::default().fg(Color::Red).patch(bg))
		};

		let kind_str = entry
			.kind
			.map(|k| format!(" ({})", k.as_str()))
			.unwrap_or_default();

		let name_style = if entry.installed {
			Style::default().fg(Color::DarkGray).patch(bg)
		} else if app.dep_marked.contains(&entry.mod_id) {
			Style::default().fg(Color::White).patch(bg)
		} else {
			Style::default().fg(Color::Gray).patch(bg)
		};

		let marker_style = if app.dep_marked.contains(&entry.mod_id) {
			Style::default().fg(Color::Green).patch(bg)
		} else {
			Style::default().fg(Color::DarkGray).patch(bg)
		};

		let version_span = if entry.version.is_empty() {
			Span::raw(String::new())
		} else {
			Span::styled(format!(" v{}", entry.version), bg.fg(Color::DarkGray))
		};

		let line = Line::from(vec![
			Span::styled(format!("{}{}", indent, marker), marker_style),
			Span::styled(entry.name.clone(), name_style),
			version_span,
			Span::styled(kind_str, Style::default().fg(Color::Blue).patch(bg)),
			status,
		]);
		lines.push(line);
	}

	if app.dep_installing {
		lines.push(Line::raw(""));
		lines.push(Line::from(Span::styled(
			"Installing...",
			Style::default().fg(Color::Yellow),
		)));
	} else {
		let marked_count = app.dep_marked.len();
		lines.push(Line::raw(""));
		lines.push(Line::from(Span::styled(
			format!(
				"Space: toggle │ a: toggle all │ Enter: install {} │ Esc: back",
				if marked_count > 0 {
					format!("({} selected)", marked_count)
				} else {
					"".to_string()
				}
			),
			Style::default().fg(Color::DarkGray),
		)));
	}

	let paragraph = Paragraph::new(lines)
		.block(
			Block::default()
				.title(Span::styled(
					" Install Dependencies ",
					Style::default()
						.fg(Color::Green)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::DarkGray)),
		)
		.wrap(Wrap { trim: false });

	f.render_widget(paragraph, area);
}

pub(super) fn draw_install_progress_popup(
	f: &mut Frame,
	app: &mut ManageApp,
) {
	let area = centered_rect(70, 20, f.area());
	f.render_widget(Clear, area);

	let visible_height = area.height.saturating_sub(2) as usize;
	let max_scroll = app.dep_output.len().saturating_sub(visible_height);
	if app.dep_output_scroll > max_scroll {
		app.dep_output_scroll = max_scroll;
	}

	let title = if app.dep_installing {
		" Installing Dependencies... "
	} else {
		" Install Result "
	};

	let lines: Vec<Line> = app
		.dep_output
		.iter()
		.map(|l| Line::from(Span::raw(l.clone())))
		.collect();

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
				.border_style(Style::default().fg(Color::Yellow)),
		)
		.scroll((app.dep_output_scroll as u16, 0));

	f.render_widget(paragraph, area);
}
