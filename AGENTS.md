# AGENTS.md

Orientation for working in this repo.

## What this is

`ankigen` is a Rust CLI + library that compiles plain-text flashcards into an
Anki `.apkg`. Its defining feature is **stable identity**: each card gets a
persisted `// @id ULID` comment, and the Anki note GUID is derived from that id
(not the content), so edits update existing cards instead of duplicating them.

There is intentionally **no Markdown engine**: the field syntax is a small,
documented subset, and raw HTML passes through.

## Commands

```sh
cargo test
cargo clippy --all-targets
cargo fmt
cargo run -- build examples/cards -o /tmp/deck.apkg
```

- If `cargo` isn't on PATH (NixOS), prefix with the repo flake's dev shell:
  `nix develop -c cargo test`.
- `examples/cards/` is a ready-to-build sample. Building it writes `@id` lines
  and `deck-id.txt` into the tree; don't leave those in a commit by accident.

## Repository etiquette

- Commit directly to `main`; no feature branches.
- Conventional commits with a scope: `feat(deck): …`, `fix(lsp): …`, `docs: …`;
  `!` marks a breaking change.

## Layout

Where the non-obvious things live (the rest is findable from `src/lib.rs`):

- `src/pipeline.rs` — build orchestration.
- `src/check.rs` — validation and model resolution, **shared by `build` and the
  language server**.
- `src/source/` — the block parser (`cards.rs`) and `@id` write-back
  (`writeback.rs`).
- `src/render/` — inline grammar, cloze expansion, block grouping + HTML.
- `src/id/` — ULID card ids, the note GUID, deck-id minting.
- `src/anki/models.rs` — the 5 frozen note models.
- `src/deck.rs` — directory-derived deck paths and `deck-id.txt`.
- `src/lsp/` — `ankigen lsp`.
- `tests/integration_build.rs` builds an apkg and asserts on its sqlite;
  `tests/lsp.rs` drives the server over an in-memory connection.

Parser and language-server details load from `.claude/rules/` when you work in
those directories.

## Invariants — do not break

### GUIDs

- Always pass an explicit GUID to genanki
  (`Note::new_with_options(.., Some(guid))`). Its default GUID hashes the
  fields, so it changes with content and would duplicate cards.
- GUID format is `ankigen::<model>::<ulid>`.

### Frozen models

- Never change a model id, a `ModelKey::as_str()` value, or a model's field
  list (`anki/models.rs`). Any of these orphans every existing note.
- The `frozen` tests in `anki/models.rs` pin these values. If one fails, revert
  the change rather than updating the expected value.

### Write-back

- Write-back is atomic and idempotent: once every card has its id (and `model=`
  where due), a rebuild must leave files byte-identical.
- `// @id` goes at the *start* of a block.
- Every card's id line carries a `model=<key>` suffix. Legacy bare-`@id` cards
  get it added by a single in-place line rewrite on first rebuild.
- Legacy `<!-- @id … -->` lines are still parsed. Convert them to `// @id` only
  under `--upgrade-source`, or when a `model=` pin rewrites the line anyway.

### Note types

- The `model=` marker is the durable source of truth for a card's note type.
  Never re-infer it from `id_present`: that flag flips to true after the first
  build and would orphan cards.
- `q:`/`a:` and `q:`/`a:`/`e:` both use the Basic+Example model for new cards.
- The legacy `Basic` model exists only for cards minted before that unification
  (`model=basic`).
- `q:`/`r:` is a two-way card on the Basic+Reversed model (one note, two
  templates).
- A pinned card can't change note type. `check::resolve_model` rejects a card
  whose markers no longer match its `model=` pin instead of orphaning it. The
  one exception: `q:`/`a:` is valid under both `basic` and `basic-example`.

### Deck ids

- Deck ids live in a per-directory `deck-id.txt`, card ids in `@id`. Users
  commit both; treat them as the source of truth.
- `deck-id.txt` is keyed relative to its directory: `<id>` is the directory's
  deck, `<id>::Topic` a `topic:` subdeck.
- Never key by the full deck name. It depends on the build root, whose
  directory name is the top deck segment unless `--deck` renames it.
- Legacy name-keyed lines are adopted on first use.
- Conflicting ids for one deck are an error, not a guess.

## Dependencies

- `genanki-rs` is pinned to the **bwkimmel fork** (git rev) for updated deps.
- The fork requires `zip ^7.4.0`, but 7.4.0 was yanked from crates.io, so
  `Cargo.toml` has a `[patch.crates-io]` pointing `zip` at its upstream git tag.
- Dev-dep `rusqlite` must match the fork's version (0.38) so `libsqlite3-sys`
  stays unified.
