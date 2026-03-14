use anyhow::Result;
use crossterm::event::{
	self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::terminal::{
	disable_raw_mode, enable_raw_mode, EnterAlternateScreen,
	LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use std::io;
use tokio::sync::mpsc;

use crate::providers::SourceRegistry;
use crate::storage::Storage;
use crate::types::VersionFilters;

use super::app::{
	AsyncOp, AsyncResult, DetailField, Focus, ManageApp, Mode, StatusKind,
};
use super::render;

pub fn run_tui(
	storage: &Storage,
	registry: &SourceRegistry,
	mods: Vec<crate::types::TrackedMod>,
	modpack_name: String,
	modpack_version: String,
	modpack_loader: String,
	filters: VersionFilters,
) -> Result<()> {
	enable_raw_mode()?;
	let mut stdout = io::stdout();
	crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
	let backend = CrosstermBackend::new(stdout);
	let mut terminal = ratatui::Terminal::new(backend)?;

	let mut app =
		ManageApp::new(mods, modpack_name, modpack_version, modpack_loader);

	let (async_tx, mut async_rx) = mpsc::channel::<AsyncResult>(16);

	let result = run_app(
		&mut terminal,
		&mut app,
		storage,
		registry,
		&filters,
		&mut async_rx,
		&async_tx,
	);

	disable_raw_mode()?;
	crossterm::execute!(
		terminal.backend_mut(),
		LeaveAlternateScreen,
		DisableMouseCapture
	)?;
	terminal.show_cursor()?;

	result
}

fn run_app(
	terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
	app: &mut ManageApp,
	storage: &Storage,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	async_rx: &mut mpsc::Receiver<AsyncResult>,
	async_tx: &mpsc::Sender<AsyncResult>,
) -> Result<()> {
	loop {
		terminal.draw(|f| render::draw(f, app))?;

		app.tick_status();

		if event::poll(std::time::Duration::from_millis(50))? {
			match event::read()? {
				Event::Key(key)
					if key.kind == KeyEventKind::Press
						&& !handle_key(
							app, key, storage, registry, filters, async_tx,
						)? =>
				{
					return Ok(());
				}
				Event::Mouse(mouse) if app.mode == Mode::EditDescription => {
					if let Some(ref mut textarea) = app.description_textarea {
						let _ = textarea.input(mouse);
					}
				}
				_ => {}
			}
		}

		while let Ok(result) = async_rx.try_recv() {
			handle_async_result(app, result);
		}
	}
}

fn handle_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	async_tx: &mpsc::Sender<AsyncResult>,
) -> Result<bool> {
	match app.mode {
		Mode::Normal => {
			handle_normal_key(app, key, storage, registry, filters, async_tx)
		}
		Mode::EditEnv => handle_env_key(app, key, storage),
		Mode::EditVersion => handle_version_key(app, key, storage),
		Mode::EditDescription => handle_description_key(app, key, storage),
		Mode::EditCategories => handle_categories_key(app, key, storage),
		Mode::DepTree => handle_dep_tree_key(app, key),
		Mode::RemoveConfirm => handle_remove_key(app, key, storage),
		Mode::UpdateCheck => handle_update_key(app, key, storage),
	}
}

fn handle_normal_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	async_tx: &mpsc::Sender<AsyncResult>,
) -> Result<bool> {
	match app.focus {
		Focus::ModList => handle_mod_list_key(app, key),
		Focus::Detail => handle_detail_focus_key(
			app, key, storage, registry, filters, async_tx,
		),
	}
}

fn handle_mod_list_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
) -> Result<bool> {
	match key.code {
		KeyCode::Esc => {
			app.should_quit = true;
			return Ok(false);
		}
		KeyCode::Tab => app.cycle_category_filter(),
		KeyCode::Up => app.mod_list_move_up(),
		KeyCode::Down => app.mod_list_move_down(),
		KeyCode::Enter => app.switch_to_detail(),
		KeyCode::Right => app.switch_to_detail(),
		KeyCode::Backspace => {
			app.search_pop_char();
		}
		KeyCode::Char(c) => app.search_push_char(c),
		_ => {}
	}
	Ok(true)
}

