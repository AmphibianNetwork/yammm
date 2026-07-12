use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::api::adoptium::{self, AdoptiumClient};
use crate::output;

pub fn detect_java_version(java_path: &Path) -> Result<i32> {
	let output = run_java_command(java_path, &["-version"]);

	match output {
		Ok(output) if output.status.success() => {
			let stderr = String::from_utf8_lossy(&output.stderr);
			parse_java_version(&stderr)
				.ok_or_else(|| {
					crate::errors::YammmError::invalid_args(
						"Could not parse Java version from output",
					)
				})
				.map_err(Into::into)
		}
		Ok(_) => Err(crate::errors::YammmError::invalid_args(format!(
			"Java at {} returned an error",
			java_path.display()
		))
		.into()),
		Err(_) => Err(crate::errors::YammmError::invalid_args(format!(
			"No Java installation found at {}",
			java_path.display()
		))
		.into()),
	}
}

/// Runs a Java binary with the given arguments.
/// On macOS ARM, if the binary is x86_64, prefixes with `arch -x86_64`
/// so Rosetta 2 can translate it.
fn run_java_command(
	java_path: &Path,
	args: &[&str],
) -> std::io::Result<std::process::Output> {
	let prefix = java_launch_prefix(java_path);
	let mut cmd = std::process::Command::new(&prefix[0]);
	if prefix.len() > 1 {
		cmd.args(&prefix[1..]);
	}
	for arg in args {
		cmd.arg(arg);
	}
	cmd.output()
}

pub fn parse_java_version(stderr: &str) -> Option<i32> {
	for line in stderr.lines() {
		let line = line.trim();
		if let Some(version_str) = line
			.strip_prefix("java version \"")
			.or_else(|| line.strip_prefix("openjdk version \""))
		{
			let version_str = version_str.trim_end_matches('"');
			return Some(parse_java_version_string(version_str));
		}
	}
	None
}

/// Parses a Java version string (e.g. `17.0.5` or `1.8.0_352`) and returns
/// the major version number. For legacy `1.x` format, extracts the minor version
/// (e.g. `1.8` → 8).
fn parse_java_version_string(s: &str) -> i32 {
	let mut parts = s.split(&['.', '_', '-'][..]);
	if let Some(first) = parts.next()
		&& let Ok(major) = first.parse::<i32>()
	{
		if major == 1 {
			if let Some(second) = parts.next() {
				return second.parse::<i32>().unwrap_or(8);
			}
			return 8;
		}
		return major;
	}
	8
}

/// Determines the Java major version a Minecraft version was designed for.
/// This is the exact version to use, not a minimum — MC 1.12.2 gets Java 8
/// even though Java 21 could technically run it.
/// MC ≤1.16.5 → Java 8; MC 1.17 → Java 16; MC 1.18–1.20.4 → Java 17;
/// MC 1.20.5–1.25.x → Java 21; MC 26+ → Java 25.
pub fn required_java_version(mc_version: &str) -> i32 {
	let parts: Vec<u32> = mc_version
		.split('.')
		.filter_map(|p| p.parse().ok())
		.collect();

	let major = parts.first().copied().unwrap_or(0);
	let minor = parts.get(1).copied().unwrap_or(0);
	let patch = parts.get(2).copied().unwrap_or(0);

	if major == 1 {
		// MC 1.20.5+ requires Java 21
		if minor > 20 || (minor == 20 && patch >= 5) {
			return 21;
		}
		// MC 1.18+ requires Java 17
		if minor >= 18 {
			return 17;
		}
		// MC 1.17 requires Java 16
		if minor == 17 {
			return 16;
		}
		// MC 1.16 and earlier use Java 8
		return 8;
	}

	// Post-1.x versioning (e.g. 26.x): requires Java 25
	// MC 26.x+ bundles are compiled for class file version 69 (Java 25).
	if major >= 26 {
		return 25;
	}
	if major >= 21 {
		return 21;
	}

	8
}

/// Determines the Java major version to use for a given MC version + loader combo.
/// Returns the higher of the MC base requirement and the loader's own minimum.
/// Modern Fabric/Quilt/Forge/NeoForge loaders all require at least Java 17
/// regardless of the MC version, so we bump up when the loader needs it.
pub fn required_java_version_for_loader(
	mc_version: &str,
	loader: &crate::types::LoaderType,
) -> i32 {
	let base = required_java_version(mc_version);

	let loader_min = match loader {
		crate::types::LoaderType::Fabric | crate::types::LoaderType::Quilt => {
			21
		}
		crate::types::LoaderType::Forge
		| crate::types::LoaderType::NeoForge => 17,
	};

	std::cmp::max(base, loader_min)
}

