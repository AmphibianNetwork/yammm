use std::fmt;

use console::style;

use crate::matrix::TestCase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
	Init,
	Add,
	Remove,
	AddBack,
	LaunchServer,
	LaunchClient,
}

impl Phase {
	pub fn label(&self) -> &'static str {
		match self {
			Phase::Init => "init",
			Phase::Add => "add",
			Phase::Remove => "remove",
			Phase::AddBack => "add-back",
			Phase::LaunchServer => "launch-server",
			Phase::LaunchClient => "launch-client",
		}
	}

	pub fn all() -> &'static [Phase] {
		&[
			Phase::Init,
			Phase::Add,
			Phase::Remove,
			Phase::AddBack,
			Phase::LaunchServer,
			Phase::LaunchClient,
		]
	}
}

impl fmt::Display for Phase {
	fn fmt(
		&self,
		f: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		write!(f, "{}", self.label())
	}
}

#[derive(Debug, Clone)]
pub enum PhaseOutcome {
	Pass,
	Fail(String),
	Skip(String),
}

impl PhaseOutcome {
	pub fn is_pass(&self) -> bool {
		matches!(self, PhaseOutcome::Pass)
	}

	pub fn is_fail(&self) -> bool {
		matches!(self, PhaseOutcome::Fail(_))
	}
}

impl fmt::Display for PhaseOutcome {
	fn fmt(
		&self,
		f: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		match self {
			PhaseOutcome::Pass => write!(f, "PASS"),
			PhaseOutcome::Fail(reason) => write!(f, "FAIL: {reason}"),
			PhaseOutcome::Skip(reason) => write!(f, "SKIP: {reason}"),
		}
	}
}

#[derive(Debug, Clone)]
pub struct PhaseResult {
	pub phase: Phase,
	pub outcome: PhaseOutcome,
	#[expect(dead_code)]
	pub duration_secs: u64,
}

#[derive(Debug, Clone)]
pub struct TestRecord {
	pub test: TestCase,
	pub phases: Vec<PhaseResult>,
}

impl TestRecord {
	pub fn label(&self) -> String {
		self.test.label()
	}

	pub fn overall_pass(&self) -> bool {
		self.phases.iter().all(|p| p.outcome.is_pass())
	}

	pub fn has_failure(&self) -> bool {
		self.phases.iter().any(|p| p.outcome.is_fail())
	}

	pub fn has_skip(&self) -> bool {
		self.phases
			.iter()
			.any(|p| matches!(p.outcome, PhaseOutcome::Skip(_)))
	}

	#[expect(dead_code)]
	pub fn phase_result(
		&self,
		phase: Phase,
	) -> Option<&PhaseResult> {
		self.phases.iter().find(|p| p.phase == phase)
	}
}

pub struct Report {
	records: Vec<TestRecord>,
}

impl Report {
	pub fn new() -> Self {
		Self {
			records: Vec::new(),
		}
	}

	pub fn record(
		&mut self,
		test: TestCase,
		phases: Vec<PhaseResult>,
	) {
		self.records.push(TestRecord { test, phases });
	}

	pub fn total_tests(&self) -> usize {
		self.records.len()
	}

	pub fn pass_count(&self) -> usize {
		self.records.iter().filter(|r| r.overall_pass()).count()
	}

	pub fn fail_count(&self) -> usize {
		self.records.iter().filter(|r| r.has_failure()).count()
	}

	pub fn skip_count(&self) -> usize {
		self.records
			.iter()
			.filter(|r| r.has_skip() && !r.has_failure())
			.count()
	}

	pub fn has_failures(&self) -> bool {
		self.records.iter().any(|r| r.has_failure())
	}

	pub fn phase_stats(&self) -> PhaseStats {
		let mut stats = PhaseStats::default();
		for record in &self.records {
			for phase_result in &record.phases {
				match &phase_result.outcome {
					PhaseOutcome::Pass => stats.pass(phase_result.phase),
					PhaseOutcome::Fail(_) => stats.fail(phase_result.phase),
					PhaseOutcome::Skip(_) => stats.skip(phase_result.phase),
				}
			}
		}
		stats
	}

	pub fn print_summary(&self) {
		println!();
		println!(
			"{}",
			style("═══════════════════════════════════════════").bold()
		);
		println!("{}", style(" Results").bold());
		println!(
			"{}",
			style("═══════════════════════════════════════════").bold()
		);
		println!();

		println!(
			"  {}   {}/{} tests",
			style("PASS").green().bold(),
			self.pass_count(),
			self.total_tests()
		);
		println!(
			"  {}   {}/{} tests",
			style("FAIL").red().bold(),
			self.fail_count(),
			self.total_tests()
		);
		println!(
			"  {}   {}/{} tests",
			style("SKIP").yellow().bold(),
			self.skip_count(),
			self.total_tests()
		);

		let stats = self.phase_stats();
		println!();
		println!("{}", style("  Phase breakdown:").bold());
		for &phase in Phase::all() {
			let (p, f, s) = stats.get(phase);
			let total = p + f + s;
			if total == 0 {
				continue;
			}
			println!(
				"    {:15} {} {} {}",
				phase.label(),
				style(format!("{p}p")).green(),
				style(format!("{f}f")).red(),
				style(format!("{s}s")).yellow(),
			);
		}

		let failures: Vec<_> =
			self.records.iter().filter(|r| r.has_failure()).collect();

		if !failures.is_empty() {
			println!();
			println!("{}", style("Failures:").red().bold());
			for rec in &failures {
				for pr in &rec.phases {
					if pr.outcome.is_fail() {
						let reason = match &pr.outcome {
							PhaseOutcome::Fail(r) => r.as_str(),
							_ => "",
						};
						println!(
							"  {} {} :: {}  ({})",
							style("✗").red(),
							rec.label(),
							style(pr.phase.label()).bold(),
							reason,
						);
					}
				}
			}
		}

		println!();
	}
}

#[derive(Default)]
pub struct PhaseStats {
	data: std::collections::HashMap<Phase, (usize, usize, usize)>,
}

impl PhaseStats {
	fn entry(
		&mut self,
		phase: Phase,
	) -> &mut (usize, usize, usize) {
		self.data.entry(phase).or_insert((0, 0, 0))
	}

	pub fn pass(
		&mut self,
		phase: Phase,
	) {
		self.entry(phase).0 += 1;
	}

	pub fn fail(
		&mut self,
		phase: Phase,
	) {
		self.entry(phase).1 += 1;
	}

	pub fn skip(
		&mut self,
		phase: Phase,
	) {
		self.entry(phase).2 += 1;
	}

	pub fn get(
		&self,
		phase: Phase,
	) -> (usize, usize, usize) {
		*self.data.get(&phase).unwrap_or(&(0, 0, 0))
	}
}
