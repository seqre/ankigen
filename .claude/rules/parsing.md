---
paths:
  - "src/source/**"
  - "src/render/**"
---

# Parsing

Two layers:

- `source/cards.rs` hand-scans block structure: markers, directives,
  continuation, the `@id` line, and `//` comments (skipped except inside a
  fenced code block). It tracks per-line byte offsets for diagnostics and
  write-back.
- `render/inline.rs` is the winnow grammar for inline syntax.

Rules:

- Cloze `==…==` is expanded *before* the inline parse.
- The block parser recovers per block. `parse_lenient` returns the cards that
  parsed, a `Diag` per problem (each spanning the offending line), and a
  `BlockSyntax` for every block: its comment/`@id` lines and each field's
  source ranges (`FieldSrc::pieces`).
- `parse` is the strict wrapper that `build` uses.
- The inline grammar (`parse_inlines_spanned`), block `layout` and
  `cloze::shorthand_spans` all report byte ranges into the *field*. Map them
  back to the file with `FieldSrc::source_spans`.
