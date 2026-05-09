# 00 — Architecture & Security Model

## What we're building

A macOS desktop app (Linux/Windows possible later) that:

1. **Imports Japanese text** from Aozora Bunko, web pages, plain text, and
   eventually OCR'd images
2. **Renders text** in a reader with vertical writing, ruby, and word-level
   interactivity
3. **Looks up words** via Yomitan-format dictionaries (the entire
   community-maintained ecosystem)
4. **Tracks vocabulary** in a local SQLite database — every word you've
   seen, where you saw it, what state you've marked it in
5. **Mines to Anki** via AnkiConnect, with handlebars-based card templates
6. **Surfaces what you've learned** through a menu bar app, daily
   notifications, and a word-of-day window
7. **Optionally augments** with local LLMs for grammar breakdowns,
   example sentences, and definition rewriting

## Multi-crate workspace

```
aozora-tool/
├─ Cargo.toml                    # workspace root
├─ crates/
│  ├─ jp-core/                   # shared types, errors, settings
│  ├─ jp-tokenizer/              # vibrato or longest-match wrapper
│  ├─ jp-dict/                   # Yomitan format, deinflection, lookup
│  ├─ jp-vocab/                  # vocab DB, encounter logging, retention
│  ├─ jp-anki/                   # AnkiConnect client, handlebars templates
│  ├─ jp-importer/               # pluggable: aozora/html/text/ocr
│  ├─ jp-reader/                 # EPUB rendering helpers
│  ├─ jp-llm/                    # Phase 10: local LLM providers
│  └─ jp-ocr/                    # Phase 9: screen capture + OCR
├─ src-tauri/                    # the desktop shell, wires everything
└─ src/                          # Leptos frontend
```

Each crate has clear inputs/outputs and minimal cross-dependencies. The
Tauri shell is intentionally thin — most logic lives in the crates so it
can be tested without spinning up a window.

Dependency graph (intentionally one-way; no cycles):

```
jp-core   (no deps on other workspace crates)
   ↑
jp-tokenizer ← jp-dict ← jp-vocab
                   ↑          ↑
              jp-importer  jp-anki
                   ↑          ↑
              jp-reader   jp-llm   jp-ocr
                          ↑     ↑
                       src-tauri (depends on all)
```

`jp-core` defines the types every other crate uses: `Word`, `Sentence`,
`Source`, `Provider`, `Status`. No business logic lives there.

## Stack

```
┌─────────────────────────────────────────────┐
│ Frontend: Leptos (Rust → Wasm)              │
│   runs inside system WebKit (macOS)         │
│   sandboxed by OS                           │
└──────────────────┬──────────────────────────┘
                   │ Tauri IPC (allowlisted)
┌──────────────────┴──────────────────────────┐
│ Backend: Tauri 2 + Rust crates              │
│   ├─ Sources: Aozora repo, web fetch, OCR   │
│   ├─ Importers: chūki, HTML clean, OCR text │
│   ├─ Dictionary: Yomitan-format engine      │
│   ├─ Vocab DB: SQLite (vocab + encounters)  │
│   ├─ AnkiConnect: HTTP to localhost:8765    │
│   └─ LLM: HTTP to localhost:11434 (default) │
└─────────────────────────────────────────────┘
```

External runtime dependencies (all optional):

- **Java** (Phase 2 Aozora importer fallback, optional once Phase 7 lands)
- **Anki** (only if user wants Anki mining)
- **Ollama / LM Studio / similar** (only if user wants LLM features)

## Why Tauri + Leptos (recap)

Decided in v1; revisited and confirmed for v2. Pure-cargo dependency tree.
Real webview means we can render rich text with vertical writing + ruby
without reinventing those rendering primitives. Leptos's fine-grained
reactivity scales well to the UI complexity we're heading for (multiple
windows, real-time encounter tracking, live updating stats).

## Security model

Three layers, mostly unchanged from v1:

### 1. Build-time

- All deps via `cargo` and `crates.io`. No `npm`, no `pip` (except as
  build-time helpers for the optional ttu integration).
