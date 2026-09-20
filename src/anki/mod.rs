//! genanki glue: the frozen [`models`] and the [`notes`] builder.

pub mod models;
pub mod notes;

pub use models::{build, model_id};
pub use notes::build_note;
