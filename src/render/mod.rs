//! Field rendering: raw source text → Anki-ready HTML.
//!
//! Block structure (paragraphs / lists / fenced code) is grouped line-wise;
//! inline syntax is parsed by the [`inline`] winnow grammar; cloze shorthand is
//! expanded first by [`cloze`]. Media references are resolved to package
//! basenames via the shared [`MediaResolver`].

pub mod cloze;
pub mod inline;

use std::ops::Range;
use std::path::Path;

use crate::error::{AnkigenError, Result};
use crate::media::MediaResolver;
use crate::model::SourceSpan;

use inline::{Inline, parse_inlines};

pub use inline::parse_inlines_spanned;

/// Per-field rendering context (carries the bits needed for media resolution
/// and for building located diagnostics).
pub struct RenderCtx<'a> {
    pub file: &'a Path,
    pub text: &'a str,
    pub src_dir: &'a Path,
    pub span: SourceSpan,
    pub resolver: &'a mut MediaResolver,
    /// Missing-media references are collected here instead of aborting on the
    /// first one, so the build can report every miss at once.
    pub missing: &'a mut Vec<AnkigenError>,
}

/// Render a normal field (question, answer, example, back-extra).
pub fn render_rich(field: &str, ctx: &mut RenderCtx) -> Result<String> {
    render_blocks(field, ctx)
}

/// Render a cloze field: expand `==…==` first, then render normally.
pub fn render_cloze(field: &str, ctx: &mut RenderCtx) -> Result<String> {
    let expanded = cloze::expand(field).map_err(|_| {
        AnkigenError::parse(
            ctx.file,
            ctx.text,
            ctx.span,
            "cloze field mixes `==…==` shorthand with native `{{c…}}`",
            Some("use one style or the other within a single card"),
        )
    })?;
    render_blocks(&expanded, ctx)
}

/// One block of a field, as byte ranges into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Consecutive plain lines; the inline grammar runs over the whole range.
    Para(Range<usize>),
    /// Each item's text, list marker excluded.
    List {
        ordered: bool,
        items: Vec<Range<usize>>,
    },
    /// A fenced code block: `whole` includes the fence lines, `body` is what
    /// lies between them.
    Code {
        whole: Range<usize>,
        body: Range<usize>,
    },
}

/// Group a field's lines into paragraphs, lists and fenced code blocks.
pub fn layout(field: &str) -> Vec<Block> {
    // (start offset, text) of every line.
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    for line in field.split('\n') {
        lines.push((offset, line));
        offset += line.len() + 1;
    }
    let end = |i: usize| lines[i].0 + lines[i].1.len();
    let item = |i: usize, text: &str| end(i) - text.len()..end(i);

    let mut blocks = Vec::new();
    let mut para: Option<Range<usize>> = None;
    let mut i = 0;
    while i < lines.len() {
        let (start, line) = lines[i];
        if line.trim_start().starts_with("```") {
            blocks.extend(para.take().map(Block::Para));
            let first = i + 1;
            i = first;
            while i < lines.len() && lines[i].1.trim() != "```" {
                i += 1;
            }
            let body = if i > first {
                lines[first].0..end(i - 1)
            } else {
                end(first - 1)..end(first - 1)
            };
            if i < lines.len() {
                i += 1; // consume closing fence
            }
            blocks.push(Block::Code {
                whole: start..end(i - 1),
                body,
            });
            continue;
        }
        if let Some((ordered, text)) = list_item(line) {
            blocks.extend(para.take().map(Block::Para));
            let mut items = vec![item(i, text)];
            i += 1;
            while i < lines.len() {
                match list_item(lines[i].1) {
                    Some((o, t)) if o == ordered => {
                        items.push(item(i, t));
                        i += 1;
                    }
                    _ => break,
                }
            }
            blocks.push(Block::List { ordered, items });
            continue;
        }
        if line.trim().is_empty() {
            blocks.extend(para.take().map(Block::Para));
            i += 1;
            continue;
        }
        para = Some(match para.take() {
            Some(p) => p.start..end(i),
            None => start..end(i),
        });
        i += 1;
    }
    blocks.extend(para.take().map(Block::Para));
    blocks
}

