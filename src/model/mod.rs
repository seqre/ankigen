//! Card data model: the serde-defined [`spec`] (the format itself) and the
//! parser-output [`card`] types.

pub mod card;
pub mod spec;

pub use card::{MediaKind, ParsedCard, ParsedFile, SourceSpan};
pub use spec::{CardKindSpec, CardSpec, ModelKey};
