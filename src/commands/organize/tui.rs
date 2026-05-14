use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
	EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
	enable_raw_mode,
};
use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
	Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use std::io;

#[cfg(feature = "syntax-highlight")]
use syntect::highlighting::{FontStyle, Highlighter};

use super::app::{Mode, OrganizeApp};
use super::{OrganizeResult, OrphanConfig, Side, assign_config};

pub fn run_tui(
	orphan_configs: &[OrphanConfig],
	mod_names: &[String],
	side: Side,
	root_dir: &std::path::Path,
) -> Result<OrganizeResult> {
	enable_raw_mode()?;
	let mut stdout = io::stdout();
	crossterm::execute!(stdout, EnterAlternateScreen)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = ratatui::Terminal::new(backend)?;

	let mut app =
		OrganizeApp::new(orphan_configs.to_vec(), mod_names.to_vec(), side);

	let result = run_app(&mut terminal, &mut app, root_dir);

	disable_raw_mode()?;
	crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
	terminal.show_cursor()?;

	result?;

	Ok(OrganizeResult {
		assigned: app.result.assigned,
		ignored_count: app.result.ignored_count,
		skipped_count: app.result.skipped_count,
		ignored_new: app.result.ignored_new.unwrap_or_default(),
	})
}

fn run_app(
	terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
	app: &mut OrganizeApp,
	root_dir: &std::path::Path,
) -> Result<()> {
	loop {
		terminal.draw(|f| draw(f, app))?;

		if event::poll(std::time::Duration::from_millis(100))?
			&& let Event::Key(key) = event::read()?
			&& key.kind == KeyEventKind::Press
			&& !handle_key(app, key, root_dir)?
		{
			return Ok(());
		}
	}
}

fn handle_key(
	app: &mut OrganizeApp,
	key: crossterm::event::KeyEvent,
	root_dir: &std::path::Path,
) -> Result<bool> {
	if key.code == KeyCode::F(1) {
		app.show_help = !app.show_help;
		return Ok(true);
	}
	if app.show_help {
		if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
			app.show_help = false;
		}
		return Ok(true);
	}

	match app.mode {
		Mode::ModSelect => match key.code {
			KeyCode::Char('q') => {
				if app.query.is_empty() {
					app.result.skipped_count +=
						app.orphan_configs.len() - app.current_idx;
					return Ok(false);
				}
				app.query.push('q');
				app.update_filter();
			}
			KeyCode::Char(c) => {
				app.query.push(c);
				app.update_filter();
			}
			KeyCode::Backspace => {
				app.query.pop();
				app.update_filter();
			}
			KeyCode::Up if app.mod_list_state.selected > 0 => {
				app.mod_list_state.selected -= 1;
			}
			KeyCode::Down
				if app.mod_list_state.selected
					< app
						.mod_list_state
						.filtered_indices
						.len()
						.saturating_sub(1) =>
			{
				app.mod_list_state.selected += 1;
			}
			KeyCode::Enter
				if !app.mod_list_state.filtered_indices.is_empty() =>
			{
				app.select_mod();
			}
			KeyCode::Esc => {
				app.skip_config();
				if app.current_idx >= app.orphan_configs.len() {
					return Ok(false);
				}
			}
			_ => {}
		},
		Mode::DestSelect => match key.code {
			KeyCode::Up if app.dest_list_state.selected > 0 => {
				app.dest_list_state.selected -= 1;
			}
			KeyCode::Down => {
				let dest_count = app.dest_options().len();
				if app.dest_list_state.selected < dest_count.saturating_sub(1) {
					app.dest_list_state.selected += 1;
				}
			}
			KeyCode::Enter => {
				let dest = app.dest_options();
				let selected = dest.get(app.dest_list_state.selected);
				if let Some(dest) = selected {
					if dest.idx == 3 {
						if let Some(config) = app.current_config() {
							let relative = config.rel_path.clone();
							if app.result.ignored_new.is_none() {
								app.result.ignored_new = Some(Vec::new());
							}
							app.result
								.ignored_new
								.as_mut()
								.unwrap()
								.push(relative);
							app.result.ignored_count += 1;
						}
					} else if let Some(config) = app.current_config() {
						let config = config.clone();
						let mod_id = app.selected_mod_name();
						match assign_config(
							&config, &mod_id, dest.idx, app.side, root_dir,
						) {
							Ok(key) => {
								*app.result.assigned.entry(key).or_insert(0) +=
									1;
							}
							Err(e) => {
								crate::output::error(format!(
									"Failed to assign config: {}",
									e
								));
							}
						}
					}
				}
				if !app.advance() {
					return Ok(false);
				}
			}
			KeyCode::Esc => {
				app.cancel_dest();
			}
			_ => {}
		},
	}
	Ok(true)
}

