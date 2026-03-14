//! Slugify: convert names to filesystem-safe directory names.
//! Falls back to `"unnamed"` if the result would be empty.

/// Convert a string into a filesystem-safe slug.
///
/// Examples:
/// - `"Fabric API"` → `"fabric-api"`
/// - `"Sodium (Reforged)"` → `"sodium-reforged"`
/// - `"!!!"` → `"unnamed"`
pub fn slugify(s: &str) -> String {
	let mut result = String::with_capacity(s.len());
	let mut prev_dash = true;

	for c in s.to_lowercase().chars() {
		if c.is_alphanumeric() {
			result.push(c);
			prev_dash = false;
		} else if !prev_dash {
			result.push('-');
			prev_dash = true;
		}
	}

	if result.ends_with('-') {
		result.pop();
	}

	if result.is_empty() {
		"unnamed".to_string()
	} else {
		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_slugify_basic() {
		assert_eq!(slugify("Hello World"), "hello-world");
	}

	#[test]
	fn test_slugify_special_chars() {
		let slug = slugify("Test Mod!@#$%");
		assert!(!slug.contains('!'));
		assert!(!slug.contains('@'));
		assert!(!slug.contains('#'));
		assert!(!slug.contains('$'));
		assert!(!slug.contains('%'));
	}

	#[test]
	fn test_slugify_numbers() {
		assert_eq!(slugify("Mod 123"), "mod-123");
	}

	#[test]
	fn test_slugify_empty() {
		assert_eq!(slugify(""), "unnamed");
	}

	#[test]
	fn test_slugify_only_special() {
		assert_eq!(slugify("!!!"), "unnamed");
	}

	#[test]
	fn test_slugify_multiple_spaces() {
		assert_eq!(slugify("hello   world"), "hello-world");
	}

	#[test]
	fn test_slugify_leading_trailing_special() {
		assert_eq!(slugify("---hello---"), "hello");
	}

	#[test]
	fn test_slugify_already_slugified() {
		assert_eq!(slugify("my-mod-pack"), "my-mod-pack");
	}

	#[test]
	fn test_slugify_uppercase() {
		assert_eq!(slugify("FABRIC API"), "fabric-api");
	}
}
