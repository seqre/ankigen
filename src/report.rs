//! Verbosity-gated build reporting.
//!
//! The build is silent by default. A `Reporter` accumulates counts as the
//! pipeline runs and writes to **stderr** (stdout stays clean), gated on a
//! verbosity level: `0` silent, `1` an end-of-build summary, `2`+ per-item
//! lines as events happen.

use std::path::Path;

/// Accumulates build statistics and emits stderr output gated on `level`.
#[derive(Default)]
pub struct Reporter {
    level: u8,
    source_files: usize,
    decks: usize,
    new_decks: usize,
    total_cards: usize,
    new_cards: usize,
    written_back: usize,
    media: usize,
    orphaned: usize,
}

impl Reporter {
    pub fn new(level: u8) -> Self {
        Self {
            level,
            ..Self::default()
        }
    }

    /// Whether any reporting is requested (level ≥ 1). Used to skip the extra
    /// work (media listing, orphan scan) that only feeds the summary.
    pub fn enabled(&self) -> bool {
        self.level >= 1
    }

    pub fn source_file(&mut self, path: &Path) {
        self.source_files += 1;
        if self.level >= 2 {
            eprintln!("  read    {}", path.display());
        }
    }

    pub fn deck(&mut self, name: &str, id: i64, is_new: bool) {
        self.decks += 1;
        if is_new {
            self.new_decks += 1;
        }
        if self.level >= 2 {
            let tag = if is_new { "new" } else { "existing" };
            eprintln!("  deck    {name}  (id {id}, {tag})");
        }
    }

    pub fn card(&mut self, id: &str, is_new: bool, file: &Path) {
        self.total_cards += 1;
        if is_new {
            self.new_cards += 1;
            if self.level >= 2 {
                eprintln!("  card +  {id}  {}", file.display());
            }
        }
    }

    pub fn write_back(&mut self, path: &Path) {
        self.written_back += 1;
        if self.level >= 2 {
            eprintln!("  wrote   {}", path.display());
        }
    }

    pub fn media(&mut self, basename: &str, src: &Path) {
        self.media += 1;
        if self.level >= 2 {
            eprintln!("  media   {basename}  <- {}", src.display());
        }
    }

    pub fn orphan(&mut self, path: &Path) {
        self.orphaned += 1;
        if self.level >= 2 {
            eprintln!("  orphan  {}", path.display());
        }
    }

    /// Print the end-of-build summary (level ≥ 1).
    pub fn finish(&self, check: bool) {
        if self.level < 1 {
            return;
        }
        eprintln!("ankigen build summary:");
        eprintln!("  source files  {}", self.source_files);
        eprintln!("  decks         {} ({} new)", self.decks, self.new_decks);
        eprintln!(
            "  cards         {} ({} new)",
            self.total_cards, self.new_cards
        );
        if check {
            eprintln!("  (check mode: no @id, deck-id.txt, or .apkg written)");
        } else {
            eprintln!("  written back  {} file(s)", self.written_back);
        }
        eprintln!("  media         {} file(s)", self.media);
        eprintln!("  orphaned      {}", self.orphaned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_accumulate() {
        let mut r = Reporter::new(1);
        r.source_file(Path::new("a.md"));
        r.deck("CS", 1, true);
        r.deck("CS::Net", 2, false);
        r.card("01", true, Path::new("a.md"));
        r.card("02", false, Path::new("a.md"));
        r.media("ab-cat.png", Path::new("/x/cat.png"));
        r.orphan(Path::new("/x/dog.png"));
        assert_eq!(r.source_files, 1);
        assert_eq!(r.decks, 2);
        assert_eq!(r.new_decks, 1);
        assert_eq!(r.total_cards, 2);
        assert_eq!(r.new_cards, 1);
        assert_eq!(r.media, 1);
        assert_eq!(r.orphaned, 1);
    }

    #[test]
    fn enabled_tracks_level() {
        assert!(!Reporter::new(0).enabled());
        assert!(Reporter::new(1).enabled());
        assert!(Reporter::new(2).enabled());
    }
}