fn draw(
	f: &mut Frame,
	app: &mut OrganizeApp,
) {
	let size = f.area();
	f.render_widget(Clear, size);

	let chunks = Layout::default()
		.direction(Direction::Vertical)
		.constraints([
			Constraint::Length(3),
			Constraint::Min(0),
			Constraint::Length(1),
		])
		.split(size);

	draw_header(f, app, chunks[0]);
	draw_body(f, app, chunks[1]);
	draw_footer(f, app, chunks[2]);

	if app.mode == Mode::DestSelect {
		draw_dest_popup(f, app);
	}

	if app.show_help {
		draw_help_overlay(f, app);
	}
}

fn draw_header(
	f: &mut Frame,
	app: &OrganizeApp,
	area: Rect,
) {
	let total = app.total_configs();
	let current = (app.current_idx + 1).min(total);

	let config = app.current_config();
	let file_name = config.map(|c| c.file_name.as_str()).unwrap_or("none");

	let title = Span::styled(
		format!(" Organize Configs ({}/{}) — {} ", current, total, file_name),
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
		Span::styled("Filter: ", Style::default().fg(Color::Yellow));
	let query_text = Span::styled(
		if app.query.is_empty() {
			"type to search...".to_string()
		} else {
			app.query.clone()
		},
		if app.query.is_empty() {
			Style::default().fg(Color::DarkGray)
		} else {
			Style::default().fg(Color::White)
		},
	);

	let line = Line::from(vec![query_label, query_text]);
	let paragraph = Paragraph::new(line);
	f.render_widget(paragraph, inner);
}

fn draw_body(
	f: &mut Frame,
	app: &mut OrganizeApp,
	area: Rect,
) {
	let chunks = Layout::default()
		.direction(Direction::Horizontal)
		.constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
		.split(area);

	draw_mod_list(f, app, chunks[0]);
	draw_config_preview(f, app, chunks[1]);
}

fn draw_mod_list(
	f: &mut Frame,
	app: &mut OrganizeApp,
	area: Rect,
) {
	let items: Vec<ListItem> = app
		.mod_list_state
		.filtered_indices
		.iter()
		.map(|&idx| {
			let name = &app.mod_names[idx];
			ListItem::new(Line::from(Span::styled(
				name.clone(),
				Style::default(),
			)))
		})
		.collect();

	let list = List::new(items)
		.block(
			Block::default()
				.title(Span::styled(
					format!(" Mods ({}) ", app.mod_names.len()),
					Style::default()
						.fg(Color::Green)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::DarkGray)),
		)
		.highlight_style(
			Style::default()
				.bg(Color::DarkGray)
				.add_modifier(Modifier::BOLD),
		)
		.highlight_symbol("▶ ");

	let mut state = ListState::default();
	state.select(Some(app.mod_list_state.selected));
	f.render_stateful_widget(list, area, &mut state);
}

fn draw_config_preview(
	f: &mut Frame,
	app: &OrganizeApp,
	area: Rect,
) {
	let config = app.current_config();
	let lines = match config {
		Some(c) => highlight_content(app, &c.content, &c.file_name),
		None => vec![Line::from(Span::styled(
			"No more configs to organize",
			Style::default().fg(Color::DarkGray),
		))],
	};

	let title = config.map(|c| c.file_name.as_str()).unwrap_or("Preview");

	let paragraph = Paragraph::new(lines)
		.block(
			Block::default()
				.title(Span::styled(
					format!(" {} ", title),
					Style::default()
						.fg(Color::Magenta)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::DarkGray)),
		)
		.wrap(Wrap { trim: false })
		.scroll((0, 0));

	f.render_widget(paragraph, area);
}

#[cfg(feature = "syntax-highlight")]
fn highlight_content(
	app: &OrganizeApp,
	content: &str,
	file_name: &str,
) -> Vec<Line<'static>> {
	let syntax = app
		.syntax_set
		.find_syntax_for_file(std::path::Path::new(file_name))
		.unwrap_or_else(|_| Some(app.syntax_set.find_syntax_plain_text()))
		.unwrap_or_else(|| app.syntax_set.find_syntax_plain_text());

	let highlighter =
		Highlighter::new(&app.theme_set.themes["base16-ocean.dark"]);
	let mut highlight_state = syntect::highlighting::HighlightState::new(
		&highlighter,
		syntect::parsing::ScopeStack::new(),
	);
	let mut parse_state = syntect::parsing::ParseState::new(syntax);

	let mut lines = Vec::new();
	for line in syntect::util::LinesWithEndings::from(content) {
		let ops = parse_state.parse_line(line, &app.syntax_set);
		let ops = match ops {
			Ok(o) => o,
			Err(_) => continue,
		};
		let ranges = syntect::highlighting::RangedHighlightIterator::new(
			&mut highlight_state,
			&ops,
			line,
			&highlighter,
		);
		let mut spans: Vec<Span<'_>> = Vec::new();
		for (style, text, _range) in ranges {
			let fg = syntect_to_ratatui_color(style.foreground);
			let mut rat_style = Style::default().fg(fg);
			if style.font_style.contains(FontStyle::BOLD) {
				rat_style = rat_style.add_modifier(Modifier::BOLD);
			}
			if style.font_style.contains(FontStyle::ITALIC) {
				rat_style = rat_style.add_modifier(Modifier::ITALIC);
			}
			if style.font_style.contains(FontStyle::UNDERLINE) {
				rat_style = rat_style.add_modifier(Modifier::UNDERLINED);
			}
			spans.push(Span::styled(text.to_string(), rat_style));
		}
		lines.push(Line::from(spans));
	}

	lines
}

