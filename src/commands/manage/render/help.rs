use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::app::{Focus, ManageApp, Mode};

pub(super) fn draw_footer(
	f: &mut Frame,
	app: &ManageApp,
	area: Rect,
) {
	let help = match app.mode {
		Mode::Normal => match app.focus {
			Focus::ModList => {
				"F1: help │ ↑↓ navigate │ type to filter │ Tab: category │ Enter: detail │ Esc: quit"
			}
			Focus::Detail => {
				"F1: help │ ↑↓/Enter: fields │ Tab: back to list │ v/e/d/c/t/r/u: shortcuts"
			}
		},
		Mode::EditEnv => "↑↓ select │ Enter: confirm │ Esc: cancel",
		Mode::EditVersion => "↑↓ select │ Enter: confirm │ Esc: cancel",
		Mode::EditDescription => {
			"Type to edit │ Esc: save │ Ctrl+A: select all │ Ctrl+C/V/X: copy/paste/cut"
		}
		Mode::EditCategories => "↑↓/Enter: toggle │ n: new │ Esc: done",
		Mode::DepTree => "↑↓ scroll │ i: install missing │ Esc: back",
		Mode::InstallDeps => {
			"↑↓: navigate │ Space: toggle │ a: toggle all │ Enter: install │ Esc: back"
		}
		Mode::InstallProgress => "↑↓: scroll │ Esc/Enter: close",
		Mode::RemoveConfirm => "Enter: confirm │ Esc: cancel",
		Mode::UpdateCheck => "Enter: apply │ Esc: back",
	};

	let style = Style::default().fg(Color::DarkGray);
	let paragraph = Paragraph::new(Span::styled(help, style));
	f.render_widget(paragraph, area);
}
