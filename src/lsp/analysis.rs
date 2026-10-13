//! Everything the server knows about one document, computed from its text
//! alone (plus the media files next to it). No protocol types here, so it is
//! testable without a client.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::check;
use crate::model::{BlockSyntax, CardKindSpec, Diag, Marker, ParsedCard, ParsedFile};
use crate::render::inline::Inline;
use crate::render::{cloze, walk_inlines};
use crate::source::cards::{Parsed, parse_lenient};

pub struct Analysis {
    pub file: ParsedFile,
    /// Every problem `ankigen build` would report for this file on its own,
    /// in source order. Duplicate ids need the other files: see
    /// [`duplicate_id_diags`].
    pub diags: Vec<Diag>,
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
    for block in &file.blocks {
        missing_media(block, src_dir, &mut diags);
    }

    diags.sort_by_key(|d| d.span.byte_start);
    Analysis { file, diags }
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
                ("gone.png", "media file not found: gone.png"),
                (
                    "q: {{c1::a}} and ==b==",
                    "cloze field mixes `==…==` shorthand with native `{{c…}}`"
                ),
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
}
