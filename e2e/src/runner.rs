use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::matrix::{LaunchSide, TestCase};
use crate::report::{Phase, PhaseOutcome, PhaseResult, Report};

static PORT_COUNTER: AtomicU16 = AtomicU16::new(0);

fn next_port() -> u16 {
	if PORT_COUNTER.load(Ordering::Relaxed) == 0 {
		let base = 30000 + (std::process::id() % 10000) as u16;
		PORT_COUNTER.store(base, Ordering::Relaxed);
	}
	PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub struct RunnerConfig {
	pub yammm_bin: PathBuf,
	pub timeout_secs: u64,
	pub no_cleanup: bool,
	pub work_dir: PathBuf,
	pub side: LaunchSide,
	pub mod_slug: Option<String>,
}

pub struct Runner {
	config: RunnerConfig,
	aborted: bool,
}

impl Runner {
	pub fn new(config: RunnerConfig) -> Self {
		Self {
			config,
			aborted: false,
		}
	}

	#[allow(dead_code)]
	pub fn abort(&mut self) {
		self.aborted = true;
	}

	#[allow(dead_code)]
	pub fn is_aborted(&self) -> bool {
		self.aborted
	}

	pub fn run_all(
		&mut self,
		tests: &[TestCase],
	) -> Report {
		let total = tests.len();
		let mut report = Report::new();

		println!();
		log(&format!("Running {total} tests"));
		log(&format!(
			"Sides: {}",
			match self.config.side {
				LaunchSide::Server => "server",
				LaunchSide::Client => "client",
				LaunchSide::Both => "client+server",
			}
		));
		log(&format!("Work dir:     {}", self.config.work_dir.display()));
		println!();

		let pb = ProgressBar::new(total as u64);
		pb.set_style(
			ProgressStyle::with_template(
				"  {pos}/{len} [{bar:40.cyan/blue}] {msg}",
			)
			.unwrap(),
		);

		for (i, test) in tests.iter().enumerate() {
			if self.aborted {
				break;
			}

			pb.set_message(test.label());
			pb.set_position(i as u64);
			pb.suspend(|| {
				let phases = self.run_test(test);
				report.record(test.clone(), phases);
			});

			if self.aborted {
				break;
			}
		}

		pb.set_position(total as u64);
		pb.finish_and_clear();

		report
	}

	fn run_test(
		&mut self,
		test: &TestCase,
	) -> Vec<PhaseResult> {
		let label = test.label();
		let port = next_port();
		let test_dir = self.config.work_dir.join(&label);
		let mod_slug = self
			.config
			.mod_slug
			.clone()
			.unwrap_or_else(|| test.loader.default_mod_slug().to_string());

		let _ = fs::remove_dir_all(&test_dir);
		let _ = fs::create_dir_all(&test_dir);

		println!();
		println!(
			"{}  {}",
			style(&label).bold(),
			style(format!(
				"(port {}, timeout {}s, mod: {})",
				port, self.config.timeout_secs, mod_slug
			))
			.dim(),
		);
		println!("  {}", style(format!("dir: {}", test_dir.display())).dim());
		divider();

		let mut phases = Vec::new();

		let init_result = self.run_phase_init(test, &test_dir, &label);
		phases.push(init_result.clone());
		if !init_result.outcome.is_pass() {
			self.skip_remaining_phases(&mut phases);
			self.cleanup(&test_dir);
			return phases;
		}

		let add_result = self.run_phase_add(&test_dir, &mod_slug, &label);
		phases.push(add_result.clone());

		if add_result.outcome.is_pass() {
			let remove_result =
				self.run_phase_remove(&test_dir, &mod_slug, &label);
			phases.push(remove_result.clone());

			if remove_result.outcome.is_pass() {
				let add_back_result =
					self.run_phase_add_back(&test_dir, &mod_slug, &label);
				phases.push(add_back_result);
			} else {
				phases.push(PhaseResult {
					phase: Phase::AddBack,
					outcome: PhaseOutcome::Skip("remove failed".into()),
					duration_secs: 0,
				});
			}
		} else {
			phases.push(PhaseResult {
				phase: Phase::Remove,
				outcome: PhaseOutcome::Skip("add failed".into()),
				duration_secs: 0,
			});
			phases.push(PhaseResult {
				phase: Phase::AddBack,
				outcome: PhaseOutcome::Skip("add failed".into()),
				duration_secs: 0,
			});
		}

		if self.config.side.should_test_server() {
			let server_result =
				self.run_phase_server(&test_dir, port, test, &label);
			phases.push(server_result);
			force_kill_java();
		}

		if self.config.side.should_test_client() {
			let client_result = self.run_phase_client(&test_dir, test, &label);
			phases.push(client_result);
			force_kill_java();
		}

		self.cleanup(&test_dir);
		phases
	}

	fn run_phase_init(
		&mut self,
		test: &TestCase,
		test_dir: &Path,
		label: &str,
	) -> PhaseResult {
		let start = Instant::now();
		print_phase_start("init", label);

		let output = Command::new(&self.config.yammm_bin)
			.arg("-C")
			.arg(test_dir)
			.arg("init")
			.arg("-n")
			.arg(format!("e2e-{label}"))
			.arg("-V")
			.arg(test.mc_version)
			.arg("-L")
			.arg(test.loader.init_flag())
			.arg("-o")
			.arg(test_dir)
			.output()
			.context("failed to spawn yammm init");

		let duration = start.elapsed().as_secs();
		match output {
			Ok(out) => {
				if out.status.success() {
					print_phase_result("init", &PhaseOutcome::Pass, duration);
					PhaseResult {
						phase: Phase::Init,
						outcome: PhaseOutcome::Pass,
						duration_secs: duration,
					}
				} else {
					let stderr = String::from_utf8_lossy(&out.stderr);
					let reason = extract_error(&stderr);
					print_phase_result(
						"init",
						&PhaseOutcome::Fail(reason.clone()),
						duration,
					);
					PhaseResult {
						phase: Phase::Init,
						outcome: PhaseOutcome::Fail(reason),
						duration_secs: duration,
					}
				}
			}
			Err(e) => {
				let reason = format!("spawn failed: {e}");
				print_phase_result(
					"init",
					&PhaseOutcome::Fail(reason.clone()),
					duration,
				);
				PhaseResult {
					phase: Phase::Init,
					outcome: PhaseOutcome::Fail(reason),
					duration_secs: duration,
				}
			}
		}
	}

	fn run_phase_add(
		&mut self,
		test_dir: &Path,
		mod_slug: &str,
		label: &str,
	) -> PhaseResult {
		self.run_add_command(test_dir, mod_slug, label, Phase::Add)
	}

	fn run_phase_remove(
		&mut self,
		test_dir: &Path,
		mod_slug: &str,
		label: &str,
	) -> PhaseResult {
		let start = Instant::now();
		print_phase_start("remove", label);

		let output = Command::new(&self.config.yammm_bin)
			.arg("-C")
			.arg(test_dir)
			.arg("remove")
			.arg(mod_slug)
			.arg("-y")
			.arg("--force")
			.output()
			.context("failed to spawn yammm remove");

		let duration = start.elapsed().as_secs();
		match output {
			Ok(out) => {
				if out.status.success() {
					print_phase_result("remove", &PhaseOutcome::Pass, duration);
					PhaseResult {
						phase: Phase::Remove,
						outcome: PhaseOutcome::Pass,
						duration_secs: duration,
					}
				} else {
					let stderr = String::from_utf8_lossy(&out.stderr);
					let reason = extract_error(&stderr);
					print_phase_result(
						"remove",
						&PhaseOutcome::Fail(reason.clone()),
						duration,
					);
					PhaseResult {
						phase: Phase::Remove,
						outcome: PhaseOutcome::Fail(reason),
						duration_secs: duration,
					}
				}
			}
			Err(e) => {
				let reason = format!("spawn failed: {e}");
				print_phase_result(
					"remove",
					&PhaseOutcome::Fail(reason.clone()),
					duration,
				);
				PhaseResult {
					phase: Phase::Remove,
					outcome: PhaseOutcome::Fail(reason),
					duration_secs: duration,
				}
			}
		}
	}

	fn run_phase_add_back(
		&mut self,
		test_dir: &Path,
		mod_slug: &str,
		label: &str,
	) -> PhaseResult {
		self.run_add_command(test_dir, mod_slug, label, Phase::AddBack)
	}

	fn run_add_command(
		&mut self,
		test_dir: &Path,
		mod_slug: &str,
		label: &str,
		phase: Phase,
	) -> PhaseResult {
		let phase_label = phase.label();
		let start = Instant::now();
		print_phase_start(phase_label, label);

		let output = Command::new(&self.config.yammm_bin)
			.arg("-C")
			.arg(test_dir)
			.arg("add")
			.arg("--source")
			.arg("modrinth")
			.arg(mod_slug)
			.arg("-y")
			.output()
			.context("failed to spawn yammm add");

		let duration = start.elapsed().as_secs();
		match output {
			Ok(out) => {
				if out.status.success() {
					print_phase_result(
						phase_label,
						&PhaseOutcome::Pass,
						duration,
					);
					PhaseResult {
						phase,
						outcome: PhaseOutcome::Pass,
						duration_secs: duration,
					}
				} else {
					let combined = format!(
						"{}\n{}",
						String::from_utf8_lossy(&out.stdout),
						String::from_utf8_lossy(&out.stderr)
					);
					let reason = extract_error(&combined);
					if combined.contains("no version found")
						|| combined.contains("not found")
						|| combined.contains("No compatible")
					{
						print_phase_result(
							phase_label,
							&PhaseOutcome::Skip(format!(
								"mod unavailable: {reason}"
							)),
							duration,
						);
						PhaseResult {
							phase,
							outcome: PhaseOutcome::Skip(format!(
								"mod unavailable: {reason}"
							)),
							duration_secs: duration,
						}
					} else {
						print_phase_result(
							phase_label,
							&PhaseOutcome::Fail(reason.clone()),
							duration,
						);
						PhaseResult {
							phase,
							outcome: PhaseOutcome::Fail(reason),
							duration_secs: duration,
						}
					}
				}
			}
			Err(e) => {
				let reason = format!("spawn failed: {e}");
				print_phase_result(
					phase_label,
					&PhaseOutcome::Fail(reason.clone()),
					duration,
				);
				PhaseResult {
					phase,
					outcome: PhaseOutcome::Fail(reason),
					duration_secs: duration,
				}
			}
		}
	}

	fn run_phase_server(
		&mut self,
		test_dir: &Path,
		port: u16,
		test: &TestCase,
		label: &str,
	) -> PhaseResult {
		let start = Instant::now();
		print_phase_start("launch-server", label);

		let mut child = match self.spawn_launch_server(test_dir, port) {
			Ok(c) => c,
			Err(e) => {
				let reason = format!("spawn failed: {e}");
				print_phase_result(
					"launch-server",
					&PhaseOutcome::Fail(reason.clone()),
					0,
				);
				return PhaseResult {
					phase: Phase::LaunchServer,
					outcome: PhaseOutcome::Fail(reason),
					duration_secs: 0,
				};
			}
		};

		let stdout = child.stdout.take();
		let stderr = child.stderr.take();

		let done = Arc::new(AtomicBool::new(false));
		let saw_done = Arc::new(AtomicBool::new(false));
		let log_path = test_dir.join("server_launch.log");
		let log_writer =
			Arc::new(std::sync::Mutex::new(fs::File::create(&log_path).ok()));

		let done_clone = done.clone();
		let saw_done_clone = saw_done.clone();
		let log_writer_clone = log_writer.clone();
		let stream_handle = std::thread::spawn(move || {
			if let Some(out) = stdout {
				tee_lines(
					out,
					&done_clone,
					&saw_done_clone,
					&log_writer_clone,
					"Done (",
				);
			}
		});

		let done_clone2 = done.clone();
		let saw_done_clone2 = saw_done.clone();
		let log_writer_clone2 = log_writer.clone();
		let stderr_handle = std::thread::spawn(move || {
			if let Some(err) = stderr {
				tee_lines(
					err,
					&done_clone2,
					&saw_done_clone2,
					&log_writer_clone2,
					"Done (",
				);
			}
		});

		let timed_out = self.wait_for_server(&mut child, &saw_done);

		done.store(true, Ordering::Relaxed);
		force_kill_java();
		let _ = child.kill();
		let _ = child.wait();
		join_with_timeout(stream_handle, Duration::from_secs(3));
		join_with_timeout(stderr_handle, Duration::from_secs(3));

		if self.aborted {
			return PhaseResult {
				phase: Phase::LaunchServer,
				outcome: PhaseOutcome::Skip("aborted".into()),
				duration_secs: start.elapsed().as_secs(),
			};
		}

		let server_done =
			self.check_server_done(test_dir, &log_path, timed_out);
		let log_contents = fs::read_to_string(&log_path).unwrap_or_default();

		let duration = start.elapsed().as_secs();

		let outcome = if server_done {
			match test.known_issue {
				Some(_) => PhaseOutcome::Pass,
				None => PhaseOutcome::Pass,
			}
		} else if timed_out {
			PhaseOutcome::Fail("timed out".into())
		} else if let Some(issue) = test.known_issue {
			PhaseOutcome::Skip(format!("known issue: {issue}"))
		} else {
			classify_failure(&log_contents)
		};

		print_phase_result("launch-server", &outcome, duration);

		PhaseResult {
			phase: Phase::LaunchServer,
			outcome,
			duration_secs: duration,
		}
	}

	fn run_phase_client(
		&mut self,
		test_dir: &Path,
		test: &TestCase,
		label: &str,
	) -> PhaseResult {
		let start = Instant::now();
		print_phase_start("launch-client", label);

		let client_timeout = Duration::from_secs(self.config.timeout_secs * 2);

		let mut child = match self.spawn_launch_client(test_dir) {
			Ok(c) => c,
			Err(e) => {
				let reason = format!("spawn failed: {e}");
				print_phase_result(
					"launch-client",
					&PhaseOutcome::Fail(reason.clone()),
					0,
				);
				return PhaseResult {
					phase: Phase::LaunchClient,
					outcome: PhaseOutcome::Fail(reason),
					duration_secs: 0,
				};
			}
		};

		let stdout = child.stdout.take();
		let stderr = child.stderr.take();

		let done = Arc::new(AtomicBool::new(false));
		let saw_ready = Arc::new(AtomicBool::new(false));
		let log_path = test_dir.join("client_launch.log");
		let log_writer =
			Arc::new(std::sync::Mutex::new(fs::File::create(&log_path).ok()));

		let done_clone = done.clone();
		let saw_ready_clone = saw_ready.clone();
		let log_writer_clone = log_writer.clone();
		let stream_handle = std::thread::spawn(move || {
			if let Some(out) = stdout {
				tee_lines(
					out,
					&done_clone,
					&saw_ready_clone,
					&log_writer_clone,
					"Setting user:",
				);
			}
		});

		let done_clone2 = done.clone();
		let saw_ready_clone2 = saw_ready.clone();
		let log_writer_clone2 = log_writer.clone();
		let stderr_handle = std::thread::spawn(move || {
			if let Some(err) = stderr {
				tee_lines(
					err,
					&done_clone2,
					&saw_ready_clone2,
					&log_writer_clone2,
					"Setting user:",
				);
			}
		});

		let client_start = Instant::now();
		loop {
			if self.aborted {
				break;
			}

			match child.try_wait() {
				Ok(Some(status)) => {
					let log_contents =
						fs::read_to_string(&log_path).unwrap_or_default();
					let duration = start.elapsed().as_secs();
					done.store(true, Ordering::Relaxed);
					force_kill_java();
					join_with_timeout(stream_handle, Duration::from_secs(3));
					join_with_timeout(stderr_handle, Duration::from_secs(3));

					if status.success() || saw_ready.load(Ordering::Relaxed) {
						let outcome = PhaseOutcome::Pass;
						print_phase_result("launch-client", &outcome, duration);
						return PhaseResult {
							phase: Phase::LaunchClient,
							outcome,
							duration_secs: duration,
						};
					} else {
						let reason = if log_contents
							.contains("Could not find or load main class")
						{
							"wrong main class".into()
						} else if log_contents.contains("NullPointerException")
						{
							"NullPointerException".into()
						} else {
							format!(
								"exit code: {}",
								status.code().unwrap_or(-1)
							)
						};
						let outcome = PhaseOutcome::Fail(reason);
						print_phase_result("launch-client", &outcome, duration);
						return PhaseResult {
							phase: Phase::LaunchClient,
							outcome,
							duration_secs: duration,
						};
					}
				}
				Ok(None) => {}
				Err(_) => break,
			}

			if saw_ready.load(Ordering::Relaxed) {
				std::thread::sleep(Duration::from_secs(3));
				let _ = child.kill();
				let _ = child.wait();
				break;
			}

			if client_start.elapsed() >= client_timeout {
				let _ = child.kill();
				let _ = child.wait();
				break;
			}

			std::thread::sleep(Duration::from_millis(500));
		}

		done.store(true, Ordering::Relaxed);
		force_kill_java();
		join_with_timeout(stream_handle, Duration::from_secs(3));
		join_with_timeout(stderr_handle, Duration::from_secs(3));

		if self.aborted {
			return PhaseResult {
				phase: Phase::LaunchClient,
				outcome: PhaseOutcome::Skip("aborted".into()),
				duration_secs: start.elapsed().as_secs(),
			};
		}

		let client_ready = saw_ready.load(Ordering::Relaxed)
			|| self.check_client_done(test_dir, &log_path);

		let duration = start.elapsed().as_secs();

		let outcome = if client_ready {
			match test.known_issue {
				Some(_) => PhaseOutcome::Pass,
				None => PhaseOutcome::Pass,
			}
		} else if client_start.elapsed() >= client_timeout {
			PhaseOutcome::Fail("timed out (client)".into())
		} else if let Some(issue) = test.known_issue {
			PhaseOutcome::Skip(format!("known issue: {issue}"))
		} else {
			let log_contents =
				fs::read_to_string(&log_path).unwrap_or_default();
			classify_failure(&log_contents)
		};

		print_phase_result("launch-client", &outcome, duration);

		PhaseResult {
			phase: Phase::LaunchClient,
			outcome,
			duration_secs: duration,
		}
	}

	fn spawn_launch_server(
		&self,
		test_dir: &Path,
		port: u16,
	) -> Result<Child> {
		let child = Command::new(&self.config.yammm_bin)
			.arg("-C")
			.arg(test_dir)
			.arg("launch")
			.arg("server")
			.arg("--eula")
			.arg("--port")
			.arg(port.to_string())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.with_context(|| {
				format!("failed to spawn yammm at {:?}", self.config.yammm_bin)
			})?;

		Ok(child)
	}

	fn spawn_launch_client(
		&self,
		test_dir: &Path,
	) -> Result<Child> {
		let child = Command::new(&self.config.yammm_bin)
			.arg("-C")
			.arg(test_dir)
			.arg("launch")
			.arg("client")
			.arg("--offline")
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.with_context(|| {
				format!("failed to spawn yammm at {:?}", self.config.yammm_bin)
			})?;

		Ok(child)
	}

	fn wait_for_server(
		&self,
		child: &mut Child,
		saw_done: &AtomicBool,
	) -> bool {
		let start = Instant::now();
		let timeout = Duration::from_secs(self.config.timeout_secs);

		loop {
			match child.try_wait() {
				Ok(Some(_)) => {
					let _ = child.wait();
					break;
				}
				Ok(None) => {}
				Err(_) => break,
			}

			if start.elapsed() >= timeout {
				let _ = child.kill();
				let _ = child.wait();
				return true;
			}

			if saw_done.load(Ordering::Relaxed) {
				std::thread::sleep(Duration::from_millis(500));
				let _ = child.kill();
				let _ = child.wait();
				break;
			}

			std::thread::sleep(Duration::from_millis(500));
		}

		start.elapsed().as_secs() >= self.config.timeout_secs
	}

	fn check_server_done(
		&self,
		test_dir: &Path,
		log_path: &Path,
		timed_out: bool,
	) -> bool {
		if let Ok(contents) = fs::read_to_string(log_path)
			&& contents.contains("Done (")
		{
			return true;
		}

		let mc_log = test_dir.join("server/logs/latest.log");
		if let Ok(contents) = fs::read_to_string(mc_log)
			&& contents.contains("Done (")
		{
			return true;
		}

		timed_out
	}

	fn check_client_done(
		&self,
		test_dir: &Path,
		log_path: &Path,
	) -> bool {
		if let Ok(contents) = fs::read_to_string(log_path)
			&& (contents.contains("Setting user:")
				|| contents.contains("LWJGL")
				|| contents.contains("OpenAL initialized"))
		{
			return true;
		}

		let client_log = test_dir.join("client/logs/latest.log");
		if let Ok(contents) = fs::read_to_string(client_log)
			&& (contents.contains("Setting user:")
				|| contents.contains("LWJGL"))
		{
			return true;
		}

		false
	}

	fn skip_remaining_phases(
		&mut self,
		phases: &mut Vec<PhaseResult>,
	) {
		let reason = "init failed";
		phases.push(PhaseResult {
			phase: Phase::Add,
			outcome: PhaseOutcome::Skip(reason.into()),
			duration_secs: 0,
		});
		phases.push(PhaseResult {
			phase: Phase::Remove,
			outcome: PhaseOutcome::Skip(reason.into()),
			duration_secs: 0,
		});
		phases.push(PhaseResult {
			phase: Phase::AddBack,
			outcome: PhaseOutcome::Skip(reason.into()),
			duration_secs: 0,
		});
		if self.config.side.should_test_server() {
			phases.push(PhaseResult {
				phase: Phase::LaunchServer,
				outcome: PhaseOutcome::Skip(reason.into()),
				duration_secs: 0,
			});
		}
		if self.config.side.should_test_client() {
			phases.push(PhaseResult {
				phase: Phase::LaunchClient,
				outcome: PhaseOutcome::Skip(reason.into()),
				duration_secs: 0,
			});
		}
	}

	fn cleanup(
		&self,
		test_dir: &Path,
	) {
		if !self.config.no_cleanup {
			let _ = fs::remove_dir_all(test_dir);
		}
	}
}

fn tee_lines<R: Read>(
	reader: R,
	done: &AtomicBool,
	saw_marker: &AtomicBool,
	log_writer: &Arc<std::sync::Mutex<Option<fs::File>>>,
	marker: &str,
) {
	let mut buf_reader = BufReader::new(reader);
	let mut line = String::new();

	loop {
		if done.load(Ordering::Relaxed) {
			break;
		}

		line.clear();
		match buf_reader.read_line(&mut line) {
			Ok(0) => break,
			Ok(_) => {}
			Err(_) => {
				if done.load(Ordering::Relaxed) {
					break;
				}
				std::thread::sleep(Duration::from_millis(50));
				continue;
			}
		}

		let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
		println!("    {trimmed}");

		if let Ok(mut guard) = log_writer.lock()
			&& let Some(ref mut f) = *guard
		{
			let _ = f.write_all(line.as_bytes());
			let _ = f.flush();
		}

		if line.contains(marker) {
			saw_marker.store(true, Ordering::Relaxed);
		}
	}

	let _ = buf_reader;
}

fn classify_failure(log_contents: &str) -> PhaseOutcome {
	if log_contents.contains("NullPointerException") {
		return PhaseOutcome::Fail("NullPointerException".into());
	}
	if let Some(line) = log_contents
		.lines()
		.find(|l| l.contains("Could not find or load main class"))
	{
		return PhaseOutcome::Fail(format!("wrong main class: {line}"));
	}
	if log_contents.contains("NoSuchMethodError") {
		return PhaseOutcome::Fail("NoSuchMethodError".into());
	}
	if log_contents.contains("error decoding response body") {
		return PhaseOutcome::Fail("API deserialization error".into());
	}
	if log_contents.contains("version conflict") {
		return PhaseOutcome::Skip("no loader versions available".into());
	}
	if let Some(line) = log_contents.lines().find(|l| l.contains("Error:")) {
		return PhaseOutcome::Fail(line.to_string());
	}
	PhaseOutcome::Fail("unknown outcome".into())
}

fn extract_error(output: &str) -> String {
	let lines: Vec<&str> = output.lines().collect();
	let error_lines: Vec<&str> = lines
		.iter()
		.filter(|l| {
			l.contains("error")
				|| l.contains("Error")
				|| l.contains("FAILED")
				|| l.contains("failed")
		})
		.take(3)
		.cloned()
		.collect();
	if error_lines.is_empty() {
		let last_lines: Vec<&str> =
			lines.iter().rev().take(3).cloned().collect();
		last_lines.join("; ")
	} else {
		error_lines.join("; ")
	}
}

fn join_with_timeout(
	handle: std::thread::JoinHandle<()>,
	timeout: Duration,
) {
	let start = Instant::now();
	while start.elapsed() < timeout {
		if handle.is_finished() {
			let _ = handle.join();
			return;
		}
		std::thread::sleep(Duration::from_millis(50));
	}
}

fn kill_java_processes() {
	#[cfg(unix)]
	{
		let patterns = [
			"MinecraftServer",
			"BootstrapLauncher",
			"KotServer",
			"fml",
			"MinecraftClient",
		];
		for pattern in patterns {
			let _ = Command::new("pkill").arg("-f").arg(pattern).output();
		}
	}
	#[cfg(windows)]
	{
		let names = ["javaw.exe", "java.exe"];
		for name in names {
			let _ = Command::new("taskkill").args(["/F", "/IM", name]).output();
		}
	}
}

fn force_kill_java() {
	kill_java_processes();
	std::thread::sleep(Duration::from_millis(500));
	kill_java_processes();
	std::thread::sleep(Duration::from_millis(500));
	#[cfg(unix)]
	{
		let _ = Command::new("pkill")
			.arg("-9")
			.arg("-f")
			.arg("net.minecraft")
			.output();
		let _ = Command::new("pkill")
			.arg("-9")
			.arg("-f")
			.arg("BundlerClassPathCapture")
			.output();
	}
	#[cfg(windows)]
	{
		let _ = Command::new("taskkill")
			.args(["/F", "/IM", "java.exe"])
			.output();
	}
}

fn log(msg: &str) {
	println!("{} {msg}", style("[e2e]").cyan());
}

fn divider() {
	println!(
		"{}",
		style(
			"─────────────────────────────────────────────────────────────────"
		)
		.dim()
	);
}

fn print_phase_start(
	phase: &str,
	label: &str,
) {
	println!("  {} [{}]", style(phase).bold(), label);
}

fn print_phase_result(
	phase: &str,
	outcome: &PhaseOutcome,
	duration: u64,
) {
	let styled = match outcome {
		PhaseOutcome::Pass => style("PASS".to_string()).green().bold(),
		PhaseOutcome::Fail(reason) => {
			style(format!("FAIL: {reason}")).red().bold()
		}
		PhaseOutcome::Skip(reason) => {
			style(format!("SKIP: {reason}")).yellow().bold()
		}
	};
	println!("    {} {:3}s  {}", styled, duration, phase);
}

pub fn detect_java_version() -> Option<String> {
	let output = Command::new("java").arg("-version").output().ok()?;
	let stderr = String::from_utf8_lossy(&output.stderr);
	let version_line = stderr.lines().next()?;
	let nums: String = version_line
		.chars()
		.filter(|c| c.is_ascii_digit())
		.collect();
	Some(nums)
}

pub fn find_yammm_bin() -> Result<PathBuf> {
	let exe = std::env::current_exe()?;
	let dir = exe.parent().unwrap();
	let bin = if dir
		.file_name()
		.is_some_and(|n| n == "debug" || n == "release")
	{
		dir.join("yammm")
	} else {
		dir.join("../yammm")
	};

	if bin.exists() {
		return Ok(bin);
	}

	let fallback = std::env::current_dir()
		.unwrap_or_default()
		.join("target/debug/yammm");
	if fallback.exists() {
		return Ok(fallback);
	}

	anyhow::bail!("yammm binary not found. Build it first with: cargo build")
}

pub fn build_yammm() -> Result<PathBuf> {
	log("Building yammm...");
	let cwd = std::env::current_dir().unwrap_or_default();
	let project_root = if cwd.join("Cargo.toml").exists() {
		cwd
	} else {
		cwd.parent().map(|p| p.to_path_buf()).unwrap_or(cwd)
	};
	let status = Command::new("cargo")
		.args(["build"])
		.current_dir(project_root)
		.status()
		.context("failed to run cargo build")?;

	if !status.success() {
		anyhow::bail!("cargo build failed");
	}

	find_yammm_bin()
}

pub fn find_project_root(yammm_bin: &Path) -> PathBuf {
	yammm_bin
		.parent()
		.and_then(|p| p.parent())
		.and_then(|p| p.parent())
		.filter(|p| p.join("Cargo.toml").exists())
		.map(|p| p.to_path_buf())
		.unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}
