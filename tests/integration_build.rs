//! End-to-end: build an `.apkg`, unzip it, open the sqlite collection, and
//! assert the notes — including the headline guarantee that editing a card's
//! content (keeping its `@id`) keeps the same GUID, so Anki updates in place.

use std::io::{Read, Write};
use std::path::Path;

use ankigen::anki::models::{MODEL_BASIC, MODEL_BASIC_EXAMPLE, MODEL_CLOZE, MODEL_REVERSED};
use ankigen::pipeline::{BuildOptions, build};

struct NoteRow {
    guid: String,
    mid: i64,
    tags: String,
    flds: String,
}

/// The template ordinals of every card in the apkg, sorted.
fn read_card_ords(apkg: &Path) -> Vec<i64> {
    let file = std::fs::File::open(apkg).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("collection.anki2")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();

    let mut db = tempfile::NamedTempFile::new().unwrap();
    db.write_all(&bytes).unwrap();
    db.flush().unwrap();

    let conn = rusqlite::Connection::open(db.path()).unwrap();
    let mut stmt = conn.prepare("SELECT ord FROM cards ORDER BY ord").unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// Extract `collection.anki2` from the apkg zip and read all notes.
fn read_notes(apkg: &Path) -> Vec<NoteRow> {
    let file = std::fs::File::open(apkg).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("collection.anki2")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();

    let mut db = tempfile::NamedTempFile::new().unwrap();
    db.write_all(&bytes).unwrap();
    db.flush().unwrap();

    let conn = rusqlite::Connection::open(db.path()).unwrap();
    let mut stmt = conn
        .prepare("SELECT guid, mid, tags, flds FROM notes ORDER BY guid")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok(NoteRow {
                guid: r.get(0)?,
                mid: r.get(1)?,
                tags: r.get(2)?,
                flds: r.get(3)?,
            })
        })
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn build_opts(output: &Path) -> BuildOptions {
    BuildOptions {
        output: output.to_path_buf(),
        deck_prefix: None,
        check: false,
        write_back: true,
        upgrade_source: false,
        verbose: 0,
    }
}

#[test]
fn builds_notes_and_keeps_guid_on_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(cards.join("CS")).unwrap();
    let note_file = cards.join("CS/net.md");
    std::fs::write(
        &note_file,
        "topic: Networking\ntags: net\n\nq: What does HTTP stand for?\na: HyperText Transfer Protocol\n\nq: The handshake starts with ==SYN==.\n",
    )
    .unwrap();
    let out = tmp.path().join("deck.apkg");

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();

    let notes = read_notes(&out);
    assert_eq!(notes.len(), 2, "two cards");

    // A brand-new `q:/a:` card unifies on the Basic+Example note type.
    let basic = notes
        .iter()
        .find(|n| n.mid == MODEL_BASIC_EXAMPLE)
        .expect("basic+example note");
    let cloze = notes
        .iter()
        .find(|n| n.mid == MODEL_CLOZE)
        .expect("cloze note");

    assert!(basic.guid.starts_with("ankigen::basic-example::"));
    assert_eq!(basic.tags.trim(), "net");
    assert!(basic.flds.contains("HyperText Transfer Protocol"));
    assert!(cloze.flds.contains("{{c1::SYN}}"), "cloze auto-numbered");

    let basic_guid_before = basic.guid.clone();

    // The id line is written back pinned to the resolved note type.
    let text = std::fs::read_to_string(&note_file).unwrap();
    assert!(
        text.contains("// @id") && text.contains("model=basic-example"),
        "id + model marker written back"
    );
    // Edit the card's *content* but keep its `@id`.
    let edited = text.replace(
        "HyperText Transfer Protocol",
        "HyperText Transfer Protocol (a stateless protocol)",
    );
    std::fs::write(&note_file, edited).unwrap();

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();

    let notes2 = read_notes(&out);
    let basic2 = notes2
        .iter()
        .find(|n| n.mid == MODEL_BASIC_EXAMPLE)
        .unwrap();
    assert_eq!(
        basic2.guid, basic_guid_before,
        "editing content must NOT change the GUID (Anki updates in place)"
    );
    assert!(
        basic2.flds.contains("stateless protocol"),
        "content updated"
    );
}

