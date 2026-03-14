//! Pure utility functions with no application state dependency.

pub mod format;
pub mod fs;
pub mod java;
pub mod maven;
pub mod slug;

pub use format::{
	format_size, print_error, system_time_to_date, today_iso8601, truncate_str,
};
pub use fs::{
	create_symlink, find_file_recursive, list_files, write_secret_file,
};
pub use java::{
	current_os_name, java_launch_prefix, ADD_OPENS_ARG, CLASSPATH_SEPARATOR,
};
pub use slug::slugify;
