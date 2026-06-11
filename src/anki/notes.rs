//! Build a genanki [`Note`] from already-rendered fields.
//!
//! We **always** pass an explicit GUID — never genanki's default, which hashes
//! the field contents (unstable + content-derived) and would silently
//! duplicate cards on re-import.

use genanki_rs::Note;

use crate::error::Result;
use crate::model::ModelKey;

use super::models;

/// `fields` must already be in the model's field order (see [`crate::build`]).
pub fn build_note(key: ModelKey, fields: &[String], guid: &str, tags: &[String]) -> Result<Note> {
    let model = models::build(key);
    let field_refs: Vec<&str> = fields.iter().map(String::as_str).collect();
    let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
    let note = Note::new_with_options(
        model,
        field_refs,
        Some(true),
        Some(tag_refs),
        Some(guid),
    )?;
    Ok(note)
}