- `cargo deny check` in CI.
- Pin versions explicitly; review `Cargo.lock` diffs.
- Workspace `[workspace.dependencies]` for shared deps so versions stay
  aligned across crates.

### 2. Runtime — frontend ↔ backend bridge

Tauri 2 capability/permission system is the security boundary. Each window
gets its own capability scope:

- **Main window** (library + settings) — broadest scope. Can refresh
  index, download works, modify settings, query vocab, send to Anki.
- **Reader windows** (one per opened EPUB) — restricted. Can read EPUBs
  from app data dir; can call lookup; can record encounters; cannot modify
  library or settings.
- **OCR overlay window** (Phase 9) — narrow. Screen capture + lookup
  only.
- **Word-of-day window** (Phase 6) — narrow. Read vocab DB + dismiss
  word + send to Anki only.

Per-window capabilities mean a compromised reader can't reach into the
settings or trigger downloads.

### 3. Runtime — untrusted content

- EPUBs and OCR'd text are *content*, not code. Rendered in WebKit
  (OS-sandboxed). JS disabled in reader windows.
- Dictionary `.zip` files are parsed into SQLite, never executed. Users
  can drop in any Yomitan-format dictionary; we treat them as untrusted
  input and validate aggressively.
- LLM responses are treated as untrusted input — we never `eval`, never
  blindly execute, never put unescaped LLM output into card templates.

### 4. Network — explicit allowlist per capability

Capabilities declare allowed network destinations:

- Main window: `https://www.aozora.gr.jp/*`,
  `https://aozorahack.org/*`,
  `http://127.0.0.1:8765/*` (AnkiConnect)
- Reader/OCR/word-of-day windows: `http://127.0.0.1:8765/*` only
- LLM features (any window): `http://localhost:11434/*` and
  `http://127.0.0.1:11434/*`, plus any user-configured local ports
  (validated to be localhost)

No cloud LLM, no web extension, no telemetry. Network surface is tightly
bounded.

## Threat model — explicit scope

We **do** defend against:

- Supply-chain attacks via npm — mitigated by not using npm
- Supply-chain attacks via crates.io — `cargo deny`, lockfile, review
- Malicious EPUBs / dictionaries trying to escape the renderer or import
  process — sandbox + validation
- Malicious LLM output trying to inject into card templates or vocab DB
  via crafted prompts — output sanitization, content-type-aware
  rendering
- Path-traversal via crafted file paths in IPC — validation in commands
- Network MITM on Aozora downloads — HTTPS

We **do not** defend against:

- Compromised Aozora Bunko or aozorabunko_text repo serving malicious
  content
- Attackers with code execution as the user
- Side-channel attacks on the OS sandbox
- Determined adversaries with physical access
- Malicious local LLM models — if a user runs an attacker-supplied
  model, they own the LLM output. We sanitize output before storing,
  but we don't claim to defend against backdoored models.

## Validation strategy

Each phase has acceptance criteria. Across phases, three works are gold
standard (Kokoro / Rashomon / Ningen Shikkaku).

After Phase 5, add a vocabulary regression suite: lookup-and-mine ~50
known words, snapshot the vocab DB state, assert no regressions on
subsequent runs.

After Phase 6, snapshot the daily review HTML output for a frozen vocab
DB so layout regressions are caught.

## Development principles

- **Phase boundaries are firm.** Don't start Phase 5 until Phase 4 is
  solid.
- **Crate boundaries are firmer.** No `pub use` across crate boundaries
  for internal helpers; export only the public API.
- **One binary, one config.** No prod/dev feature flags multiplying
  complexity.
- **Fail loudly.** Aozora's CSV schema has changed before; assert
  layouts. Yomitan dictionary versions change; validate on import.
- **Tests on the boundary.** Unit-test the dictionary engine, retention
  algorithm, and EPUB renderer exhaustively. UI layer is allowed lighter
  testing.
- **Journal as you go.** `JOURNAL.md` in repo root: dated entries on
  what you tried, what worked, what broke.
