use std::collections::BTreeMap;
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::source::{hash_image, invalid, is_reparse_point, open_image, same_file_state};

const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_RELATIVE_PATH_BYTES: usize = 4096;

#[derive(Debug, Serialize)]
pub struct ExportInventory {
    pub schema_version: u32,
    pub engine_version: &'static str,
    pub files: BTreeMap<String, ExportFile>,
    pub skipped: BTreeMap<String, &'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExportFile {
    pub length: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct ExportComparison {
    pub schema_version: u32,
    pub engine_version: &'static str,
    pub before_skipped: BTreeMap<String, &'static str>,
    pub after_skipped: BTreeMap<String, &'static str>,
    pub paths: BTreeMap<String, PathObservation>,
    pub interpretation: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PathObservation {
    pub observation: &'static str,
    pub before: Option<ExportFile>,
    pub after: Option<ExportFile>,
}

struct InventoryWalker {
    files: BTreeMap<String, ExportFile>,
    skipped: BTreeMap<String, &'static str>,
    entries: usize,
    root: Metadata,
}

impl InventoryWalker {
    fn visit(
        &mut self,
        path: &Path,
        relative: &str,
        depth: usize,
        metadata: &Metadata,
    ) -> io::Result<()> {
        if depth >= MAX_DEPTH {
            return Err(invalid("logical export exceeds directory depth limit"));
        }
        for item in fs::read_dir(path)? {
            let item = item?;
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                return Err(invalid("logical export exceeds entry limit"));
            }
            let name = item.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid("logical export contains a non-UTF-8 name"))?;
            let key = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if key.len() > MAX_RELATIVE_PATH_BYTES {
                return Err(invalid("logical export exceeds relative path length limit"));
            }
            let child = item.path();
            let state = fs::symlink_metadata(&child)?;
            if state.file_type().is_symlink() || is_reparse_point(&state) {
                self.skipped.insert(key, "link_or_reparse_point");
            } else if !on_root_device(&self.root, &state) {
                self.skipped.insert(key, "different_filesystem");
            } else if state.is_dir() {
                self.visit(&child, &key, depth + 1, &state)?;
            } else if state.is_file() {
                let mut file = open_image(&child)?;
                if !same_file_state(&state, &file.metadata()?) {
                    return Err(invalid("logical export changed while opening an entry"));
                }
                let sha256 = hash_image(&mut file, state.len())?;
                if !same_file_state(&state, &file.metadata()?) {
                    return Err(invalid("logical export entry changed during hashing"));
                }
                self.files.insert(
                    key,
                    ExportFile {
                        length: state.len(),
                        sha256,
                    },
                );
            } else {
                self.skipped.insert(key, "non_regular_entry");
            }
        }
        let after = fs::symlink_metadata(path)?;
        if !after.is_dir() || is_reparse_point(&after) || !same_file_state(metadata, &after) {
            return Err(invalid("logical export directory changed during traversal"));
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn on_root_device(root: &Metadata, entry: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    root.dev() == entry.dev()
}

#[cfg(not(unix))]
pub(crate) fn on_root_device(_: &Metadata, _: &Metadata) -> bool {
    true
}

/// Hash files in an immutable, previously acquired logical export directory.
pub fn inventory(directory: &Path) -> io::Result<ExportInventory> {
    let root = fs::symlink_metadata(directory)?;
    if !root.is_dir() || is_reparse_point(&root) {
        return Err(invalid("logical export must be a regular directory"));
    }
    let mut walker = InventoryWalker {
        files: BTreeMap::new(),
        skipped: BTreeMap::new(),
        entries: 0,
        root: root.clone(),
    };
    walker.visit(directory, "", 0, &root)?;
    Ok(ExportInventory {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION"),
        files: walker.files,
        skipped: walker.skipped,
    })
}

/// Compare accessible logical files by relative path and SHA-256.
pub fn compare_exports(before: &Path, after: &Path) -> io::Result<ExportComparison> {
    let before = inventory(before)?;
    let after = inventory(after)?;
    let mut paths = BTreeMap::new();
    let mut remaining = after.files;
    for (path, file) in before.files {
        let after_file = remaining.remove(&path);
        let observation = match &after_file {
            Some(other) if file == *other => "same_bytes_at_path",
            Some(_) => "different_bytes_at_path",
            None if unavailable(&after.skipped, &path) => "after_path_not_inspected",
            None => "observed_before_only",
        };
        paths.insert(
            path,
            PathObservation {
                observation,
                before: Some(file),
                after: after_file,
            },
        );
    }
    for (path, file) in remaining {
        let observation = if unavailable(&before.skipped, &path) {
            "before_path_not_inspected"
        } else {
            "observed_after_only"
        };
        paths.insert(
            path,
            PathObservation {
                observation,
                before: None,
                after: Some(file),
            },
        );
    }
    Ok(ExportComparison {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION"),
        before_skipped: before.skipped,
        after_skipped: after.skipped,
        paths,
        interpretation: "Capture order and scope are supplied by the operator. Path absence may reflect different acquisition permissions, scope, or directory structure.",
    })
}

fn unavailable(skipped: &BTreeMap<String, &'static str>, path: &str) -> bool {
    path.match_indices('/')
        .any(|(index, _)| skipped.contains_key(&path[..index]))
        || skipped.contains_key(path)
}
