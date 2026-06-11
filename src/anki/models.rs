//! The four frozen note models.
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

pub fn model_id(key: ModelKey) -> i64 {
    match key {
        ModelKey::Basic => MODEL_BASIC,
        ModelKey::BasicExample => MODEL_BASIC_EXAMPLE,
        ModelKey::Cloze => MODEL_CLOZE,
        ModelKey::TypeIn => MODEL_TYPEIN,
    }
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
            vec![Field::new("Front"), Field::new("Back")],
            vec![
                Template::new("Card 1")
                    .qfmt("{{Front}}")
                    .afmt(r#"{{FrontSide}}<hr id="answer">{{Back}}"#),
            ],
        )
        .css(CSS)
        .sort_field_index(0),

        ModelKey::BasicExample => Model::new(
            MODEL_BASIC_EXAMPLE,
            "ankigen Basic+Example",
            vec![
                Field::new("Front"),
                Field::new("Back"),
                Field::new("Example"),
            ],
            vec![
                Template::new("Card 1").qfmt("{{Front}}").afmt(
                    r#"{{FrontSide}}<hr id="answer">{{Back}}{{#Example}}<div class="extra">{{Example}}</div>{{/Example}}"#,
                ),
            ],
        )
        .css(CSS)
        .sort_field_index(0),

        ModelKey::TypeIn => Model::new(
            MODEL_TYPEIN,
            "ankigen Type-in",
            vec![Field::new("Front"), Field::new("Answer")],
            vec![
                Template::new("Card 1")
                    .qfmt(r#"{{Front}}<br>{{type:Answer}}"#)
                    .afmt(r#"{{FrontSide}}<hr id="answer">{{Answer}}<br>{{type:Answer}}"#),
            ],
        )
        .css(CSS)
        .sort_field_index(0),

        ModelKey::Cloze => Model::new(
            MODEL_CLOZE,
            "ankigen Cloze",
            vec![Field::new("Text"), Field::new("Back Extra")],
            vec![
                Template::new("Cloze").qfmt("{{cloze:Text}}").afmt(
                    r#"{{cloze:Text}}{{#Back Extra}}<div class="extra">{{Back Extra}}</div>{{/Back Extra}}"#,
                ),
            ],
        )
        .css(CSS)
        .model_type(ModelType::Cloze)
        .sort_field_index(0),
    }
}
