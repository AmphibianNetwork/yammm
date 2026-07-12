use serde::{Deserialize, Serialize};

#[derive(
	Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
	#[default]
	Mod,
	ResourcePack,
	Shader,
}

impl ProjectType {
	pub const VARIANTS: &[ProjectType] = &[
		ProjectType::Mod,
		ProjectType::ResourcePack,
		ProjectType::Shader,
	];

	pub const EXPORT_ENTRIES: &[(ProjectType, &str, &str)] = &[
		(ProjectType::Mod, "mods", ".jar"),
		(ProjectType::ResourcePack, "resourcepacks", ".zip"),
		(ProjectType::Shader, "shaderpacks", ".zip"),
	];

	pub fn as_str(&self) -> &'static str {
		match self {
			ProjectType::Mod => "mod",
			ProjectType::ResourcePack => "resourcepack",
			ProjectType::Shader => "shader",
		}
	}
}

impl std::str::FromStr for ProjectType {
	type Err = ProjectTypeParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"mod" => Ok(ProjectType::Mod),
			"resourcepack" | "resource_pack" => Ok(ProjectType::ResourcePack),
			"shader" | "shaderpack" | "shader_pack" => Ok(ProjectType::Shader),
			other => Err(ProjectTypeParseError {
				input: other.to_string(),
			}),
		}
	}
}

#[derive(Debug, thiserror::Error)]
#[error("Unknown project type: {input}")]
pub struct ProjectTypeParseError {
	pub input: String,
}

impl std::fmt::Display for ProjectType {
	fn fmt(
		&self,
		f: &mut std::fmt::Formatter<'_>,
	) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}
