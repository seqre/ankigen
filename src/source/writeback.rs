//! Idempotent `// @id …` injection, `model=` pinning, and source upgrades.
//!
//! Minimal-diff: for each card lacking an id we insert a comment line at the
//! **start** of its block, and for each legacy `q:/a:` card whose `@id` line
//! lacks a `model=` suffix we rewrite that one line in place to pin its note
//! type (a one-time migration). With `upgrade_source`, existing `<!-- @id … -->`
//! lines are also rewritten to the canonical `// @id …` form. Edits are applied
//! back-to-front so byte offsets stay valid and every untouched byte is
//! preserved. Once every card carries its id (and `model=` where needed),
//! re-running produces no edits — the file is byte-identical.

use crate::model::{CardKindSpec, IdSyntax, ModelKey, ParsedFile};

/// Returns the rewritten file with ids injected / model markers pinned (and,
/// with `upgrade_source`, legacy `<!-- @id -->` lines converted), or `None` if
/// nothing to do. Cards must already have their (minted) id set in `spec.id`
/// and their note type set in `resolved_model`.
pub fn inject_ids(input: &str, parsed: &ParsedFile, upgrade_source: bool) -> Option<String> {
    let eol = if input.contains("\r\n") { "\r\n" } else { "\n" };

    // (start, end, replacement); an insertion is `start == end`.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for c in &parsed.cards {
        let Some(id) = c.spec.id.as_deref() else {
            continue;
        };
        // Only the ambiguous `q:/a:` kind persists its resolved note type.
        let is_basic_kind = matches!(c.spec.kind, CardKindSpec::Basic { .. });
        let model = || c.resolved_model.expect("model resolved before write-back");

        if !c.id_present {
            let line = id_line(id, is_basic_kind.then(model));
            let at = c.block_span.byte_start;
            edits.push((at, at, format!("{line}{eol}")));
            continue;
        }

        // Rewrite the existing line to pin a legacy card's note type, or to
        // upgrade its syntax; an existing `model=` marker is always kept.
        let pin = is_basic_kind && c.model_marker.is_none();
        let upgrade = upgrade_source && c.id_syntax == Some(IdSyntax::Html);
        if pin || upgrade {
            let marker = if pin { Some(model()) } else { c.model_marker };
            let span = c.id_span.expect("id present ⇒ id_span recorded");
            let start = span.byte_start;
            edits.push((start, start + span.byte_len, id_line(id, marker)));
        }
    }

    if edits.is_empty() {
        return None;
    }

    // Apply from the end so earlier offsets remain valid.
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = input.to_string();
    for (start, end, repl) in edits {
        out.replace_range(start..end, &repl);
    }
    Some(out)
}

/// The canonical `@id` comment line (without line ending).
fn id_line(id: &str, model: Option<ModelKey>) -> String {
    match model {
        Some(m) => format!("// @id {id} model={}", m.as_str()),
        None => format!("// @id {id}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::source::cards;

    #[test]
    fn injects_then_idempotent() {
        let src = "q: Q\na: A\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        pf.cards[0].spec.id = Some("01XYZ".into());
        pf.cards[0].resolved_model = Some(ModelKey::BasicExample);

        let out = inject_ids(src, &pf, false).unwrap();
        // A brand-new `q:/a:` card unifies on Basic+Example, pinned on its line.
        assert_eq!(out, "// @id 01XYZ model=basic-example\nq: Q\na: A\n");

        // Re-parsing the output: the id + marker are present, so no further edits.
        let pf2 = cards::parse(&out, Path::new("t")).unwrap();
        assert!(inject_ids(&out, &pf2, false).is_none());
        assert!(inject_ids(&out, &pf2, true).is_none());
    }

    #[test]
    fn upgrades_legacy_basic_in_place() {
        // A card that already carries a bare `@id` predates the unification.
        let src = "<!-- @id 01XYZ -->\nq: Q\na: A\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        assert!(pf.cards[0].id_present);
        assert_eq!(pf.cards[0].model_marker, None);
        pf.cards[0].resolved_model = Some(ModelKey::Basic);

        let out = inject_ids(src, &pf, false).unwrap();
        assert_eq!(out, "// @id 01XYZ model=basic\nq: Q\na: A\n");

        // Re-parsing the upgraded file: marker present ⇒ byte-identical rebuild.
        let pf2 = cards::parse(&out, Path::new("t")).unwrap();
        assert!(inject_ids(&out, &pf2, false).is_none());
    }

    #[test]
    fn honors_existing_marker() {
        let src = "<!-- @id 01XYZ model=basic-example -->\nq: Q\na: A\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        assert_eq!(pf.cards[0].model_marker, Some(ModelKey::BasicExample));
        pf.cards[0].resolved_model = Some(ModelKey::BasicExample);
        // Legacy HTML syntax is left alone unless an upgrade is requested.
        assert!(inject_ids(src, &pf, false).is_none());
    }

    #[test]
    fn upgrade_source_converts_html_ids() {
        let src = "<!-- @id 01AAA model=basic -->\r\nq: Q\r\na: A\r\n\r\n\
                   <!-- @id 01BBB -->\r\nq: Q2\r\nt: 200\r\n\r\n\
                   // @id 01CCC model=basic-example\r\nq: Q3\r\na: A3\r\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        for (c, m) in
            pf.cards
                .iter_mut()
                .zip([ModelKey::Basic, ModelKey::TypeIn, ModelKey::BasicExample])
        {
            c.resolved_model = Some(m);
        }

        let out = inject_ids(src, &pf, true).unwrap();
        assert_eq!(
            out,
            "// @id 01AAA model=basic\r\nq: Q\r\na: A\r\n\r\n\
             // @id 01BBB\r\nq: Q2\r\nt: 200\r\n\r\n\
             // @id 01CCC model=basic-example\r\nq: Q3\r\na: A3\r\n"
        );

        let pf2 = cards::parse(&out, Path::new("t")).unwrap();
        assert!(inject_ids(&out, &pf2, true).is_none());
    }

    #[test]
    fn preserves_crlf() {
        let src = "q: Q\r\na: A\r\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        pf.cards[0].spec.id = Some("01XYZ".into());
        pf.cards[0].resolved_model = Some(ModelKey::BasicExample);
        let out = inject_ids(src, &pf, false).unwrap();
        assert_eq!(out, "// @id 01XYZ model=basic-example\r\nq: Q\r\na: A\r\n");
    }
}
