use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use super::app::{Focus, ManageApp, Mode};
use super::{source_color, source_short_tag};

pub(super) fn draw_mod_list(
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
			let line = if m.unresolved {
				Line::from(vec![
					Span::styled(m.name.clone(), style),
					Span::raw(" "),
					Span::styled(
						format!("[{}]", source_short_tag(&m.source)),
						style.fg(source_color(&m.source)),
					),
					Span::raw(" "),
					Span::styled(
						"[!]".to_string(),
						Style::default().fg(Color::Yellow),
					),
				])
			} else {
				Line::from(vec![
					Span::styled(m.name.clone(), style),
					Span::raw(" "),
					Span::styled(
						format!("[{}]", source_short_tag(&m.source)),
						style.fg(source_color(&m.source)),
					),
				])
			};
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
