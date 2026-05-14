use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::super::app::{Focus, ManageApp, Mode};

pub(super) fn draw_help_overlay(
	f: &mut Frame,
	app: &ManageApp,
) {
	let lines = build_help_lines(app);
	let overlay_height =
		(lines.len() as u16 + 2).min(f.area().height.saturating_sub(4));
	let area = centered_rect(70, overlay_height, f.area());
	f.render_widget(Clear, area);

	let paragraph = Paragraph::new(lines)
		.block(
			Block::default()
				.title(Span::styled(
					" Keybindings ",
					Style::default()
						.fg(Color::Cyan)
						.add_modifier(Modifier::BOLD),
				))
				.borders(Borders::ALL)
				.border_style(Style::default().fg(Color::Cyan)),
		)
		.wrap(Wrap { trim: false });

	f.render_widget(paragraph, area);
}

fn build_help_lines(app: &ManageApp) -> Vec<Line<'static>> {
	let key_style = Style::default()
		.fg(Color::Yellow)
		.add_modifier(Modifier::BOLD);
	let desc_style = Style::default().fg(Color::White);
	let dim_style = Style::default().fg(Color::DarkGray);
	let section_style = Style::default()
		.fg(Color::Cyan)
		.add_modifier(Modifier::BOLD);

	let mut lines = vec![
		Line::from(Span::styled("Global", section_style)),
		key_value_line("F1", "Toggle this help", key_style, desc_style),
		key_value_line("Esc", "Back / Quit / Cancel", key_style, desc_style),
		Line::raw(""),
	];

	match app.mode {
		Mode::Normal => match app.focus {
			Focus::ModList => {
				lines.push(Line::from(Span::styled("Mod List", section_style)));
				lines.push(key_value_line(
					"Up/Down", "Navigate", key_style, desc_style,
				));
				lines.push(key_value_line(
					"Tab",
					"Cycle category filter",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"Enter/Right",
					"Switch to detail",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"Backspace",
					"Delete search char",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"type",
					"Filter mods by name",
					key_style,
					dim_style,
				));
			}
			Focus::Detail => {
				lines.push(Line::from(Span::styled(
					"Detail Pane",
					section_style,
				)));
				lines.push(key_value_line(
					"Up/Down",
					"Move between fields",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"Enter",
					"Activate selected field",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"Tab/Left",
					"Back to mod list",
					key_style,
					desc_style,
				));
				lines.push(Line::raw(""));
				lines.push(Line::from(Span::styled(
					"Quick Shortcuts",
					section_style,
				)));
				lines.push(key_value_line(
					"v", "Version", key_style, desc_style,
				));
				lines.push(key_value_line(
					"e",
					"Environment",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"d",
					"Description",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"c",
					"Categories",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"t",
					"Dependency tree",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"r",
					"Remove mod",
					key_style,
					desc_style,
				));
				lines.push(key_value_line(
					"u",
					"Check for updates",
					key_style,
					desc_style,
				));
			}
		},
		Mode::EditEnv | Mode::EditVersion => {
			lines.push(key_value_line(
				"Up/Down", "Select", key_style, desc_style,
			));
			lines.push(key_value_line(
				"Enter", "Confirm", key_style, desc_style,
			));
			lines.push(key_value_line("Esc", "Cancel", key_style, desc_style));
		}
		Mode::EditDescription => {
			lines.push(key_value_line(
				"Esc",
				"Save and close",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"Ctrl+Shift+V",
				"Paste from clipboard",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"Ctrl+Shift+C",
				"Copy to clipboard",
				key_style,
				desc_style,
			));
		}
		Mode::EditCategories => {
			lines.push(key_value_line(
				"Up/Down/Enter",
				"Toggle category",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"n",
				"New category",
				key_style,
				desc_style,
			));
			lines.push(key_value_line("Esc", "Done", key_style, desc_style));
		}
		Mode::DepTree => {
			lines.push(key_value_line(
				"Up/Down", "Scroll", key_style, desc_style,
			));
			lines.push(key_value_line(
				"i",
				"Install missing deps",
				key_style,
				desc_style,
			));
			lines.push(key_value_line("Esc", "Back", key_style, desc_style));
		}
		Mode::InstallDeps => {
			lines.push(key_value_line(
				"Up/Down", "Navigate", key_style, desc_style,
			));
			lines.push(key_value_line(
				"Space",
				"Toggle mark",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"a",
				"Toggle all",
				key_style,
				desc_style,
			));
			lines.push(key_value_line(
				"Enter",
				"Install marked",
				key_style,
				desc_style,
			));
			lines.push(key_value_line("Esc", "Back", key_style, desc_style));
		}
		Mode::InstallProgress => {
			lines.push(key_value_line(
				"Up/Down", "Scroll", key_style, desc_style,
			));
			lines.push(key_value_line(
				"Esc/Enter",
				"Close",
				key_style,
				desc_style,
			));
		}
		Mode::RemoveConfirm => {
			lines.push(key_value_line(
				"Enter",
				"Confirm remove",
				key_style,
				desc_style,
			));
			lines.push(key_value_line("Esc", "Cancel", key_style, desc_style));
		}
		Mode::UpdateCheck => {
			lines.push(key_value_line(
				"Enter",
				"Apply update",
				key_style,
				desc_style,
			));
			lines.push(key_value_line("Esc", "Cancel", key_style, desc_style));
		}
	}

	lines.push(Line::raw(""));
	lines.push(Line::from(Span::styled(
		"Press Esc or Enter to close",
		dim_style,
	)));

	lines
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