/// Returns JVM arguments to mitigate the Log4Shell vulnerability (CVE-2021-44228)
/// for MC versions that ship with vulnerable log4j 2.x (< 2.15.0).
/// MC ≤ 1.18.1 bundle log4j versions vulnerable to JNDI lookup attacks;
/// MC 1.18.2+ ship log4j 2.17.1+ which is already patched.
pub fn log4j_mitigation_args(mc_version: &str) -> Vec<String> {
	let parts: Vec<u32> = mc_version
		.split('.')
		.filter_map(|p| p.parse().ok())
		.collect();

	let major = parts.first().copied().unwrap_or(0);
	let minor = parts.get(1).copied().unwrap_or(0);
	let patch = parts.get(2).copied().unwrap_or(0);

	let needs_mitigation = if major == 1 {
		minor < 18 || (minor == 18 && patch <= 1)
	} else if (21..26).contains(&major) {
		false
	} else {
		major < 26
	};

	if needs_mitigation {
		vec!["-Dlog4j2.formatMsgNoLookups=true".to_string()]
	} else {
		Vec::new()
	}
}

/// Returns true for MC versions that ship log4j 2.8.1 and may need log4j
/// config patching. The embedded `log4j2.xml` itself is compatible, but when
/// a custom classloader (e.g. Fabric KnotClassloader) interferes with log4j's
/// config discovery, log4j falls back to its default pattern which uses bare
/// `%d`, `%thread` etc. that log4j 2.9+ cannot parse.
pub fn needs_log4j_config_override(mc_version: &str) -> bool {
	let parts: Vec<u32> = mc_version
		.split('.')
		.filter_map(|p| p.parse().ok())
		.collect();

	let major = parts.first().copied().unwrap_or(0);
	let minor = parts.get(1).copied().unwrap_or(0);
	let patch = parts.get(2).copied().unwrap_or(0);

	if major == 1 {
		minor < 18 || (minor == 18 && patch <= 1)
	} else {
		major < 21
	}
}

const LOG4J_COMPAT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
  <Appenders>
    <Console name="SysOut" target="SYSTEM_OUT">
      <PatternLayout pattern="%d{HH:mm:ss} [%t] %-5level %logger{36} - %msg%n"/>
    </Console>
    <RollingRandomAccessFile name="File" fileName="logs/latest.log" filePattern="logs/%d{yyyy-MM-dd}-%i.log.gz">
      <PatternLayout pattern="%d{HH:mm:ss} [%t] %-5level %logger{36} - %msg%n"/>
      <Policies>
        <TimeBasedTriggeringPolicy/>
        <OnStartupTriggeringPolicy/>
      </Policies>
    </RollingRandomAccessFile>
  </Appenders>
  <Loggers>
    <Root level="info">
      <AppenderRef ref="SysOut"/>
      <AppenderRef ref="File"/>
    </Root>
  </Loggers>
</Configuration>
"#;

/// Replaces the embedded log4j2.xml in a server/client JAR with a version that
/// uses format specifiers compatible with modern log4j 2.x. Old MC versions
/// ship log4j 2.8.1 whose embedded config uses bare `%d`, `%thread` etc. which
/// log4j 2.9+ cannot parse. System properties like `-Dlog4j2.configurationFile`
/// are unreliable because custom classloaders (e.g. Fabric KnotClassloader)
/// may load the embedded config first, so patching the JAR is the only fix
/// that guarantees our config takes effect.
pub fn patch_log4j_config_in_jar(jar_path: &Path) -> Result<()> {
	let file = std::fs::File::open(jar_path)?;
	let mut archive = zip::ZipArchive::new(file)?;

	if archive.by_name("log4j2.xml").is_err() {
		return Ok(());
	}

	let tmp_path = jar_path.with_extension("jar.tmp");
	let tmp_file = std::fs::File::create(&tmp_path)?;
	let mut writer = zip::ZipWriter::new(tmp_file);
	let options = zip::write::SimpleFileOptions::default();

	for i in 0..archive.len() {
		let mut entry = archive.by_index(i)?;
		let name = entry.name().to_string();

		if name == "log4j2.xml" {
			writer.start_file::<_, ()>(&name, options)?;
			std::io::Write::write_all(
				&mut writer,
				LOG4J_COMPAT_XML.as_bytes(),
			)?;
		} else if entry.is_dir() {
			writer.add_directory::<_, ()>(&name, options)?;
		} else {
			let mut buf = Vec::new();
			std::io::Read::read_to_end(&mut entry, &mut buf)?;
			writer.start_file::<_, ()>(&name, options)?;
			std::io::Write::write_all(&mut writer, &buf)?;
		}
	}

	writer.finish()?;
	drop(archive);
	std::fs::rename(&tmp_path, jar_path)?;
	Ok(())
}