#[test]
fn legacy_basic_card_stays_basic() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    let note_file = cards.join("legacy.cards");
    // A card built by an earlier ankigen: a bare `@id`, no `model=` marker.
    std::fs::write(
        &note_file,
        "<!-- @id 01ARZ3NDEKTSV4RRFFQ69G5FAV -->\nq: Q\na: A\n",
    )
    .unwrap();
    let out = tmp.path().join("deck.apkg");

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();

    let notes = read_notes(&out);
    assert_eq!(notes.len(), 1);
    assert_eq!(
        notes[0].mid, MODEL_BASIC,
        "legacy card keeps the Basic model"
    );
    assert_eq!(notes[0].guid, "ankigen::basic::01ARZ3NDEKTSV4RRFFQ69G5FAV");

    // The legacy line is upgraded once to pin the note type.
    let text = std::fs::read_to_string(&note_file).unwrap();
    assert_eq!(
        text,
        "// @id 01ARZ3NDEKTSV4RRFFQ69G5FAV model=basic\nq: Q\na: A\n"
    );

    // Rebuild: marker present ⇒ source byte-identical and GUID unchanged.
    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();
    assert_eq!(std::fs::read_to_string(&note_file).unwrap(), text);
    let notes2 = read_notes(&out);
    assert_eq!(notes2[0].guid, "ankigen::basic::01ARZ3NDEKTSV4RRFFQ69G5FAV");
}

#[test]
fn upgrade_source_converts_html_ids_keeping_guids() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    let note_file = cards.join("html.cards");
    let html = "<!-- @id 01ARZ3NDEKTSV4RRFFQ69G5FAV model=basic-example -->\nq: Q\na: A\n";
    std::fs::write(&note_file, html).unwrap();
    let out = tmp.path().join("deck.apkg");

    // Without the flag, legacy HTML ids are read but left untouched.
    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();
    assert_eq!(std::fs::read_to_string(&note_file).unwrap(), html);
    let guid = read_notes(&out)[0].guid.clone();

    let opts = BuildOptions {
        upgrade_source: true,
        ..build_opts(&out)
    };
    build(std::slice::from_ref(&cards), &opts).unwrap();
    let text = std::fs::read_to_string(&note_file).unwrap();
    assert_eq!(
        text,
        "// @id 01ARZ3NDEKTSV4RRFFQ69G5FAV model=basic-example\nq: Q\na: A\n"
    );
    assert_eq!(read_notes(&out)[0].guid, guid, "upgrade keeps the GUID");

    // Already canonical ⇒ a further upgrade is a no-op.
    build(std::slice::from_ref(&cards), &opts).unwrap();
    assert_eq!(std::fs::read_to_string(&note_file).unwrap(), text);
}

#[test]
fn check_mode_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    let f = cards.join("a.cards");
    std::fs::write(&f, "q: Q\na: A\n").unwrap();
    let before = std::fs::read_to_string(&f).unwrap();

    let out = tmp.path().join("deck.apkg");
    let opts = BuildOptions {
        output: out.clone(),
        deck_prefix: None,
        check: true,
        write_back: true,
        upgrade_source: false,
        verbose: 0,
    };
    build(std::slice::from_ref(&cards), &opts).unwrap();

    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        before,
        "source untouched"
    );
    assert!(!out.exists(), "no .apkg emitted");
    assert!(
        !cards.join("deck-id.txt").exists(),
        "no deck-id.txt written"
    );
}

#[test]
fn math_renders_to_mathjax_delimiters() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    std::fs::write(
        cards.join("math.cards"),
        "q: Bound for $x < y$?\na: It is $a+b$ at most:\n$$\\frac{a}{b}$$\n",
    )
    .unwrap();
    let out = tmp.path().join("deck.apkg");

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();

    let notes = read_notes(&out);
    assert_eq!(notes.len(), 1);
    let flds = &notes[0].flds;
    // Inline `$…$` → `\(…\)`, display `$$…$$` → `\[…\]`; `<` entity-escaped.
    assert!(flds.contains("\\(x &lt; y\\)"), "inline math: {flds}");
    assert!(flds.contains("\\(a+b\\)"), "inline math in answer: {flds}");
    assert!(flds.contains("\\[\\frac{a}{b}\\]"), "display math: {flds}");
}

#[test]
fn missing_media_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    std::fs::write(cards.join("a.cards"), "q: Pic\na: [image:nope.png]\n").unwrap();
    let out = tmp.path().join("deck.apkg");
    let err = build(std::slice::from_ref(&cards), &build_opts(&out));
    assert!(err.is_err(), "missing media should fail the build");
}

#[test]
fn all_missing_media_reported_together() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    // Two cards, three missing references total — all should be collected.
    std::fs::write(
        cards.join("a.cards"),
        "q: One\na: [image:gone1.png] and [image:gone2.png]\n\nq: Two\na: [sound:gone3.mp3]\n",
    )
    .unwrap();
    let out = tmp.path().join("deck.apkg");
    let err = build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap_err();
    match err {
        ankigen::AnkigenError::MissingMediaBatch { count, errors } => {
            assert_eq!(
                count, 3,
                "every missing reference is collected, not just the first"
            );
            assert_eq!(errors.len(), 3);
        }
        other => panic!("expected MissingMediaBatch, got {other:?}"),
    }
}

