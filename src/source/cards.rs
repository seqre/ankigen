//! Parser for the native `cards` text format.
//!
//! Splits a file into blank-line-separated blocks, reads an optional leading
//! `// @id …` (or legacy `<!-- @id … -->`), collects `q:`/`a:`/`r:`/`e:`/`t:`
//! markers (with multi-line continuation) and `topic:`/`tags:` directives, then
//! infers the card type. Byte offsets are tracked throughout so diagnostics
//! point at the offending line. (Inline field syntax is parsed separately by
//! the winnow grammar in [`crate::render`].)
//!
//! Parsing recovers per block: a block with a problem yields [`Diag`]s instead
//! of a card and the scan carries on, so one pass reports every error in a
//! file. [`parse_lenient`] exposes that (plus the per-block [`BlockSyntax`]
//! the language server highlights from); [`parse`] is the strict wrapper the
//! build uses.

use std::path::Path;

use crate::error::{AnkigenError, Result};
use crate::model::card::SourceSpan;
use crate::model::{
    BlockSyntax, CardKindSpec, CardSpec, Diag, FieldSrc, IdSyntax, LineKind, Marker, ModelKey,
    ParsedCard, ParsedFile, SyntaxLine,
};
use crate::render::cloze;

/// The outcome of a recovering parse: every card from the blocks that parsed
/// cleanly, and a [`Diag`] for each problem in the ones that did not.
pub struct Parsed {
    pub file: ParsedFile,
    pub diags: Vec<Diag>,
}

/// Parse `input`, failing with every problem found (not just the first).
pub fn parse(input: &str, path: &Path) -> Result<ParsedFile> {
    let Parsed { file, diags } = parse_lenient(input, path);
    if diags.is_empty() {
        return Ok(file);
    }
    Err(AnkigenError::many(
        diags
            .iter()
            .map(|d| AnkigenError::from_diag(path, input, d))
            .collect(),
    ))
}

pub fn parse_lenient(input: &str, path: &Path) -> Parsed {
    let mut topic: Option<String> = None;
    let mut file_tags: Vec<String> = Vec::new();
    let mut cards: Vec<ParsedCard> = Vec::new();
    let mut blocks: Vec<BlockSyntax> = Vec::new();
    let mut diags: Vec<Diag> = Vec::new();

    for block in split_blocks(input) {
        let (syntax, outcome) = parse_block(&block, &mut diags);
        blocks.push(syntax);
        match outcome {
            Some(Outcome::Card(card)) => cards.push(card),
            Some(Outcome::Directive {
                topic: t,
                tags: mut tg,
            }) => {
                if let Some(t) = t
                    && topic.is_none()
                {
                    topic = Some(t);
                }
                file_tags.append(&mut tg);
            }
            Some(Outcome::Empty) | None => {}
        }
    }

    Parsed {
        file: ParsedFile {
            path: path.to_path_buf(),
            text: input.to_string(),
            topic,
            file_tags,
            cards,
            blocks,
        },
        diags,
    }
}

struct RawBlock {
    text: String,
    byte_start: usize,
    byte_len: usize,
    line_start: u32,
}

impl RawBlock {
    fn span(&self) -> SourceSpan {
        SourceSpan {
            byte_start: self.byte_start,
            byte_len: self.byte_len,
            line_start: self.line_start,
        }
    }
}

/// Split into maximal runs of consecutive non-blank lines, tracking offsets.
fn split_blocks(input: &str) -> Vec<RawBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0usize;
    // (start_byte, start_line, end_byte) of the block currently being built.
    let mut cur: Option<(usize, u32, usize)> = None;

    for (line_no, line) in (1u32..).zip(input.split_inclusive('\n')) {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let blank = content.trim().is_empty();
        if blank {
            if let Some((start, sline, end)) = cur.take() {
                blocks.push(RawBlock {
                    text: input[start..end].to_string(),
                    byte_start: start,
                    byte_len: end - start,
                    line_start: sline,
                });
            }
        } else {
            let content_end = offset + content.len();
            match &mut cur {
                None => cur = Some((offset, line_no, content_end)),
                Some((_, _, end)) => *end = content_end,
            }
        }
        offset += line.len();
    }
    if let Some((start, sline, end)) = cur.take() {
        blocks.push(RawBlock {
            text: input[start..end].to_string(),
            byte_start: start,
            byte_len: end - start,
            line_start: sline,
        });
    }
    blocks
}

enum Outcome {
    Card(ParsedCard),
    Directive {
        topic: Option<String>,
        tags: Vec<String>,
    },
    Empty,
}

