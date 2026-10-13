//! Semantic tokens: what to colour, as byte ranges. Like [`super::analysis`],
//! this is protocol-free; the server encodes the result.
//!
//! Tokens are produced by *painting* byte ranges of the document and reading
//! the runs back, so a later, more specific token (italic inside bold, a
//! cloze hint inside a deletion) simply overwrites part of an earlier one and
//! the output can never overlap.

use std::ops::Range;

use crate::model::{BlockSyntax, FieldSrc, LineKind, Marker, ParsedFile};
use crate::render::inline::Inline;
use crate::render::{Block, cloze, layout, walk_inlines};

use super::analysis::is_rich;

/// The token types the server emits. Most are standard LSP types, so any
/// colour scheme styles them; `Strong`, `Emphasis` and `Cloze` are ours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TokenKind {
    /// `//` comments and the body of an `@id` line.
    Comment = 1,
    /// A marker: `q:`, `a:`, `tags:`, …
    Keyword,
    /// `@id`.
    Decorator,
    /// A card's ULID.
    Number,
    /// `model` in `model=<key>`.
    Property,
    /// A `model=` value or a tag.
    EnumMember,
    /// A `topic:` name.
    Namespace,
    /// Cloze delimiters: `==`, `{{c1::`, `}}`.
    Operator,
    /// A cloze hint (`::hint`).
    Parameter,
    /// Inline code, fenced code, a link's URL, a `t:` answer.
    String,
    /// `$…$` / `$$…$$` math.
    Regexp,
    /// `[image:…]` / `[sound:…]`.
    Macro,
    /// `**bold**`.
    Strong,
    /// `*italic*`.
    Emphasis,
    /// The text of a cloze deletion.
    Cloze,
}

impl TokenKind {
    /// Every kind, in legend order: a token's index is `kind as u8 - 1`.
    pub const ALL: [TokenKind; 15] = [
        TokenKind::Comment,
        TokenKind::Keyword,
        TokenKind::Decorator,
        TokenKind::Number,
        TokenKind::Property,
        TokenKind::EnumMember,
        TokenKind::Namespace,
        TokenKind::Operator,
        TokenKind::Parameter,
        TokenKind::String,
        TokenKind::Regexp,
        TokenKind::Macro,
        TokenKind::Strong,
        TokenKind::Emphasis,
        TokenKind::Cloze,
    ];

    /// The LSP token type name.
    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Comment => "comment",
            TokenKind::Keyword => "keyword",
            TokenKind::Decorator => "decorator",
            TokenKind::Number => "number",
            TokenKind::Property => "property",
            TokenKind::EnumMember => "enumMember",
            TokenKind::Namespace => "namespace",
            TokenKind::Operator => "operator",
            TokenKind::Parameter => "parameter",
            TokenKind::String => "string",
            TokenKind::Regexp => "regexp",
            TokenKind::Macro => "macro",
            TokenKind::Strong => "strong",
            TokenKind::Emphasis => "emphasis",
            TokenKind::Cloze => "cloze",
        }
    }

    pub fn index(self) -> u32 {
        self as u32 - 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// A byte range of the document, within one line.
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// The tokens of a document, in source order and never overlapping.
pub fn semantic_tokens(text: &str, file: &ParsedFile) -> Vec<Token> {
    let mut canvas = Canvas {
        text,
        paint: vec![0; text.len()],
    };
    for block in &file.blocks {
        paint_block(&mut canvas, block);
    }
    canvas.tokens()
}

/// One byte of paint per byte of text: `0` for none, else a [`TokenKind`].
struct Canvas<'a> {
    text: &'a str,
    paint: Vec<u8>,
}

impl Canvas<'_> {
    fn paint(&mut self, range: Range<usize>, kind: TokenKind) {
        self.paint[range].fill(kind as u8);
    }

    /// Paint a range of a field's value, wherever its lines sit in the source.
    fn paint_field(&mut self, field: &FieldSrc, range: Range<usize>, kind: TokenKind) {
        for span in field.source_spans(range) {
            self.paint(span.byte_start..span.byte_start + span.byte_len, kind);
        }
    }

    fn tokens(&self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut start = 0;
        while start < self.paint.len() {
            let kind = self.paint[start];
            let mut end = start + 1;
            // Line breaks are never painted, so a run cannot span lines.
            while end < self.paint.len() && self.paint[end] == kind {
                end += 1;
            }
            // Whitespace left between two tokens has nothing to colour.
            if kind != 0 && !self.text[start..end].trim().is_empty() {
                tokens.push(Token {
                    range: start..end,
                    kind: TokenKind::ALL[kind as usize - 1],
                });
            }
            start = end;
        }
        debug_assert!(
            tokens
                .iter()
                .all(|t| !self.text[t.range.clone()].contains('\n'))
        );
        tokens
    }
}