/// `- `/`* ` ⇒ unordered; `N. ` ⇒ ordered. Returns `(ordered, item_text)`.
fn list_item(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return Some((false, rest));
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && let Some(rest) = t[digits..].strip_prefix(". ")
    {
        return Some((true, rest));
    }
    None
}

/// Call `f` with every inline node of a field and its byte range in it,
/// parents before children. Fenced code is not inline syntax and is skipped.
pub fn walk_inlines(field: &str, f: &mut impl FnMut(&Inline, Range<usize>)) {
    for block in layout(field) {
        match block {
            Block::Para(range) => walk_fragment(field, range, f),
            Block::List { items, .. } => {
                for range in items {
                    walk_fragment(field, range, f);
                }
            }
            Block::Code { .. } => {}
        }
    }
}

fn walk_fragment(field: &str, range: Range<usize>, f: &mut impl FnMut(&Inline, Range<usize>)) {
    for (node, r) in parse_inlines_spanned(&field[range.clone()]) {
        let abs = range.start + r.start..range.start + r.end;
        f(&node, abs.clone());
        // Children were parsed from the node minus its delimiters.
        match node {
            Inline::Bold(_) => walk_fragment(field, abs.start + 2..abs.end - 2, f),
            Inline::Italic(_) => walk_fragment(field, abs.start + 1..abs.end - 1, f),
            _ => {}
        }
    }
}

fn render_blocks(field: &str, ctx: &mut RenderCtx) -> Result<String> {
    let blocks = layout(field);
    // A lone paragraph renders inline (no <p>) for tidy one-liner cards.
    if let [Block::Para(t)] = blocks.as_slice() {
        return render_inlines(&parse_inlines(&field[t.clone()]), ctx);
    }
    let mut html = String::new();
    for (idx, b) in blocks.iter().enumerate() {
        if idx > 0 {
            html.push('\n');
        }
        match b {
            Block::Para(t) => {
                html.push_str("<p>");
                html.push_str(&render_inlines(&parse_inlines(&field[t.clone()]), ctx)?);
                html.push_str("</p>");
            }
            Block::List { ordered, items } => {
                html.push_str(if *ordered { "<ol>" } else { "<ul>" });
                for it in items {
                    html.push_str("<li>");
                    html.push_str(&render_inlines(&parse_inlines(&field[it.clone()]), ctx)?);
                    html.push_str("</li>");
                }
                html.push_str(if *ordered { "</ol>" } else { "</ul>" });
            }
            Block::Code { body, .. } => {
                html.push_str("<pre><code>");
                html.push_str(&escape_html(&field[body.clone()]));
                html.push_str("</code></pre>");
            }
        }
    }
    Ok(html)
}

/// What a media reference whose file is missing renders as (HTML-escaped, so
/// the card shows it literally): a visible cue to rebuild once the media is back.
const MISSING_MEDIA: &str = "&lt;missing media&gt;";

fn render_inlines(nodes: &[Inline], ctx: &mut RenderCtx) -> Result<String> {
    let mut s = String::new();
    for n in nodes {
        match n {
            Inline::Text(t) => s.push_str(&escape_html(t)),
            Inline::Bold(c) => {
                s.push_str("<strong>");
                s.push_str(&render_inlines(c, ctx)?);
                s.push_str("</strong>");
            }
            Inline::Italic(c) => {
                s.push_str("<em>");
                s.push_str(&render_inlines(c, ctx)?);
                s.push_str("</em>");
            }
            Inline::Code(t) => {
                s.push_str("<code>");
                s.push_str(&escape_html(t));
                s.push_str("</code>");
            }
            // Anki's MathJax delimiters: `\(…\)` inline, `\[…\]` display. The
            // LaTeX body passes through verbatim except `&`/`<`/`>`, which must
            // be entity-escaped for valid field HTML; the browser decodes them
            // before MathJax reads the text node, so the math is unchanged.
            Inline::Math { source, display } => {
                let (open, close) = if *display {
                    ("\\[", "\\]")
                } else {
                    ("\\(", "\\)")
                };
                s.push_str(open);
                s.push_str(&escape_html(source));
                s.push_str(close);
            }
            Inline::Link { text, url } => {
                s.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_attr(url),
                    escape_html(text)
                ));
            }
            Inline::Break => s.push_str("<br>"),
            Inline::Image { path, caption } => {
                let Some(src) = resolve_src(ctx, path)? else {
                    s.push_str(MISSING_MEDIA);
                    continue;
                };
                match caption {
                    Some(cap) => s.push_str(&format!(
                        "<figure><img src=\"{}\" alt=\"{}\"><figcaption>{}</figcaption></figure>",
                        escape_attr(&src),
                        escape_attr(cap),
                        escape_html(cap)
                    )),
                    None => s.push_str(&format!("<img src=\"{}\">", escape_attr(&src))),
                }
            }
            Inline::Sound { path } => {
                let Some(src) = resolve_src(ctx, path)? else {
                    s.push_str(MISSING_MEDIA);
                    continue;
                };
                s.push_str(&format!("[sound:{src}]"));
            }
        }
    }
    Ok(s)
}

