//! Resolve media references to collision-proof package basenames.
//!
//! A basename is `"<8 hex of blake3(contents)>-<original name>"`, which is
//! unique across the package (two `cat.png` in different folders get different
//! prefixes) yet de-duplicates byte-identical files (same prefix + name ⇒ same
//! basename ⇒ packaged once).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct MediaResolver {
    by_abs: HashMap<PathBuf, String>,
    used: HashSet<String>,
    files: Vec<PathBuf>,
}

impl MediaResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve `raw` relative to `src_dir`, returning the package basename.
    /// Returns `Err` (with the io error) if the file is missing/unreadable.
    pub fn resolve(&mut self, src_dir: &Path, raw: &str) -> std::io::Result<String> {
        let abs = std::fs::canonicalize(src_dir.join(raw))?;
        if let Some(basename) = self.by_abs.get(&abs) {
            return Ok(basename.clone());
        }
        let bytes = std::fs::read(&abs)?;
        let hash = blake3::hash(&bytes);
        let prefix = &hash.to_hex()[..8];
        let orig = abs
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("media");
        let basename = format!("{prefix}-{orig}");
        self.by_abs.insert(abs.clone(), basename.clone());
        // Package each distinct basename exactly once.
        if self.used.insert(basename.clone()) {
            self.files.push(abs);
        }
        Ok(basename)
    }

    /// Absolute paths to hand to `Package::new`, as `&str`.
    pub fn media_paths(&self) -> Vec<&str> {
        self.files.iter().filter_map(|p| p.to_str()).collect()
    }
}
