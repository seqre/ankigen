---
paths:
  - "src/lsp/**"
  - "tests/lsp.rs"
---

# Language server

- `lsp/analysis.rs` (diagnostics, hover, completion) and `lsp/tokens.rs`
  (semantic tokens) are protocol-free: byte ranges in, byte ranges out.
  `lsp/text.rs` converts byte offsets to and from LSP positions.
- Call the library's parser and grammar; never re-lex. The editor has to agree
  with `build`.
- The server is read-only: no write-back.
- `TokenKind::ALL` is the semantic-token legend, and its order is the wire
  encoding.
- `lsp/mod.rs` owns the protocol loop and cross-file duplicate-id detection.
- Block-level parsing details are in `.claude/rules/parsing.md` (loads with
  `src/source/**` and `src/render/**`).
