//! Maven coordinate parsing and URL/path construction.
//!
//! Handles `group:artifact:version[:classifier][@ext]` format and
//! XML metadata version parsing (shared by Forge and NeoForge).

/// Parsed Maven coordinates: `group:artifact:version[:classifier][@ext]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoords<'a> {
	pub group: &'a str,
	pub artifact: &'a str,
	pub version: &'a str,
	pub classifier: Option<&'a str>,
	pub ext: &'a str,
}

impl<'a> MavenCoords<'a> {
	/// Parse Maven coordinates from a string.
	///
	/// Examples: `"net.fabricmc:fabric-loader:0.16.5"`,
	/// `"net.neoforged:neoforge:21.1.172:installer@jar"`
	pub fn parse(coords: &'a str) -> Self {
		let mut parts = coords.split(':');
		let group = parts.next().unwrap_or("");
		let artifact = parts.next().unwrap_or("");
		let version_raw = parts.next().unwrap_or("");
		let classifier_raw = parts.next();

		let (version, default_ext) = split_version_ext(version_raw);

		let (classifier, ext) = match classifier_raw {
			Some(cls) => {
				if let Some(at) = cls.rfind('@') {
					(Some(&cls[..at]), &cls[at + 1..])
				} else {
					(Some(cls), default_ext)
				}
			}
			None => (None, default_ext),
		};

		Self {
			group,
			artifact,
			version,
			classifier,
			ext,
		}
	}

	/// Group path with `.` replaced by `/` (e.g. `net/fabricmc`).
	pub fn group_path(&self) -> String {
		self.group.replace('.', "/")
	}

	/// Safe artifact-version stem (e.g. `fabric-loader-0.16.5`). Replaces `+` with `_`.
	pub fn artifact_version_stem(&self) -> String {
		format!("{}-{}", self.artifact.replace('+', "_"), self.version)
	}

	/// Full filename including extension. With classifier: `neoforge-21.1.172-installer.jar`.
	pub fn filename(&self) -> String {
		match self.classifier {
			Some(cls) => {
				format!(
					"{}-{}-{}.{}",
					self.artifact.replace('+', "_"),
					self.version,
					cls,
					self.ext
				)
			}
			None => {
				format!(
					"{}-{}.{}",
					self.artifact.replace('+', "_"),
					self.version,
					self.ext
				)
			}
		}
	}

	/// Relative repository path (e.g. `net/fabricmc/fabric-loader/0.16.5/fabric-loader-0.16.5.jar`).
	pub fn relative_path(&self) -> String {
		format!(
			"{}/{}/{}/{}",
			self.group_path(),
			self.artifact,
			self.version,
			self.filename()
		)
	}
}

/// Splits `version@ext` → `(version, ext)`. Defaults ext to `"jar"`.
pub fn split_version_ext(version: &str) -> (&str, &str) {
	if let Some(idx) = version.rfind('@') {
		let (ver, ext) = version.split_at(idx);
		(ver, &ext[1..])
	} else {
		(version, "jar")
	}
}

/// Converts Maven coordinates to a relative repository path.
pub fn coords_to_path(coords: &str) -> String {
	MavenCoords::parse(coords).relative_path()
}

/// Constructs a download URL from a Maven base URL and coordinates.
pub fn maven_url(
	base_url: &str,
	maven_coords: &str,
) -> String {
	let c = MavenCoords::parse(maven_coords);
	let base = base_url.trim_end_matches('/');
	format!(
		"{}/{}/{}/{}/{}-{}.jar",
		base,
		c.group_path(),
		c.artifact,
		c.version,
		c.artifact,
		c.version
	)
}

/// Extracts `artifact-version.jar` from Maven coordinates. Replaces `+` with `_`.
pub fn filename(coords: &str) -> String {
	let c = MavenCoords::parse(coords);
	format!("{}-{}.jar", c.artifact.replace('+', "_"), c.version)
}

/// Extracts the `artifact-version` stem (no extension). Used for classpath dedup.
pub fn artifact_version_stem(coords: &str) -> String {
	MavenCoords::parse(coords).artifact_version_stem()
}