fn paint_block(canvas: &mut Canvas, block: &BlockSyntax) {
    for line in &block.lines {
        let range = line.span.byte_start..line.span.byte_start + line.span.byte_len;
        canvas.paint(range.clone(), TokenKind::Comment);
        if line.kind == LineKind::Id {
            paint_id_line(canvas, range);
        }
    }
    for field in &block.fields {
        let start = field.marker_span.byte_start;
        canvas.paint(
            start..start + field.marker.prefix().len(),
            TokenKind::Keyword,
        );
        let text = &field.text;
        match field.marker {
            Marker::Tags => paint_words(canvas, field, TokenKind::EnumMember),
            Marker::Topic => paint_trimmed(canvas, field, TokenKind::Namespace),
            Marker::T => paint_trimmed(canvas, field, TokenKind::String),
            m => {
                debug_assert!(is_rich(m));
                let is_cloze = m == Marker::Q
                    && (cloze::has_shorthand_cloze(text) || cloze::has_native_cloze(text));
                let deletions = if is_cloze {
                    deletions(text)
                } else {
                    Vec::new()
                };
                // Deletion text first, so inline syntax inside it shows through.
                for d in &deletions {
                    canvas.paint_field(field, d.inner.clone(), TokenKind::Cloze);
                }
                paint_inlines(canvas, field);
                for d in &deletions {
                    canvas.paint_field(field, d.open.clone(), TokenKind::Operator);
                    canvas.paint_field(field, d.close.clone(), TokenKind::Operator);
                    if let Some(hint) = text[d.inner.clone()].find("::") {
                        let hint = d.inner.start + hint..d.inner.end;
                        canvas.paint_field(field, hint, TokenKind::Parameter);
                    }
                }
            }
        }
    }
}

/// Pick out the `@id`, the ULID and the `model=<key>` pair of an id line.
fn paint_id_line(canvas: &mut Canvas, line: Range<usize>) {
    let text = &canvas.text[line.clone()];
    let Some(at) = text.find("@id") else {
        return;
    };
    let mut words = Vec::new();
    let mut pos = at + 3;
    for word in text[pos..].split_whitespace() {
        let start = pos + text[pos..].find(word).unwrap_or(0);
        pos = start + word.len();
        words.push(line.start + start..line.start + pos);
    }
    canvas.paint(line.start + at..line.start + at + 3, TokenKind::Decorator);
    for (i, word) in words.into_iter().enumerate() {
        let w = &canvas.text[word.clone()];
        if i == 0 && w != "-->" {
            canvas.paint(word, TokenKind::Number);
        } else if let Some(value) = w.strip_prefix("model=") {
            let value_start = word.end - value.len();
            canvas.paint(word.start..word.start + 5, TokenKind::Property);
            canvas.paint(value_start..word.end, TokenKind::EnumMember);
        }
    }
}

fn paint_words(canvas: &mut Canvas, field: &FieldSrc, kind: TokenKind) {
    let mut pos = 0;
    for word in field.text.split_whitespace() {
        let start = pos + field.text[pos..].find(word).unwrap_or(0);
        pos = start + word.len();
        canvas.paint_field(field, start..pos, kind);
    }
}

fn paint_trimmed(canvas: &mut Canvas, field: &FieldSrc, kind: TokenKind) {
    let text = &field.text;
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    if start < end {
        canvas.paint_field(field, start..end, kind);
    }
}

fn paint_inlines(canvas: &mut Canvas, field: &FieldSrc) {
    walk_inlines(&field.text, &mut |node, range| {
        let (range, kind) = match node {
            Inline::Bold(_) => (range, TokenKind::Strong),
            Inline::Italic(_) => (range, TokenKind::Emphasis),
            Inline::Code(_) => (range, TokenKind::String),
            Inline::Math { .. } => (range, TokenKind::Regexp),
            Inline::Image { .. } | Inline::Sound { .. } => (range, TokenKind::Macro),
            // `[text](url)`: only the URL is coloured.
            Inline::Link { url, .. } => {
                (range.end - 1 - url.len()..range.end - 1, TokenKind::String)
            }
            Inline::Text(_) | Inline::Break => return,
        };
        canvas.paint_field(field, range, kind);
    });
    for block in layout(&field.text) {
        if let Block::Code { whole, .. } = block {
            canvas.paint_field(field, whole, TokenKind::String);
        }
    }
}

