//! Parser for the native `cards` text format.
//!
//! Splits a file into blank-line-separated blocks, reads an optional leading
//! `// @id …` (or legacy `<!-- @id … -->`), collects `q:`/`a:`/`e:`/`t:` markers (with multi-line
//! continuation) and `topic:`/`tags:` directives, then infers the card type.
//! Byte offsets are tracked throughout so diagnostics point at the offending
//! block. (Inline field syntax is parsed separately by the winnow grammar in
//! [`crate::render`].)

use std::path::Path;

use crate::error::{AnkigenError, Result};
use crate::model::card::SourceSpan;
use crate::model::{CardKindSpec, CardSpec, IdSyntax, ModelKey, ParsedCard, ParsedFile};
use crate::render::cloze;

pub fn parse(input: &str, path: &Path) -> Result<ParsedFile> {
    let mut topic: Option<String> = None;
    let mut file_tags: Vec<String> = Vec::new();
    let mut cards: Vec<ParsedCard> = Vec::new();

    for block in split_blocks(input) {
        match parse_block(&block, input, path)? {
            Outcome::Card(card) => cards.push(card),
            Outcome::Directive {
                topic: t,
                tags: mut tg,
            } => {
                if let Some(t) = t
                    && topic.is_none()
                {
                    topic = Some(t);
                }
                file_tags.append(&mut tg);
            }
            Outcome::Empty => {}
        }
    }

    Ok(ParsedFile {
        path: path.to_path_buf(),
        text: input.to_string(),
        topic,
        file_tags,
        cards,
    })
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
    let mut line_no = 1u32;
    // (start_byte, start_line, end_byte) of the block currently being built.
    let mut cur: Option<(usize, u32, usize)> = None;

    for line in input.split_inclusive('\n') {
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
        line_no += 1;
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

#[derive(Clone, Copy)]
enum Marker {
    Q,
    A,
    E,
    T,
    Tags,
    Topic,
}

fn marker(line: &str) -> Option<(Marker, &str)> {
    const TABLE: [(&str, Marker); 6] = [
        ("q:", Marker::Q),
        ("a:", Marker::A),
        ("e:", Marker::E),
        ("t:", Marker::T),
        ("tags:", Marker::Tags),
        ("topic:", Marker::Topic),
    ];
    for (prefix, m) in TABLE {
        if let Some(rest) = line.strip_prefix(prefix) {
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

/// Parse a `// @id <ulid> [model=<key>]` comment (or the legacy
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

fn parse_block(block: &RawBlock, full: &str, path: &Path) -> Result<Outcome> {
    let span = block.span();
    let err = |msg: &str, help: Option<&str>| AnkigenError::parse(path, full, span, msg, help);

    // Optional id comment (before any field), possibly carrying a
    // `model=<key>` suffix.
    let mut id: Option<String> = None;
    let mut model_marker: Option<ModelKey> = None;
    let mut id_span: Option<SourceSpan> = None;
    let mut id_syntax: Option<IdSyntax> = None;

    let (mut q, mut a, mut e, mut t) = (None, None, None, None);
    let mut topic: Option<String> = None;
    let mut card_tags: Vec<String> = Vec::new();
    let mut cur: Option<Marker> = None;

    let set = |slot: &mut Option<String>, val: &str, name: &str| -> Result<()> {
        if slot.is_some() {
            return Err(err(&format!("duplicate `{name}` marker in one card"), None));
        }
        *slot = Some(val.to_string());
        Ok(())
    };

    // Inside a fenced code block, `//` lines are field content, not comments.
    let mut in_fence = false;
    let mut line_offset = span.byte_start;
    for (line_start, raw) in (span.line_start..).zip(block.text.split_inclusive('\n')) {
        let byte_start = line_offset;
        line_offset += raw.len();
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);

        if !in_fence {
            if let Some((found, model_tok, syntax)) = parse_id_comment(line) {
                let line_span = SourceSpan {
                    byte_start,
                    byte_len: line.len(),
                    line_start,
                };
                let line_err = |msg: &str, help: Option<&str>| {
                    AnkigenError::parse(path, full, line_span, msg, help)
                };
                if cur.is_some() {
                    return Err(line_err(
                        "the `@id` comment must come before the card's fields",
                        Some("move this line above `q:`"),
                    ));
                }
                if id.is_some() {
                    return Err(line_err("duplicate `@id` comment in one card", None));
                }
                id = Some(found.to_string());
                id_syntax = Some(syntax);
                id_span = Some(line_span);
                if let Some(tok) = model_tok {
                    match ModelKey::parse(tok) {
                        Some(m) => model_marker = Some(m),
                        None => {
                            return Err(line_err(
                                &format!("unknown `model={tok}` in the `@id` comment"),
                                Some("expected `model=basic` or `model=basic-example`"),
                            ));
                        }
                    }
                }
                continue;
            }
            if is_comment(line) {
                continue;
            }
        }
        // A fence may open on the marker line itself (`a: ```rust`), so track
        // it against the field value; a new marker always starts outside one.
        let content = match marker(line) {
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

        match marker(line) {
            Some((Marker::Q, v)) => {
                set(&mut q, v, "q:")?;
                cur = Some(Marker::Q);
            }
            Some((Marker::A, v)) => {
                set(&mut a, v, "a:")?;
                cur = Some(Marker::A);
            }
            Some((Marker::E, v)) => {
                set(&mut e, v, "e:")?;
                cur = Some(Marker::E);
            }
            Some((Marker::T, v)) => {
                set(&mut t, v, "t:")?;
                cur = Some(Marker::T);
            }
            Some((Marker::Tags, v)) => {
                card_tags.extend(v.split_whitespace().map(str::to_string));
                cur = Some(Marker::Tags);
            }
            Some((Marker::Topic, v)) => {
                topic = Some(v.trim().to_string());
                cur = Some(Marker::Topic);
            }
            None => match cur {
                Some(Marker::Q) => append(&mut q, line),
                Some(Marker::A) => append(&mut a, line),
                Some(Marker::E) => append(&mut e, line),
                Some(Marker::T) => append(&mut t, line),
                _ => {
                    return Err(err(
                        "expected a marker (`q:`, `a:`, `e:`, `t:`, `topic:`, `tags:`)",
                        Some("every card starts with `q:`"),
                    ));
                }
            },
        }
    }

    // No question ⇒ either a directive block or an error.
    let Some(q) = q else {
        if a.is_some() || t.is_some() || e.is_some() {
            return Err(err("card has fields but no `q:`", Some("add a `q:` line")));
        }
        if topic.is_some() || !card_tags.is_empty() {
            return Ok(Outcome::Directive {
                topic,
                tags: card_tags,
            });
        }
        return Ok(Outcome::Empty); // e.g. a stray `// @id`
    };

    if topic.is_some() {
        return Err(err(
            "`topic:` is a file-level directive and cannot be on a card",
            Some("put `topic:` in its own block at the top of the file"),
        ));
    }
    if q.trim().is_empty() {
        return Err(err("`q:` is empty", None));
    }

    let kind = if cloze::has_shorthand_cloze(&q) || cloze::has_native_cloze(&q) {
        if a.is_some() || t.is_some() {
            return Err(err(
                "a cloze card cannot also have `a:`/`t:`",
                Some("cloze cards use only `q:` (plus an optional `e:`)"),
            ));
        }
        CardKindSpec::Cloze {
            text: q,
            back_extra: e.filter(|s| !s.trim().is_empty()),
        }
    } else if let Some(t) = t {
        if a.is_some() {
            return Err(err("use either `a:` or `t:`, not both", None));
        }
        CardKindSpec::TypeIn {
            question: q,
            answer: t.trim().to_string(),
        }
    } else if let (Some(a), Some(e)) = (a.clone(), e.clone()) {
        CardKindSpec::BasicExample {
            question: q,
            answer: a,
            example: e,
        }
    } else if let Some(a) = a {
        CardKindSpec::Basic {
            question: q,
            answer: a,
        }
    } else {
        return Err(err(
            "card has `q:` but no answer",
            Some("add `a:` (basic), `t:` (type-in), or use `==cloze==`"),
        ));
    };

    let id_present = id.is_some();
    Ok(Outcome::Card(ParsedCard {
        spec: CardSpec {
            id,
            tags: card_tags,
            kind,
        },
        block_span: span,
        id_present,
        id_span,
        id_syntax,
        model_marker,
        resolved_model: None,
    }))
}

fn append(slot: &mut Option<String>, line: &str) {
    if let Some(s) = slot.as_mut() {
        s.push('\n');
        s.push_str(line);
    }
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
}
