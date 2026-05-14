use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::types::ModSource;

use super::app;
use super::app::{DetailField, ManageApp, Mode, StatusKind};

mod dep_tree;
mod detail;
mod help;
mod help_overlay;
mod modlist;

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
		dep_tree::draw_dep_tree(f, app, chunks[1]);
	} else if app.mode == Mode::InstallDeps {
		dep_tree::draw_install_deps(f, app, chunks[1]);
	} else {
		draw_dual_pane(f, app, chunks[1]);
	}

	if status_height > 0 {
		draw_status_bar(f, app, chunks[2]);
	}

	match app.mode {
		Mode::EditEnv => detail::draw_env_popup(f, app),
		Mode::EditVersion => detail::draw_version_popup(f, app),
		Mode::EditDescription => detail::draw_description_popup(f, app),
		Mode::EditCategories => detail::draw_categories_popup(f, app),
		Mode::RemoveConfirm => detail::draw_remove_popup(f, app),
		Mode::UpdateCheck => detail::draw_update_popup(f, app),
		Mode::InstallProgress => dep_tree::draw_install_progress_popup(f, app),
		_ => {}
	}

	help::draw_footer(f, app, chunks[3]);

	if app.show_help {
		help_overlay::draw_help_overlay(f, app);
	}
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

	modlist::draw_mod_list(f, app, chunks[0]);
	detail::draw_detail(f, app, chunks[1]);
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
