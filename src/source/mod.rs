//! Front-ends. A [`CardSource`] turns raw input into the canonical
//! [`ParsedFile`] and knows how to persist freshly minted ids back into that
//! format. The native [`CardsSource`] is the only implementation today; a
//! YAML/CSV front-end would be a second impl, leaving the build pipeline
//! untouched.

pub mod cards;
pub mod writeback;

use std::path::Path;

use crate::error::Result;
use crate::model::ParsedFile;

pub trait CardSource {
    fn format_id(&self) -> &'static str;
    fn parse(&self, input: &str, path: &Path) -> Result<ParsedFile>;
    /// Rewrite `input` to persist minted ids (and, with `upgrade_source`,
    /// convert legacy syntax to its canonical form), or `None` if nothing changed.
    fn persist_ids(&self, input: &str, parsed: &ParsedFile, upgrade_source: bool)
    -> Option<String>;
}

pub struct CardsSource;

impl CardSource for CardsSource {
    fn format_id(&self) -> &'static str {
        "cards"
    }

    fn parse(&self, input: &str, path: &Path) -> Result<ParsedFile> {
        cards::parse(input, path)
    }

    fn persist_ids(
        &self,
        input: &str,
        parsed: &ParsedFile,
        upgrade_source: bool,
    ) -> Option<String> {
        writeback::inject_ids(input, parsed, upgrade_source)
    }
}