/// A cloze deletion's parts, as byte ranges of the field.
struct Deletion {
    open: Range<usize>,
    inner: Range<usize>,
    close: Range<usize>,
}

/// Every deletion in a cloze field, shorthand (`==…==`) and native
/// (`{{cN::…}}`) alike.
fn deletions(text: &str) -> Vec<Deletion> {
    let mut found: Vec<Deletion> = cloze::shorthand_spans(text)
        .into_iter()
        .map(|r| Deletion {
            open: r.start..r.start + 2,
            inner: r.start + 2..r.end - 2,
            close: r.end - 2..r.end,
        })
        .collect();

    let mut pos = 0;
    while let Some(at) = text[pos..].find("{{c") {
        let start = pos + at;
        let digits_start = start + 3;
        let digits = text[digits_start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        let open_end = digits_start + digits + 2;
        pos = digits_start;
        if digits == 0 || !text[digits_start + digits..].starts_with("::") {
            continue;
        }
        let Some(close) = text[open_end..].find("}}") else {
            continue;
        };
        let close = open_end + close;
        found.push(Deletion {
            open: start..open_end,
            inner: open_end..close,
            close: close..close + 2,
        });
        pos = close + 2;
    }
    found
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::source::cards::parse_lenient;

    fn tokens(text: &str) -> Vec<(&str, TokenKind)> {
        let file = parse_lenient(text, Path::new("t.cards")).file;
        semantic_tokens(text, &file)
            .into_iter()
            .map(|t| (&text[t.range], t.kind))
            .collect()
    }

    use TokenKind::*;

    #[test]
    fn structure_and_directives() {
        assert_eq!(
            tokens(
                "topic: Net work\ntags: a b::c\n\n// note\n// @id 01ABC model=type-in\nq: Q\nt:  200 \n"
            ),
            [
                ("topic:", Keyword),
                ("Net work", Namespace),
                ("tags:", Keyword),
                ("a", EnumMember),
                ("b::c", EnumMember),
                ("// note", Comment),
                ("// ", Comment),
                ("@id", Decorator),
                ("01ABC", Number),
                ("model", Property),
                ("=", Comment),
                ("type-in", EnumMember),
                ("q:", Keyword),
                ("t:", Keyword),
                ("200", String),
            ]
        );
        // The legacy spelling, and no id to mistake `-->` for.
        assert_eq!(
            tokens("<!-- @id 01ABC -->\nq: Q\na: A\n")[..4],
            [
                ("<!-- ", Comment),
                ("@id", Decorator),
                ("01ABC", Number),
                (" -->", Comment)
            ]
        );
    }

    #[test]
    fn inline_syntax_nests_without_overlap() {
        assert_eq!(
            tokens("q: a **b *c* d** `e` $f$\na: [x](http://y) [image:cap:p.png] \\*z\n"),
            [
                ("q:", Keyword),
                ("**b ", Strong),
                ("*c*", Emphasis),
                (" d**", Strong),
                ("`e`", String),
                ("$f$", Regexp),
                ("a:", Keyword),
                ("http://y", String),
                ("[image:cap:p.png]", Macro),
            ]
        );
    }

    #[test]
    fn code_fences_and_multi_line_fields() {
        assert_eq!(
            tokens("q: Q\na: see **this\nthing**\n```rust\n// not a comment\n```\n- `x`\n"),
            [
                ("q:", Keyword),
                ("a:", Keyword),
                ("**this", Strong),
                ("thing**", Strong),
                ("```rust", String),
                ("// not a comment", String),
                ("```", String),
                ("`x`", String),
            ]
        );
    }

    #[test]
    fn cloze_deletions() {
        assert_eq!(
            tokens("q: ==Paris::capital== is in ==**France**==\ne: ==not cloze==\n"),
            [
                ("q:", Keyword),
                ("==", Operator),
                ("Paris", Cloze),
                ("::capital", Parameter),
                ("==", Operator),
                ("==", Operator),
                ("**France**", Strong),
                ("==", Operator),
                ("e:", Keyword),
            ]
        );
        assert_eq!(
            tokens("q: {{c1::Rome}} and {{c22::x::hint}}\n"),
            [
                ("q:", Keyword),
                ("{{c1::", Operator),
                ("Rome", Cloze),
                ("}}", Operator),
                ("{{c22::", Operator),
                ("x", Cloze),
                ("::hint", Parameter),
                ("}}", Operator),
            ]
        );
    }

    #[test]
    fn blocks_that_fail_to_parse_are_still_highlighted() {
        // No answer yet: an error, but the card being typed keeps its colours.
        assert_eq!(
            tokens("q: **new**\n"),
            [("q:", Keyword), ("**new**", Strong)]
        );
    }
}
