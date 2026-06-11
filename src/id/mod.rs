//! Stable identity.
//!
//! Card ids are ULIDs persisted in the source via `<!-- @id … -->`. The Anki
//! GUID is a plain, human-readable string derived from `(model, ulid)` — never
//! hashed, never content-derived, so editing a card keeps its identity (and
//! thus its Anki review history). Model ids are hardcoded constants (see
//! [`crate::anki::models`]); deck ids are minted in the same random range and
//! persisted per-directory (see [`crate::deck`]).

use ulid::Ulid;

use crate::model::ModelKey;

/// Mint a fresh card id.
pub fn mint_card_id() -> Ulid {
    Ulid::new()
}

/// The Anki note GUID: `ankigen::<model>::<ulid>`.
pub fn guid_for(id: &Ulid, model: ModelKey) -> String {
    format!("ankigen::{}::{}", model.as_str(), id)
}

/// Mint a deck id in `[2^30, 2^31)` (the equivalent of Python's
/// `random.randrange(1 << 30, 1 << 31)`).
pub fn mint_deck_id() -> i64 {
    use rand::RngExt;
    rand::rng().random_range((1i64 << 30)..(1i64 << 31))
}
