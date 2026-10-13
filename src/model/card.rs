//! Parser output types: cards plus the source locations needed for
//! diagnostics and id write-back. These are not serialized.

use std::ops::Range;
use std::path::PathBuf;

use super::spec::{CardSpec, ModelKey};

/// A byte range into a single source file (plus a 1-based line for messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub byte_start: usize,
    pub byte_len: usize,
    pub line_start: u32,
}

/// A located problem found while parsing or validating a source file. The
/// build turns these into [`crate::AnkigenError`]s; the language server into
/// editor diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub span: SourceSpan,
    pub message: String,
    pub help: Option<String>,
}

/// A column-0 line prefix that starts a field or directive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Q,
    A,
    E,
    T,
    R,
    Tags,
    Topic,
}

impl Marker {
    pub const ALL: [Marker; 7] = [
        Marker::Q,
        Marker::A,
        Marker::E,
        Marker::T,
        Marker::R,
        Marker::Tags,
        Marker::Topic,
    ];

    /// The marker as written, colon included.
    pub fn prefix(self) -> &'static str {
        match self {
            Marker::Q => "q:",
            Marker::A => "a:",
            Marker::E => "e:",
            Marker::T => "t:",
            Marker::R => "r:",
            Marker::Tags => "tags:",
            Marker::Topic => "topic:",
        }
    }

    /// A card field (which may continue over following lines), as opposed to
    /// the single-line `tags:`/`topic:` directives.
    pub fn is_field(self) -> bool {
        !matches!(self, Marker::Tags | Marker::Topic)
    }
}

/// One marker's value as it sits in the source.
#[derive(Clone, Debug)]
pub struct FieldSrc {
    pub marker: Marker,
    /// The line carrying the marker.
    pub marker_span: SourceSpan,
    /// The source range of each line of the value, in order: the rest of the
    /// marker line, then every continuation line (comments are skipped).
    pub pieces: Vec<SourceSpan>,
    /// The value: the `pieces` joined by `\n`.
    pub text: String,
}

impl FieldSrc {
    /// Map a byte range of `text` back to the source: one span per line it
    /// touches.
    pub fn source_spans(&self, range: Range<usize>) -> Vec<SourceSpan> {
        let mut spans = Vec::new();
        let mut start = 0;
        for piece in &self.pieces {
            let end = start + piece.byte_len;
            let (lo, hi) = (range.start.max(start), range.end.min(end));
            if lo < hi {
                spans.push(SourceSpan {
                    byte_start: piece.byte_start + (lo - start),
                    byte_len: hi - lo,
                    line_start: piece.line_start,
                });
            }
            start = end + 1;
        }
        spans
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// A `//` comment.
    Comment,
    /// An `@id` comment, in either spelling.
    Id,
}

/// A whole line that is not field content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxLine {
    pub span: SourceSpan,
    pub kind: LineKind,
}

/// The lexical structure of one blank-line-separated block. Recorded for every
/// block, including ones that failed to parse into a card.
#[derive(Clone, Debug)]
pub struct BlockSyntax {
    pub span: SourceSpan,
    pub lines: Vec<SyntaxLine>,
    pub fields: Vec<FieldSrc>,
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
    /// is the durable, source-of-truth note type; write-back adds it to every card.
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
    /// Every block of the file, in order (see [`BlockSyntax`]).
    pub blocks: Vec<BlockSyntax>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
}