fn marker(line: &str) -> Option<(Marker, &str)> {
    for m in Marker::ALL {
        if let Some(rest) = line.strip_prefix(m.prefix()) {
            return Some((m, rest.strip_prefix(' ').unwrap_or(rest)));
        }
    }
    None
}

/// A `//` line comment (at column 0, like markers). It is dropped wherever it
/// appears in a block, except inside a fenced code block.
fn is_comment(line: &str) -> bool {
    line.starts_with("//")
}

/// Parse an id comment line (`// @id <ulid> [model=<key>]` or the legacy
/// `<!-- @id <ulid> [model=<key>] -->`), returning the id, the optional raw
/// `model=` token, and which syntax was used.
fn parse_id_comment(line: &str) -> Option<(&str, Option<&str>, IdSyntax)> {
    let line = line.trim();
    let (inner, syntax) = match line.strip_prefix("//") {
        Some(rest) => (rest, IdSyntax::Slash),
        None => (
            line.strip_prefix("<!--")?.strip_suffix("-->")?,
            IdSyntax::Html,
        ),
    };
    let rest = inner.trim().strip_prefix("@id")?.trim();
    let mut parts = rest.split_whitespace();
    let id = parts.next().filter(|s| !s.is_empty())?;
    let model = parts.find_map(|tok| tok.strip_prefix("model="));
    Some((id, model, syntax))
}

fn diag(span: SourceSpan, message: impl Into<String>, help: Option<&str>) -> Diag {
    Diag {
        span,
        message: message.into(),
        help: help.map(str::to_string),
    }
}

/// The `@id` comment of a block, once seen.
struct IdLine {
    id: String,
    span: SourceSpan,
    syntax: IdSyntax,
    model: Option<ModelKey>,
}

/// Scan one block's lines, then classify it. Problems are pushed onto `diags`
/// (each pointing at the line it concerns) and make the outcome `None`; the
/// block's syntax is returned either way.
fn parse_block(block: &RawBlock, diags: &mut Vec<Diag>) -> (BlockSyntax, Option<Outcome>) {
    let span = block.span();
    let diags_before = diags.len();

    // Optional id comment (before any field), possibly carrying a
    // `model=<key>` suffix.
    let mut id: Option<IdLine> = None;
    let mut lines: Vec<SyntaxLine> = Vec::new();
    let mut fields: Vec<FieldSrc> = Vec::new();
    let mut stray_reported = false;

    // Inside a fenced code block, `//` lines are field content, not comments.
    let mut in_fence = false;
    let mut line_offset = span.byte_start;
    for (line_start, raw) in (span.line_start..).zip(block.text.split_inclusive('\n')) {
        let byte_start = line_offset;
        line_offset += raw.len();
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let line_span = SourceSpan {
            byte_start,
            byte_len: line.len(),
            line_start,
        };

        if !in_fence {
            if let Some((found, model_tok, syntax)) = parse_id_comment(line) {
                lines.push(SyntaxLine {
                    span: line_span,
                    kind: LineKind::Id,
                });
                if !fields.is_empty() {
                    diags.push(diag(
                        line_span,
                        "the `@id` comment must come before the card's fields",
                        Some("move this line above `q:`"),
                    ));
                } else if id.is_some() {
                    diags.push(diag(line_span, "duplicate `@id` comment in one card", None));
                } else {
                    let model = model_tok.and_then(|tok| {
                        let model = ModelKey::parse(tok);
                        if model.is_none() {
                            diags.push(diag(
                                line_span,
                                format!("unknown `model={tok}` in the `@id` comment"),
                                Some(
                                    "expected `basic`, `basic-example`, `reversed`, `type-in` or `cloze`",
                                ),
                            ));
                        }
                        model
                    });
                    id = Some(IdLine {
                        id: found.to_string(),
                        span: line_span,
                        syntax,
                        model,
                    });
                }
                continue;
            }
            if is_comment(line) {
                lines.push(SyntaxLine {
                    span: line_span,
                    kind: LineKind::Comment,
                });
                continue;
            }
        }
        // A fence may open on the marker line itself (`a: ```rust`), so track
        // it against the field value; a new marker always starts outside one.
        let found = marker(line);
        let content = match found {
            Some((_, v)) => {
                in_fence = false;
                v
            }
            None => line,
        };
        in_fence = if in_fence {
            content.trim() != "```"
        } else {
            content.trim_start().starts_with("```")
        };

        match found {
            Some((m, v)) => {
                if m.is_field() && fields.iter().any(|f| f.marker == m) {
                    diags.push(diag(
                        line_span,
                        format!("duplicate `{}` marker in one card", m.prefix()),
                        None,
                    ));
                }
                fields.push(FieldSrc {
                    marker: m,
                    marker_span: line_span,
                    pieces: vec![SourceSpan {
                        byte_start: byte_start + (line.len() - v.len()),
                        byte_len: v.len(),
                        line_start,
                    }],
                    text: v.to_string(),
                });
            }
            None => match fields.last_mut() {
                Some(f) if f.marker.is_field() => {
                    f.text.push('\n');
                    f.text.push_str(line);
                    f.pieces.push(line_span);
                }
                _ if stray_reported => {}
                _ => {
                    stray_reported = true;
                    diags.push(diag(
                        line_span,
                        "expected a marker (`q:`, `a:`, `r:`, `e:`, `t:`, `topic:`, `tags:`)",
                        Some("every card starts with `q:`"),
                    ));
                }
            },
        }
    }

    let outcome = if diags.len() > diags_before {
        None
    } else {
        match classify(span, id, &fields) {
            Ok(outcome) => Some(outcome),
            Err(d) => {
                diags.push(d);
                None
            }
        }
    };
    (
        BlockSyntax {
            span,
            lines,
            fields,
        },
        outcome,
    )
}

