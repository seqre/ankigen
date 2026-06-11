//! Deck paths and the per-directory deck-id registry.
//!
//! The Anki deck path comes **only from directories** (file names are ignored),
//! so many files in one folder merge into one deck. A file's `topic:` adds one
//! extra subdeck segment. Deck ids are *not* derived — each directory keeps a
//! `deck-id.txt` mapping deck path → id, minting a fresh random id (same range
//! as Python's `random.randrange(1 << 30, 1 << 31)`) the first time a deck is
//! seen, exactly like card `@id`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::error::Result;
use crate::id::mint_deck_id;
use crate::util::atomic_write;

const DECK_ID_FILE: &str = "deck-id.txt";

/// Build the deck path components for a file: optional `--deck` prefix, then the
/// file's directory components relative to `root`, then the file's `topic:`.
pub fn deck_components(
    file: &Path,
    root: &Path,
    topic: Option<&str>,
    prefix: Option<&str>,
) -> Vec<String> {
    let mut comps = Vec::new();
    if let Some(p) = prefix.filter(|p| !p.is_empty()) {
        comps.push(p.to_string());
    }
    if let Some(parent) = file.parent()
        && let Ok(rel) = parent.strip_prefix(root)
    {
        for c in rel.components() {
            if let Component::Normal(os) = c {
                comps.push(os.to_string_lossy().to_string());
            }
        }
    }
    if let Some(t) = topic.filter(|t| !t.is_empty()) {
        comps.push(t.to_string());
    }
    comps
}

/// Join components into an Anki deck name (`::` separated), or `Default`.
pub fn deck_name(components: &[String]) -> String {
    if components.is_empty() {
        "Default".to_string()
    } else {
        components.join("::")
    }
}

/// Per-directory `deck-id.txt` registry. Mints stable ids and (optionally)
/// writes them back.
pub struct DeckRegistry {
    dirs: HashMap<PathBuf, DirReg>,
    persist: bool,
}

struct DirReg {
    entries: Vec<(String, i64)>, // (deck path, id), order preserved
    dirty: bool,
}

impl DeckRegistry {
    pub fn new(persist: bool) -> Self {
        Self {
            dirs: HashMap::new(),
            persist,
        }
    }

    /// The id for `deck_path`, looked up in `dir`'s `deck-id.txt`. Mints + records
    /// a fresh id if absent.
    pub fn id_for(&mut self, dir: &Path, deck_path: &str) -> i64 {
        let reg = self
            .dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| load_dir(dir));
        if let Some((_, id)) = reg.entries.iter().find(|(p, _)| p == deck_path) {
            return *id;
        }
        let id = mint_deck_id();
        reg.entries.push((deck_path.to_string(), id));
        reg.dirty = true;
        id
    }

    /// Write back any directories whose registry gained new decks.
    pub fn flush(&self) -> Result<()> {
        if !self.persist {
            return Ok(());
        }
        for (dir, reg) in &self.dirs {
            if reg.dirty {
                write_dir(dir, reg)?;
            }
        }
        Ok(())
    }
}

fn load_dir(dir: &Path) -> DirReg {
    let mut entries = Vec::new();
    if let Ok(content) = std::fs::read_to_string(dir.join(DECK_ID_FILE)) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((id_str, name)) = line.split_once(char::is_whitespace)
                && let Ok(id) = id_str.trim().parse::<i64>()
            {
                entries.push((name.trim().to_string(), id));
            }
        }
    }
    DirReg {
        entries,
        dirty: false,
    }
}

fn write_dir(dir: &Path, reg: &DirReg) -> Result<()> {
    let mut s = String::from(
        "# ankigen deck ids — keep this file so your Anki decks stay stable across rebuilds\n",
    );
    for (name, id) in &reg.entries {
        s.push_str(&format!("{id}\t{name}\n"));
    }
    atomic_write(&dir.join(DECK_ID_FILE), &s)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_only_path() {
        let comps = deck_components(
            Path::new("cards/CS/lesson1.md"),
            Path::new("cards"),
            None,
            None,
        );
        assert_eq!(deck_name(&comps), "CS");
    }

    #[test]
    fn topic_adds_segment() {
        let comps = deck_components(
            Path::new("cards/CS/networking.md"),
            Path::new("cards"),
            Some("Networking"),
            None,
        );
        assert_eq!(deck_name(&comps), "CS::Networking");
    }

    #[test]
    fn prefix_and_root_file() {
        let comps = deck_components(
            Path::new("cards/top.md"),
            Path::new("cards"),
            None,
            Some("Exam"),
        );
        assert_eq!(deck_name(&comps), "Exam");
    }
}
