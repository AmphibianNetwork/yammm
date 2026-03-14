use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Processor {
	#[serde(default)]
	pub sides: Option<Vec<String>>,
	pub jar: String,
	#[serde(default)]
	pub classpath: Vec<String>,
	#[serde(default)]
	pub args: Vec<String>,
}

pub fn run_processor(
	processor: &Processor,
	_side: &str,
	classpath_jars: &[PathBuf],
	resolved_args: &[String],
	java_path: &Path,
) -> Result<()> {
	let processor_jar = find_jar_in_classpath(&processor.jar, classpath_jars)?;

	let mut cp_parts: Vec<String> =
		vec![processor_jar.to_string_lossy().to_string()];
	for cp_entry in &processor.classpath {
		if let Some(path) = find_jar_in_classpath_opt(cp_entry, classpath_jars)
		{
			cp_parts.push(path.to_string_lossy().to_string());
		} else {
			tracing::warn!("Processor classpath entry not found: {}", cp_entry);
		}
	}

	let classpath = cp_parts.join(crate::utils::CLASSPATH_SEPARATOR);

	let main_class = find_main_class(processor_jar)?;

	let prefix = crate::utils::java_launch_prefix(java_path);
	let mut cmd = std::process::Command::new(&prefix[0]);
	if prefix.len() > 1 {
		cmd.args(&prefix[1..]);
	}
	cmd.arg("-cp").arg(&classpath).arg(&main_class);
	for arg in resolved_args {
		cmd.arg(arg);
	}

	tracing::debug!("Running processor: {}", main_class);

	let output = cmd.output()?;
	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		return Err(crate::errors::YammmError::general(format!(
			"Processor {} failed with exit code {}: {}",
			processor.jar,
			output.status.code().unwrap_or(-1),
			stderr.trim()
		))
		.into());
	}

	Ok(())
}

pub fn find_main_class(jar_path: &Path) -> Result<String> {
	let file = std::fs::File::open(jar_path)?;
	let mut archive = zip::ZipArchive::new(file)?;

	let mut manifest_bytes = Vec::new();
	archive
		.by_name("META-INF/MANIFEST.MF")
		.with_context(|| "No MANIFEST.MF found in processor JAR")?
		.read_to_end(&mut manifest_bytes)?;

	let manifest = String::from_utf8_lossy(&manifest_bytes);
	for line in manifest.lines() {
		if let Some(main_class) = line.strip_prefix("Main-Class: ") {
			return Ok(main_class.trim().to_string());
		}
	}

	Err(crate::errors::YammmError::general(format!(
		"No Main-Class found in MANIFEST.MF of {}",
		jar_path.display()
	))
	.into())
}

fn find_jar_in_classpath<'a>(
	maven_coords: &str,
	classpath_jars: &'a [PathBuf],
) -> Result<&'a PathBuf> {
	find_jar_in_classpath_opt(maven_coords, classpath_jars)
		.ok_or_else(|| {
			crate::errors::YammmError::general(format!(
				"JAR not found in classpath for: {}",
				maven_coords
			))
		})
		.map_err(Into::into)
}

fn find_jar_in_classpath_opt<'a>(
	maven_coords: &str,
	classpath_jars: &'a [PathBuf],
) -> Option<&'a PathBuf> {
	let mut parts = maven_coords.split(':');
	let _group = parts.next()?;
	let artifact = parts.next()?;
	let version_raw = parts.next()?;

	let (version, _) = crate::utils::maven::split_version_ext(version_raw);
	let expected_fragment = format!("{}-{}", artifact, version);
	classpath_jars
		.iter()
		.find(|p| p.to_string_lossy().contains(&expected_fragment))
}

pub fn filter_processors_by_side<'a>(
	processors: &'a [Processor],
	side: &str,
) -> Vec<&'a Processor> {
	processors
		.iter()
		.filter(|p| match &p.sides {
			None => true,
			Some(sides) => sides.iter().any(|s| s == side),
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_filter_no_sides_matches_all() {
		let processors = vec![Processor {
			sides: None,
			jar: "proc1".to_string(),
			classpath: vec![],
			args: vec![],
		}];
		let result = filter_processors_by_side(&processors, "client");
		assert_eq!(result.len(), 1);
		let result = filter_processors_by_side(&processors, "server");
		assert_eq!(result.len(), 1);
	}

	#[test]
	fn test_filter_client_only() {
		let processors = vec![Processor {
			sides: Some(vec!["client".to_string()]),
			jar: "proc1".to_string(),
			classpath: vec![],
			args: vec![],
		}];
		assert_eq!(filter_processors_by_side(&processors, "client").len(), 1);
		assert_eq!(filter_processors_by_side(&processors, "server").len(), 0);
	}

	#[test]
	fn test_filter_both_sides() {
		let processors = vec![Processor {
			sides: Some(vec!["client".to_string(), "server".to_string()]),
			jar: "proc1".to_string(),
			classpath: vec![],
			args: vec![],
		}];
		assert_eq!(filter_processors_by_side(&processors, "client").len(), 1);
		assert_eq!(filter_processors_by_side(&processors, "server").len(), 1);
	}

	#[test]
	fn test_filter_mixed() {
		let processors = vec![
			Processor {
				sides: None,
				jar: "universal".to_string(),
				classpath: vec![],
				args: vec![],
			},
			Processor {
				sides: Some(vec!["client".to_string()]),
				jar: "client-only".to_string(),
				classpath: vec![],
				args: vec![],
			},
			Processor {
				sides: Some(vec!["server".to_string()]),
				jar: "server-only".to_string(),
				classpath: vec![],
				args: vec![],
			},
		];
		let client = filter_processors_by_side(&processors, "client");
		assert_eq!(client.len(), 2);
		let server = filter_processors_by_side(&processors, "server");
		assert_eq!(server.len(), 2);
	}

	#[test]
	fn test_processor_deserialization() {
		let json = r#"{
			"sides": ["client"],
			"jar": "net/neoforged:neoforge:20.4.1",
			"classpath": ["net/neoforged:neoforge:20.4.1"],
			"args": ["--install", "{ROOT}"]
		}"#;
		let proc: Processor = serde_json::from_str(json).unwrap();
		assert_eq!(proc.sides.as_deref(), Some(&["client".to_string()][..]));
		assert_eq!(proc.jar, "net/neoforged:neoforge:20.4.1");
		assert_eq!(proc.args.len(), 2);
	}

	#[test]
	fn test_processor_deserialization_defaults() {
		let json = r#"{ "jar": "test" }"#;
		let proc: Processor = serde_json::from_str(json).unwrap();
		assert!(proc.sides.is_none());
		assert!(proc.classpath.is_empty());
		assert!(proc.args.is_empty());
	}

	#[test]
	fn test_find_jar_in_classpath_found() {
		let jars = vec![
			PathBuf::from("/libs/neoforge-20.4.1.jar"),
			PathBuf::from("/libs/fabric-api-0.92.0.jar"),
		];
		let result =
			find_jar_in_classpath_opt("net.neoforge:neoforge:20.4.1", &jars);
		assert!(result.is_some());
		assert_eq!(
			result.unwrap(),
			&PathBuf::from("/libs/neoforge-20.4.1.jar")
		);
	}

	#[test]
	fn test_find_jar_in_classpath_not_found() {
		let jars = vec![PathBuf::from("/libs/other-1.0.jar")];
		let result =
			find_jar_in_classpath_opt("net.neoforge:neoforge:20.4.1", &jars);
		assert!(result.is_none());
	}
}
