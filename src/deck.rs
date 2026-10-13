//! Deck paths and the per-directory deck-id registry.
//!
//! The Anki deck path comes **only from directories** (file names are ignored),
//! so many files in one folder merge into one deck. A file's `topic:` adds one
//! extra subdeck segment. Deck ids are *not* derived — each directory keeps a
//! `deck-id.txt` holding the id of that directory's deck (a bare `<id>` line)
//! and of each `topic:` subdeck beneath it (`<id>::Topic`), minting a fresh
//! random id (same range as Python's `random.randrange(1 << 30, 1 << 31)`) the
//! first time a deck is seen, exactly like card `@id`.
//!
//! The key is relative to the directory on purpose: the full deck name depends
//! on the build root and `--deck`, so keying on it would mint a second id for
//! the same directory whenever either changes. Files written by earlier
//! versions *were* keyed by full name; those lines are adopted on first use
//! (see [`DeckRegistry::id_for`]).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::error::{AnkigenError, Result};
use crate::id::mint_deck_id;
use crate::util::atomic_write;

const DECK_ID_FILE: &str = "deck-id.txt";

/// The name of the build root `root` as the top deck segment: its directory
/// name, resolved through the filesystem for paths like `.` or `..` that do
/// not spell one (an empty path is the current directory). `None` only for a
/// root with no name at all (e.g. `/`).
pub fn root_name(root: &Path) -> Option<String> {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_string());
    let here = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    name(root).or_else(|| name(&std::fs::canonicalize(here).ok()?))
}

/// Build the deck path components for a file: the `top` segment (the build
/// root's name, or its `--deck` replacement), then the file's directory
/// components relative to `root`, then the file's `topic:`.
pub fn deck_components(
    file: &Path,
    root: &Path,
    topic: Option<&str>,
    top: Option<&str>,
) -> Vec<String> {
    let mut comps = Vec::new();
    if let Some(p) = top.filter(|p| !p.is_empty()) {
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
    entries: Vec<(Option<String>, i64)>, // (topic, or None for the directory's deck; id)
    legacy: Vec<(String, i64)>,          // (full deck name, id) lines not yet adopted
    dirty: bool,
}

impl DeckRegistry {
    pub fn new(persist: bool) -> Self {
        Self {
            dirs: HashMap::new(),
            persist,
        }
    }

    /// The id of `dir`'s deck (or of its `topic` subdeck), looked up in
    /// `dir`'s `deck-id.txt`, plus whether it was freshly minted (`true`) or
    /// already known (`false`). Mints + records a fresh id if absent.
    ///
    /// `deck_name` is the full deck name of this build; it is not a key, only
    /// a hint for adopting a legacy name-keyed line: one naming exactly this
    /// deck wins, otherwise any whose last segment is the topic (or the
    /// directory name). Legacy lines that disagree on the id are an error
    /// rather than a guess, since picking wrong would orphan a deck.
    pub fn id_for(
        &mut self,
        dir: &Path,
        topic: Option<&str>,
        deck_name: &str,
    ) -> Result<(i64, bool)> {
        let topic = topic.filter(|t| !t.is_empty());
        let reg = self
            .dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| load_dir(dir));
        if let Some((_, id)) = reg.entries.iter().find(|(t, _)| t.as_deref() == topic) {
            return Ok((*id, false));
        }

        let tail = match topic {
            Some(t) => Some(t.to_string()),
            None => dir.file_name().map(|n| n.to_string_lossy().to_string()),
        };
        let mut adopt: Vec<usize> = (0..reg.legacy.len())
            .filter(|&i| reg.legacy[i].0 == deck_name)
            .collect();
        if adopt.is_empty()
            && let Some(tail) = tail
        {
            let suffix = format!("::{tail}");
            adopt = (0..reg.legacy.len())
                .filter(|&i| reg.legacy[i].0 == tail || reg.legacy[i].0.ends_with(&suffix))
                .collect();
        }

        let (id, minted) = match adopt.first() {
            Some(&first) => {
                let id = reg.legacy[first].1;
                if adopt.iter().any(|&i| reg.legacy[i].1 != id) {
                    let mut ids: Vec<i64> = adopt.iter().map(|&i| reg.legacy[i].1).collect();
                    ids.dedup();
                    let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
                    return Err(AnkigenError::AmbiguousDeckId {
                        file: dir.join(DECK_ID_FILE).display().to_string(),
                        deck: match topic {
                            Some(t) => format!("topic `{t}`"),
                            None => "this directory's deck".to_string(),
                        },
                        ids: ids.join(", "),
                    });
                }
                let mut i = 0;
                reg.legacy.retain(|_| {
                    i += 1;
                    !adopt.contains(&(i - 1))
                });
                (id, false)
            }
            None => (mint_deck_id(), true),
        };
        reg.entries.push((topic.map(str::to_string), id));
        reg.dirty = true;
        Ok((id, minted))
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
    let mut legacy = Vec::new();
    if let Ok(content) = std::fs::read_to_string(dir.join(DECK_ID_FILE)) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // `<id>` or `<id>::<topic>`; anything else with a leading id is a
            // legacy `<id> <full deck name>` line.
            if let Ok(id) = line.parse::<i64>() {
                entries.push((None, id));
            } else if let Some((id_str, topic)) = line.split_once("::")
                && let Ok(id) = id_str.parse::<i64>()
            {
                entries.push((Some(topic.to_string()), id));
            } else if let Some((id_str, name)) = line.split_once(char::is_whitespace)
                && let Ok(id) = id_str.parse::<i64>()
            {
                legacy.push((name.trim().to_string(), id));
            }
        }
    }
    DirReg {
        entries,
        legacy,
        dirty: false,
    }
}