fn handle_detail_focus_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	async_tx: &mpsc::Sender<AsyncResult>,
) -> Result<bool> {
	match key.code {
		KeyCode::Up => app.detail_move_up(),
		KeyCode::Down => app.detail_move_down(),
		KeyCode::Enter => {
			activate_selected_field(app, storage, registry, filters, async_tx)
		}
		KeyCode::Tab | KeyCode::Left => app.switch_to_mod_list(),
		KeyCode::Esc => app.switch_to_mod_list(),
		KeyCode::Char('v') => {
			app.activate_field(DetailField::Version);
			app.start_edit_version();
			spawn_fetch_versions(app, registry, filters, async_tx);
		}
		KeyCode::Char('e') => {
			app.activate_field(DetailField::Env);
			app.start_edit_env();
		}
		KeyCode::Char('d') => {
			app.activate_field(DetailField::Description);
			app.start_edit_description();
		}
		KeyCode::Char('c') => {
			app.activate_field(DetailField::Categories);
			app.start_edit_categories();
		}
		KeyCode::Char('t') => {
			app.activate_field(DetailField::DepTree);
			app.start_dep_tree(storage);
		}
		KeyCode::Char('r') => {
			app.activate_field(DetailField::Remove);
			app.start_remove(storage);
		}
		KeyCode::Char('u') => {
			app.activate_field(DetailField::UpdateCheck);
			app.start_update_check();
			spawn_update_check(app, registry, filters, async_tx);
		}
		_ => {}
	}
	Ok(true)
}

fn activate_selected_field(
	app: &mut ManageApp,
	storage: &Storage,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	async_tx: &mpsc::Sender<AsyncResult>,
) {
	match app.selected_field() {
		DetailField::Version => {
			app.start_edit_version();
			spawn_fetch_versions(app, registry, filters, async_tx);
		}
		DetailField::Env => app.start_edit_env(),
		DetailField::Description => app.start_edit_description(),
		DetailField::Categories => app.start_edit_categories(),
		DetailField::DepTree => app.start_dep_tree(storage),
		DetailField::Remove => app.start_remove(storage),
		DetailField::UpdateCheck => {
			app.start_update_check();
			spawn_update_check(app, registry, filters, async_tx);
		}
	}
}

fn handle_env_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
) -> Result<bool> {
	match key.code {
		KeyCode::Up if app.env_picker_selected > 0 => {
			app.env_picker_selected -= 1;
		}
		KeyCode::Down if app.env_picker_selected < 2 => {
			app.env_picker_selected += 1;
		}
		KeyCode::Enter => app.confirm_edit_env(storage),
		KeyCode::Esc => app.mode = Mode::Normal,
		_ => {}
	}
	Ok(true)
}

fn handle_version_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
) -> Result<bool> {
	if app.version_loading {
		return Ok(true);
	}
	match key.code {
		KeyCode::Up if app.version_list_selected > 0 => {
			app.version_list_selected -= 1;
		}
		KeyCode::Down
			if app.version_list_selected
				< app.version_list.len().saturating_sub(1) =>
		{
			app.version_list_selected += 1;
		}
		KeyCode::Enter => app.confirm_edit_version(storage),
		KeyCode::Esc => {
			app.mode = Mode::Normal;
			app.version_loading = false;
			app.pending_async = None;
		}
		_ => {}
	}
	Ok(true)
}

fn handle_description_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
) -> Result<bool> {
	if key.code == KeyCode::Esc {
		app.confirm_edit_description(storage);
		return Ok(true);
	}

	if let Some(ref mut textarea) = app.description_textarea {
		if key.modifiers.contains(
			crossterm::event::KeyModifiers::CONTROL
				| crossterm::event::KeyModifiers::SHIFT,
		) && key.code == KeyCode::Char('V')
		{
			if let Ok(mut cb) = arboard::Clipboard::new() {
				if let Ok(text) = cb.get_text() {
					textarea.insert_str(text);
				}
			}
			return Ok(true);
		}
		if key.modifiers.contains(
			crossterm::event::KeyModifiers::CONTROL
				| crossterm::event::KeyModifiers::SHIFT,
		) && key.code == KeyCode::Char('C')
		{
			textarea.copy();
			let text = textarea.yank_text();
			if !text.is_empty() {
				if let Ok(mut cb) = arboard::Clipboard::new() {
					let _ = cb.set_text(text);
				}
			}
			return Ok(true);
		}
		let _ = textarea.input(key);
	}

	Ok(true)
}

fn handle_categories_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
) -> Result<bool> {
	if app.category_new_mode {
		match key.code {
			KeyCode::Esc => {
				app.category_new_mode = false;
				app.category_new_input.clear();
			}
			KeyCode::Enter => {
				app.add_new_category(&app.category_new_input.clone());
				app.category_new_mode = false;
				app.category_new_input.clear();
			}
			KeyCode::Backspace => {
				app.category_new_input.pop();
			}
			KeyCode::Char(c) => {
				app.category_new_input.push(c);
			}
			_ => {}
		}
		return Ok(true);
	}

	let total_items = app.all_categories.len() + 1;
	match key.code {
		KeyCode::Up if app.category_picker_selected > 0 => {
			app.category_picker_selected -= 1;
		}
		KeyCode::Down
			if app.category_picker_selected < total_items.saturating_sub(1) =>
		{
			app.category_picker_selected += 1;
		}
		KeyCode::Enter => {
			if app.category_picker_selected < app.all_categories.len() {
				let cat =
					app.all_categories[app.category_picker_selected].clone();
				app.toggle_category_for_selected(&cat);
			} else {
				app.category_new_mode = true;
				app.category_new_input.clear();
			}
		}
		KeyCode::Char('n') => {
			app.category_new_mode = true;
			app.category_new_input.clear();
		}
		KeyCode::Esc => app.confirm_edit_categories(storage),
		_ => {}
	}
	Ok(true)
}

