//! End-to-end: build an `.apkg`, unzip it, open the sqlite collection, and
//! assert the notes — including the headline guarantee that editing a card's
//! content (keeping its `@id`) keeps the same GUID, so Anki updates in place.

use std::io::{Read, Write};
use std::path::Path;

use ankigen::anki::models::{MODEL_BASIC, MODEL_CLOZE};
use ankigen::pipeline::{BuildOptions, build};

struct NoteRow {
    guid: String,
    mid: i64,
    tags: String,
    flds: String,
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

    let basic = notes
        .iter()
        .find(|n| n.mid == MODEL_BASIC)
        .expect("basic note");
    let cloze = notes
        .iter()
        .find(|n| n.mid == MODEL_CLOZE)
        .expect("cloze note");

    assert!(basic.guid.starts_with("ankigen::basic::"));
    assert_eq!(basic.tags.trim(), "net");
    assert!(basic.flds.contains("HyperText Transfer Protocol"));
    assert!(cloze.flds.contains("{{c1::SYN}}"), "cloze auto-numbered");

    let basic_guid_before = basic.guid.clone();

    // Edit the card's *content* but keep its `@id` (written back on build 1).
    let text = std::fs::read_to_string(&note_file).unwrap();
    assert!(text.contains("<!-- @id"), "ids were written back");
    let edited = text.replace(
        "HyperText Transfer Protocol",
        "HyperText Transfer Protocol (a stateless protocol)",
    );
    std::fs::write(&note_file, edited).unwrap();

    build(std::slice::from_ref(&cards), &build_opts(&out)).unwrap();

    let notes2 = read_notes(&out);
    let basic2 = notes2.iter().find(|n| n.mid == MODEL_BASIC).unwrap();
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
fn missing_media_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cards = tmp.path().join("cards");
    std::fs::create_dir_all(&cards).unwrap();
    std::fs::write(cards.join("a.cards"), "q: Pic\na: [image:nope.png]\n").unwrap();
    let out = tmp.path().join("deck.apkg");
    let err = build(std::slice::from_ref(&cards), &build_opts(&out));
    assert!(err.is_err(), "missing media should fail the build");
}
