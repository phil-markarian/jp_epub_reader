# Journal

## 2026-05-10 — Phase 0 setup

- Multi-crate workspace scaffolded (Path B): root `Cargo.toml` with shared
  `[workspace.dependencies]`, seven Phase 1-6 crate stubs under `crates/`.
- `deny.toml` added with GPL-3 license allowlist.
- `.gitignore` added (Rust + Tauri + macOS).
- Repo created on GitHub (`phil-markarian/jp_epub_reader`, public) and
  wired up as `origin`.

Outstanding before Phase 0 is "done":

- Install missing toolchain: `cargo-tauri`, `trunk`, `cargo-leptos`,
  `cargo-deny`, `wasm32-unknown-unknown` target, Java 21.
- Scaffold `src-tauri/` + Leptos frontend (Path A via `create-tauri-app`
  is easiest once the CLI is installed).
- Verify `cargo check --workspace`, `cargo tauri dev`, `cargo deny check`.

## 2026-05-11 — State catch-up

The repo moved past the initial scaffold and now has a working Phase 1-3
slice. This entry backfills the actual implemented state so the journal matches
the codebase.

Implemented:

- Tauri 2 desktop shell under `src-tauri/` with Leptos CSR frontend under
  `frontend/`.
- Phase 1 source flow: refresh and cache the Aozora master index, parse the
  extended CSV, search works by title / yomi / author / romaji, and resolve
  source text either from a local `aozorabunko_text` checkout or remote cache.
- Phase 2 importer flow: detect Java, bundle AozoraEpub3 resources, convert
  resolved Aozora text into EPUB, and emit a UTF-8 sidecar for later vocab
  work.
- Phase 3 library flow: persist imported works in SQLite, backfill older
  on-disk imports into the DB, list/delete/open library entries, and open a
  dedicated reader window per work.
- Reader window using `foliate-js`: fetch EPUB bytes over Tauri IPC, render in
  a separate webview, and expose basic reader controls (theme, paginated vs
  scrolled flow, font scale, line height, keybind settings).

Verified today:

- `cargo check --workspace` passes.

Known current shape:

- `jp-dict`, `jp-tokenizer`, `jp-anki`, and most of the broader Phase 4+
  roadmap are still crate stubs or documentation only.
- `jp-vocab` currently contains library persistence and migrations, but not yet
  the vocab / encounter / review model described in the docs.
- Capability files exist for main vs reader windows, but the runtime permission
  surface is still basic and has not yet been tightened to the full
  architecture spec.

Next priorities:

- Tighten the boundary between "planned in docs" and "implemented in code" so
  the docs and journal stay trustworthy.
- Decide whether the next real milestone is Phase 4 vocab persistence or more
  hardening of the existing import/reader path.
