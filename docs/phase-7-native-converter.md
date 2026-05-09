# Phase 7 — Native Aozora Converter

**Goal:** replace the AozoraEpub3.jar subprocess with a pure-Rust
converter from Aozora chūki notation to EPUB 3.

**Time:** open-ended.

This is v1 phase 6, unchanged. Removing the Java runtime dependency.
See appendix-a-aozora-format.md for the chūki notation reference.

## Approach

Ship behind a flag (`AozoraStrategy::Native` or `NativeAuto`), expand
coverage incrementally, run alongside the jar in dev to diff output.

The pipeline (six stages):

```
SJIS .txt
  ↓ encoding_rs decode
UTF-8 String
  ↓ split_header_footer
Headered { title, author, body }
  ↓ tokenize (chūki state machine)
Vec<Token>
  ↓ build_ast
Document { metadata, blocks: Vec<Block> }
  ↓ render_chapter
Vec<(filename, xhtml)>
  ↓ rbook pack
EPUB
```

## Crate layout

```
crates/jp-importer/src/aozora/native/
├─ mod.rs
├─ tokenize.rs      # state machine over chars
├─ ast.rs           # Document, Block, Inline types
├─ chuki.rs         # parse chuki content strings
├─ gaiji.rs         # gaiji resolver (chuki_utf.txt port)
├─ render.rs        # AST → XHTML
└─ pack.rs          # XHTML → EPUB via rbook
```

## Implementation outline

(Full code skeleton in v1 phase 6 doc.)

The hardest pieces:

**Ruby base detection** — when `《》` appears without explicit `｜`, walk
back over CJK Unified Ideographs to find the base. Watch out for kanji
variation selectors and rare blocks.

**Chuki content parsing** — `［＃...］` content has a sub-grammar:
`大見出し`, `ここから3字下げ`, `「文字」に傍点`, etc. Build a parser for
the finite vocabulary.

**Emphasis backreferences** — `［＃「彼」に傍点］` modifies *previously
emitted* text. Need a backlog buffer in the AST builder, not a streaming
emit.

**Gaiji resolution** — embed `chuki_utf.txt` from AozoraEpub3 via
`include_str!`. Build a HashMap on first call.

**Rendering** — XHTML with `lang="ja"`, `epub:type` attributes,
appropriate CSS for tategaki + ruby. Match AozoraEpub3's output style
where possible.

**EPUB packing** — `rbook` crate. Set `page-progression-direction="rtl"`
for vertical Japanese.

## Validation

Snapshot diff harness:

1. Pick 100 random work IDs from the index
2. Run both jar and native on each
3. Diff output XHTML structurally
4. Track regressions in a JSON file committed to repo
5. CI runs this on the three test works (Kokoro, Rashomon, Ningen
   Shikkaku) on every PR

When the harness passes for 100 random works, flip default to
`NativeAuto`.

## Acceptance criteria

- [ ] Three test works produce valid EPUB 3
- [ ] EPUBs open in Apple Books with correct tategaki + ruby + headings
- [ ] Diff harness passes for 100 random Aozora works
- [ ] Java is no longer required for the default flow
- [ ] User-facing setting allows fallback to jar for problem files

## What's next

Phase 8: web importer.
