//! Everything the server knows about one document, computed from its text
//! alone (plus the media files next to it). No protocol types here, so it is
//! testable without a client.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::check;
use crate::model::{
    BlockSyntax, CardKindSpec, Diag, Marker, ModelKey, ParsedCard, ParsedFile, SourceSpan,
};
use crate::pipeline::is_media_ext;
use crate::render::inline::Inline;
use crate::render::{cloze, walk_inlines};
use crate::source::cards::{Parsed, parse_lenient};

pub struct Analysis {
    pub file: ParsedFile,
    /// Every problem `ankigen build` would report for this file on its own,
    /// in source order. Duplicate ids need the other files: see
    /// [`duplicate_id_diags`].
    pub diags: Vec<Diag>,
    /// Missing media files, in source order. Shown as warnings: `build`
    /// tolerates them under `--ignore-missing-media`.
    pub warnings: Vec<Diag>,
}

pub fn analyze(text: &str, path: &Path) -> Analysis {
    let Parsed { file, mut diags } = parse_lenient(text, path);

    for card in &file.cards {
        if let Err(d) = check::resolve_model(card) {
            diags.push(d);
        }
        if let CardKindSpec::Cloze { text, .. } = &card.spec.kind
            && cloze::expand(text).is_err()
        {
            let span = block_of(&file, card)
                .and_then(|b| b.fields.iter().find(|f| f.marker == Marker::Q))
                .map_or(card.block_span, |f| f.marker_span);
            diags.push(Diag {
                span,
                message: "cloze field mixes `==…==` shorthand with native `{{c…}}`".into(),
                help: Some("use one style or the other within a single card".into()),
            });
        }
    }

    let src_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut warnings = Vec::new();
    for block in &file.blocks {
        missing_media(block, src_dir, &mut warnings);
    }

    diags.sort_by_key(|d| d.span.byte_start);
    Analysis {
        file,
        diags,
        warnings,
    }
}

/// The block a card was parsed from.
pub fn block_of<'a>(file: &'a ParsedFile, card: &ParsedCard) -> Option<&'a BlockSyntax> {
    file.blocks.iter().find(|b| b.span == card.block_span)
}

/// Does this marker's value go through the inline grammar? (`t:` is compared
/// literally by Anki; `tags:`/`topic:` are directives.)
pub fn is_rich(marker: Marker) -> bool {
    matches!(marker, Marker::Q | Marker::A | Marker::E | Marker::R)
}

fn missing_media(block: &BlockSyntax, src_dir: &Path, diags: &mut Vec<Diag>) {
    for field in block.fields.iter().filter(|f| is_rich(f.marker)) {
        walk_inlines(&field.text, &mut |node, range| {
            let (Inline::Image { path, .. } | Inline::Sound { path }) = node else {
                return;
            };
            if path.starts_with("http://")
                || path.starts_with("https://")
                || src_dir.join(path).is_file()
            {
                return;
            }
            // The path is the last thing before the closing `]`.
            let path_range = range.end - 1 - path.len()..range.end - 1;
            let span = field
                .source_spans(path_range)
                .into_iter()
                .next()
                .unwrap_or(field.marker_span);
            diags.push(Diag {
                span,
                message: format!("media file not found: {path}"),
                help: Some("paths are relative to this file".into()),
            });
        });
    }
}

/// A diagnostic for each card of `file` whose `@id` is used by another card:
/// one in `others` (id → the file holding it) or an earlier one in this file.
pub fn duplicate_id_diags(file: &ParsedFile, others: &HashMap<String, PathBuf>) -> Vec<Diag> {
    let mut seen: HashMap<&str, ()> = HashMap::new();
    let mut diags = Vec::new();
    for card in &file.cards {
        let (Some(id), Some(span)) = (card.spec.id.as_deref(), card.id_span) else {
            continue;
        };
        let place = if seen.insert(id, ()).is_some() {
            "earlier in this file".to_string()
        } else if let Some(other) = others.get(id) {
            format!("in {}", other.display())
        } else {
            continue;
        };
        diags.push(Diag {
            span,
            message: format!("duplicate @id `{id}` (also used {place})"),
            help: Some(
                "each card's id must be unique; delete this line and ankigen mints a fresh one"
                    .into(),
            ),
        });
    }
    diags
}

