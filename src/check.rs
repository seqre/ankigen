//! Validation shared by the build pipeline and the language server.

use crate::model::{CardKindSpec, Diag, ModelKey, ParsedCard};

/// Resolve a card's note type. The `model=` marker on the `@id` line is the
/// durable source of truth: a pinned card whose markers now describe another
/// note type is rejected, since Anki refuses to change a note's type and the
/// swap would orphan it.
///
/// A card with no marker is either brand new or predates pinning, and takes
/// the note type its markers describe. Only `q:/a:` is ambiguous there: one
/// that already had an `@id` predates the Basic/Basic+Example unification
/// (⇒ `Basic`), while a brand-new one unifies on `BasicExample` (rendered with
/// an empty example). `r:` postdates pinning, so a bare `@id` under it was
/// minted for some other note type.
pub fn resolve_model(card: &ParsedCard) -> Result<ModelKey, Diag> {
    // Every failure involves the `@id` line, so that is where it points.
    let fail = |message: String, help: &str| Diag {
        span: card.id_span.unwrap_or(card.block_span),
        message,
        help: Some(help.to_string()),
    };
    let kind = &card.spec.kind;
    let natural = match kind {
        CardKindSpec::Basic { .. } => ModelKey::BasicExample,
        other => other.model_key(),
    };
    match card.model_marker {
        Some(m) if m == natural => Ok(m),
        Some(ModelKey::Basic) if matches!(kind, CardKindSpec::Basic { .. }) => Ok(ModelKey::Basic),
        Some(ModelKey::Basic) if matches!(kind, CardKindSpec::BasicExample { .. }) => Err(fail(
            "card has an example but is pinned to the legacy Basic note type".into(),
            "remove the `e:` example, or drop the `model=basic` marker to migrate it",
        )),
        Some(m) => Err(fail(
            format!(
                "card is pinned to `model={}` but its markers now make it a `{}` card",
                m.as_str(),
                natural.as_str()
            ),
            "restore the original markers, or remove the `@id` line to create a new card",
        )),
        None if natural == ModelKey::Reversed && card.id_present => Err(fail(
            "`r:` on a card whose `@id` belongs to a one-way note".into(),
            "remove the `@id` line to create a new two-way card, or change `r:` back",
        )),
        None if card.id_present => Ok(kind.model_key()),
        None => Ok(natural),
    }
}
