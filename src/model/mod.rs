//! Card data model: the serde-defined [`spec`] (the format itself) and the
//! parser-output [`card`] types.

pub mod card;
pub mod spec;

pub use card::{
    BlockSyntax, Diag, FieldSrc, IdSyntax, LineKind, Marker, MediaKind, ParsedCard, ParsedFile,
    SourceSpan, SyntaxLine,
};
pub use spec::{CardKindSpec, CardSpec, ModelKey};
