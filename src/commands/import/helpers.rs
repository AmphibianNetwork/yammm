pub fn extract_slug_from_path(path: &str) -> String {
	let path = path.replace('\\', "/");
	let filename = path.rsplit('/').next().unwrap_or(path.as_str());
	let stem = filename
		.strip_suffix(".jar")
		.or_else(|| filename.strip_suffix(".zip"))
		.unwrap_or(filename);
	let parts: Vec<&str> = stem.split('-').collect();
	for i in 1..parts.len() {
		if parts[i].chars().next().is_some_and(|c| c.is_ascii_digit()) {
			return parts[..i].join("-");
		}
	}
	stem.to_string()
}

pub fn extract_version_from_path(path: &str) -> String {
	let path = path.replace('\\', "/");
	let filename = path.rsplit('/').next().unwrap_or(path.as_str());
	let filename = filename
		.strip_suffix(".jar")
		.or_else(|| filename.strip_suffix(".zip"))
		.unwrap_or(filename);
	let parts: Vec<&str> = filename.split('-').collect();
	for i in 1..parts.len() {
		let candidate = parts[i..].join("-");
		if candidate.chars().next().is_some_and(|c| c.is_ascii_digit()) {
			return candidate;
		}
	}
	"0.0.0".to_string()
}
