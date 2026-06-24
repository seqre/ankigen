//! The build pipeline: discover → parse → mint ids → write back → render →
//! emit `.apkg`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use genanki_rs::{Deck, Note, Package};
use walkdir::WalkDir;

use crate::anki::build_note;
use crate::deck::{self, DeckRegistry};
use crate::error::{AnkigenError, Result};
use crate::id::{guid_for_str, mint_card_id};
use crate::media::MediaResolver;
use crate::model::{CardKindSpec, ModelKey, ParsedCard, ParsedFile};
use crate::render::{RenderCtx, render_cloze, render_rich};
use crate::report::Reporter;
use crate::source::{CardSource, CardsSource};
use crate::util::atomic_write;

/// File extensions treated as media when scanning for orphaned files.
const MEDIA_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "tiff", "tif", "mp3", "ogg", "wav", "m4a",
    "flac", "opus", "mp4", "webm", "mov",
];

pub struct BuildOptions {
    pub output: PathBuf,
    pub deck_prefix: Option<String>,
    /// Parse + validate only: no write-back, no `.apkg`.
    pub check: bool,
    /// Persist minted card ids and `deck-id.txt`.
    pub write_back: bool,
    /// Output verbosity: 0 silent, 1 summary, 2+ per-item (to stderr).
    pub verbose: u8,
}

struct Loaded {
    parsed: ParsedFile,
    root: PathBuf,
    input: String,
}

pub fn build(paths: &[PathBuf], opts: &BuildOptions) -> Result<()> {
    let mut report = Reporter::new(opts.verbose);
    let source = CardsSource;
    let mut loaded = load_all(&source, paths, &mut report)?;
    mint_and_check_ids(&mut loaded, &mut report)?;
    resolve_models(&mut loaded)?;

    let persist = opts.write_back && !opts.check;
    if persist {
        write_back_ids(&source, &loaded, &mut report)?;
    }

    // Render every card (validates media + cloze) and group notes by deck.
    let mut resolver = MediaResolver::new();
    let mut registry = DeckRegistry::new(persist);
    let mut decks: BTreeMap<String, (i64, Vec<Note>)> = BTreeMap::new();
    let mut missing: Vec<AnkigenError> = Vec::new();

    for l in &loaded {
        let file = &l.parsed.path;
        let dir = file.parent().unwrap_or_else(|| Path::new("."));
        let comps = deck::deck_components(
            file,
            &l.root,
            l.parsed.topic.as_deref(),
            opts.deck_prefix.as_deref(),
        );
        let name = deck::deck_name(&comps);
        let first_seen = !decks.contains_key(&name);
        let (deck_id, minted) = registry.id_for(dir, &name);
        if first_seen {
            report.deck(&name, deck_id, minted);
        }

        for card in &l.parsed.cards {
            let (key, mut fields) =
                render_card(card, file, &l.parsed.text, &mut resolver, &mut missing)?;
            let mut tags = l.parsed.file_tags.clone();
            tags.extend(card.spec.tags.iter().cloned());
            let id = card.spec.id.as_deref().expect("id minted above");
            let guid = guid_for_str(id, key);
            fields.push(id.to_string());
            let note = build_note(key, &fields, &guid, &tags)?;
            decks
                .entry(name.clone())
                .or_insert_with(|| (deck_id, Vec::new()))
                .1
                .push(note);
        }
    }

    // Surface every missing-media reference at once rather than the first.
    if !missing.is_empty() {
        return Err(AnkigenError::MissingMediaBatch {
            count: missing.len(),
            errors: missing,
        });
    }

    if report.enabled() {
        for (src, basename) in resolver.media_entries() {
            report.media(basename, src);
        }
        scan_orphans(&loaded, &resolver, &mut report);
    }

    if !opts.check {
        emit_package(decks, &resolver, &opts.output)?;
    }
    registry.flush()?;
    report.finish(opts.check);
    Ok(())
}

