//! The five frozen note models.
//!
//! Each model's id is a hardcoded constant (generated once via
//! `random.randrange(1 << 30, 1 << 31)`). **These ids and the field lists must
//! never change** — Anki refuses to update notes whose note type changed.

use genanki_rs::{Field, Model, ModelType, Template};

use crate::model::ModelKey;

pub const MODEL_BASIC: i64 = 1_116_934_346;
pub const MODEL_BASIC_EXAMPLE: i64 = 1_163_615_497;
pub const MODEL_CLOZE: i64 = 1_748_340_386;
pub const MODEL_TYPEIN: i64 = 1_284_618_157;
pub const MODEL_REVERSED: i64 = 1_851_392_596;

pub fn model_id(key: ModelKey) -> i64 {
    match key {
        ModelKey::Basic => MODEL_BASIC,
        ModelKey::BasicExample => MODEL_BASIC_EXAMPLE,
        ModelKey::Cloze => MODEL_CLOZE,
        ModelKey::TypeIn => MODEL_TYPEIN,
        ModelKey::Reversed => MODEL_REVERSED,
    }
}

/// A model's field names, in order. Frozen along with the ids.
pub fn field_names(key: ModelKey) -> &'static [&'static str] {
    match key {
        ModelKey::Basic => &["Front", "Back", "ID"],
        ModelKey::BasicExample | ModelKey::Reversed => &["Front", "Back", "Example", "ID"],
        ModelKey::Cloze => &["Text", "Back Extra", "ID"],
        ModelKey::TypeIn => &["Front", "Answer", "ID"],
    }
}

fn fields(key: ModelKey) -> Vec<Field> {
    field_names(key)
        .iter()
        .map(|name| Field::new(name))
        .collect()
}

const CSS: &str = r#".card {
  font-family: -apple-system, "Segoe UI", Roboto, sans-serif;
  font-size: 20px;
  text-align: center;
  color: #1a1a1a;
  background: #ffffff;
}
.night_mode .card { color: #e6e6e6; background: #2b2b2b; }
.extra { font-size: 16px; color: #666; margin-top: 1em; }
.night_mode .extra { color: #aaa; }
.cloze { font-weight: bold; color: #1565c0; }
.night_mode .cloze { color: #64b5f6; }
figure { margin: 0; }
figcaption { font-size: 0.85em; color: #666; margin-top: 0.3em; }
img { max-width: 100%; height: auto; }
ul, ol { text-align: left; display: inline-block; }
pre { text-align: left; }
code { background: rgba(135,131,120,0.15); border-radius: 3px; padding: 0 0.3em; }
a { color: #1565c0; }
hr#answer { border: none; border-top: 1px solid #ccc; margin: 1em 0; }
"#;

/// Build the genanki [`Model`] for a card type. Cheap; called per note.
pub fn build(key: ModelKey) -> Model {
    match key {
        ModelKey::Basic => Model::new(
            MODEL_BASIC,
            "ankigen Basic",
            fields(key),
            vec![
                Template::new("Card 1")
                    .qfmt("{{Front}}")
                    .afmt(r#"{{FrontSide}}<hr id="answer">{{Back}}"#),
            ],
        )
        .css(CSS)
        .sort_field_index(2),

        ModelKey::BasicExample => Model::new(
            MODEL_BASIC_EXAMPLE,
            "ankigen Basic+Example",
            fields(key),
            vec![
                Template::new("Card 1").qfmt("{{Front}}").afmt(
                    r#"{{FrontSide}}<hr id="answer">{{Back}}{{#Example}}<div class="extra">{{Example}}</div>{{/Example}}"#,
                ),
            ],
        )
        .css(CSS)
        .sort_field_index(3),

        ModelKey::TypeIn => Model::new(
            MODEL_TYPEIN,
            "ankigen Type-in",
            fields(key),
            vec![
                Template::new("Card 1")
                    .qfmt(r#"{{Front}}<br>{{type:Answer}}"#)
                    .afmt(r#"{{FrontSide}}<hr id="answer">{{Answer}}<br>{{type:Answer}}"#),
            ],
        )
        .css(CSS)
        .sort_field_index(2),

        ModelKey::Reversed => Model::new(
            MODEL_REVERSED,
            "ankigen Basic+Reversed",
            fields(key),
            vec![
                Template::new("Forward").qfmt("{{Front}}").afmt(
                    r#"{{FrontSide}}<hr id="answer">{{Back}}{{#Example}}<div class="extra">{{Example}}</div>{{/Example}}"#,
                ),
                Template::new("Reverse").qfmt("{{Back}}").afmt(
                    r#"{{FrontSide}}<hr id="answer">{{Front}}{{#Example}}<div class="extra">{{Example}}</div>{{/Example}}"#,
                ),
            ],
        )
        .css(CSS)
        .sort_field_index(3),

        ModelKey::Cloze => Model::new(
            MODEL_CLOZE,
            "ankigen Cloze",
            fields(key),
            vec![
                Template::new("Cloze").qfmt("{{cloze:Text}}").afmt(
                    r#"{{cloze:Text}}{{#Back Extra}}<div class="extra">{{Back Extra}}</div>{{/Back Extra}}"#,
                ),
            ],
        )
        .css(CSS)
        .model_type(ModelType::Cloze)
        .sort_field_index(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [ModelKey; 5] = [
        ModelKey::Basic,
        ModelKey::BasicExample,
        ModelKey::Cloze,
        ModelKey::TypeIn,
        ModelKey::Reversed,
    ];

    // These three tests pin values that existing decks depend on. A failure
    // means the change orphans notes: revert it, don't update the expectation.

    #[test]
    fn frozen_model_ids() {
        let ids: Vec<i64> = KEYS.into_iter().map(model_id).collect();
        assert_eq!(
            ids,
            [
                1_116_934_346,
                1_163_615_497,
                1_748_340_386,
                1_284_618_157,
                1_851_392_596
            ]
        );
        for key in KEYS {
            assert_eq!(build(key).id, model_id(key));
        }
    }

    #[test]
    fn frozen_model_keys() {
        let keys: Vec<&str> = KEYS.into_iter().map(ModelKey::as_str).collect();
        assert_eq!(
            keys,
            ["basic", "basic-example", "cloze", "type-in", "reversed"]
        );
    }

    #[test]
    fn frozen_field_lists() {
        let fields: Vec<&[&str]> = KEYS.into_iter().map(field_names).collect();
        let expected: [&[&str]; 5] = [
            &["Front", "Back", "ID"],
            &["Front", "Back", "Example", "ID"],
            &["Text", "Back Extra", "ID"],
            &["Front", "Answer", "ID"],
            &["Front", "Back", "Example", "ID"],
        ];
        assert_eq!(fields, expected);
    }
}
