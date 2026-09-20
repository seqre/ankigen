//! Idempotent `<!-- @id … -->` injection and `model=` pinning.
//!
//! Minimal-diff: for each card lacking an id we insert a comment line at the
//! **start** of its block, and for each legacy `q:/a:` card whose `@id` line
//! lacks a `model=` suffix we rewrite that one line in place to pin its note
//! type (a one-time migration). Edits are applied back-to-front so byte offsets
//! stay valid and every untouched byte is preserved. Once every card carries
//! its id (and `model=` where needed), re-running produces no edits — the file
//! is byte-identical.

use crate::model::{CardKindSpec, ParsedFile};

/// Returns the rewritten file with ids injected / model markers pinned, or
/// `None` if nothing to do. Cards must already have their (minted) id set in
/// `spec.id` and their note type set in `resolved_model`.
pub fn inject_ids(input: &str, parsed: &ParsedFile) -> Option<String> {
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
            let line = if is_basic_kind {
                format!("<!-- @id {id} model={} -->{eol}", model().as_str())
            } else {
                format!("<!-- @id {id} -->{eol}")
            };
            let at = c.block_span.byte_start;
            edits.push((at, at, line));
        } else if is_basic_kind && c.model_marker.is_none() {
            // Legacy card: pin the note type on its existing `@id` line.
            let span = c.id_span.expect("id present ⇒ id_span recorded");
            let start = span.byte_start;
            let end = start + span.byte_len;
            edits.push((start, end, format!("<!-- @id {id} model={} -->", model().as_str())));
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::model::ModelKey;
    use crate::source::cards;

    #[test]
    fn injects_then_idempotent() {
        let src = "q: Q\na: A\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        pf.cards[0].spec.id = Some("01XYZ".into());
        pf.cards[0].resolved_model = Some(ModelKey::BasicExample);

        let out = inject_ids(src, &pf).unwrap();
        // A brand-new `q:/a:` card unifies on Basic+Example, pinned on its line.
        assert_eq!(out, "<!-- @id 01XYZ model=basic-example -->\nq: Q\na: A\n");

        // Re-parsing the output: the id + marker are present, so no further edits.
        let pf2 = cards::parse(&out, Path::new("t")).unwrap();
        assert!(inject_ids(&out, &pf2).is_none());
    }

    #[test]
    fn upgrades_legacy_basic_in_place() {
        // A card that already carries a bare `@id` predates the unification.
        let src = "<!-- @id 01XYZ -->\nq: Q\na: A\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        assert!(pf.cards[0].id_present);
        assert_eq!(pf.cards[0].model_marker, None);
        pf.cards[0].resolved_model = Some(ModelKey::Basic);

        let out = inject_ids(src, &pf).unwrap();
        assert_eq!(out, "<!-- @id 01XYZ model=basic -->\nq: Q\na: A\n");

        // Re-parsing the upgraded file: marker present ⇒ byte-identical rebuild.
        let pf2 = cards::parse(&out, Path::new("t")).unwrap();
        assert!(inject_ids(&out, &pf2).is_none());
    }

    #[test]
    fn honors_existing_marker() {
        let src = "<!-- @id 01XYZ model=basic-example -->\nq: Q\na: A\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        assert_eq!(pf.cards[0].model_marker, Some(ModelKey::BasicExample));
        pf.cards[0].resolved_model = Some(ModelKey::BasicExample);
        assert!(inject_ids(src, &pf).is_none());
    }

    #[test]
    fn preserves_crlf() {
        let src = "q: Q\r\na: A\r\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        pf.cards[0].spec.id = Some("01XYZ".into());
        pf.cards[0].resolved_model = Some(ModelKey::BasicExample);
        let out = inject_ids(src, &pf).unwrap();
        assert_eq!(
            out,
            "<!-- @id 01XYZ model=basic-example -->\r\nq: Q\r\na: A\r\n"
        );
    }
}