fn load_all(
    source: &CardsSource,
    paths: &[PathBuf],
    report: &mut Reporter,
) -> Result<Vec<Loaded>> {
    let mut loaded = Vec::new();
    for (file, root) in discover(paths)? {
        let input = std::fs::read_to_string(&file)
            .map_err(|e| AnkigenError::Io(format!("{}: {e}", file.display())))?;
        report.source_file(&file);
        let parsed = source.parse(&input, &file)?;
        loaded.push(Loaded {
            parsed,
            root,
            input,
        });
    }
    Ok(loaded)
}

fn mint_and_check_ids(loaded: &mut [Loaded], report: &mut Reporter) -> Result<()> {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for l in loaded.iter_mut() {
        let path = l.parsed.path.clone();
        for card in &mut l.parsed.cards {
            // `id_present` is the durable "new" marker: it stays false for cards
            // we mint here (it reflects the *source*, not the in-memory spec).
            let is_new = !card.id_present;
            if card.spec.id.is_none() {
                card.spec.id = Some(mint_card_id().to_string());
            }
            let id = card.spec.id.clone().unwrap();
            report.card(&id, is_new, &path);
            if seen.insert(id.clone(), path.clone()).is_some() {
                return Err(AnkigenError::DuplicateId { id });
            }
        }
    }
    Ok(())
}

/// Resolve each card's note type, honoring the durable `model=` marker and
/// migrating legacy `q:/a:` cards. Only `CardKindSpec::Basic` is ambiguous:
/// with no marker, a card that already had an `@id` predates the Basic/
/// Basic+Example unification (⇒ `Basic`), while a brand-new card unifies on
/// `BasicExample` (rendered with an empty example). The resolved key is the
/// single source of truth shared by write-back and rendering.
fn resolve_models(loaded: &mut [Loaded]) -> Result<()> {
    for l in loaded.iter_mut() {
        for card in &mut l.parsed.cards {
            let model = match &card.spec.kind {
                CardKindSpec::Basic { .. } => match card.model_marker {
                    Some(m) => m,
                    None if card.id_present => ModelKey::Basic,
                    None => ModelKey::BasicExample,
                },
                CardKindSpec::BasicExample { .. }
                    if card.model_marker == Some(ModelKey::Basic) =>
                {
                    return Err(AnkigenError::parse(
                        &l.parsed.path,
                        &l.parsed.text,
                        card.block_span,
                        "card has an example but is pinned to the legacy Basic note type",
                        Some("remove the `e:` example, or drop the `model=basic` marker to migrate it"),
                    ));
                }
                other => other.model_key(),
            };
            card.resolved_model = Some(model);
        }
    }
    Ok(())
}

fn write_back_ids(source: &CardsSource, loaded: &[Loaded], report: &mut Reporter) -> Result<()> {
    for l in loaded {
        if let Some(new_text) = source.persist_ids(&l.input, &l.parsed) {
            atomic_write(&l.parsed.path, &new_text)
                .map_err(|e| AnkigenError::Io(format!("{}: {e}", l.parsed.path.display())))?;
            report.write_back(&l.parsed.path);
        }
    }
    Ok(())
}

fn emit_package(
    decks: BTreeMap<String, (i64, Vec<Note>)>,
    resolver: &MediaResolver,
    output: &Path,
) -> Result<()> {
    let genanki_decks: Vec<Deck> = decks
        .into_iter()
        .map(|(name, (id, notes))| {
            let mut d = Deck::new(id, &name, "");
            for n in notes {
                d.add_note(n);
            }
            d
        })
        .collect();

    // genanki uses `path.file_name()` as the media filename inside the apkg,
    // so we copy each file into a tempdir under its hashed basename so that
    // what genanki records matches the `<img src="…">` we emitted.
    let tmp = tempfile::tempdir()
        .map_err(|e| AnkigenError::Io(format!("tempdir: {e}")))?;
    let mut staged: Vec<PathBuf> = Vec::new();
    for (src, basename) in resolver.media_entries() {
        let dst = tmp.path().join(basename);
        std::fs::copy(src, &dst)
            .map_err(|e| AnkigenError::Io(format!("copying media {}: {e}", src.display())))?;
        staged.push(dst);
    }

    let media: Vec<&str> = staged.iter().filter_map(|p| p.to_str()).collect();
    let mut package = Package::new(genanki_decks, media)?;
    let out = output
        .to_str()
        .ok_or_else(|| AnkigenError::Io("output path is not valid UTF-8".into()))?;
    package.write_to_file(out)?;
    Ok(())
}

