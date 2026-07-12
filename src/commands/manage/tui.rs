use anyhow::Result;
use crossterm::event::{
	self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::terminal::{
	EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
	enable_raw_mode,
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
	registry: std::sync::Arc<SourceRegistry>,
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
	registry: std::sync::Arc<SourceRegistry>,
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
							app, key, storage, &registry, filters, async_tx,
						)? =>
				{
					return Ok(());
				}
				Event::Mouse(mouse) if app.mode == Mode::EditDescription => {
					if let Some(ref mut textarea) = app.description_textarea {
						let _ = textarea.input(mouse);
					}
				}
				Event::Mouse(mouse) => {
					handle_mouse(app, mouse);
				}
				_ => {}
			}
		}

		while let Ok(result) = async_rx.try_recv() {
			handle_async_result(app, result);
		}
	}
}

fn handle_mouse(
	app: &mut ManageApp,
	mouse: crossterm::event::MouseEvent,
) {
	use crossterm::event::MouseEventKind;
	match mouse.kind {
		MouseEventKind::ScrollUp => match app.mode {
			Mode::DepTree => app.dep_tree_move_up(),
			Mode::InstallDeps => app.dep_move_up(),
			Mode::InstallProgress if app.dep_output_scroll > 0 => {
				app.dep_output_scroll -= 1;
			}
			_ => {}
		},
		MouseEventKind::ScrollDown => match app.mode {
			Mode::DepTree => app.dep_tree_move_down(),
			Mode::InstallDeps => app.dep_move_down(),
			Mode::InstallProgress => {
				app.dep_output_scroll += 1;
			}
			_ => {}
		},
		_ => {}
	}
}

fn handle_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
	registry: &std::sync::Arc<SourceRegistry>,
	filters: &VersionFilters,
	async_tx: &mpsc::Sender<AsyncResult>,
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
		Mode::Normal => {
			handle_normal_key(app, key, storage, registry, filters, async_tx)
		}
		Mode::EditEnv => handle_env_key(app, key, storage),
		Mode::EditVersion => handle_version_key(app, key, storage),
		Mode::EditDescription => handle_description_key(app, key, storage),
		Mode::EditCategories => handle_categories_key(app, key, storage),
		Mode::DepTree => handle_dep_tree_key(app, key),
		Mode::InstallDeps => handle_install_deps_key(
			app, key, storage, registry, filters, async_tx,
		),
		Mode::InstallProgress => handle_install_progress_key(app, key),
		Mode::RemoveConfirm => handle_remove_key(app, key, storage),
		Mode::UpdateCheck => handle_update_key(app, key, storage),
	}
}

fn handle_normal_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
	registry: &std::sync::Arc<SourceRegistry>,
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
	registry: &std::sync::Arc<SourceRegistry>,
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
	registry: &std::sync::Arc<SourceRegistry>,
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
			if let Ok(mut cb) = arboard::Clipboard::new()
				&& let Ok(text) = cb.get_text()
			{
				textarea.insert_str(text);
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
			if !text.is_empty()
				&& let Ok(mut cb) = arboard::Clipboard::new()
			{
				let _ = cb.set_text(text);
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
				let input = app.category_new_input.clone();
				app.add_new_category(&input);
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
	match key.code {
		KeyCode::Esc => app.mode = Mode::Normal,
		KeyCode::Up => app.dep_tree_move_up(),
		KeyCode::Down => app.dep_tree_move_down(),
		KeyCode::Char('i') => {
			let has_missing = app.dep_entries.iter().any(|e| !e.installed);
			if has_missing {
				app.start_install_deps();
			}
		}
		_ => {}
	}
	Ok(true)
}

fn handle_install_progress_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
) -> Result<bool> {
	match key.code {
		KeyCode::Esc | KeyCode::Enter => {
			app.mode = Mode::DepTree;
		}
		KeyCode::Up => {
			if app.dep_output_scroll > 0 {
				app.dep_output_scroll -= 1;
			}
		}
		KeyCode::Down => {
			app.dep_output_scroll += 1;
		}
		_ => {}
	}
	Ok(true)
}

