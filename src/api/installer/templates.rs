use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::profile::extract_file_from_installer;

pub struct TemplateContext<'a> {
	pub data: &'a BTreeMap<String, DataEntry>,
	pub side: &'a str,
	pub library_dir: &'a Path,
	pub installer_jar: &'a Path,
	pub mc_jar: &'a Path,
	pub root_dir: &'a Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataEntry {
	#[serde(default)]
	pub client: String,
	#[serde(default)]
	pub server: String,
}

pub fn resolve_data_value(
	value: &str,
	ctx: &TemplateContext<'_>,
) -> Result<String> {
	if let Some(coords) =
		value.strip_prefix('[').and_then(|v| v.strip_suffix(']'))
	{
		let path = maven_coords_to_path(coords);
		Ok(ctx.library_dir.join(&path).to_string_lossy().to_string())
	} else if value.starts_with('/') {
		extract_file_from_installer(value, ctx.installer_jar)
	} else if value.starts_with('\'') && value.ends_with('\'') {
		Ok(value[1..value.len() - 1].to_string())
	} else if value.contains('{') {
		interpolate_variables(value, ctx)
	} else {
		Ok(value.to_string())
	}
}

pub fn maven_coords_to_path(coords: &str) -> String {
	crate::utils::maven::coords_to_path(coords)
}

pub fn resolve_template_args(
	args: &[String],
	ctx: &TemplateContext<'_>,
) -> Result<Vec<String>> {
	let mut resolved = Vec::new();

	for arg in args {
		let resolved_arg = if arg.starts_with('[') && arg.ends_with(']') {
			let coords = &arg[1..arg.len() - 1];
			let path = maven_coords_to_path(coords);
			ctx.library_dir.join(&path).to_string_lossy().to_string()
		} else if arg.contains('{') {
			interpolate_variables(arg, ctx)?
		} else {
			arg.clone()
		};
		resolved.push(resolved_arg);
	}

	Ok(resolved)
}

fn interpolate_variables(
	value: &str,
	ctx: &TemplateContext<'_>,
) -> Result<String> {
	let mut result = String::with_capacity(value.len());
	let mut chars = value.char_indices().peekable();
	while let Some((i, c)) = chars.next() {
		if c == '{' {
			let start = i;
			let key_start = i + 1;
			let mut found_end = false;
			for (j, nc) in chars.by_ref() {
				if nc == '}' {
					let key = &value[key_start..j];
					let replacement = resolve_variable(key, ctx)?;
					result.push_str(&replacement);
					found_end = true;
					break;
				}
			}
			if !found_end {
				result.push_str(&value[start..]);
				break;
			}
		} else {
			result.push(c);
		}
	}
	Ok(result)
}

