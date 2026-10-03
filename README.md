# ankigen

Compile simple text flashcards into an Anki `.apkg` — and keep a **stable
identity** for every card, so when you fix a typo and rebuild, Anki *updates*
the existing card instead of duplicating it or wiping its review history.

```
ankigen build cards/ -o deck.apkg
```

## Why

Editing decks by hand in Anki is tedious, and re-importing hand-built `.apkg`s
usually duplicates cards. `ankigen` writes a small `// @id …` marker next
to each card the first time it builds; the Anki note GUID is derived from that
id (not from the content), so edits are matched to the existing note and your
scheduling is preserved.

## Install

```sh
cargo install --path .      # or: cargo build --release  → target/release/ankigen
```

## Authoring format

Cards are **blocks separated by a blank line**. Markers start at column 0; a
field continues onto following lines until the next marker or a blank line.
**Every card uses `q:`**, and the type is inferred:

| You write… | Card type |
|---|---|
| `q:` + `a:` | Basic + Example (empty example) |
| `q:` + `a:` + `e:` | Basic + Example |
| `q:` + `t:` | Type-in (`{{type:Answer}}`) |
| `q:` with `==…==` or `{{c1::…}}` | Cloze (optional `e:` → *Back Extra*) |

`q:`+`a:` and `q:`+`a:`+`e:` share one note type, so adding or removing an
example never duplicates the card. (Cards built by older ankigen versions stay
on the legacy *Basic* note type — see below.)

```
topic: Networking
tags: networking exam

q: What does **HTTP** stand for?
a: HyperText Transfer Protocol — a *stateless* protocol.

q: OSI layers, top to bottom
a:
1. Application
2. **Transport** (TCP/UDP)
3. Network

q: The TCP handshake is ==SYN== then ==SYN-ACK== then ==ACK==.
e: Establishes a connection before data flows.

q: Type the HTTP "OK" status code
t: 200

q: Star topology
a: [image:A star network:img/star.png]
```

### Comments

A line starting with `//` (at column 0) is a comment and is ignored anywhere —
before a card, between its fields, or in a block of its own. Inside a fenced
```` ``` ```` code block, `//` lines are kept as code. The `// @id …` line is a
comment too, and may sit among other comments as long as it precedes `q:`.

```
// TODO: reword
// @id 01KXER6WW489V21WS294KYJTY1 model=basic-example
q: How are spacecraft anomalies handled?
a: Flight controllers act as first responders.
```

### Directives

- `topic: Name` — a **file-level** key (declared once). It adds one subdeck
  segment under the file's directory (see *Decks*).
- `tags: a b c` — space-separated. A standalone block sets file-level tags; a
  `tags:` line inside a card adds card-level tags. `::` makes hierarchical tags.

### Field formatting

A small, predictable subset (not full Markdown): `**bold**`, `*italic*`,
`` `code` ``, `$…$`/`$$…$$` math, `[text](url)` links, fenced ```` ``` ````
code blocks, `- `/`* ` and `1.` lists, a single newline → line break, and `\`
to escape a special character. Anything else can be written as raw HTML and is
passed through. *(Note: `*italic*` cannot itself contain `**bold**`.)*

### Cloze

`==word==` becomes `{{c1::word}}`, auto-numbered left to right. `==word::hint==`
adds a hint. You can also write native `{{c1::word}}` directly — but don't mix
the two styles in one card. `==` inside `` `code` `` is left literal.

### Math

Write math with `$…$` (inline) or `$$…$$` (display). The body is
[LaTeX](https://en.wikipedia.org/wiki/LaTeX) — the input language Anki's
built-in MathJax understands — and renders on Anki Desktop, AnkiDroid, and
AnkiMobile with no extra setup:

- `Energy is $E = mc^2$.` — inline
- `$$\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$$` — a centered display equation

`\$` is a literal dollar sign. The body passes through untouched (so `<`, `>`,
`&` work inside it); a display equation must not contain a blank line.

### Media

Relative to the source file:

- `[image:dog.png]` — an image
- `[image:A caption:dog.png]` — image with a visible caption
- `[sound:clip.mp3]` — audio

## Decks

The deck path comes **only from directory structure** — file names are ignored,
so you can split a topic across many files and they merge:

```
cards/CS/lesson1.md   (no topic)            → deck  CS
cards/CS/lesson2.md   (no topic)            → deck  CS   (merged)
cards/CS/networking.md  (topic: Networking) → deck  CS::Networking
```

`--deck PREFIX` prepends a top segment above the whole tree.

## Identity — commit these files

`ankigen` writes two kinds of id back into your source tree. **Commit them to
version control** so your decks stay stable across machines and rebuilds:

- `// @id ULID` — one per card, at the top of its block. A `q:`/`a:`
  card also carries a `model=<key>` suffix (`// @id ULID model=basic-example`)
  pinning its note type so it can never silently change. The older
  `<!-- @id … -->` spelling is still read; `--upgrade-source` rewrites it to
  `// @id …` (same id, so the GUID and review history are unchanged).
- `deck-id.txt` — one per directory, mapping deck path → a stable deck id.

Deleting an id starts that card/deck fresh (the old one is orphaned in Anki).
The note GUID is simply `ankigen::<model>::<ulid>`.

> **Note types are frozen.** The four models (Basic, Basic+Example, Type-in,
> Cloze) have fixed ids and field lists. Anki refuses to update notes whose note
> type changed, so these never change. New `q:`/`a:` cards use *Basic+Example*;
> the *Basic* model is retained only so decks built by earlier versions keep
> updating in place. On the first rebuild after upgrading, each legacy `q:`/`a:`
> card's `@id` line is upgraded once to `model=basic` to pin it.

## CLI

```
ankigen build <paths...> [options]

  -o, --output <FILE>   output .apkg (default: deck.apkg)
      --deck <PREFIX>   deck prefix prepended above the tree
      --check           parse + validate only; write nothing
      --no-write-back   build + emit, but don't persist ids into sources
      --upgrade-source  also rewrite legacy syntax to its canonical form
                        (`<!-- @id … -->` → `// @id …`)
  -v, --verbose         report to stderr: -v a summary (decks, cards, new
                        cards, media, orphaned media), -vv per-item lines
```

A `-v` summary and `-vv` per-item lines go to **stderr**, so stdout stays
clean. Missing media (a card references a file that isn't there) is collected
across the whole build and reported all at once before failing; orphaned media
(files on disk no card references) is reported under `-v`/`-vv`.

Exit codes: `0` ok, non-zero on a parse/validation/IO error (with a
line-pointed diagnostic).

## Status / limitations

v1 supports four card types; multiple-choice and LaTeX are out of scope (Anki's
native `\(…\)` MathJax passes through if hand-written). Built on the
[`genanki-rs`](https://github.com/bwkimmel/genanki-rs) fork.