/// What hovering a card shows: Markdown, and the span of the card's block.
pub fn hover(analysis: &Analysis, offset: usize) -> Option<(String, SourceSpan)> {
    let file = &analysis.file;
    let card = file.cards.iter().find(|c| {
        let s = c.block_span;
        (s.byte_start..=s.byte_start + s.byte_len).contains(&offset)
    })?;

    let mut out = match check::resolve_model(card) {
        Ok(model) => {
            let (name, cards) = match model {
                ModelKey::Basic => ("Basic (legacy)", "1 card: `q` → `a`".to_string()),
                ModelKey::BasicExample => ("Basic + Example", "1 card: `q` → `a`".to_string()),
                ModelKey::Reversed => (
                    "Basic + Reversed",
                    "2 cards: `q` → `r` and `r` → `q`".to_string(),
                ),
                ModelKey::TypeIn => ("Type-in", "1 card: type the answer to `q`".to_string()),
                ModelKey::Cloze => {
                    let n = match &card.spec.kind {
                        CardKindSpec::Cloze { text, .. } => cloze_card_count(text),
                        _ => 0,
                    };
                    let cards = if n == 1 {
                        "1 card".to_string()
                    } else {
                        format!("{n} cards, one per deletion")
                    };
                    ("Cloze", cards)
                }
            };
            format!("**{name}** — `model={}`\n\n{cards}", model.as_str())
        }
        Err(d) => format!("**Note type conflict**\n\n{}", d.message),
    };

    let tags: Vec<String> = file
        .file_tags
        .iter()
        .chain(&card.spec.tags)
        .map(|t| format!("`{t}`"))
        .collect();
    if !tags.is_empty() {
        out.push_str(&format!("\n\nTags: {}", tags.join(" ")));
    }
    match &card.spec.id {
        Some(id) => out.push_str(&format!("\n\nid `{id}`")),
        None => out.push_str("\n\nNew card: its `@id` is minted by the next `ankigen build`."),
    }
    Some((out, card.block_span))
}

