//! Parser output types: cards plus the source locations needed for
//! diagnostics and id write-back. These are not serialized.

use std::path::PathBuf;

use super::spec::{CardSpec, ModelKey};

/// A byte range into a single source file (plus a 1-based line for messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub byte_start: usize,
    pub byte_len: usize,
    pub line_start: u32,
}

/// How a card's `@id` comment is spelled in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdSyntax {
    /// `// @id …` — the canonical form written by write-back.
    Slash,
    /// `<!-- @id … -->` — the legacy form: still read, and rewritten to
    /// [`IdSyntax::Slash`] by `--upgrade-source`.
    Html,
}

#[derive(Clone, Debug)]
pub struct ParsedCard {
    pub spec: CardSpec,
    /// Span of the whole block, used as the write-back anchor.
    pub block_span: SourceSpan,
    /// `false` ⇒ this card had no `@id` comment and one must be injected.
    pub id_present: bool,
    /// Span of the existing `@id` line, if present. Used to rewrite that line
    /// in place when pinning a `model=` suffix or upgrading its syntax.
    pub id_span: Option<SourceSpan>,
    /// Which comment syntax the existing `@id` line uses, if present.
    pub id_syntax: Option<IdSyntax>,
    /// The `model=<key>` suffix parsed from the `@id` comment, if present. This
    /// is the durable, source-of-truth note type for ambiguous `q:/a:` cards.
    pub model_marker: Option<ModelKey>,
    /// The note type resolved for this card by the pipeline (see
    /// `pipeline::resolve_models`). `None` until resolved.
    pub resolved_model: Option<ModelKey>,
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