#[test]
fn reversed_card_makes_one_note_with_two_cards() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    let note_file = cards.join("vocab.cards");
    std::fs::write(&note_file, "q: hello\nr: hola\ne: ¡Hola, amigo!\n").unwrap();
    let out = tmp.path().join("deck.apkg");

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();

    let notes = read_notes(&out);
    assert_eq!(notes.len(), 1, "one note");
    assert_eq!(notes[0].mid, MODEL_REVERSED);
    assert!(notes[0].guid.starts_with("ankigen::reversed::"));
    let flds: Vec<&str> = notes[0].flds.split('\x1f').collect();
    assert_eq!(flds[..3], ["hello", "hola", "¡Hola, amigo!"]);
    assert_eq!(read_card_ords(&out), [0, 1], "forward + reverse cards");

    // The id line is pinned, and a rebuild leaves the source byte-identical.
    let text = std::fs::read_to_string(&note_file).unwrap();
    assert!(text.starts_with("// @id ") && text.contains(" model=reversed\nq: hello\n"));
    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();
    assert_eq!(std::fs::read_to_string(&note_file).unwrap(), text);
    assert_eq!(read_notes(&out)[0].guid, notes[0].guid);
}

#[test]
fn changing_a_pinned_cards_note_type_is_rejected() {
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    for src in [
        // A pinned card changing to any other note type.
        format!("// @id {id} model=basic-example\nq: Q\nt: A\n"),
        format!("// @id {id} model=type-in\nq: Q\na: A\n"),
        format!("// @id {id} model=cloze\nq: Q\na: A\n"),
        // An existing one-way card (pinned, or with a bare id) gaining `r:`.
        format!("// @id {id} model=basic-example\nq: Q\nr: R\n"),
        format!("// @id {id}\nq: Q\nr: R\n"),
        // An existing two-way card losing `r:`.
        format!("// @id {id} model=reversed\nq: Q\na: A\n"),
        format!("// @id {id} model=reversed\nq: Q\nt: A\n"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let cards = tmp.path().join("cards");
        std::fs::create_dir_all(&cards).unwrap();
        let note_file = cards.join("a.cards");
        std::fs::write(&note_file, &src).unwrap();
        let out = tmp.path().join("deck.apkg");

        let res = build(std::slice::from_ref(&cards), &build_opts(&out));
        assert!(res.is_err(), "{src:?} must not silently orphan the note");
        assert_eq!(std::fs::read_to_string(&note_file).unwrap(), src);
    }
}

#[test]
fn every_card_is_pinned_and_legacy_ids_gain_the_pin_once() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    let note_file = cards.join("a.cards");
    // Bare ids from an earlier ankigen, plus one brand-new card.
    std::fs::write(
        &note_file,
        "// @id 01ARZ3NDEKTSV4RRFFQ69G5FAA\nq: Q1\na: A1\ne: E1\n\n\
         // @id 01ARZ3NDEKTSV4RRFFQ69G5FAB\nq: Q2\nt: 200\n\n\
         // @id 01ARZ3NDEKTSV4RRFFQ69G5FAC\nq: A ==cloze==.\n\n\
         q: Q4\nt: new\n",
    )
    .unwrap();
    let out = tmp.path().join("deck.apkg");

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();
    let guids: Vec<String> = read_notes(&out).into_iter().map(|n| n.guid).collect();
    for guid in [
        "ankigen::basic-example::01ARZ3NDEKTSV4RRFFQ69G5FAA",
        "ankigen::type-in::01ARZ3NDEKTSV4RRFFQ69G5FAB",
        "ankigen::cloze::01ARZ3NDEKTSV4RRFFQ69G5FAC",
    ] {
        assert!(guids.iter().any(|g| g == guid), "{guid} keeps its GUID");
    }

    let text = std::fs::read_to_string(&note_file).unwrap();
    assert!(text.starts_with(
        "// @id 01ARZ3NDEKTSV4RRFFQ69G5FAA model=basic-example\nq: Q1\na: A1\ne: E1\n\n\
         // @id 01ARZ3NDEKTSV4RRFFQ69G5FAB model=type-in\nq: Q2\nt: 200\n\n\
         // @id 01ARZ3NDEKTSV4RRFFQ69G5FAC model=cloze\nq: A ==cloze==.\n\n\
         // @id "
    ));
    assert!(text.ends_with(" model=type-in\nq: Q4\nt: new\n"));

    // Dropping the example from the pinned card keeps its note (and GUID).
    let edited = text.replace("e: E1\n", "");
    std::fs::write(&note_file, &edited).unwrap();
    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();
    assert_eq!(std::fs::read_to_string(&note_file).unwrap(), edited);
    let mut guids2: Vec<String> = read_notes(&out).into_iter().map(|n| n.guid).collect();
    let mut guids = guids;
    guids.sort();
    guids2.sort();
    assert_eq!(guids2, guids);
}
