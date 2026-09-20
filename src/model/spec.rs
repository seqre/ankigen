//! The canonical, format-agnostic card model.
//!
//! This is *the* definition of a card. The text parser lowers source into
//! [`CardSpec`]s; a future YAML/CSV front-end would deserialize straight into
//! `Vec<CardSpec>` via serde. Field strings are stored **raw** (pre-render) so
//! that HTML rendering is shared by every front-end.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CardSpec {
    /// ULID, once assigned. `None` until minted + persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub kind: CardKindSpec,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CardKindSpec {
    Basic {
        question: String,
        answer: String,
    },
    BasicExample {
        question: String,
        answer: String,
        example: String,
    },
    Cloze {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        back_extra: Option<String>,
    },
    TypeIn {
        question: String,
        /// Plain text: Anki's `{{type:Answer}}` compares literally.
        answer: String,
    },
}

impl CardKindSpec {
    pub fn model_key(&self) -> ModelKey {
        match self {
            CardKindSpec::Basic { .. } => ModelKey::Basic,
            CardKindSpec::BasicExample { .. } => ModelKey::BasicExample,
            CardKindSpec::Cloze { .. } => ModelKey::Cloze,
            CardKindSpec::TypeIn { .. } => ModelKey::TypeIn,
        }
    }
}

/// Stable discriminator for the four note types. Its [`ModelKey::as_str`]
/// values are **frozen** — they namespace the GUID and feed the note type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKey {
    Basic,
    BasicExample,
    Cloze,
    TypeIn,
}

impl ModelKey {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKey::Basic => "basic",
            ModelKey::BasicExample => "basic-example",
            ModelKey::Cloze => "cloze",
            ModelKey::TypeIn => "type-in",
        }
    }

    /// Inverse of [`ModelKey::as_str`]: parse a persisted model key.
    pub fn parse(s: &str) -> Option<ModelKey> {
        match s {
            "basic" => Some(ModelKey::Basic),
            "basic-example" => Some(ModelKey::BasicExample),
            "cloze" => Some(ModelKey::Cloze),
            "type-in" => Some(ModelKey::TypeIn),
            _ => None,
        }
    }
}