fn handle_dep_tree_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
) -> Result<bool> {
	if key.code == KeyCode::Esc {
		app.mode = Mode::Normal;
	}
	Ok(true)
}

fn handle_remove_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
) -> Result<bool> {
	match key.code {
		KeyCode::Enter => {
			app.confirm_remove(storage);
		}
		KeyCode::Esc => app.mode = Mode::Normal,
		_ => {}
	}
	Ok(true)
}

fn handle_update_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
) -> Result<bool> {
	match key.code {
		KeyCode::Enter => {
			if app.update_result.is_some() {
				app.apply_update(storage);
			} else {
				app.mode = Mode::Normal;
			}
		}
		KeyCode::Esc => {
			app.mode = Mode::Normal;
			app.update_loading = false;
			app.pending_async = None;
		}
		_ => {}
	}
	Ok(true)
}

fn spawn_fetch_versions(
	app: &ManageApp,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	tx: &mpsc::Sender<AsyncResult>,
) {
	if app.pending_async != Some(AsyncOp::FetchVersions) {
		return;
	}
	let m = match app.selected_mod() {
		Some(m) => m,
		None => return,
	};

	let source_id = m.source.source_id().to_string();
	let source = m.source.clone();
	let filters = filters.clone();
	let provider = match registry.get(&source) {
		Ok(p) => p.clone(),
		Err(_) => return,
	};
	let tx = tx.clone();

	tokio::spawn(async move {
		let result = provider.get_versions(&source_id, &filters).await;
		let mapped = result.map_err(|e| e.to_string());
		let _ = tx.send(AsyncResult::Versions(mapped)).await;
	});
}

fn spawn_update_check(
	app: &ManageApp,
	registry: &SourceRegistry,
	filters: &VersionFilters,
	tx: &mpsc::Sender<AsyncResult>,
) {
	if app.pending_async != Some(AsyncOp::CheckUpdate) {
		return;
	}
	let m = match app.selected_mod() {
		Some(m) => m,
		None => return,
	};

	let source_id = m.source.source_id().to_string();
	let current_version = m.version.clone();
	let id = m.id.clone();
	let name = m.name.clone();
	let source = m.source.clone();
	let filters = filters.clone();
	let provider = match registry.get(&source) {
		Ok(p) => p.clone(),
		Err(_) => return,
	};
	let tx = tx.clone();

	tokio::spawn(async move {
		let result = provider.get_latest_version(&source_id, &filters).await;
		match result {
			Ok(latest) => {
				if latest.version != current_version {
					let update = crate::commands::update::ModUpdate {
						id,
						name,
						current_version,
						latest_version: latest.version,
						download_url: latest.download_url,
						hash: latest.hash,
						hash_type: latest.hash_type,
					};
					let _ = tx
						.send(AsyncResult::UpdateCheck(Ok(Some(update))))
						.await;
				} else {
					let _ = tx.send(AsyncResult::UpdateCheck(Ok(None))).await;
				}
			}
			Err(e) => {
				let _ =
					tx.send(AsyncResult::UpdateCheck(Err(e.to_string()))).await;
			}
		}
	});
}

fn handle_async_result(
	app: &mut ManageApp,
	result: AsyncResult,
) {
	match result {
		AsyncResult::Versions(versions_result) => {
			if app.mode == Mode::EditVersion {
				match versions_result {
					Ok(versions) => {
						app.version_list = versions;
						app.version_list_selected = 0;
						app.version_loading = false;
					}
					Err(e) => {
						app.version_loading = false;
						app.pending_async = None;
						app.mode = Mode::Normal;
						app.set_status(
							StatusKind::Error,
							format!("Failed to fetch versions: {}", e),
						);
					}
				}
			}
		}
		AsyncResult::UpdateCheck(update_result) => {
			if app.mode == Mode::UpdateCheck {
				app.update_loading = false;
				match update_result {
					Ok(maybe_update) => {
						app.update_result = maybe_update;
					}
					Err(e) => {
						app.update_error = Some(e);
						app.pending_async = None;
					}
				}
			}
		}
	}
}
