//! Virtual filesystem (VFS) tree that maps virtual paths to real source paths.
//! The tree is built in memory, then "realized" on disk by creating directories
//! and symlinks pointing back to the original source files.

use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum VfsEntry {
	Dir {
		children: BTreeMap<String, VfsEntry>,
	},
	File {
		source: PathBuf,
	},
}

#[derive(Debug, Clone)]
pub struct VfsTree {
	root: VfsEntry,
}

impl VfsTree {
	/// Creates an empty VFS tree with a root directory.
	pub fn new() -> Self {
		Self {
			root: VfsEntry::Dir {
				children: BTreeMap::new(),
			},
		}
	}

	/// Adds a file entry at `virtual_path` that points to `source` on disk.
	/// Intermediate directories are created automatically.
	pub fn add_file(
		&mut self,
		virtual_path: &Path,
		source: PathBuf,
	) {
		let mut current = &mut self.root;
		let components: Vec<&std::ffi::OsStr> =
			virtual_path.components().map(|c| c.as_os_str()).collect();

		for (i, component) in components.iter().enumerate() {
			let name = component.to_string_lossy().to_string();
			if i == components.len() - 1 {
				if let VfsEntry::Dir { children } = current {
					children.insert(
						name,
						VfsEntry::File {
							source: source.clone(),
						},
					);
				}
			} else {
				if let VfsEntry::Dir { children } = current {
					children.entry(name.clone()).or_insert_with(|| {
						VfsEntry::Dir {
							children: BTreeMap::new(),
						}
					});
					current = children.get_mut(&name).unwrap();
				}
			}
		}
	}

	/// Adds an empty directory entry at `virtual_path`.
	/// Intermediate parent directories are created automatically.
	pub fn add_dir(
		&mut self,
		virtual_path: &Path,
	) {
		let mut current = &mut self.root;
		for component in virtual_path.components() {
			let name = component.as_os_str().to_string_lossy().to_string();
			if let VfsEntry::Dir { children } = current {
				children
					.entry(name.clone())
					.or_insert_with(|| VfsEntry::Dir {
						children: BTreeMap::new(),
					});
				current = children.get_mut(&name).unwrap();
			}
		}
	}

	/// Adds a directory at `virtual_path` and recursively populates it from
	/// the real `source_dir`, mirroring its entire file tree into the VFS.
	pub fn add_dir_from_source(
		&mut self,
		virtual_path: &Path,
		source_dir: &Path,
	) {
		if !source_dir.exists() {
			return;
		}
		self.add_dir(virtual_path);
		self.populate_dir(virtual_path, source_dir);
	}

	fn populate_dir(
		&mut self,
		virtual_path: &Path,
		source_dir: &Path,
	) {
		let entries = match std::fs::read_dir(source_dir) {
			Ok(e) => e,
			Err(_) => return,
		};

		for entry in entries.flatten() {
			let name = entry.file_name().to_string_lossy().to_string();
			let src_path = entry.path();
			let virt_child = virtual_path.join(&name);

			if src_path.is_dir() {
				self.add_dir(&virt_child);
				self.populate_dir(&virt_child, &src_path);
			} else if src_path.is_file() {
				self.add_file(&virt_child, src_path);
			}
		}
	}

	pub fn root(&self) -> &VfsEntry {
		&self.root
	}
}

/// Realizes the VFS on disk: creates directories and symlinks pointing from
/// each virtual file path to its real source. Existing files are left untouched.
pub fn realize_vfs(
	tree: &VfsTree,
	target: &Path,
) -> Result<()> {
	std::fs::create_dir_all(target)?;
	realize_entry(tree.root(), target)?;
	crate::output::success("VFS realized (links)");
	Ok(())
}

fn realize_entry(
	entry: &VfsEntry,
	target: &Path,
) -> Result<()> {
	match entry {
		VfsEntry::Dir { children } => {
			std::fs::create_dir_all(target)?;
			for (name, child) in children {
				realize_entry(child, &target.join(name))?;
			}
		}
		VfsEntry::File { source } => {
			if let Some(parent) = target.parent() {
				std::fs::create_dir_all(parent)?;
			}
			// Create a symlink from target → canonical source path
			if !target.exists() {
				let source =
					source.canonicalize().unwrap_or_else(|_| source.clone());
				crate::utils::create_symlink(&source, target)?;
			}
		}
	}
	Ok(())
}