fn render_card(
    card: &ParsedCard,
    file: &Path,
    text: &str,
    resolver: &mut MediaResolver,
    missing: &mut Vec<AnkigenError>,
) -> Result<(ModelKey, Vec<String>)> {
    let key = card.resolved_model.expect("model resolved above");
    let src_dir = file.parent().unwrap_or_else(|| Path::new("."));
    let mut ctx = RenderCtx {
        file,
        text,
        src_dir,
        span: card.block_span,
        resolver,
        missing,
    };
    let fields = match &card.spec.kind {
        CardKindSpec::Basic { question, answer } => {
            let mut fields = vec![
                render_rich(question, &mut ctx)?,
                render_rich(answer, &mut ctx)?,
            ];
            // New `q:/a:` cards unify on Basic+Example with an empty example;
            // legacy cards stay on the 2-field Basic note type.
            if key == ModelKey::BasicExample {
                fields.push(String::new());
            }
            fields
        }
        CardKindSpec::BasicExample {
            question,
            answer,
            example,
        } => vec![
            render_rich(question, &mut ctx)?,
            render_rich(answer, &mut ctx)?,
            render_rich(example, &mut ctx)?,
        ],
        CardKindSpec::TypeIn { question, answer } => {
            vec![render_rich(question, &mut ctx)?, answer.clone()]
        }
        CardKindSpec::Cloze { text, back_extra } => {
            let text_html = render_cloze(text, &mut ctx)?;
            let extra = match back_extra {
                Some(b) => render_rich(b, &mut ctx)?,
                None => String::new(),
            };
            vec![text_html, extra]
        }
    };
    Ok((key, fields))
}

/// Expand the CLI paths into `(file, root)` pairs. A directory is its files'
/// root; a file passed directly uses its own parent as root.
fn discover(paths: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            for entry in WalkDir::new(p).sort_by_file_name() {
                let entry =
                    entry.map_err(|e| AnkigenError::Io(format!("walking {}: {e}", p.display())))?;
                let path = entry.path();
                if entry.file_type().is_file() && has_card_ext(path) {
                    out.push((path.to_path_buf(), p.clone()));
                }
            }
        } else if p.is_file() {
            let root = p.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            out.push((p.clone(), root));
        } else {
            return Err(AnkigenError::Io(format!("path not found: {}", p.display())));
        }
    }
    Ok(out)
}

fn has_card_ext(f: &Path) -> bool {
    matches!(
        f.extension().and_then(|e| e.to_str()),
        Some("md") | Some("cards")
    )
}

fn is_media_ext(f: &Path) -> bool {
    f.extension()
        .and_then(|e| e.to_str())
        .map(|e| MEDIA_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk the distinct source roots for media-extension files and report any that
/// no card referenced (orphaned). Best-effort: unreadable entries are skipped.
fn scan_orphans(loaded: &[Loaded], resolver: &MediaResolver, report: &mut Reporter) {
    use std::collections::HashSet;

    let referenced: HashSet<&Path> = resolver.referenced().collect();
    let mut roots: Vec<&Path> = loaded.iter().map(|l| l.root.as_path()).collect();
    roots.sort();
    roots.dedup();

    let mut seen: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        for entry in WalkDir::new(root).sort_by_file_name() {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if !entry.file_type().is_file() || !is_media_ext(path) {
                continue;
            }
            let Ok(abs) = std::fs::canonicalize(path) else {
                continue;
            };
            if referenced.contains(abs.as_path()) {
                continue;
            }
            if seen.insert(abs.clone()) {
                report.orphan(&abs);
            }
        }
    }
}
