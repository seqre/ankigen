//! Inline field grammar, parsed with `winnow`.
//!
//! A deliberately small, in-house subset: `**bold**`, `*italic*`, `` `code` ``,
//! `[text](url)` links, `[image:…]`/`[sound:…]` media, single newline → break,
//! and `\` escapes. Cloze (`==…==`) is handled *before* this step (see
//! [`super::cloze`]), so it is not part of this grammar.

use winnow::combinator::{alt, delimited, preceded};
use winnow::token::{any, take_until, take_while};
use winnow::{ModalResult, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Code(String),
    Link { text: String, url: String },
    Image { path: String, caption: Option<String> },
    Sound { path: String },
    Break,
}

/// Characters that can start a non-text element. A run of "plain" text stops at
/// any of these; if no element actually matches, the single char is consumed as
/// literal text by the fallback branch.
fn is_special(c: char) -> bool {
    matches!(c, '\\' | '\n' | '[' | '*' | '`')
}

/// Parse a field fragment into inline nodes. The grammar always makes progress,
/// so this consumes the whole input.
pub fn parse_inlines(input: &str) -> Vec<Inline> {
    let mut s = input;
    let nodes: Vec<Inline> =
        winnow::combinator::repeat(0.., element).parse_next(&mut s).unwrap_or_default();
    merge_text(nodes)
}

fn element(input: &mut &str) -> ModalResult<Inline> {
    alt((
        escaped,
        '\n'.value(Inline::Break),
        image,
        sound,
        link,
        bold,
        code,
        italic,
        // `alt` tuples cap at 9, so the text fallbacks share a branch.
        alt((text_run, any.map(|c: char| Inline::Text(c.to_string())))),
    ))
    .parse_next(input)
}

fn escaped(input: &mut &str) -> ModalResult<Inline> {
    preceded('\\', any)
        .map(|c: char| Inline::Text(c.to_string()))
        .parse_next(input)
}

fn text_run(input: &mut &str) -> ModalResult<Inline> {
    take_while(1.., |c: char| !is_special(c))
        .map(|s: &str| Inline::Text(s.to_string()))
        .parse_next(input)
}

fn bold(input: &mut &str) -> ModalResult<Inline> {
    delimited("**", take_until(0.., "**"), "**")
        .map(|inner: &str| Inline::Bold(parse_inlines(inner)))
        .parse_next(input)
}

fn italic(input: &mut &str) -> ModalResult<Inline> {
    delimited("*", take_until(0.., "*"), "*")
        .map(|inner: &str| Inline::Italic(parse_inlines(inner)))
        .parse_next(input)
}

fn code(input: &mut &str) -> ModalResult<Inline> {
    delimited("`", take_until(0.., "`"), "`")
        .map(|inner: &str| Inline::Code(inner.to_string()))
        .parse_next(input)
}

fn link(input: &mut &str) -> ModalResult<Inline> {
    (
        delimited("[", take_until(0.., "]"), "]"),
        delimited("(", take_until(0.., ")"), ")"),
    )
        .map(|(text, url): (&str, &str)| Inline::Link {
            text: text.to_string(),
            url: url.to_string(),
        })
        .parse_next(input)
}

fn image(input: &mut &str) -> ModalResult<Inline> {
    delimited("[image:", take_until(0.., "]"), "]")
        .map(|inner: &str| {
            let (caption, path) = split_caption_path(inner);
            Inline::Image { path, caption }
        })
        .parse_next(input)
}

fn sound(input: &mut &str) -> ModalResult<Inline> {
    delimited("[sound:", take_until(0.., "]"), "]")
        .map(|inner: &str| Inline::Sound {
            path: inner.to_string(),
        })
        .parse_next(input)
}

/// `[image:CAPTION:PATH]` ⇒ `(Some(caption), path)`; `[image:PATH]` ⇒ `(None, path)`.
/// A leading `http(s)://` is always treated as a (captionless) path.
fn split_caption_path(inner: &str) -> (Option<String>, String) {
    if inner.starts_with("http://") || inner.starts_with("https://") {
        return (None, inner.to_string());
    }
    match inner.split_once(':') {
        Some((cap, path)) => (Some(cap.to_string()), path.to_string()),
        None => (None, inner.to_string()),
    }
}

/// Coalesce adjacent `Text` nodes so output and tests are tidy.
fn merge_text(nodes: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(nodes.len());
    for n in nodes {
        match (out.last_mut(), n) {
            (Some(Inline::Text(prev)), Inline::Text(t)) => prev.push_str(&t),
            (_, n) => out.push(n),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text() {
        assert_eq!(parse_inlines("hello world"), vec![Inline::Text("hello world".into())]);
    }

    #[test]
    fn bold_italic_code() {
        assert_eq!(
            parse_inlines("a **b** *c* `d`"),
            vec![
                Inline::Text("a ".into()),
                Inline::Bold(vec![Inline::Text("b".into())]),
                Inline::Text(" ".into()),
                Inline::Italic(vec![Inline::Text("c".into())]),
                Inline::Text(" ".into()),
                Inline::Code("d".into()),
            ]
        );
    }

    #[test]
    fn italic_nested_in_bold() {
        // Bold may contain italic (the common direction). The reverse —
        // `*italic*` containing `**bold**` — is unsupported: a single-`*`
        // delimiter cannot span `**`.
        assert_eq!(
            parse_inlines("**a *b* c**"),
            vec![Inline::Bold(vec![
                Inline::Text("a ".into()),
                Inline::Italic(vec![Inline::Text("b".into())]),
                Inline::Text(" c".into()),
            ])]
        );
    }

    #[test]
    fn media_tokens() {
        assert_eq!(
            parse_inlines("[image:dog.png]"),
            vec![Inline::Image { path: "dog.png".into(), caption: None }]
        );
        assert_eq!(
            parse_inlines("[image:A dog:dog.png]"),
            vec![Inline::Image { path: "dog.png".into(), caption: Some("A dog".into()) }]
        );
        assert_eq!(
            parse_inlines("[sound:bark.mp3]"),
            vec![Inline::Sound { path: "bark.mp3".into() }]
        );
    }

    #[test]
    fn link_vs_literal_bracket() {
        assert_eq!(
            parse_inlines("[here](http://x)"),
            vec![Inline::Link { text: "here".into(), url: "http://x".into() }]
        );
        // A bare bracket is literal text.
        assert_eq!(parse_inlines("[nope]"), vec![Inline::Text("[nope]".into())]);
    }

    #[test]
    fn escapes() {
        assert_eq!(parse_inlines(r"\*not bold\*"), vec![Inline::Text("*not bold*".into())]);
    }
}