#[cfg(feature = "syntax-highlight")]
fn syntect_to_ratatui_color(color: syntect::highlighting::Color) -> Color {
	Color::Rgb(color.r, color.g, color.b)
}

#[cfg(not(feature = "syntax-highlight"))]
fn highlight_content(
	_app: &OrganizeApp,
	content: &str,
	_file_name: &str,
) -> Vec<Line<'static>> {
	content
		.lines()
		.map(|line| {
			Line::from(Span::styled(line.to_string(), Style::default()))
		})
		.collect()
}

fn draw_footer(
	f: &mut Frame,
	app: &OrganizeApp,
	area: Rect,
) {
	let help = match app.mode {
		Mode::ModSelect => {
			"F1: help │ ↑↓ navigate │ Enter: select mod │ Esc: skip │ q: quit"
		}
		Mode::DestSelect => {
			"F1: help │ ↑↓ navigate │ Enter: confirm │ Esc: back"
		}
	};

	let style = Style::default().fg(Color::DarkGray);
	let paragraph = Paragraph::new(Span::styled(help, style));
	f.render_widget(paragraph, area);
}

fn draw_help_overlay(
	f: &mut Frame,
	app: &OrganizeApp,
) {
	let key_style = Style::default()
		.fg(Color::Yellow)
		.add_modifier(Modifier::BOLD);
	let desc_style = Style::default().fg(Color::White);
	let dim_style = Style::default().fg(Color::DarkGray);
	let section_style = Style::default()
		.fg(Color::Cyan)
		.add_modifier(Modifier::BOLD);

	let mut lines = Vec::new();

	match app.mode {
		Mode::ModSelect => {
			lines.push(Line::from(Span::styled("Mod Select", section_style)));
			lines.push(key_value_line(
				"Up/Down",
				"Navigate filtered list",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"Enter",
				"Select mod",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"Esc",
				"Skip config",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"q",
				"Quit (when search empty)",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"type",
				"Filter mods by name",
				key_style,
				dim_style,
			));
			lines.push(key_value_line(
				"Backspace",
				"Delete search char",
				key_style,
				desc_style,
			));
		}
		Mode::DestSelect => {
			lines.push(Line::from(Span::styled(
				"Destination Select",
				section_style,
			)));
			lines.push(key_value_line(
				"Up/Down",
				"Navigate options",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"Enter",
				"Confirm assignment",
				key_style,
				desc_style,
			));
			lines.push(key_value_line("Esc", "Back", key_style, desc_style));
		}
	}

	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(
		"Press Esc or Enter to close",
		dim_style,
	)));

	let overlay_height =
		(lines.len() as u16 + 2).min(f.area().height.saturating_sub(4));
	let area = centered_rect(60, overlay_height, f.area());
	f.render_widget(Clear, area);

	let paragraph = Paragraph::new(lines).block(
		Block::default()
			.title(Span::styled(" Keybindings ", section_style))
			.borders(Borders::ALL)
			.border_style(Style::default().fg(Color::Cyan)),
	);

	f.render_widget(paragraph, area);
}

fn key_value_line<'a>(
	key: &'a str,
	desc: &'a str,
	key_style: Style,
	desc_style: Style,
) -> Line<'a> {
	Line::from(vec![
		Span::styled(format!("  {:<18}", key), key_style),
		Span::styled(desc, desc_style),
	])
}

fn draw_dest_popup(
	f: &mut Frame,
	app: &mut OrganizeApp,
) {
	let dest_options = app.dest_options();
	let items: Vec<ListItem> = dest_options
		.iter()
		.map(|d| {
			ListItem::new(Line::from(Span::styled(&d.label, Style::default())))
		})
		.collect();

	let popup_height = (dest_options.len() + 2) as u16 + 2;
	let area = centered_rect(60, popup_height, f.area());

	let list = List::new(items)
		.block(
			Block::default()
				.title(Span::styled(
					format!(" Assign to: {} ", app.selected_mod_name()),
					Style::default()
						.fg(Color::Yellow)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::Yellow)),
		)
		.highlight_style(
			Style::default()
				.bg(Color::DarkGray)
				.add_modifier(Modifier::BOLD),
		)
		.highlight_symbol("▶ ");

	f.render_widget(Clear, area);

	let mut state = ListState::default();
	state.select(Some(app.dest_list_state.selected));
	f.render_stateful_widget(list, area, &mut state);
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
