//! `ankigen` — compile simple text flashcards into an Anki `.apkg`, with stable
//! per-card identity so edits update existing cards instead of duplicating them.
//!
//! This is primarily a library; the `ankigen` binary is a thin wrapper over
//! [`run`].

pub mod anki;
pub mod error;
pub mod id;
pub mod media;
pub mod model;
pub mod render;
pub mod source;

pub use error::{AnkigenError, Result};
pub use model::{CardKindSpec, CardSpec, ModelKey, ParsedCard, ParsedFile};
