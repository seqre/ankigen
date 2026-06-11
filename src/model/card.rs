//! Parser output types: cards plus the source locations needed for
//! diagnostics and id write-back. These are not serialized.

use std::path::PathBuf;

use super::spec::CardSpec;

/// A byte range into a single source file (plus a 1-based line for messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub byte_start: usize,
    pub byte_len: usize,
    pub line_start: u32,
}

#[derive(Clone, Debug)]
pub struct ParsedCard {
    pub spec: CardSpec,
    /// Span of the whole block, used as the write-back anchor.
    pub block_span: SourceSpan,
    /// `false` ⇒ this card had no `<!-- @id -->` and one must be injected.
    pub id_present: bool,
}

#[derive(Clone, Debug)]
pub struct ParsedFile {
    pub path: PathBuf,
    pub text: String,
    /// File-level `topic:` directive, if any (adds one subdeck segment).
    pub topic: Option<String>,
    pub file_tags: Vec<String>,
    pub cards: Vec<ParsedCard>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
}
