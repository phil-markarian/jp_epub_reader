# Phase 0 — Setup

**Goal:** working Tauri + Leptos window with the multi-crate workspace
structure in place. Zero functionality. Just a green build that opens a
window.

**Time:** one evening.

## Toolchain install

Same as v1 — Rust, wasm target, Tauri CLI, Leptos CLI, Java for the
optional fallback, cargo-deny for supply-chain hygiene.

```bash
# Rust (skip if already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup update stable
rustup target add wasm32-unknown-unknown

cargo install tauri-cli --version "^2.0" --locked
cargo install cargo-leptos --locked
cargo install create-tauri-app --locked
cargo install trunk --locked
cargo install cargo-deny --locked

# Java for AozoraEpub3 fallback
brew install --cask temurin@21
```

Verify:

```bash
rustc --version          # 1.80+ recommended
cargo tauri --version    # 2.x
trunk --version
java -version            # 21.x.x
```

## Scaffold

Two paths — pick one. Path A is faster but you'll have to retrofit the
workspace structure. Path B sets up the workspace correctly from the
start.

### Path A (fast) — `create-tauri-app` then refactor

```bash
cargo create-tauri-app
# App name: aozora-tool, identifier: dev.yourname.aozora,
# Frontend: Rust, Template: Leptos, Package manager: none
cd aozora-tool
```

Then manually refactor into a workspace by moving `src-tauri/Cargo.toml`
to be a workspace member.

### Path B (correct) — set up workspace by hand

```bash
mkdir aozora-tool && cd aozora-tool
```

Root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "src-tauri",
    "crates/jp-core",
    "crates/jp-tokenizer",
    "crates/jp-dict",
    "crates/jp-vocab",
    "crates/jp-anki",
    "crates/jp-importer",
    "crates/jp-reader",
    # Phase 9+
    # "crates/jp-llm",
    # "crates/jp-ocr",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Your Name"]
license = "GPL-3.0-or-later"

[workspace.dependencies]
# Shared across crates so versions stay aligned
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
rusqlite = { version = "0.32", features = ["bundled"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "stream", "gzip"] }
encoding_rs = "0.8"
zip = { version = "2", default-features = false, features = ["deflate"] }
csv = "1.3"
```

Create the crate skeletons:

```bash
mkdir -p crates
cd crates
for crate in jp-core jp-tokenizer jp-dict jp-vocab jp-anki jp-importer jp-reader; do
    cargo new --lib "$crate"
done
cd ..
```

Each crate's `Cargo.toml` should reference workspace versions:

```toml
[package]
name = "jp-core"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
thiserror = { workspace = true }
```

Then scaffold the Tauri app inside `src-tauri/`:

```bash
mkdir -p src-tauri/src
# ... follow Tauri 2's manual setup, or use create-tauri-app
# pointed at the existing directory
```

For most users, **Path A is faster** — let `create-tauri-app` set up
Tauri+Leptos, then add the `crates/` directory and adjust the workspace
config. The only thing you have to manually do is convert the root
`Cargo.toml` to a workspace.

## Initial crate stubs

Each crate gets a placeholder `lib.rs`. Example for `jp-core`:

```rust
//! Shared types and errors used across the workspace.
//! No business logic; just data structures.

pub mod error;
pub mod source;
pub mod settings;

pub use error::{Error, Result};
```

Create stub files even if empty — `cargo check --workspace` will validate
the structure compiles before you add content.

## Initialize JOURNAL and supply-chain config

`JOURNAL.md`:

```markdown
# Journal

## YYYY-MM-DD — Phase 0 setup
- Toolchain installed
- Multi-crate workspace scaffolded
- `cargo tauri dev` opens a window — phase 0 done
```

`deny.toml` at workspace root:

```toml
[graph]
all-features = false

[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
yanked = "deny"

[licenses]
allow = [
    "MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause", "BSD-3-Clause", "ISC",
    "Unicode-DFS-2016", "Unicode-3.0",
    "Zlib", "MPL-2.0", "CC0-1.0",
    "GPL-3.0", "GPL-3.0-or-later",  # we are GPL-3
]
confidence-threshold = 0.8

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

## First run

```bash
cargo check --workspace      # verify the workspace compiles
cargo tauri dev              # build and launch
```

A window opens with the Leptos demo. If you see it, Phase 0 is done.

## Acceptance criteria

- [ ] `cargo check --workspace` passes
- [ ] `cargo tauri dev` opens a window without errors
- [ ] `cargo deny check` passes
- [ ] All seven Phase 1-6 crate stubs compile (empty lib.rs is fine)
- [ ] Repo committed to git
- [ ] `JOURNAL.md` has its first entry

## Common Phase 0 problems

**Workspace dependency conflicts:** if you add a dep to one crate without
declaring it in `[workspace.dependencies]`, version drift creeps in. Use
`cargo tree --duplicates` periodically to catch it.

**Tauri can't find frontend:** if Path B's manual setup, `Trunk.toml` and
`tauri.conf.json` need to point at each other correctly. Easiest fix: do
Path A and refactor.

**Build fails on `wry` or `webkit2gtk`:** macOS only needs Xcode CLI
tools. Run `xcode-select --install`.

**`cargo tauri` not found:** `~/.cargo/bin` not on PATH. Add
`source "$HOME/.cargo/env"` to your shell rc.

## What we did NOT do in Phase 0

Resist adding features here:

- No Aozora code yet (Phase 1)
- No vocab DB schema yet (Phase 4)
- No styling beyond the template
- No CI

Phase 0 is the toolchain working. That's it.
