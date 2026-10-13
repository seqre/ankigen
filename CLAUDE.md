# CLAUDE.md

Orientation for working in this repo.

## What this is

`ankigen` is a Rust CLI + library that compiles plain-text flashcards into an
Anki `.apkg`. Its defining feature is **stable identity**: each card gets a
persisted `// @id ULID` comment, and the Anki note GUID is derived from that id
(not the content), so edits update existing cards instead of duplicating them.

## Layout

```
src/
  model/      serde card model (spec.rs) + parser output types (card.rs)
  source/     CardSource trait + the `cards` parser (cards.rs) + @id write-back
  render/     winnow inline grammar (inline.rs), cloze expansion (cloze.rs),
              block grouping + HTML (mod.rs)
  id/         ULID card ids, the `ankigen::<model>::<ulid>` GUID, deck-id minting
  anki/       the 5 frozen genanki models + note builder
  media/      blake3 content-hash basenames
  deck/ (deck.rs)  directory-derived deck paths + per-directory deck-id.txt
  check.rs    validation shared by build and the language server (model resolution)
  lsp/        `ankigen lsp`: mod.rs (protocol loop, cross-file duplicate ids),
              analysis.rs (diagnostics/hover/completion), tokens.rs (semantic
              tokens), text.rs (byte offset ⇄ LSP position)
  pipeline.rs orchestration; cli.rs (clap); error.rs (miette diagnostics)
tests/integration_build.rs  builds an apkg, opens its sqlite, asserts notes
tests/lsp.rs                drives the server over an in-memory connection
```

## Invariants — do not break

- **Always pass an explicit GUID** to genanki (`Note::new_with_options(.., Some(guid))`).
  Its default GUID hashes the fields (unstable + content-derived) and would
  duplicate cards. GUID format is `ankigen::<model>::<ulid>`.
- **Model ids and field lists are frozen** (`anki/models.rs`). Changing a model
  id, a `ModelKey::as_str()`, or a field list orphans every existing note.
- **Write-back is insert + one-time `model=` pin, atomic, idempotent.** `// @id`
  goes at the *start* of a block. Every card's id line also carries a
  `model=<key>` suffix; legacy bare-`@id` cards get this suffix added by a single
  in-place line rewrite on first rebuild. Legacy `<!-- @id … -->` lines are
  still parsed and are only converted to `// @id` under `--upgrade-source`
  (or when a `model=` pin rewrites the line anyway). Once every card has its id (and
  `model=` where due), a rebuild must leave files byte-identical.
- **`q:`/`a:` (and `q:`/`a:`/`e:`) both use the Basic+Example model for new
  cards.** The `model=` marker is the durable source of truth for every
  card's note type — never re-infer it from `id_present` (that flag flips to
  true after the first build and would orphan cards). The legacy `Basic` model
  is kept only for cards minted before the unification (`model=basic`).
- **`q:`/`r:` is a two-way card on the Basic+Reversed model** (one note, two
  templates).
- **A pinned card can't change note type.** `resolve_models` rejects a card
  whose markers no longer match its `model=` pin (the one exception: `q:`/`a:`
  is valid under both `basic` and `basic-example`), rather than orphaning it.
- **Deck ids live in `deck-id.txt` per directory** and card ids in `@id`. Both
  are committed by users; treat them as source of truth. `deck-id.txt` is keyed
  relative to its directory (`<id>` = the directory's deck, `<id>::Topic` = a `topic:`
  subdeck), never by the full deck name — that depends on the build root, whose
  directory name is the top deck segment unless `--deck` renames it.
  Legacy name-keyed lines are adopted on first use; conflicting ids
  for one deck are an error, not a guess.

## Parsing

Two layers: `source/cards.rs` hand-scans block structure (markers, directives,
continuation, the `@id` line, and `//` comments, which are skipped except inside
a fenced code block), tracking per-line byte offsets for diagnostics and
write-back; `render/inline.rs` is the
winnow grammar for inline syntax. Cloze `==…==` is expanded *before* the inline
parse. The block parser **recovers per block**: `parse_lenient` returns the cards
that parsed plus a `Diag` per problem (each spanning the offending line), and
a `BlockSyntax` for every block — its comment/`@id` lines and each field's
source ranges (`FieldSrc::pieces`). `parse` is the strict wrapper `build` uses.
The inline grammar (`parse_inlines_spanned`), block `layout` and
`cloze::shorthand_spans` all report byte ranges into the field; map those back
to the file with `FieldSrc::source_spans`.

## Language server

`lsp/analysis.rs` and `lsp/tokens.rs` are protocol-free (byte ranges in, byte
ranges out) and must call the library's parser/grammar rather than re-lex:
the editor has to agree with `build`. The server is read-only — no write-back.
`TokenKind::ALL` is the semantic-token legend; its order is the wire encoding.

There is intentionally **no Markdown engine** (the field syntax is a
small, documented subset; raw HTML passes through).

## Dependencies

`genanki-rs` is pinned to the **bwkimmel fork** (git rev) for updated deps. It
requires `zip ^7.4.0`, but 7.4.0 was yanked from crates.io, so `Cargo.toml` has
a `[patch.crates-io]` pointing `zip` at its upstream git tag. Dev-dep `rusqlite`
must match the fork's version (0.38) so libsqlite3-sys stays unified.

## Working here

```sh
cargo test
cargo clippy --all-targets
cargo run -- build examples/cards -o /tmp/deck.apkg
```

The plan that produced this code is in the session; `examples/cards/` is a
ready-to-build sample (it gains `@id`/`deck-id.txt` on first build).