fn resolve_variable(
	key: &str,
	ctx: &TemplateContext<'_>,
) -> Result<String> {
	match key {
		"INSTALLER" => Ok(ctx.installer_jar.to_string_lossy().to_string()),
		"MINECRAFT_JAR" => Ok(ctx.mc_jar.to_string_lossy().to_string()),
		"LIBRARY_DIR" => Ok(ctx.library_dir.to_string_lossy().to_string()),
		"ROOT" => Ok(ctx.root_dir.to_string_lossy().to_string()),
		"SIDE" => Ok(ctx.side.to_string()),
		"MINECRAFT_VERSION" => {
			let filename = ctx
				.mc_jar
				.file_stem()
				.and_then(|s| s.to_str())
				.unwrap_or("unknown");
			Ok(filename.to_string())
		}
		_ => {
			if let Some(entry) = ctx.data.get(key) {
				let value = if ctx.side == "client" {
					&entry.client
				} else {
					&entry.server
				};
				resolve_data_value(value, ctx)
			} else {
				Ok(format!("{{{}}}", key))
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::BTreeMap;

	fn make_ctx<'a>(
		data: &'a BTreeMap<String, DataEntry>,
		side: &'a str,
	) -> TemplateContext<'a> {
		TemplateContext {
			data,
			side,
			library_dir: Path::new("/libs"),
			installer_jar: Path::new("/install/installer.jar"),
			mc_jar: Path::new("/libs/1.20.4.jar"),
			root_dir: Path::new("/root"),
		}
	}

	#[test]
	fn test_resolve_data_value_maven_coords() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		let result =
			resolve_data_value("[net.neoforge:neoforge:20.4.1]", &ctx).unwrap();
		assert!(result.contains("net"));
		assert!(result.contains("neoforge"));
	}

	#[test]
	fn test_resolve_data_value_literal() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		let result = resolve_data_value("'hello world'", &ctx).unwrap();
		assert_eq!(result, "hello world");
	}

	#[test]
	fn test_resolve_data_value_plain_string() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		let result = resolve_data_value("plainvalue", &ctx).unwrap();
		assert_eq!(result, "plainvalue");
	}

	#[test]
	fn test_resolve_data_value_variable() {
		let mut data = BTreeMap::new();
		data.insert(
			"MY_KEY".to_string(),
			DataEntry {
				client: "client_val".to_string(),
				server: "server_val".to_string(),
			},
		);
		let ctx = make_ctx(&data, "client");
		let result = resolve_data_value("{MY_KEY}", &ctx).unwrap();
		assert_eq!(result, "client_val");
	}

	#[test]
	fn test_resolve_data_value_variable_server_side() {
		let mut data = BTreeMap::new();
		data.insert(
			"MY_KEY".to_string(),
			DataEntry {
				client: "client_val".to_string(),
				server: "server_val".to_string(),
			},
		);
		let ctx = make_ctx(&data, "server");
		let result = resolve_data_value("{MY_KEY}", &ctx).unwrap();
		assert_eq!(result, "server_val");
	}

	#[test]
	fn test_resolve_variable_install() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		assert_eq!(
			resolve_variable("INSTALLER", &ctx).unwrap(),
			"/install/installer.jar"
		);
	}

	#[test]
	fn test_resolve_variable_minecraft_jar() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		assert_eq!(
			resolve_variable("MINECRAFT_JAR", &ctx).unwrap(),
			"/libs/1.20.4.jar"
		);
	}

	#[test]
	fn test_resolve_variable_library_dir() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		assert_eq!(resolve_variable("LIBRARY_DIR", &ctx).unwrap(), "/libs");
	}

	#[test]
	fn test_resolve_variable_root() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		assert_eq!(resolve_variable("ROOT", &ctx).unwrap(), "/root");
	}

	#[test]
	fn test_resolve_variable_side() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "server");
		assert_eq!(resolve_variable("SIDE", &ctx).unwrap(), "server");
	}

	#[test]
	fn test_resolve_variable_minecraft_version() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		assert_eq!(
			resolve_variable("MINECRAFT_VERSION", &ctx).unwrap(),
			"1.20.4"
		);
	}

	#[test]
	fn test_resolve_variable_unknown_passthrough() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		assert_eq!(
			resolve_variable("UNKNOWN_VAR", &ctx).unwrap(),
			"{UNKNOWN_VAR}"
		);
	}

	#[test]
	fn test_interpolate_variables_multiple() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		let result = interpolate_variables("{ROOT}/mods/{SIDE}", &ctx).unwrap();
		assert_eq!(result, "/root/mods/client");
	}

	#[test]
	fn test_maven_coords_to_path() {
		let path = maven_coords_to_path("net.neoforge:neoforge:20.4.1");
		assert!(path.contains("net/neoforge/neoforge"));
	}

	#[test]
	fn test_resolve_template_args_plain() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		let args = vec!["--install".to_string()];
		let result = resolve_template_args(&args, &ctx).unwrap();
		assert_eq!(result, vec!["--install".to_string()]);
	}

	#[test]
	fn test_resolve_template_args_with_variable() {
		let data = BTreeMap::new();
		let ctx = make_ctx(&data, "client");
		let args = vec!["{ROOT}/mods".to_string()];
		let result = resolve_template_args(&args, &ctx).unwrap();
		assert_eq!(result, vec!["/root/mods".to_string()]);
	}

	#[test]
	fn test_data_entry_deserialization() {
		let json = r#"{ "client": "client_val", "server": "server_val" }"#;
		let entry: DataEntry = serde_json::from_str(json).unwrap();
		assert_eq!(entry.client, "client_val");
		assert_eq!(entry.server, "server_val");
	}

	#[test]
	fn test_data_entry_deserialization_defaults() {
		let json = r#"{}"#;
		let entry: DataEntry = serde_json::from_str(json).unwrap();
		assert!(entry.client.is_empty());
		assert!(entry.server.is_empty());
	}
}