/// Best-effort fallback: writes a log4j2.xml to the given directory and returns
/// JVM args pointing to it via system properties. Unreliable with custom
/// classloaders (e.g. Fabric KnotClassloader) which may load the embedded
/// config first. Prefer `patch_log4j_config_in_jar` when the JAR is available
/// for modification.
pub fn write_log4j_config(dir: &Path) -> Result<Vec<String>> {
	let config_path = dir.join("log4j2.xml");
	std::fs::write(&config_path, LOG4J_COMPAT_XML)?;
	let path_str = config_path.to_string_lossy().to_string();
	Ok(vec![
		format!("-Dlog4j.configurationFile={}", path_str),
		format!("-Dlog4j2.configurationFile={}", path_str),
	])
}

/// Returns the command-line prefix needed to invoke the given Java binary.
/// On macOS ARM with an x64 JDK, prefixes `arch -x86_64` so Rosetta 2
/// translates the binary. Otherwise, returns just the java path.
pub fn java_launch_prefix(java_path: &Path) -> Vec<String> {
	crate::utils::java_launch_prefix(java_path)
}

fn find_cached_java(
	cache_dir: &Path,
	major_version: i32,
) -> Option<PathBuf> {
	let java_base = cache_dir.join("java");

	// Try the native arch first, then x64 fallback (Rosetta 2 on macOS ARM)
	let candidates =
		if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
			vec![
				adoptium::java_dir_name(major_version, Some("aarch64")),
				adoptium::java_dir_name(major_version, Some("x64")),
			]
		} else {
			vec![adoptium::java_dir_name(major_version, None)]
		};

	for dir_name in candidates {
		let dir = java_base.join(&dir_name);
		let java_bin = adoptium::java_binary_path(&dir);
		if java_bin.exists() {
			return Some(java_bin);
		}
		let contents_home = dir.join("Contents").join("Home");
		let java_bin = adoptium::java_binary_path(&contents_home);
		if java_bin.exists() {
			return Some(java_bin);
		}
	}

	None
}

/// Resolves a Java installation for the given MC version and loader:
/// checks user override → cached JDK at exact version → auto-install from Adoptium.
/// Always uses the exact Java version each MC version was designed for,
/// never falling back to whatever system Java happens to be installed.
pub async fn resolve_java(
	cache_dir: &Path,
	mc_version: &str,
	loader: &crate::types::LoaderType,
	http_client: &reqwest::Client,
	java_override: Option<&Path>,
) -> Result<(PathBuf, i32)> {
	let required = required_java_version_for_loader(mc_version, loader);

	if let Some(path) = java_override {
		let version = detect_java_version(path)?;
		return Ok((path.to_path_buf(), version));
	}

	if let Some(java_bin) = find_cached_java(cache_dir, required)
		&& let Ok(major) = detect_java_version(&java_bin)
	{
		return Ok((java_bin, major));
	}

	install_jdk(cache_dir, required, http_client).await
}