fn write_dir(dir: &Path, reg: &DirReg) -> Result<()> {
    let mut s = String::from(
        "# ankigen deck ids — keep this file so your Anki decks stay stable across rebuilds\n",
    );
    for (topic, id) in &reg.entries {
        match topic {
            Some(t) => s.push_str(&format!("{id}::{t}\n")),
            None => s.push_str(&format!("{id}\n")),
        }
    }
    for (name, id) in &reg.legacy {
        s.push_str(&format!("{id}\t{name}\n"));
    }
    atomic_write(&dir.join(DECK_ID_FILE), &s)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("ankigen-deck-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    #[test]
    fn directory_only_path() {
        let comps = deck_components(
            Path::new("cards/CS/lesson1.md"),
            Path::new("cards"),
            None,
            root_name(Path::new("cards")).as_deref(),
        );
        assert_eq!(deck_name(&comps), "cards::CS");
    }

    #[test]
    fn root_name_is_the_directory_name() {
        assert_eq!(
            root_name(Path::new("a/My Deck")).as_deref(),
            Some("My Deck")
        );
        assert_eq!(
            root_name(Path::new("a/My Deck/")).as_deref(),
            Some("My Deck")
        );
        // `.` spells no name, so it is resolved to the real directory.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            root_name(Path::new(".")),
            cwd.file_name().map(|n| n.to_string_lossy().to_string())
        );
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
    fn id_for_reports_new_then_existing() {
        let tmp = tmp_dir("new");
        let mut reg = DeckRegistry::new(false);
        let (id1, new1) = reg.id_for(&tmp, None, "CS").unwrap();
        assert!(new1, "first lookup mints a fresh id");
        let (id2, new2) = reg.id_for(&tmp, None, "CS").unwrap();
        assert!(!new2, "second lookup reuses the existing id");
        assert_eq!(id1, id2);
        let (_, new3) = reg.id_for(&tmp, Some("Net"), "CS::Net").unwrap();
        assert!(new3, "a topic subdeck is new again");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn id_is_independent_of_the_deck_name() {
        let tmp = tmp_dir("name");
        let mut reg = DeckRegistry::new(true);
        let (id, _) = reg.id_for(&tmp, None, "CS").unwrap();
        let (net, _) = reg.id_for(&tmp, Some("Net"), "CS::Net").unwrap();
        reg.flush().unwrap();
        let written = std::fs::read_to_string(tmp.join(DECK_ID_FILE)).unwrap();

        // Another root and a `--deck` prefix rename the deck, not its id.
        let mut reg = DeckRegistry::new(true);
        assert_eq!(
            reg.id_for(&tmp, None, "Exam::Uni::CS").unwrap(),
            (id, false)
        );
        assert_eq!(
            reg.id_for(&tmp, Some("Net"), "Exam::Uni::CS::Net").unwrap(),
            (net, false)
        );
        reg.flush().unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.join(DECK_ID_FILE)).unwrap(),
            written
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn legacy_name_keyed_lines_are_adopted() {
        let tmp = tmp_dir("legacy").join("CS");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join(DECK_ID_FILE),
            "# header\n11\tCS\n11\tUni::CS\n22\tUni::CS::Net\n33\tDefault\n",
        )
        .unwrap();
        let mut reg = DeckRegistry::new(true);
        // No line names `Other::CS`, so the directory name decides.
        assert_eq!(reg.id_for(&tmp, None, "Other::CS").unwrap(), (11, false));
        assert_eq!(
            reg.id_for(&tmp, Some("Net"), "Other::CS::Net").unwrap(),
            (22, false)
        );
        reg.flush().unwrap();
        let written = std::fs::read_to_string(tmp.join(DECK_ID_FILE)).unwrap();
        let lines: Vec<&str> = written.lines().skip(1).collect();
        // Adopted lines collapse to the bare-id form; unrelated ones are kept.
        assert_eq!(lines, ["11", "22::Net", "33\tDefault"]);
        let _ = std::fs::remove_dir_all(tmp.parent().unwrap());
    }

    #[test]
    fn exact_legacy_name_beats_the_directory_name() {
        let tmp = tmp_dir("exact");
        std::fs::write(tmp.join(DECK_ID_FILE), "33\tDefault\n").unwrap();
        let mut reg = DeckRegistry::new(false);
        assert_eq!(reg.id_for(&tmp, None, "Default").unwrap(), (33, false));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn conflicting_legacy_ids_are_an_error() {
        let tmp = tmp_dir("conflict").join("CS");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(DECK_ID_FILE), "11\tCS\n12\tUni::CS\n").unwrap();
        let mut reg = DeckRegistry::new(false);
        let err = reg.id_for(&tmp, None, "Other::CS").unwrap_err();
        assert!(matches!(err, AnkigenError::AmbiguousDeckId { .. }));
        // The build's own name still resolves it.
        assert_eq!(reg.id_for(&tmp, None, "Uni::CS").unwrap(), (12, false));
        let _ = std::fs::remove_dir_all(tmp.parent().unwrap());
    }

    #[test]
    fn deck_flag_names_the_root() {
        let comps = deck_components(
            Path::new("cards/top.md"),
            Path::new("cards"),
            None,
            Some("Exam"),
        );
        assert_eq!(deck_name(&comps), "Exam");
    }
}
