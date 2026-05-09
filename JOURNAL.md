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
