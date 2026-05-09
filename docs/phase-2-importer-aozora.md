# Phase 2 — Importer: Aozora (jar-based)

**Goal:** turn a resolved SJIS .txt into a valid EPUB on disk, using
bundled AozoraEpub3 jars (both the JDK21 fork and the original) with
auto-fallback.

**Time:** one evening (plus jar bundling).

This phase is largely v1 phase 2 + appendix-d, refactored into the
`jp-importer` crate. Skim v1 for jar bundling details; this doc focuses
on what changed.

## Crate-level layout

```
crates/jp-importer/
├─ Cargo.toml
├─ src/
│  ├─ lib.rs              # public API: `Importer` trait
│  ├─ aozora/
│  │  ├─ mod.rs           # re-exports
│  │  ├─ source.rs        # AozoraSource (from Phase 1)
│  │  ├─ index.rs         # index parsing (from Phase 1)
│  │  ├─ jar.rs           # AozoraEpub3 jar runner
│  │  ├─ java.rs          # Java detection
│  │  └─ download.rs      # download + decode helpers
│  └─ types.rs            # ImportResult, ImportError
```

## The Importer trait

This is the abstraction that pays off in Phase 8 (web importer):

```rust
// crates/jp-importer/src/lib.rs

use anyhow::Result;
use std::path::{Path, PathBuf};

pub trait Importer {
    type Input;

    /// Convert an input (URL, file path, work_id) to an EPUB on disk.
    /// Returns the EPUB path on success.
    async fn import(
        &self,
        input: Self::Input,
        out_dir: &Path,
    ) -> Result<ImportResult>;
}

pub struct ImportResult {
    pub epub_path: PathBuf,
    pub source_id: String,        // unique identifier for this source
    pub title: String,
    pub author: Option<String>,
    pub raw_text_path: Option<PathBuf>, // if available, for tokenization later
}
```

The Aozora importer implements this. Web importer (Phase 8) implements
this. OCR text importer (later) implements this. All produce EPUBs the
reader consumes.

## AozoraEpub3 jar bundling — both versions

Carry over from v1 appendix-d. Both jars in `src-tauri/resources/`:

```
src-tauri/resources/
├─ aozoraepub3-jdk21/         # default
└─ aozoraepub3-original/      # fallback
```

`tauri.conf.json`:

```json
{
  "bundle": {
    "resources": [
      "resources/aozoraepub3-jdk21/**/*",
      "resources/aozoraepub3-original/**/*"
    ]
  }
}
```

## Strategy enum

```rust
// crates/jp-importer/src/aozora/jar.rs

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum AozoraStrategy {
    /// JDK21 fork (default)
    JarJdk21,
    /// Original hmdev/AozoraEpub3 (fallback)
    JarOriginal,
    /// Try JDK21 first, fall back to original on error or empty output
    JarAuto,
    /// Native Rust converter (Phase 7)
    Native,
    /// Native first, fall back to JDK21 on error
    NativeAuto,
}

impl Default for AozoraStrategy {
    fn default() -> Self { Self::JarAuto }
}
```

Implementation: see v1 appendix-d for the full subprocess code,
`epub_looks_valid` sanity check, and per-strategy dispatch. Behavior is
identical; just lives in the importer crate now instead of `src-tauri`.

## Aozora importer

```rust
// crates/jp-importer/src/aozora/mod.rs

pub struct AozoraImporter {
    pub source: AozoraSource,
    pub jars: JarPaths,
    pub strategy: AozoraStrategy,
}

pub struct AozoraInput {
    pub work: AozoraWork,        // from index
}

impl Importer for AozoraImporter {
    type Input = AozoraInput;

    async fn import(
        &self,
        input: AozoraInput,
        out_dir: &Path,
    ) -> Result<ImportResult> {
        let stem = input.work.stem.as_ref()
            .ok_or_else(|| anyhow!("work has no source URL"))?;

        // 1. Resolve to SJIS .txt via Phase 1 source
        let sjis_path = self.source
            .resolve(input.work.work_id, input.work.author_id, stem)
            .await?;

        // 2. Decode to UTF-8 sidecar (used by tokenizer in Phase 5)
        let bytes = std::fs::read(&sjis_path)?;
        let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(&bytes);
        if had_errors {
            tracing::warn!("SJIS decode had errors for work {}", input.work.work_id);
        }

        let work_dir = out_dir.join(input.work.work_id.to_string());
        std::fs::create_dir_all(&work_dir)?;
        let utf8_path = work_dir.join("source.utf8.txt");
        std::fs::write(&utf8_path, cow.as_bytes())?;

        // 3. Run jar (or native) converter
        let txt_for_jar = work_dir.join("source.txt");
        std::fs::copy(&sjis_path, &txt_for_jar)?;  // jar wants the SJIS

        let strategy = self.strategy;
        let jars = self.jars.clone();
        let work_dir_clone = work_dir.clone();
        let epub_path = tokio::task::spawn_blocking(move || {
            jar::convert(strategy, &jars, &txt_for_jar, &work_dir_clone)
        }).await??;

        Ok(ImportResult {
            epub_path,
            source_id: format!("aozora:{}", input.work.work_id),
            title: input.work.title.clone(),
            author: Some(input.work.author.clone()),
            raw_text_path: Some(utf8_path),
        })
    }
}
```

## Tauri command

```rust
#[tauri::command]
pub async fn import_aozora_work(
    app: tauri::AppHandle,
    work_id: u32,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let work = state.works.lock().unwrap()
        .iter().find(|w| w.work_id == work_id)
        .cloned()
        .ok_or("work not in index")?;

    let library = app.path().app_data_dir().unwrap().join("library");
    let resource_dir = app.path().resource_dir().unwrap();

    let importer = AozoraImporter {
        source: build_aozora_source(&state)?,
        jars: JarPaths::from_resource_dir(&resource_dir),
        strategy: state.settings.lock().unwrap()
            .get_or_default("aozora.strategy"),
    };

    let result = importer
        .import(AozoraInput { work }, &library)
        .await
        .map_err(|e| e.to_string())?;

    // Phase 4: register source in DB (placeholder until Phase 4 lands)
    // state.vocab.lock().unwrap().register_source(...)

    Ok(result)
}
```

## Acceptance criteria

- [ ] Both bundled jars resolve at runtime (dev + bundled app)
- [ ] Java detection produces clear error if `java` not on PATH
- [ ] `JarAuto` strategy converts Kokoro, Rashomon, Ningen Shikkaku
- [ ] EPUBs open in Apple Books with correct tategaki + ruby
- [ ] If no `stem` (rare for old entries), error is surfaced cleanly
- [ ] `epub_looks_valid` triggers fallback on suspicious output (test
      this by feeding an obviously-broken .txt)
- [ ] Settings UI lets user pick strategy globally
- [ ] Library row "Reconvert with..." menu lets user override per-work

## Common Phase 2 problems

(Same as v1 phase 2 / appendix-d.)

## What's next

Phase 3: library view + reader. Mostly unchanged from v1; small
adjustments to integrate with the new `ImportResult` type.
