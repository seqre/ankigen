//! Error type for ankigen.
//!
//! Most user-facing failures are *compiler-style* diagnostics with a source
//! span, so we lean on [`miette`] to render the offending line with a caret.
//! A single flexible [`AnkigenError::Parse`] variant covers the many
//! validation errors (it carries a message + label + optional help), which
//! keeps the call sites terse while still producing rich output.

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

use crate::model::card::SourceSpan as Span;

pub type Result<T> = std::result::Result<T, AnkigenError>;

#[derive(Debug, Error, Diagnostic)]
pub enum AnkigenError {
    /// A parse or validation error tied to a location in a source file.
    #[error("{message}")]
    Parse {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        span: SourceSpan,
        #[help]
        help: Option<String>,
    },

    #[error("media file not found: {path}")]
    #[diagnostic(code(ankigen::missing_media))]
    MissingMedia {
        path: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("referenced here")]
        span: SourceSpan,
    },

    #[error("duplicate @id `{id}` (used by more than one card)")]
    #[diagnostic(
        code(ankigen::duplicate_id),
        help("each card's id must be unique; delete one or let ankigen mint a fresh one")
    )]
    DuplicateId { id: String },

    #[error("{0}")]
    #[diagnostic(code(ankigen::io))]
    Io(String),

    #[error("anki package error: {0}")]
    #[diagnostic(code(ankigen::anki))]
    Anki(String),
}

impl From<genanki_rs::Error> for AnkigenError {
    fn from(e: genanki_rs::Error) -> Self {
        AnkigenError::Anki(e.to_string())
    }
}

impl From<std::io::Error> for AnkigenError {
    fn from(e: std::io::Error) -> Self {
        AnkigenError::Io(e.to_string())
    }
}

impl AnkigenError {
    /// Build a [`AnkigenError::Parse`] from an internal [`Span`].
    pub fn parse(
        file: &std::path::Path,
        text: &str,
        span: Span,
        message: impl Into<String>,
        help: Option<&str>,
    ) -> Self {
        AnkigenError::Parse {
            message: message.into(),
            src: NamedSource::new(file.display().to_string(), text.to_string()),
            span: SourceSpan::new(span.byte_start.into(), span.byte_len),
            help: help.map(|h| h.to_string()),
        }
    }

    pub fn missing_media(file: &std::path::Path, text: &str, span: Span, path: &str) -> Self {
        AnkigenError::MissingMedia {
            path: path.to_string(),
            src: NamedSource::new(file.display().to_string(), text.to_string()),
            span: SourceSpan::new(span.byte_start.into(), span.byte_len),
        }
    }
}