fn handle_install_deps_key(
	app: &mut ManageApp,
	key: crossterm::event::KeyEvent,
	storage: &Storage,
	registry: &std::sync::Arc<SourceRegistry>,
	filters: &VersionFilters,
	async_tx: &mpsc::Sender<AsyncResult>,
) -> Result<bool> {
	if app.dep_installing {
		return Ok(true);
	}
	match key.code {
		KeyCode::Up => app.dep_move_up(),
		KeyCode::Down => app.dep_move_down(),
		KeyCode::Char(' ') => app.dep_toggle_mark(),
		KeyCode::Char('a') => app.dep_toggle_all(),
		KeyCode::Enter => {
			if !app.dep_marked.is_empty() {
				app.dep_installing = true;
				app.dep_output = vec!["Installing...".to_string()];
				app.dep_output_scroll = 0;
				app.mode = Mode::InstallProgress;
				app.pending_async = Some(AsyncOp::InstallDeps);
				spawn_install_deps(app, registry, filters, storage, async_tx);
			}
		}
		KeyCode::Esc => app.mode = Mode::DepTree,
		_ => {}
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
	registry: &std::sync::Arc<SourceRegistry>,
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
		if tx.send(AsyncResult::Versions(mapped)).await.is_err() {
			tracing::debug!("TUI receiver dropped before version result");
		};
	});
}

fn spawn_update_check(
	app: &ManageApp,
	registry: &std::sync::Arc<SourceRegistry>,
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
					if tx
						.send(AsyncResult::UpdateCheck(Ok(Some(update))))
						.await
						.is_err()
					{
						tracing::debug!(
							"TUI receiver dropped before update result"
						);
					}
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

fn spawn_install_deps(
	app: &mut ManageApp,
	registry: &std::sync::Arc<SourceRegistry>,
	filters: &VersionFilters,
	storage: &Storage,
	tx: &mpsc::Sender<AsyncResult>,
) {
	if app.pending_async != Some(AsyncOp::InstallDeps) {
		return;
	}

	let marked: Vec<(String, crate::types::ModSource)> = app
		.dep_entries
		.iter()
		.filter(|e| !e.installed && app.dep_marked.contains(&e.mod_id))
		.map(|e| (e.mod_id.clone(), e.source.clone()))
		.collect();

	if marked.is_empty() {
		app.dep_installing = false;
		app.pending_async = None;
		return;
	}

	let registry = registry.clone();
	let mc_version = filters.minecraft_version.clone();
	let loader = filters.loader;

	let root_dir = storage
		.modpack_path
		.parent()
		.map(|p| p.to_path_buf())
		.unwrap_or_default();
	let config = storage.load_modpack().unwrap_or_default();
	let tx = tx.clone();

	crate::output::start_capture();
	tokio::spawn(async move {
		let storage = crate::storage::Storage::new(&root_dir, &config);

		let mut installed = Vec::new();
		let mut errors = Vec::new();

		for (mod_id, source) in &marked {
			let add_ctx = crate::commands::add::sources::AddContext {
				source,
				version_req: None,
				force: false,
				storage: &storage,
				mc_version: mc_version.as_deref(),
				loader,
				registry: registry.clone(),
				env_override: None,
				project_type_override: None,
				categories: Vec::new(),
			};

			match add_ctx.add(mod_id).await {
				Ok(_) => installed.push(mod_id.clone()),
				Err(e) => errors.push(format!("{}: {}", mod_id, e)),
			}
		}

		let captured = crate::output::stop_capture();

		if errors.is_empty() {
			if tx
				.send(AsyncResult::InstallDeps(Ok(installed)))
				.await
				.is_err()
			{
				tracing::debug!("TUI receiver dropped before install result");
			}
		} else {
			let mut output = captured;
			output.push(String::new());
			output.extend(errors.iter().cloned());
			let _ = tx
				.send(AsyncResult::InstallDeps(Err(output.join("\n"))))
				.await;
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
			if app.mode == Mode::InstallDeps || app.mode == Mode::DepTree {
				app.dep_installing = false;
				app.pending_async = None;
			}
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
		AsyncResult::InstallDeps(install_result) => {
			if app.mode == Mode::InstallProgress {
				app.dep_installing = false;
				app.pending_async = None;
				app.dep_output_scroll = 0;
				match install_result {
					Ok(installed) => {
						let count = installed.len();
						for mod_id in &installed {
							app.dep_marked.remove(mod_id);
						}
						app.dep_output = vec![format!(
							"Successfully installed {} dep(s).",
							count
						)];
						app.set_status(
							StatusKind::Success,
							format!("Installed {} dep(s)", count),
						);
					}
					Err(output) => {
						app.dep_output = if output.is_empty() {
							vec!["Install failed.".to_string()]
						} else {
							output.lines().map(String::from).collect()
						};
						app.set_status(
							StatusKind::Error,
							"Install failed".to_string(),
						);
					}
				}
			}
		}
	}
}