pub fn parse_maven_versions(
	xml: &str,
	prefix_filter: Option<&str>,
) -> Vec<String> {
	let mut versions = Vec::new();

	for line in xml.lines() {
		let line = line.trim();
		if let Some(content) = line.strip_prefix("<version>") {
			if let Some(version) = content.strip_suffix("</version>") {
				if let Some(prefix) = prefix_filter {
					if version.starts_with(prefix) {
						versions.push(version.to_string());
					}
				} else {
					versions.push(version.to_string());
				}
			}
		}
	}

	versions
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_split_version_ext_default() {
		assert_eq!(split_version_ext("1.0.0"), ("1.0.0", "jar"));
	}

	#[test]
	fn test_split_version_ext_custom() {
		assert_eq!(split_version_ext("1.0.0@zip"), ("1.0.0", "zip"));
	}

	#[test]
	fn test_coords_to_path_simple() {
		let path = coords_to_path("net.fabricmc:fabric-loader:0.16.5");
		assert_eq!(
			path,
			"net/fabricmc/fabric-loader/0.16.5/fabric-loader-0.16.5.jar"
		);
	}

	#[test]
	fn test_coords_to_path_with_classifier() {
		let path = coords_to_path("net.neoforged:neoforge:21.1.172:installer");
		assert_eq!(
			path,
			"net/neoforged/neoforge/21.1.172/neoforge-21.1.172-installer.jar"
		);
	}

	#[test]
	fn test_coords_to_path_with_classifier_and_ext() {
		let path =
			coords_to_path("net.neoforged:neoforge:21.1.172:installer@jar");
		assert_eq!(
			path,
			"net/neoforged/neoforge/21.1.172/neoforge-21.1.172-installer.jar"
		);
	}

	#[test]
	fn test_maven_url() {
		let url = maven_url(
			"https://maven.fabricmc.net/",
			"net.fabricmc:fabric-loader:0.16.5",
		);
		assert_eq!(
			url,
			"https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.5/fabric-loader-0.16.5.jar"
		);
	}

	#[test]
	fn test_maven_url_no_trailing_slash() {
		let url = maven_url(
			"https://maven.fabricmc.net",
			"net.fabricmc:fabric-loader:0.16.5",
		);
		assert_eq!(
			url,
			"https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.5/fabric-loader-0.16.5.jar"
		);
	}

	#[test]
	fn test_filename() {
		assert_eq!(
			filename("net.fabricmc:fabric-loader:0.16.5"),
			"fabric-loader-0.16.5.jar"
		);
	}

	#[test]
	fn test_filename_with_plus() {
		assert_eq!(
			filename("net.fabricmc:fabric+api:0.16.5"),
			"fabric_api-0.16.5.jar"
		);
	}

	#[test]
	fn test_artifact_version_stem() {
		assert_eq!(
			artifact_version_stem("net.fabricmc:fabric-loader:0.16.5"),
			"fabric-loader-0.16.5"
		);
	}

	#[test]
	fn test_artifact_version_stem_with_plus() {
		assert_eq!(
			artifact_version_stem("net.fabricmc:fabric+api:0.16.5"),
			"fabric_api-0.16.5"
		);
	}

	#[test]
	fn test_parse_maven_versions_no_filter() {
		let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <versioning>
    <versions>
      <version>1.0.0</version>
      <version>2.0.0</version>
    </versions>
  </versioning>
</metadata>"#;
		let versions = parse_maven_versions(xml, None);
		assert_eq!(versions, vec!["1.0.0", "2.0.0"]);
	}

	#[test]
	fn test_parse_maven_versions_with_prefix_filter() {
		let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <versioning>
    <versions>
      <version>1.21.1-52.0.40</version>
      <version>1.21.1-52.0.39</version>
      <version>1.20.4-49.1.12</version>
    </versions>
  </versioning>
</metadata>"#;
		let versions = parse_maven_versions(xml, Some("1.21.1-"));
		assert_eq!(versions, vec!["1.21.1-52.0.40", "1.21.1-52.0.39"]);
	}

	#[test]
	fn test_parse_maven_versions_neoforge_style() {
		let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <versioning>
    <versions>
      <version>21.1.172</version>
      <version>21.1.171</version>
      <version>20.4.238</version>
    </versions>
  </versioning>
</metadata>"#;
		let versions = parse_maven_versions(xml, Some("21.1."));
		assert_eq!(versions, vec!["21.1.172", "21.1.171"]);
	}
}
