# Japanese Study Tool — Project Documentation

A Tauri + Leptos desktop app for studying Japanese: read Aozora Bunko + web
content, look up words via Yomitan-format dictionaries, mine to Anki, track
your vocabulary growth, review what you've learned, OCR text from anywhere
on screen, and optionally augment with local LLMs. Pure-Rust deps, multi-crate
workspace, capability-based IPC, local-only by default.

This is v2 of the doc set. The original was Aozora-only; this version
expands the architecture to support the full study workflow.

## Phase index

| Phase | Title | Goal | Effort |
|-------|-------|------|--------|
| 0 | Setup | Toolchain + multi-crate workspace + green window | one evening |
| 1 | Source: Aozora index | Browse 18k+ works, source from aozorabunko_text repo | one weekend |
| 2 | Importer: Aozora | Click → EPUB on disk (jar fallback) | one evening |
| 3 | Library + reader | Browse downloaded works, open in viewer | one weekend |
| 4 | Vocab DB + Anki | Persist every word seen, mine to Anki | one weekend |
| 5 | Dictionary + popup | Yomitan-format engine, hover-popup, 4-button mining | 2-3 weekends |
| 6 | Review surfaces | Menu bar, daily notification, word-of-day, weekly report | one weekend |
| 7 | Native Aozora converter | Replace AozoraEpub3 jar | open-ended |
| 8 | Web importer | Save URL → EPUB | one weekend |
| 9 | OCR overlay | Hotkey → screen capture → lookup anywhere | 2 weekends |
| 10 | Local LLM | Ollama / LM Studio / MLX integration | one weekend |

Phases 0-6 are the core app — usable on its own. Phases 7-10 are extensions.
You can ship after Phase 6 and live with that for months.

## Reference docs

- `00-architecture.md` — system overview, crate boundaries, security model
- `phase-0-setup.md` — toolchain, workspace structure, scaffold
- `phase-1-source.md` — Aozora index CSV + aozorabunko_text repo
- `phase-2-importer-aozora.md` — AozoraEpub3 jar (both versions) subprocess
- `phase-3-library-reader.md` — library view, ttu / external reader handoff
- `phase-4-vocab-anki.md` — vocab DB schema, encounter retention, AnkiConnect, handlebars
- `phase-5-dictionary.md` — Yomitan-format dictionary engine, lookup popup, mining
- `phase-6-review.md` — menu bar, notifications, word-of-day, weekly HTML report
- `phase-7-native-converter.md` — pure-Rust Aozora chūki → EPUB
- `phase-8-web-importer.md` — URL → cleaned HTML → EPUB
- `phase-9-ocr-overlay.md` — Apple Vision OCR + transparent overlay window
- `phase-10-llm.md` — local-only LLM provider integration

## Appendices

- `appendix-a-aozora-format.md` — chūki notation reference (for Phase 7)
- `appendix-b-reader-options.md` — ttu / foliate-js / build your own
- `appendix-c-security-checklist.md` — what to audit, when
- `appendix-d-aozoraepub3-versions.md` — JDK21 fork + original, fallback strategy
- `appendix-e-yomitan-format.md` — dictionary zip format reference
- `appendix-f-vocab-db.md` — schema rationale, retention algorithm details

## Validation corpus

Three works that must keep working across phases:

| Work | Author | ID | Why |
|---|---|---|---|
| こころ | 夏目漱石 | 773 | Classic, lots of ruby, well-formed |
| 羅生門 | 芥川龍之介 | 127 | Short, common test target |
| 人間失格 | 太宰治 | 301 | Modern-ish, varied formatting |

Plus, after Phase 5: a vocabulary regression suite. After mining ~50 words
from Kokoro, the same 50 words should always be findable in the vocab DB
with the correct status, source attribution, and encounter sentences.

## What this app deliberately does NOT do

To keep scope honest:

- **No cloud LLM integration.** Local models only. (Phase 10 docs explain why.)
- **No browser extension.** Yomitan does this better; we link out to it.
- **No mobile.** Desktop-first; mobile would need a separate codebase.
- **No closed-source distribution.** Stack is GPL-3 across the board.
- **No SRS algorithm.** That's Anki's job; we send cards, Anki schedules them.
- **No telemetry.** Ever.