/// Infer what a cleanly scanned block is from the markers it carries.
fn classify(
    span: SourceSpan,
    id: Option<IdLine>,
    fields: &[FieldSrc],
) -> std::result::Result<Outcome, Diag> {
    let get = |m: Marker| fields.iter().find(|f| f.marker == m);
    let (q, a, e, t, r) = (
        get(Marker::Q),
        get(Marker::A),
        get(Marker::E),
        get(Marker::T),
        get(Marker::R),
    );
    let topic = fields.iter().rfind(|f| f.marker == Marker::Topic);
    let card_tags: Vec<String> = fields
        .iter()
        .filter(|f| f.marker == Marker::Tags)
        .flat_map(|f| f.text.split_whitespace().map(str::to_string))
        .collect();

    // No question ⇒ either a directive block or an error.
    let Some(q) = q else {
        if let Some(f) = a.or(t).or(r).or(e) {
            return Err(diag(
                f.marker_span,
                "card has fields but no `q:`",
                Some("add a `q:` line"),
            ));
        }
        if topic.is_some() || !card_tags.is_empty() {
            return Ok(Outcome::Directive {
                topic: topic.map(|f| f.text.trim().to_string()),
                tags: card_tags,
            });
        }
        return Ok(Outcome::Empty); // e.g. a stray `// @id`
    };

    if let Some(topic) = topic {
        return Err(diag(
            topic.marker_span,
            "`topic:` is a file-level directive and cannot be on a card",
            Some("put `topic:` in its own block at the top of the file"),
        ));
    }
    if q.text.trim().is_empty() {
        return Err(diag(q.marker_span, "`q:` is empty", None));
    }

    let example = || e.map(|f| f.text.clone()).filter(|s| !s.trim().is_empty());
    let kind = if cloze::has_shorthand_cloze(&q.text) || cloze::has_native_cloze(&q.text) {
        if let Some(f) = a.or(t).or(r) {
            return Err(diag(
                f.marker_span,
                "a cloze card cannot also have `a:`/`t:`/`r:`",
                Some("cloze cards use only `q:` (plus an optional `e:`)"),
            ));
        }
        CardKindSpec::Cloze {
            text: q.text.clone(),
            back_extra: example(),
        }
    } else if let Some(r) = r {
        if a.is_some() || t.is_some() {
            return Err(diag(
                r.marker_span,
                "use only one of `a:`, `t:` or `r:` in a card",
                Some("`r:` replaces `a:` to make the card work both ways"),
            ));
        }
        // Anki generates no reverse card from an empty back.
        if r.text.trim().is_empty() {
            return Err(diag(r.marker_span, "`r:` is empty", None));
        }
        CardKindSpec::Reversed {
            front: q.text.clone(),
            back: r.text.clone(),
            example: example(),
        }
    } else if let Some(t) = t {
        if a.is_some() {
            return Err(diag(
                t.marker_span,
                "use either `a:` or `t:`, not both",
                None,
            ));
        }
        CardKindSpec::TypeIn {
            question: q.text.clone(),
            answer: t.text.trim().to_string(),
        }
    } else if let (Some(a), Some(e)) = (a, e) {
        CardKindSpec::BasicExample {
            question: q.text.clone(),
            answer: a.text.clone(),
            example: e.text.clone(),
        }
    } else if let Some(a) = a {
        CardKindSpec::Basic {
            question: q.text.clone(),
            answer: a.text.clone(),
        }
    } else {
        return Err(diag(
            q.marker_span,
            "card has `q:` but no answer",
            Some("add `a:` (basic), `r:` (both ways), `t:` (type-in), or use `==cloze==`"),
        ));
    };

    Ok(Outcome::Card(ParsedCard {
        spec: CardSpec {
            id: id.as_ref().map(|i| i.id.clone()),
            tags: card_tags,
            kind,
        },
        block_span: span,
        id_present: id.is_some(),
        id_span: id.as_ref().map(|i| i.span),
        id_syntax: id.as_ref().map(|i| i.syntax),
        model_marker: id.as_ref().and_then(|i| i.model),
        resolved_model: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_str(s: &str) -> ParsedFile {
        parse(s, &PathBuf::from("test.cards")).unwrap()
    }

    #[test]
    fn infers_basic_and_example_and_typein() {
        let f = parse_str("q: Q1\na: A1\n\nq: Q2\na: A2\ne: E2\n\nq: Q3\nt: 200\n");
        assert_eq!(f.cards.len(), 3);
        assert!(matches!(f.cards[0].spec.kind, CardKindSpec::Basic { .. }));
        assert!(matches!(
            f.cards[1].spec.kind,
            CardKindSpec::BasicExample { .. }
        ));
        assert!(matches!(f.cards[2].spec.kind, CardKindSpec::TypeIn { .. }));
    }

    #[test]
    fn parses_model_marker() {
        let f = parse_str("<!-- @id 01ABC model=basic -->\nq: Q\na: A\n");
        assert_eq!(f.cards[0].spec.id.as_deref(), Some("01ABC"));
        assert_eq!(f.cards[0].model_marker, Some(ModelKey::Basic));
        assert!(f.cards[0].id_present);

        let g = parse_str("<!-- @id 01ABC -->\nq: Q\na: A\n");
        assert_eq!(g.cards[0].model_marker, None);
        assert!(g.cards[0].id_span.is_some());
    }

    #[test]
    fn unknown_model_marker_errors() {
        let r = parse(
            "<!-- @id 01ABC model=nope -->\nq: Q\na: A\n",
            &PathBuf::from("t.cards"),
        );
        assert!(r.is_err());
    }

    #[test]
    fn cloze_keeps_q() {
        let f = parse_str("q: The capital is ==Paris==.\ne: geography\n");
        match &f.cards[0].spec.kind {
            CardKindSpec::Cloze { text, back_extra } => {
                assert_eq!(text, "The capital is ==Paris==.");
                assert_eq!(back_extra.as_deref(), Some("geography"));
            }
            other => panic!("expected cloze, got {other:?}"),
        }
    }

    #[test]
    fn parses_slash_and_html_id_comments() {
        let f = parse_str("// @id 01ABC model=basic\nq: Q\na: A\n");
        assert_eq!(f.cards[0].spec.id.as_deref(), Some("01ABC"));
        assert_eq!(f.cards[0].model_marker, Some(ModelKey::Basic));
        assert_eq!(f.cards[0].id_syntax, Some(IdSyntax::Slash));

        let g = parse_str("<!-- @id 01ABC -->\nq: Q\na: A\n");
        assert_eq!(g.cards[0].spec.id.as_deref(), Some("01ABC"));
        assert_eq!(g.cards[0].id_syntax, Some(IdSyntax::Html));
    }

    #[test]
    fn multiline_continuation() {
        let f = parse_str("q: Q\na: line one\nline two\n");
        match &f.cards[0].spec.kind {
            CardKindSpec::Basic { answer, .. } => assert_eq!(answer, "line one\nline two"),
            _ => panic!(),
        }
    }

    #[test]
    fn slash_comments_are_ignored() {
        let src = "// a note about this file\n\n\
                   // to improve\n// @id 01ABC model=basic-example\n// another\n\
                   q: Q\n// mid-field\na: line one\n// skip me\nline two\n";
        let f = parse_str(src);
        assert_eq!(f.cards.len(), 1);
        let c = &f.cards[0];
        assert_eq!(c.spec.id.as_deref(), Some("01ABC"));
        assert_eq!(c.model_marker, Some(ModelKey::BasicExample));
        // The id span points at the id line itself, not the block start.
        let span = c.id_span.unwrap();
        assert_eq!(
            &src[span.byte_start..span.byte_start + span.byte_len],
            "// @id 01ABC model=basic-example"
        );
        assert_eq!(span.line_start, 4);
        match &c.spec.kind {
            CardKindSpec::Basic { question, answer } => {
                assert_eq!(question, "Q");
                assert_eq!(answer, "line one\nline two");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn slash_lines_in_code_fences_are_kept() {
        let f = parse_str("q: Q\na: ```c\n// keep\nint x;\n```\n// drop\n");
        match &f.cards[0].spec.kind {
            CardKindSpec::Basic { answer, .. } => {
                assert_eq!(answer, "```c\n// keep\nint x;\n```")
            }
            _ => panic!(),
        }
    }

    #[test]
    fn id_after_fields_or_twice_errors() {
        assert!(parse("q: Q\n// @id 01ABC\na: A\n", &PathBuf::from("t")).is_err());
        assert!(
            parse(
                "// @id 01ABC\n// @id 01DEF\nq: Q\na: A\n",
                &PathBuf::from("t")
            )
            .is_err()
        );
    }

    #[test]
    fn file_topic_and_id() {
        let f = parse_str("topic: Networking\n\n<!-- @id 01ABC -->\nq: Q\na: A\n");
        assert_eq!(f.topic.as_deref(), Some("Networking"));
        assert_eq!(f.cards.len(), 1);
        assert_eq!(f.cards[0].spec.id.as_deref(), Some("01ABC"));
        assert!(f.cards[0].id_present);
    }

    #[test]
    fn cloze_with_answer_errors() {
        assert!(parse("q: ==x==\na: y\n", &PathBuf::from("t")).is_err());
    }

    #[test]
    fn topic_on_card_errors() {
        assert!(parse("q: Q\na: A\ntopic: X\n", &PathBuf::from("t")).is_err());
    }

    #[test]
    fn r_marker_makes_a_reversed_card() {
        let f = parse_str("q: hello\nr: hola\nmundo\n\nq: cat\nr: gato\ne: El gato duerme.\n");
        assert_eq!(
            f.cards[0].spec.kind,
            CardKindSpec::Reversed {
                front: "hello".into(),
                back: "hola\nmundo".into(),
                example: None,
            }
        );
        assert_eq!(
            f.cards[1].spec.kind,
            CardKindSpec::Reversed {
                front: "cat".into(),
                back: "gato".into(),
                example: Some("El gato duerme.".into()),
            }
        );
    }

    #[test]
    fn r_marker_excludes_other_answers() {
        for src in [
            "q: Q\na: A\nr: R\n",
            "q: Q\nt: T\nr: R\n",
            "q: ==x==\nr: R\n",
            "q: Q\nr:\n",
            "r: R\n",
        ] {
            assert!(parse(src, &PathBuf::from("t")).is_err(), "{src:?}");
        }
    }

    #[test]
    fn recovers_per_block_and_points_at_the_line() {
        let src = "q: Q1\na: A1\nr: R1\n\nq: Q2\na: A2\n\nstray\n\nq: Q4\nq: again\na: A4\n";
        let Parsed { file, diags } = parse_lenient(src, &PathBuf::from("t"));
        // The one clean block still yields its card.
        assert_eq!(file.cards.len(), 1);
        assert_eq!(file.blocks.len(), 4);

        let at: Vec<(u32, &str)> = diags
            .iter()
            .map(|d| {
                let s = d.span;
                (s.line_start, &src[s.byte_start..s.byte_start + s.byte_len])
            })
            .collect();
        assert_eq!(at, [(3, "r: R1"), (8, "stray"), (11, "q: again")]);

        match parse(src, &PathBuf::from("t")) {
            Err(AnkigenError::Multiple { count: 3, .. }) => {}
            other => panic!("expected 3 errors, got {other:?}"),
        }
    }

    #[test]
    fn records_field_pieces() {
        let src = "// @id 01ABC\nq: one\n// note\ntwo\na:\n- x\n";
        let f = parse_str(src);
        let block = &f.blocks[0];
        assert_eq!(
            block.lines.iter().map(|l| l.kind).collect::<Vec<_>>(),
            [LineKind::Id, LineKind::Comment]
        );
        let q = &block.fields[0];
        assert_eq!((q.marker, q.text.as_str()), (Marker::Q, "one\ntwo"));
        let pieces: Vec<&str> = q
            .pieces
            .iter()
            .map(|p| &src[p.byte_start..p.byte_start + p.byte_len])
            .collect();
        assert_eq!(pieces, ["one", "two"]);
        let a = &block.fields[1];
        assert_eq!((a.pieces[0].byte_len, a.text.as_str()), (0, "\n- x"));
    }
}
