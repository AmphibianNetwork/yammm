//! Java binary detection and launch argument construction.
//!
//! Handles macOS ARM64/x86_64 Rosetta boundary — Minecraft's Java is
//! typically x86_64, so on Apple Silicon we prefix with `arch -x86_64`.
use std::path::Path;

/// Platform-specific classpath separator (`;` on Windows, `:` on Unix).
pub const CLASSPATH_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };

/// JVM argument needed by Fabric/Quilt for module access.
pub const ADD_OPENS_ARG: &str =
	"--add-opens=java.base/java.lang.invoke=ALL-UNNAMED";

/// Get the OS name in Minecraft's format (used for version manifests).
pub fn current_os_name() -> &'static str {
	if cfg!(target_os = "macos") {
		"osx"
	} else if cfg!(target_os = "windows") {
		"windows"
	} else {
		"linux"
	}
}

/// Build the command prefix for launching Java.
///
/// On macOS ARM64, if the Java binary name contains `-x64`, we prefix
/// with `arch -x86_64` to run under Rosetta 2. This is needed because
/// some Minecraft versions only ship x86_64 natives.
pub fn java_launch_prefix(java_path: &Path) -> Vec<String> {
	#[cfg(target_os = "macos")]
	{
		if cfg!(target_arch = "aarch64") {
			let java_str = java_path.to_string_lossy();
			if java_str.contains("-x64") {
				return vec![
					"arch".to_string(),
					"-x86_64".to_string(),
					java_path.to_string_lossy().to_string(),
				];
			}
		}
	}
	vec![java_path.to_string_lossy().to_string()]
}
