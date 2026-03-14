use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Loader {
	Fabric,
	Forge,
	NeoForge,
	Quilt,
}

impl FromStr for Loader {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"fabric" => Ok(Loader::Fabric),
			"forge" => Ok(Loader::Forge),
			"neoforge" => Ok(Loader::NeoForge),
			"quilt" => Ok(Loader::Quilt),
			_ => Err(format!("unknown loader: {s}")),
		}
	}
}

impl fmt::Display for Loader {
	fn fmt(
		&self,
		f: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		match self {
			Loader::Fabric => write!(f, "fabric"),
			Loader::Forge => write!(f, "forge"),
			Loader::NeoForge => write!(f, "neoforge"),
			Loader::Quilt => write!(f, "quilt"),
		}
	}
}

impl Loader {
	#[expect(dead_code)]
	pub fn all() -> &'static [Loader] {
		&[
			Loader::Fabric,
			Loader::Forge,
			Loader::NeoForge,
			Loader::Quilt,
		]
	}

	pub fn default_mod_slug(&self) -> &'static str {
		match self {
			Loader::Fabric | Loader::Quilt => "sodium",
			Loader::Forge | Loader::NeoForge => "appleskin",
		}
	}

	pub fn init_flag(&self) -> &'static str {
		match self {
			Loader::Fabric => "fabric",
			Loader::Forge => "forge",
			Loader::NeoForge => "neoforge",
			Loader::Quilt => "quilt",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaunchSide {
	Server,
	Client,
	Both,
}

impl FromStr for LaunchSide {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"server" => Ok(LaunchSide::Server),
			"client" => Ok(LaunchSide::Client),
			"both" => Ok(LaunchSide::Both),
			_ => {
				Err(format!("unknown side: {s} (expected server/client/both)"))
			}
		}
	}
}

impl LaunchSide {
	pub fn should_test_server(&self) -> bool {
		matches!(self, LaunchSide::Server | LaunchSide::Both)
	}

	pub fn should_test_client(&self) -> bool {
		matches!(self, LaunchSide::Client | LaunchSide::Both)
	}
}

#[derive(Debug, Clone)]
pub struct TestCase {
	pub mc_version: &'static str,
	pub loader: Loader,
	pub min_java: u32,
	pub known_issue: Option<&'static str>,
}

impl TestCase {
	pub const fn new(
		mc_version: &'static str,
		loader: Loader,
		min_java: u32,
	) -> Self {
		Self {
			mc_version,
			loader,
			min_java,
			known_issue: None,
		}
	}

	#[expect(dead_code)]
	pub const fn known(
		mc_version: &'static str,
		loader: Loader,
		min_java: u32,
		issue: &'static str,
	) -> Self {
		Self {
			mc_version,
			loader,
			min_java,
			known_issue: Some(issue),
		}
	}

	pub fn label(&self) -> String {
		format!("{}-{}", self.mc_version, self.loader)
	}
}

pub fn test_matrix() -> Vec<TestCase> {
	vec![
		TestCase::new("1.16.5", Loader::Fabric, 21),
		TestCase::new("1.17.1", Loader::Fabric, 21),
		TestCase::new("1.18.2", Loader::Fabric, 21),
		TestCase::new("1.19.2", Loader::Fabric, 21),
		TestCase::new("1.19.4", Loader::Fabric, 21),
		TestCase::new("1.20.1", Loader::Fabric, 21),
		TestCase::new("1.20.4", Loader::Fabric, 21),
		TestCase::new("1.20.6", Loader::Fabric, 21),
		TestCase::new("1.21.1", Loader::Fabric, 21),
		TestCase::new("1.21.4", Loader::Fabric, 21),
		TestCase::new("1.21.5", Loader::Fabric, 21),
		TestCase::new("1.21.6", Loader::Fabric, 21),
		TestCase::new("1.21.7", Loader::Fabric, 21),
		TestCase::new("1.21.8", Loader::Fabric, 21),
		TestCase::new("1.21.9", Loader::Fabric, 21),
		TestCase::new("1.21.10", Loader::Fabric, 21),
		TestCase::new("1.21.11", Loader::Fabric, 21),
		TestCase::new("26.1", Loader::Fabric, 25),
		TestCase::new("26.1.1", Loader::Fabric, 25),
		TestCase::new("26.1.2", Loader::Fabric, 25),
		TestCase::new("1.19.2", Loader::Quilt, 21),
		TestCase::new("1.19.4", Loader::Quilt, 21),
		TestCase::new("1.20.1", Loader::Quilt, 21),
		TestCase::new("1.20.4", Loader::Quilt, 21),
		TestCase::new("1.20.6", Loader::Quilt, 21),
		TestCase::new("1.21.1", Loader::Quilt, 21),
		TestCase::new("1.21.4", Loader::Quilt, 21),
		TestCase::new("1.18.2", Loader::Forge, 17),
		TestCase::new("1.19.2", Loader::Forge, 17),
		TestCase::new("1.19.4", Loader::Forge, 17),
		TestCase::new("1.20.1", Loader::Forge, 17),
		TestCase::new("1.20.4", Loader::Forge, 17),
		TestCase::new("1.20.6", Loader::Forge, 21),
		TestCase::new("1.21.1", Loader::Forge, 21),
		TestCase::new("1.21.3", Loader::Forge, 21),
		TestCase::new("1.21.4", Loader::Forge, 21),
		TestCase::new("1.21.5", Loader::Forge, 21),
		TestCase::new("1.21.6", Loader::Forge, 21),
		TestCase::new("1.21.7", Loader::Forge, 21),
		TestCase::new("1.21.8", Loader::Forge, 21),
		TestCase::new("1.21.9", Loader::Forge, 21),
		TestCase::new("1.21.10", Loader::Forge, 21),
		TestCase::new("1.21.11", Loader::Forge, 21),
		TestCase::new("26.1", Loader::Forge, 25),
		TestCase::new("26.1.1", Loader::Forge, 25),
		TestCase::new("26.1.2", Loader::Forge, 25),
		TestCase::new("1.20.4", Loader::NeoForge, 17),
		TestCase::new("1.20.6", Loader::NeoForge, 21),
		TestCase::new("1.21.1", Loader::NeoForge, 21),
		TestCase::new("1.21.4", Loader::NeoForge, 21),
		TestCase::new("1.21.5", Loader::NeoForge, 21),
		TestCase::new("1.21.6", Loader::NeoForge, 21),
		TestCase::new("1.21.7", Loader::NeoForge, 21),
		TestCase::new("1.21.8", Loader::NeoForge, 21),
		TestCase::new("1.21.9", Loader::NeoForge, 21),
		TestCase::new("1.21.10", Loader::NeoForge, 21),
		TestCase::new("1.21.11", Loader::NeoForge, 21),
		TestCase::new("26.1", Loader::NeoForge, 25),
		TestCase::new("26.1.1", Loader::NeoForge, 25),
		TestCase::new("26.1.2", Loader::NeoForge, 25),
	]
}

pub fn filter_tests(
	tests: &[TestCase],
	loaders: &[Loader],
	versions: &[String],
) -> Vec<TestCase> {
	tests
		.iter()
		.filter(|t| {
			if !loaders.is_empty() && !loaders.contains(&t.loader) {
				return false;
			}
			if !versions.is_empty()
				&& !versions.contains(&t.mc_version.to_string())
			{
				return false;
			}
			true
		})
		.cloned()
		.collect()
}
