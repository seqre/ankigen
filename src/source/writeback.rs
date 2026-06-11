//! Idempotent `<!-- @id … -->` injection.
//!
//! Insert-only and minimal-diff: for each card lacking an id we insert a
//! comment line at the **start** of its block. Edits are applied back-to-front
//! so byte offsets stay valid and every untouched byte is preserved. Re-running
//! finds all ids present and produces no edits.

use crate::model::ParsedFile;

/// Returns the rewritten file with ids injected, or `None` if nothing to do.
/// Cards must already have their (minted) id set in `spec.id`.
pub fn inject_ids(input: &str, parsed: &ParsedFile) -> Option<String> {
    let eol = if input.contains("\r\n") { "\r\n" } else { "\n" };

    let mut edits: Vec<(usize, String)> = parsed
        .cards
        .iter()
        .filter(|c| !c.id_present)
        .filter_map(|c| {
            c.spec
                .id
                .as_ref()
                .map(|id| (c.block_span.byte_start, format!("<!-- @id {id} -->{eol}")))
        })
        .collect();

    if edits.is_empty() {
        return None;
    }

    // Apply from the end so earlier offsets remain valid.
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = input.to_string();
    for (at, ins) in edits {
        out.insert_str(at, &ins);
    }
    Some(out)
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

        let out = inject_ids(src, &pf).unwrap();
        assert_eq!(out, "<!-- @id 01XYZ -->\nq: Q\na: A\n");

        // Re-parsing the output: the id is now present, so no further edits.
        let pf2 = cards::parse(&out, Path::new("t")).unwrap();
        assert!(inject_ids(&out, &pf2).is_none());
    }

    #[test]
    fn preserves_crlf() {
        let src = "q: Q\r\na: A\r\n";
        let mut pf = cards::parse(src, Path::new("t")).unwrap();
        pf.cards[0].spec.id = Some("01XYZ".into());
        let out = inject_ids(src, &pf).unwrap();
        assert_eq!(out, "<!-- @id 01XYZ -->\r\nq: Q\r\na: A\r\n");
    }
}