/// How many Anki cards a cloze text yields: one per `==…==` (each gets its
/// own number), or one per distinct `N` among native `{{cN::…}}` deletions.
fn cloze_card_count(text: &str) -> usize {
    let shorthand = cloze::shorthand_spans(text).len();
    if shorthand > 0 {
        return shorthand;
    }
    let mut numbers: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("{{c") {
        rest = &rest[at + 3..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with("::") && !numbers.contains(&&rest[..digits]) {
            numbers.push(&rest[..digits]);
        }
    }
    numbers.len()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Marker,
    Model,
    File,
    Dir,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub detail: Option<&'static str>,
    pub kind: CompletionKind,
    /// The text to put in place of `replace`.
    pub insert: String,
    /// The byte range of what the user has typed of this item so far.
    pub replace: Range<usize>,
}

/// Completions at byte `offset` of `text`, the document at `path`: markers at
/// the start of a line, `model=` values on an `@id` line, and media paths
/// inside `[image:` / `[sound:`.
pub fn complete(text: &str, path: &Path, offset: usize) -> Vec<Completion> {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let typed = &text[line_start..offset];

    if typed.contains("@id")
        && let Some(at) = typed.rfind("model=")
        && !typed[at..].contains(char::is_whitespace)
    {
        let value_start = line_start + at + "model=".len();
        let models = [
            (ModelKey::BasicExample, "q: + a: (optional e:)"),
            (ModelKey::Reversed, "q: + r:, reviewed both ways"),
            (ModelKey::TypeIn, "q: + t:"),
            (ModelKey::Cloze, "q: with ==…== deletions"),
            (ModelKey::Basic, "legacy q: + a:, from older ankigen"),
        ];
        return models
            .into_iter()
            .filter(|(m, _)| m.as_str().starts_with(&text[value_start..offset]))
            .map(|(m, detail)| Completion {
                label: m.as_str().to_string(),
                detail: Some(detail),
                kind: CompletionKind::Model,
                insert: m.as_str().to_string(),
                replace: value_start..offset,
            })
            .collect();
    }

    if let Some((partial, images)) = open_media_path(typed) {
        let src_dir = path.parent().unwrap_or_else(|| Path::new("."));
        return media_completions(src_dir, partial, images, offset);
    }

    if typed.bytes().all(|b| b.is_ascii_lowercase()) {
        let markers = [
            (Marker::Q, "question (front)"),
            (Marker::A, "answer (back)"),
            (Marker::R, "answer, reviewed both ways"),
            (Marker::T, "answer the learner types"),
            (Marker::E, "example / extra, shown on the back"),
            (Marker::Tags, "space-separated tags"),
            (Marker::Topic, "file-level subdeck name"),
        ];
        return markers
            .into_iter()
            .filter(|(m, _)| m.prefix().starts_with(typed))
            .map(|(m, detail)| Completion {
                label: m.prefix().to_string(),
                detail: Some(detail),
                kind: CompletionKind::Marker,
                insert: format!("{} ", m.prefix()),
                replace: line_start..offset,
            })
            .collect();
    }
    Vec::new()
}

/// If the cursor sits in the path of an unclosed `[image:…` / `[sound:…`,
/// the part of the path typed so far and whether it names an image.
fn open_media_path(typed: &str) -> Option<(&str, bool)> {
    let (open, is_image) = match (typed.rfind("[image:"), typed.rfind("[sound:")) {
        (Some(i), Some(s)) if i > s => (i, true),
        (Some(i), None) => (i, true),
        (_, Some(s)) => (s, false),
        (None, None) => return None,
    };
    let inner = &typed[open + "[image:".len()..];
    if inner.contains(']') || inner.starts_with("http://") || inner.starts_with("https://") {
        return None;
    }
    // `[image:caption:path`: the path follows the caption.
    let path = match inner.split_once(':') {
        Some((_, path)) if is_image => path,
        _ => inner,
    };
    Some((path, is_image))
}

/// Image extensions; every other media extension is audio or video, which
/// Anki plays through `[sound:…]`.
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "tiff", "tif",
];

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Subdirectories and media files of the wanted kind matching `partial`, a
/// path relative to `src_dir` typed up to byte `offset`.
fn media_completions(
    src_dir: &Path,
    partial: &str,
    images: bool,
    offset: usize,
) -> Vec<Completion> {
    let (dir, name) = match partial.rfind('/') {
        Some(slash) => (&partial[..=slash], &partial[slash + 1..]),
        None => ("", partial),
    };
    let Ok(entries) = std::fs::read_dir(src_dir.join(dir)) else {
        return Vec::new();
    };
    let mut found: Vec<Completion> = entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            if file_name.starts_with('.') || !file_name.starts_with(name) {
                return None;
            }
            let (label, kind) = if entry.path().is_dir() {
                (format!("{file_name}/"), CompletionKind::Dir)
            } else if is_media_ext(&entry.path()) && is_image(&entry.path()) == images {
                (file_name, CompletionKind::File)
            } else {
                return None;
            };
            Some(Completion {
                insert: label.clone(),
                label,
                detail: None,
                kind,
                replace: offset - name.len()..offset,
            })
        })
        .collect();
    found.sort_by(|a, b| a.label.cmp(&b.label));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located<'a>(text: &'a str, diags: &[Diag]) -> Vec<(&'a str, String)> {
        diags
            .iter()
            .map(|d| {
                let s = d.span;
                (
                    &text[s.byte_start..s.byte_start + s.byte_len],
                    d.message.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn reports_everything_build_would() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("here.png"), b"x").unwrap();
        let text = "q: no answer\n\n\
                    // @id 01A model=type-in\nq: Q\na: [image:here.png] [image:cap:gone.png]\n\n\
                    q: {{c1::a}} and ==b==\n\n\
                    q: ok\na: **[sound:https://x/y.mp3]**\n- [sound:nope.mp3]\n";
        let a = analyze(text, &tmp.path().join("a.cards"));
        assert_eq!(a.file.cards.len(), 3);
        let got = located(text, &a.diags);
        let got: Vec<(&str, &str)> = got.iter().map(|(s, m)| (*s, m.as_str())).collect();
        assert_eq!(
            got,
            [
                ("q: no answer", "card has `q:` but no answer"),
                (
                    "// @id 01A model=type-in",
                    "card is pinned to `model=type-in` but its markers now make it a `basic-example` card"
                ),
                (
                    "q: {{c1::a}} and ==b==",
                    "cloze field mixes `==…==` shorthand with native `{{c…}}`"
                ),
            ]
        );
        let got = located(text, &a.warnings);
        let got: Vec<(&str, &str)> = got.iter().map(|(s, m)| (*s, m.as_str())).collect();
        assert_eq!(
            got,
            [
                ("gone.png", "media file not found: gone.png"),
                ("nope.mp3", "media file not found: nope.mp3"),
            ]
        );
    }

    #[test]
    fn finds_duplicate_ids_here_and_elsewhere() {
        let text = "// @id 01A\nq: Q\nt: 1\n\n// @id 01B\nq: Q\nt: 2\n\n// @id 01A\nq: Q\nt: 3\n";
        let a = analyze(text, Path::new("a.cards"));
        let others = HashMap::from([("01B".to_string(), PathBuf::from("b.cards"))]);
        let diags = duplicate_id_diags(&a.file, &others);
        let lines: Vec<u32> = diags.iter().map(|d| d.span.line_start).collect();
        assert_eq!(lines, [5, 9]);
        assert!(diags[0].message.contains("in b.cards"));
        assert!(diags[1].message.contains("earlier in this file"));
    }

    #[test]
    fn hover_describes_the_note() {
        let text = "tags: es\n\n// @id 01A model=reversed\nq: hello\nr: hola\ntags: greet\n\n\
                    q: ==a== ==b==\n\n// @id 01B model=cloze\nq: Q\na: A\n";
        let a = analyze(text, Path::new("a.cards"));
        let at = |needle: &str| hover(&a, text.find(needle).unwrap()).unwrap().0;
        assert_eq!(
            at("hola"),
            "**Basic + Reversed** — `model=reversed`\n\n2 cards: `q` → `r` and `r` → `q`\n\n\
             Tags: `es` `greet`\n\nid `01A`"
        );
        assert!(at("==a==").starts_with("**Cloze** — `model=cloze`\n\n2 cards, one per deletion"));
        assert!(at("==a==").ends_with("minted by the next `ankigen build`."));
        assert!(
            at("q: Q").starts_with("**Note type conflict**\n\ncard is pinned to `model=cloze`")
        );
        // The directive block is not a card.
        assert!(hover(&a, 0).is_none());
        assert_eq!(cloze_card_count("{{c1::a}} {{c2::b}} {{c1::c}}"), 2);
    }

    #[test]
    fn completes_markers_models_and_media() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("img")).unwrap();
        for f in [
            "img/star.png",
            "img/notes.txt",
            "sun.mp3",
            "sunset.png",
            "a.cards",
        ] {
            std::fs::write(tmp.path().join(f), b"x").unwrap();
        }
        let path = tmp.path().join("a.cards");
        // Complete at the end of `text`, returning (label, replaced text).
        let at_end = |text: &str| -> Vec<(String, String)> {
            complete(text, &path, text.len())
                .into_iter()
                .map(|c| (c.label, text[c.replace].to_string()))
                .collect()
        };
        let labels =
            |text: &str| -> Vec<String> { at_end(text).into_iter().map(|(l, _)| l).collect() };

        assert_eq!(labels("q: Q\nt"), ["t:", "tags:", "topic:"]);
        assert_eq!(labels("q: Q\n").len(), 7);
        assert!(labels("q: Q\nnot a marker").is_empty());

        assert_eq!(
            at_end("// @id 01A model=b"),
            [
                ("basic-example".to_string(), "b".to_string()),
                ("basic".to_string(), "b".to_string())
            ]
        );
        assert!(labels("// @id 01A model=cloze ").is_empty());

        assert_eq!(labels("q: [image:"), ["img/", "sunset.png"]);
        assert_eq!(labels("q: x [sound:su"), ["sun.mp3"]);
        assert_eq!(
            at_end("q: [image:A star:img/s"),
            [("star.png".to_string(), "s".to_string())]
        );
        assert!(labels("q: [image:done.png] t").is_empty());
        assert!(labels("q: [image:https://x/").is_empty());
    }
}