/// Auto-installs a Temurin JDK from Adoptium: downloads, verifies the checksum,
/// extracts, and moves the JDK to the cache directory.
async fn install_jdk(
	cache_dir: &Path,
	major_version: i32,
	http_client: &reqwest::Client,
) -> Result<(PathBuf, i32)> {
	output::info(format!("Downloading Temurin JDK {}...", major_version));

	let client = AdoptiumClient::new().with_client(http_client.clone());
	let asset = client.get_latest_jdk(major_version).await?;

	let java_base = cache_dir.join("java");
	std::fs::create_dir_all(&java_base)?;

	let temp_dir = java_base.join("tmp");
	std::fs::create_dir_all(&temp_dir)?;
	let archive_path = temp_dir.join(&asset.file_name);

	output::bullet(format!("Downloading {}...", asset.file_name));
	// JDK archives are 100–200 MB; stream them. Adoptium publishes a checksum
	// alongside each release — verify it when it's SHA-256 (their default).
	let policy = if asset.checksum_type == "sha256" {
		crate::api::streaming::HashPolicy::Required(
			crate::api::streaming::ExpectedHash {
				hash_type: crate::types::HashType::Sha256,
				hex: &asset.checksum,
			},
		)
	} else {
		crate::api::streaming::HashPolicy::AcceptedUnhashed {
			reason: "Adoptium asset checksum is not sha256",
		}
	};
	if archive_path.exists() {
		// Stale tmp from a previous failed extract — wipe so the streaming
		// helper actually runs (it short-circuits on dest existing).
		let _ = std::fs::remove_file(&archive_path);
	}
	crate::api::streaming::download_to_file(
		http_client,
		&asset.download_url,
		&archive_path,
		policy,
		&asset.file_name,
	)
	.await?;

	output::bullet("Extracting JDK...");
	let extract_tmp = temp_dir.join(format!("extract-{}", major_version));
	if extract_tmp.exists() {
		std::fs::remove_dir_all(&extract_tmp)?;
	}
	std::fs::create_dir_all(&extract_tmp)?;

	adoptium::extract_archive(&archive_path, &extract_tmp)?;

	let jdk_inner =
		adoptium::find_extracted_jdk_dir(&extract_tmp, major_version)
			.ok_or_else(|| {
				crate::errors::YammmError::general(
					"Could not find JDK directory in extracted archive",
				)
			})?;

	let final_dir = cache_dir
		.join("java")
		.join(adoptium::java_dir_name(major_version, Some(&asset.arch)));
	if final_dir.exists() {
		std::fs::remove_dir_all(&final_dir)?;
	}
	std::fs::rename(&jdk_inner, &final_dir)?;

	let _ = std::fs::remove_dir_all(&temp_dir);

	let java_bin = adoptium::java_binary_path(&final_dir);

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let _ = std::fs::set_permissions(
			&java_bin,
			std::fs::Permissions::from_mode(0o755),
		);
	}

	let major = detect_java_version(&java_bin)?;
	output::success(format!(
		"Installed Temurin JDK {} at {}",
		major,
		final_dir.display()
	));

	Ok((java_bin, major))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_java_version_modern() {
		let output = "openjdk version \"17.0.5\" 2022-10-18\n";
		assert_eq!(parse_java_version(output), Some(17));
	}

	#[test]
	fn test_parse_java_version_legacy() {
		let output = "java version \"1.8.0_352\"\n";
		assert_eq!(parse_java_version(output), Some(8));
	}

	#[test]
	fn test_parse_java_version_21() {
		let output = "openjdk version \"21.0.1\" 2023-10-17\n";
		assert_eq!(parse_java_version(output), Some(21));
	}

	#[test]
	fn test_required_java_version_old() {
		assert_eq!(required_java_version("1.16.5"), 8);
	}

	#[test]
	fn test_required_java_version_17() {
		// MC 1.17 requires Java 16; MC 1.18+ requires Java 17
		assert_eq!(required_java_version("1.17.1"), 16);
		assert_eq!(required_java_version("1.18.1"), 17);
		assert_eq!(required_java_version("1.18"), 17);
	}

	#[test]
	fn test_required_java_version_20_5() {
		assert_eq!(required_java_version("1.20.5"), 21);
	}

	#[test]
	fn test_required_java_version_21() {
		assert_eq!(required_java_version("1.21.0"), 21);
	}

	#[test]
	fn test_required_java_version_26() {
		assert_eq!(required_java_version("26.1"), 25);
	}

	#[test]
	fn test_required_java_version_26_1_2() {
		assert_eq!(required_java_version("26.1.2"), 25);
	}

	#[test]
	fn test_required_java_version_for_loader_fabric_116() {
		assert_eq!(
			required_java_version_for_loader(
				"1.16.5",
				&crate::types::LoaderType::Fabric
			),
			21
		);
	}

	#[test]
	fn test_required_java_version_for_loader_fabric_26() {
		assert_eq!(
			required_java_version_for_loader(
				"26.1",
				&crate::types::LoaderType::Fabric
			),
			25
		);
	}

	#[test]
	fn test_required_java_version_for_loader_forge_26() {
		assert_eq!(
			required_java_version_for_loader(
				"26.1",
				&crate::types::LoaderType::Forge
			),
			25
		);
	}

	#[test]
	fn test_required_java_version_for_loader_neoforge_26() {
		assert_eq!(
			required_java_version_for_loader(
				"26.1",
				&crate::types::LoaderType::NeoForge
			),
			25
		);
	}

	#[test]
	fn test_required_java_version_for_loader_forge_121() {
		assert_eq!(
			required_java_version_for_loader(
				"1.21.1",
				&crate::types::LoaderType::Forge
			),
			21
		);
	}

	#[test]
	fn test_required_java_version_for_loader_forge_118() {
		assert_eq!(
			required_java_version_for_loader(
				"1.18.2",
				&crate::types::LoaderType::Forge
			),
			17
		);
	}

	#[test]
	fn test_required_java_version_for_loader_quilt_119() {
		assert_eq!(
			required_java_version_for_loader(
				"1.19.2",
				&crate::types::LoaderType::Quilt
			),
			21
		);
	}
}
