//! Field rendering: raw source text → Anki-ready HTML.
//!
//! Block structure (paragraphs / lists / fenced code) is grouped line-wise;
//! inline syntax is parsed by the [`inline`] winnow grammar; cloze shorthand is
//! expanded first by [`cloze`]. Media references are resolved to package
//! basenames via the shared [`MediaResolver`].

pub mod cloze;
pub mod inline;

use std::path::Path;

use crate::error::{AnkigenError, Result};
use crate::media::MediaResolver;
use crate::model::SourceSpan;

use inline::{Inline, parse_inlines};

/// Per-field rendering context (carries the bits needed for media resolution
/// and for building located diagnostics).
pub struct RenderCtx<'a> {
    pub file: &'a Path,
    pub text: &'a str,
    pub src_dir: &'a Path,
    pub span: SourceSpan,
    pub resolver: &'a mut MediaResolver,
}

/// Render a normal field (question, answer, example, back-extra).
pub fn render_rich(field: &str, ctx: &mut RenderCtx) -> Result<String> {
    let blocks = group_blocks(field);
    render_blocks(&blocks, ctx)
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
    let blocks = group_blocks(&expanded);
    render_blocks(&blocks, ctx)
}

enum Block {
    Para(String),
    List { ordered: bool, items: Vec<String> },
    Code(String),
}

fn flush_para(para: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if !para.is_empty() {
        blocks.push(Block::Para(para.join("\n")));
        para.clear();
    }
}

fn group_blocks(field: &str) -> Vec<Block> {
    let lines: Vec<&str> = field.split('\n').collect();
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("```") {
            flush_para(&mut para, &mut blocks);
            i += 1;
            let mut code = Vec::new();
            while i < lines.len() && lines[i].trim() != "```" {
                code.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1; // consume closing fence
            }
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }
        if let Some((ordered, text)) = list_item(line) {
            flush_para(&mut para, &mut blocks);
            let mut items = vec![text.to_string()];
            i += 1;
            while i < lines.len() {
                match list_item(lines[i]) {
                    Some((o, t)) if o == ordered => {
                        items.push(t.to_string());
                        i += 1;
                    }
                    _ => break,
                }
            }
            blocks.push(Block::List { ordered, items });
            continue;
        }
        if line.trim().is_empty() {
            flush_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        para.push(line.to_string());
        i += 1;
    }
    flush_para(&mut para, &mut blocks);
    blocks
}

/// `- `/`* ` ⇒ unordered; `N. ` ⇒ ordered. Returns `(ordered, item_text)`.
fn list_item(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return Some((false, rest));
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        if let Some(rest) = t[digits..].strip_prefix(". ") {
            return Some((true, rest));
        }
    }
    None
}

fn render_blocks(blocks: &[Block], ctx: &mut RenderCtx) -> Result<String> {
    // A lone paragraph renders inline (no <p>) for tidy one-liner cards.
    if let [Block::Para(t)] = blocks {
        return render_inlines(&parse_inlines(t), ctx);
    }
    let mut html = String::new();
    for (idx, b) in blocks.iter().enumerate() {
        if idx > 0 {
            html.push('\n');
        }
        match b {
            Block::Para(t) => {
                html.push_str("<p>");
                html.push_str(&render_inlines(&parse_inlines(t), ctx)?);
                html.push_str("</p>");
            }
            Block::List { ordered, items } => {
                html.push_str(if *ordered { "<ol>" } else { "<ul>" });
                for it in items {
                    html.push_str("<li>");
                    html.push_str(&render_inlines(&parse_inlines(it), ctx)?);
                    html.push_str("</li>");
                }
                html.push_str(if *ordered { "</ol>" } else { "</ul>" });
            }
            Block::Code(c) => {
                html.push_str("<pre><code>");
                html.push_str(&escape_html(c));
                html.push_str("</code></pre>");
            }
        }
    }
    Ok(html)
}

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
            Inline::Link { text, url } => {
                s.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_attr(url),
                    escape_html(text)
                ));
            }
            Inline::Break => s.push_str("<br>"),
            Inline::Image { path, caption } => {
                let src = resolve_src(ctx, path)?;
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
                let src = resolve_src(ctx, path)?;
                s.push_str(&format!("[sound:{src}]"));
            }
        }
    }
    Ok(s)
}

fn resolve_src(ctx: &mut RenderCtx, path: &str) -> Result<String> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Ok(path.to_string());
    }
    ctx.resolver
        .resolve(ctx.src_dir, path)
        .map_err(|_| AnkigenError::missing_media(ctx.file, ctx.text, ctx.span, path))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_html(s).replace('"', "&quot;")
}