/// The packaged name for a media reference, or `None` if the file is missing
/// (recorded in `ctx.missing`; [`MISSING_MEDIA`] is rendered in its place).
fn resolve_src(ctx: &mut RenderCtx, path: &str) -> Result<Option<String>> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Ok(Some(path.to_string()));
    }
    match ctx.resolver.resolve(ctx.src_dir, path) {
        Ok(basename) => Ok(Some(basename)),
        Err(_) => {
            // Collect the miss and render a marker in its place: the build fails
            // after rendering unless `--ignore-missing-media` asked for exactly that.
            ctx.missing.push(AnkigenError::missing_media(
                ctx.file, ctx.text, ctx.span, path,
            ));
            Ok(None)
        }
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_html(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(field: &str) -> String {
        let mut resolver = MediaResolver::new();
        let mut missing = Vec::new();
        let src_dir = Path::new(".");
        let mut ctx = RenderCtx {
            file: Path::new("test.md"),
            text: field,
            src_dir,
            span: SourceSpan {
                byte_start: 0,
                byte_len: field.len(),
                line_start: 1,
            },
            resolver: &mut resolver,
            missing: &mut missing,
        };
        render_rich(field, &mut ctx).unwrap()
    }

    fn render_cloze_field(field: &str) -> String {
        let mut resolver = MediaResolver::new();
        let mut missing = Vec::new();
        let src_dir = Path::new(".");
        let mut ctx = RenderCtx {
            file: Path::new("test.md"),
            text: field,
            src_dir,
            span: SourceSpan {
                byte_start: 0,
                byte_len: field.len(),
                line_start: 1,
            },
            resolver: &mut resolver,
            missing: &mut missing,
        };
        render_cloze(field, &mut ctx).unwrap()
    }

    #[test]
    fn inline_math_uses_paren_delimiters() {
        assert_eq!(render("$x^2$"), "\\(x^2\\)");
        assert_eq!(render("when $a+b$ holds"), "when \\(a+b\\) holds");
    }

    #[test]
    fn display_math_uses_bracket_delimiters() {
        assert_eq!(render("$$\\sum_{i=1}^n i$$"), "\\[\\sum_{i=1}^n i\\]");
    }

    #[test]
    fn math_body_entity_escapes_html_metachars() {
        // `<`, `>`, `&` become entities (valid HTML; the browser decodes them
        // back before MathJax reads the text node). Backslashes pass through.
        assert_eq!(render("$x < y \\& z$"), "\\(x &lt; y \\&amp; z\\)");
    }

    #[test]
    fn math_inside_cloze_survives() {
        assert_eq!(render_cloze_field("==$x^2$=="), "{{c1::\\(x^2\\)}}");
    }

    #[test]
    fn layout_ranges_index_the_field() {
        let f = "intro\nmore\n- a\n- b\n```rust\n// x\n```\n\ntail";
        let blocks = layout(f);
        assert_eq!(
            blocks,
            [
                Block::Para(0..10),
                Block::List {
                    ordered: false,
                    items: vec![13..14, 17..18],
                },
                Block::Code {
                    whole: 19..35,
                    body: 27..31,
                },
                Block::Para(37..41),
            ]
        );
        assert_eq!(&f[27..31], "// x");
        // An unclosed, empty fence still yields in-bounds ranges.
        assert_eq!(
            layout("```"),
            [Block::Code {
                whole: 0..3,
                body: 3..3
            }]
        );
    }
}
