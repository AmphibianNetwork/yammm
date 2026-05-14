use std::io;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
	EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
	enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::matrix::{LaunchSide, Loader, TestCase};

struct App {
	tests: Vec<(TestCase, bool)>,
	state: ListState,
	filter: Option<Loader>,
	#[allow(dead_code)]
	side: LaunchSide,
}

impl App {
	fn new(
		tests: Vec<TestCase>,
		side: LaunchSide,
	) -> Self {
		let items: Vec<_> = tests.into_iter().map(|t| (t, true)).collect();
		let mut state = ListState::default();
		if !items.is_empty() {
			state.select(Some(0));
		}
		Self {
			tests: items,
			state,
			filter: None,
			side,
		}
	}

	fn toggle(&mut self) {
		if let Some(i) = self.state.selected() {
			self.tests[i].1 = !self.tests[i].1;
		}
	}

	fn select_all(&mut self) {
		for item in &mut self.tests {
			item.1 = true;
		}
	}

	fn deselect_all(&mut self) {
		for item in &mut self.tests {
			item.1 = false;
		}
	}

	fn set_filter(
		&mut self,
		loader: Option<Loader>,
	) {
		self.filter = loader;
	}

	fn move_up(&mut self) {
		if let Some(i) = self.state.selected() {
			if i > 0 {
				self.state.select(Some(i - 1));
			}
		}
	}

	fn move_down(&mut self) {
		if let Some(i) = self.state.selected() {
			if i < self.tests.len() - 1 {
				self.state.select(Some(i + 1));
			}
		}
	}

	fn selected_tests(&self) -> Vec<TestCase> {
		self.tests
			.iter()
			.filter(|(_, checked)| *checked)
			.map(|(t, _)| t.clone())
			.collect()
	}

	fn selected_count(&self) -> usize {
		self.tests.iter().filter(|(_, c)| *c).count()
	}

	fn total_count(&self) -> usize {
		self.tests.len()
	}
}

pub fn interactive_select(tests: Vec<TestCase>) -> Result<Vec<TestCase>> {
	enable_raw_mode()?;
	let mut stdout = io::stdout();
	crossterm::execute!(stdout, EnterAlternateScreen)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = Terminal::new(backend)?;

	let app = App::new(tests, LaunchSide::Both);
	let result = run_app(&mut terminal, app);

	disable_raw_mode()?;
	crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
	terminal.show_cursor()?;

	result
}

fn run_app(
	terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
	mut app: App,
) -> Result<Vec<TestCase>> {
	loop {
		terminal.draw(|f| {
			let chunks = Layout::vertical([
				Constraint::Length(3),
				Constraint::Min(1),
				Constraint::Length(3),
			])
			.split(f.area());

			let title = Paragraph::new(Line::from(vec![
				Span::styled(
					" yammm-e2e ",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" Select tests to run "),
			]))
			.block(Block::default().borders(Borders::BOTTOM));
			f.render_widget(title, chunks[0]);

			let items: Vec<ListItem> = app
				.tests
				.iter()
				.map(|(test, checked)| {
					let check = if *checked { "☑" } else { "☐" };
					let loader_style = match test.loader {
						Loader::Fabric => {
							Style::default().fg(ratatui::style::Color::Cyan)
						}
						Loader::Forge => {
							Style::default().fg(ratatui::style::Color::Red)
						}
						Loader::NeoForge => {
							Style::default().fg(ratatui::style::Color::Magenta)
						}
						Loader::Quilt => Style::default()
							.fg(ratatui::style::Color::Rgb(128, 0, 128)),
					};

					let known = test
						.known_issue
						.map_or(String::new(), |i| format!("  [known: {i}]"));

					let line = Line::from(vec![
						Span::styled(
							format!("{check} "),
							Style::default().add_modifier(Modifier::BOLD),
						),
						Span::styled(
							format!("{:<8}", test.mc_version),
							Style::default(),
						),
						Span::styled(
							format!("{:<10}", test.loader),
							loader_style,
						),
						Span::styled(
							format!("java {}{}", test.min_java, known),
							Style::default().add_modifier(Modifier::DIM),
						),
					]);
					ListItem::new(line)
				})
				.collect();

			let list = List::new(items)
				.block(Block::default().borders(Borders::NONE).title(format!(
					"  {} / {} selected",
					app.selected_count(),
					app.total_count()
				)))
				.highlight_style(
					Style::default().add_modifier(Modifier::REVERSED),
				);
			f.render_stateful_widget(list, chunks[1], &mut app.state);

			let help = Paragraph::new(Line::from(vec![
				Span::styled(
					" ↑/↓",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" navigate"),
				Span::raw("  "),
				Span::styled(
					"Space",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" toggle"),
				Span::raw("  "),
				Span::styled(
					"a",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" all"),
				Span::raw("  "),
				Span::styled(
					"n",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" none"),
				Span::raw("  "),
				Span::styled(
					"1-4",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" filter loader"),
				Span::raw("  "),
				Span::styled(
					"0",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" clear filter"),
				Span::raw("  "),
				Span::styled(
					"Enter",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" run"),
				Span::raw("  "),
				Span::styled(
					"q",
					Style::default().add_modifier(Modifier::BOLD),
				),
				Span::raw(" quit"),
			]))
			.block(Block::default().borders(Borders::TOP));
			f.render_widget(help, chunks[2]);
		})?;

		if event::poll(std::time::Duration::from_millis(100))? {
			if let Event::Key(key) = event::read()? {
				if key.kind != KeyEventKind::Press {
					continue;
				}
				match key.code {
					KeyCode::Up => app.move_up(),
					KeyCode::Down => app.move_down(),
					KeyCode::Char(' ') => app.toggle(),
					KeyCode::Char('a') => app.select_all(),
					KeyCode::Char('n') => app.deselect_all(),
					KeyCode::Char('1') => {
						app.set_filter(Some(Loader::Fabric));
						app.select_all();
					}
					KeyCode::Char('2') => {
						app.set_filter(Some(Loader::Forge));
						app.select_all();
					}
					KeyCode::Char('3') => {
						app.set_filter(Some(Loader::NeoForge));
						app.select_all();
					}
					KeyCode::Char('4') => {
						app.set_filter(Some(Loader::Quilt));
						app.select_all();
					}
					KeyCode::Char('0') => {
						app.set_filter(None);
					}
					KeyCode::Enter => {
						return Ok(app.selected_tests());
					}
					KeyCode::Char('q') | KeyCode::Esc => {
						return Ok(Vec::new());
					}
					_ => {}
				}
			}
		}
	}
}
