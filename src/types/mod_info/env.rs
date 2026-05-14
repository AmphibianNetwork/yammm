use serde::{Deserialize, Serialize};

#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ModEnv {
	#[default]
	Both,
	Client,
	Server,
}

impl ModEnv {
	pub fn as_str(&self) -> &'static str {
		match self {
			ModEnv::Both => "both",
			ModEnv::Client => "client",
			ModEnv::Server => "server",
		}
	}
}

impl std::str::FromStr for ModEnv {
	type Err = ModEnvParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"both" => Ok(ModEnv::Both),
			"client" => Ok(ModEnv::Client),
			"server" => Ok(ModEnv::Server),
			other => Err(ModEnvParseError {
				input: other.to_string(),
			}),
		}
	}
}

#[derive(Debug, thiserror::Error)]
#[error("Unknown mod environment: {input}")]
pub struct ModEnvParseError {
	pub input: String,
}

impl std::fmt::Display for ModEnv {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}
