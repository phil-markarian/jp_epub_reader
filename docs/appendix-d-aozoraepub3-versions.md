# Appendix D — AozoraEpub3: Both Versions

There are two maintained variants of AozoraEpub3 in the wild. Bundle either —
or both — depending on how robust you want the fallback to be.

## The two versions

### Version A — `AozoraEpub3-JDK21` (modernized fork)

**Repo:** https://github.com/AozoraEpub3-JDK21/AozoraEpub3-JDK21
**Site:** https://aozoraepub3-jdk21.github.io/AozoraEpub3-JDK21/
**License:** GPL-3.0
**Java required:** 21+

What's different from the original:

- Builds and runs cleanly on modern JDK 21 / 24
- Updated dependencies (newer epubcheck, newer Apache POI, etc.)
- Bug fixes for edge cases that broke on newer Java versions
- Updated `chuki_utf.txt` with additional gaiji mappings contributed since
  the original went dormant
- Some UI strings translated / cleaned up
- Active maintenance (commits within the last year, issues being
  triaged)

This is the version we recommend bundling as the **default**.

### Version B — `hmdev/AozoraEpub3` (original)

**Repo:** https://github.com/hmdev/AozoraEpub3
**License:** GPL-3.0
**Java required:** 8+ (works through 21 with warnings)

What's different from the JDK21 fork:

- The reference implementation. Years of accumulated edge-case handling.
- Dormant — last meaningful commit ~2020. Issues unanswered.
- Some quirks on JDK 21 (reflective access warnings, occasional crashes
  on specific malformed inputs) that the JDK21 fork has fixed.
- For some old or weirdly-formatted Aozora files, the original happens to
  produce *better* output than the fork because the fork has changed
  heuristics that the maintainer didn't fully test against the entire
  corpus.

This is the version we recommend bundling as the **fallback**.

## Why bundle both

The two versions disagree on a small percentage of files. Roughly:

- ~95% of works: identical or near-identical EPUB output
- ~3% of works: cosmetic differences (chapter detection heuristic, ruby
  edge case)
- ~1.5% of works: one of them produces clearly better output
- ~0.5% of works: one of them errors out, the other completes

Bundling both lets you:

1. Default to the modernized fork (better Java compatibility, fewer
   warnings)
2. Fall back to the original if the fork errors or produces empty/garbage
   output
3. Eventually let the user pick per-work from the library UI

## Bundling strategy

Two parallel directory trees in `src-tauri/resources/`:

```
src-tauri/resources/
├─ aozoraepub3-jdk21/             # Version A
│  ├─ AozoraEpub3.jar
│  ├─ chuki_tag.txt
│  ├─ chuki_utf.txt
│  ├─ chuki_ivs.txt
│  ├─ template/
│  └─ lib/
└─ aozoraepub3-original/          # Version B
   ├─ AozoraEpub3.jar
   ├─ chuki_tag.txt
   ├─ chuki_utf.txt
   ├─ chuki_ivs.txt
   ├─ template/
   └─ lib/
```

`tauri.conf.json`:

```json
{
  "bundle": {
    "resources": [
      "resources/aozoraepub3-jdk21/**/*",
      "resources/aozoraepub3-original/**/*",
      "resources/ttu/**/*"
    ]
  }
}
```

This adds roughly 25-30 MB to the bundle (each version is ~12 MB
compressed, ~15 MB extracted). For a desktop app where the user has
opted in to a Japanese reader, this is fine.

## Strategy enum

Replace the simpler `Strategy` from Phase 2 with a more explicit one:

```rust
// src-tauri/src/converter/mod.rs

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum ConverterStrategy {
    /// Use the JDK21-modernized fork (default).
    JarJdk21,
    /// Use the original hmdev/AozoraEpub3 (fallback).
    JarOriginal,
    /// Try JDK21 first, fall back to original on error or empty output.
    JarAuto,
    /// Native Rust converter (Phase 6).
    Native,
    /// Native first, fall back to JDK21 jar on error.
    NativeAuto,
}

impl Default for ConverterStrategy {
    fn default() -> Self { Self::JarAuto }
}
```

## Implementation

### Per-version paths

```rust
// src-tauri/src/converter/mod.rs

use std::path::{Path, PathBuf};

pub struct JarPaths {
    pub jdk21: PathBuf,
    pub original: PathBuf,
}

impl JarPaths {
    pub fn from_resource_dir(resource_dir: &Path) -> Self {
        Self {
            jdk21: resource_dir.join("resources/aozoraepub3-jdk21"),
            original: resource_dir.join("resources/aozoraepub3-original"),
        }
    }

    pub fn for_strategy(&self, strategy: ConverterStrategy) -> Option<&Path> {
        match strategy {
            ConverterStrategy::JarJdk21 | ConverterStrategy::JarAuto => Some(&self.jdk21),
            ConverterStrategy::JarOriginal => Some(&self.original),
            _ => None,
        }
    }
}
```

### Auto-fallback runner

```rust
// src-tauri/src/converter/mod.rs

use anyhow::{anyhow, Result};

pub fn convert(
    strategy: ConverterStrategy,
    jars: &JarPaths,
    txt_path: &Path,
    out_dir: &Path,
) -> Result<PathBuf> {
    match strategy {
        ConverterStrategy::JarJdk21 => {
            aozora_epub3::convert(&jars.jdk21, txt_path, out_dir)
        }
        ConverterStrategy::JarOriginal => {
            aozora_epub3::convert(&jars.original, txt_path, out_dir)
        }
        ConverterStrategy::JarAuto => {
            // Try JDK21 fork first
            match aozora_epub3::convert(&jars.jdk21, txt_path, out_dir) {
                Ok(p) if epub_looks_valid(&p) => Ok(p),
                Ok(p) => {
                    tracing::warn!(
                        "JDK21 fork produced suspicious output for {:?}, retrying with original",
                        txt_path
                    );
                    let _ = std::fs::remove_file(&p);
                    aozora_epub3::convert(&jars.original, txt_path, out_dir)
                }
                Err(e) => {
                    tracing::warn!(
                        "JDK21 fork failed for {:?} ({}), retrying with original",
                        txt_path, e
                    );
                    aozora_epub3::convert(&jars.original, txt_path, out_dir)
                }
            }
        }
        ConverterStrategy::Native => {
            native::convert(txt_path, out_dir)
        }
        ConverterStrategy::NativeAuto => {
            match native::convert(txt_path, out_dir) {
                Ok(p) if epub_looks_valid(&p) => Ok(p),
                _ => aozora_epub3::convert(&jars.jdk21, txt_path, out_dir),
            }
        }
    }
}

/// Sanity check on a produced EPUB. Returns false if it's missing
/// critical content (suggesting a converter bug).
fn epub_looks_valid(epub_path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(epub_path) else { return false };
    if bytes.len() < 4096 {
        // Real Aozora EPUBs are at least a few tens of KB
        return false;
    }
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    // Should contain at least one xhtml in EPUB/ or OEBPS/
    let mut has_content = false;
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index(i) {
            let n = f.name();
            if (n.starts_with("EPUB/") || n.starts_with("OEBPS/"))
                && n.ends_with(".xhtml")
            {
                if f.size() > 200 {
                    has_content = true;
                    break;
                }
            }
        }
    }
    has_content
}
```

### Settings UI

Add a strategy dropdown in the Settings tab:

```rust
#[component]
fn ConverterSettings() -> impl IntoView {
    let (strategy, set_strategy) = signal(ConverterStrategy::default());

    spawn_local(async move {
        if let Ok(s) = tauri_bridge::call_no_args::<ConverterStrategy>("get_converter_strategy").await {
            set_strategy.set(s);
        }
    });

    view! {
        <section class="settings">
            <h2>"Converter"</h2>
            <p class="help">
                "Choose which engine converts Aozora .txt files to EPUB. "
                "Auto tries the modernized fork first and falls back to the original if that fails."
            </p>
            <select on:change:target=move |ev| {
                let v = ev.target().value();
                let s: ConverterStrategy = serde_json::from_str(&format!("\"{v}\"")).unwrap_or_default();
                set_strategy.set(s);
                spawn_local(async move {
                    let args = serde_json::json!({ "strategy": s });
                    let _ = tauri_bridge::call::<_, ()>("set_converter_strategy", &args).await;
                });
            }>
                <option value="JarAuto" selected=move || matches!(strategy.get(), ConverterStrategy::JarAuto)>
                    "Auto (recommended)"
                </option>
                <option value="JarJdk21" selected=move || matches!(strategy.get(), ConverterStrategy::JarJdk21)>
                    "Modernized fork (JDK 21)"
                </option>
                <option value="JarOriginal" selected=move || matches!(strategy.get(), ConverterStrategy::JarOriginal)>
                    "Original AozoraEpub3"
                </option>
                <option value="Native" selected=move || matches!(strategy.get(), ConverterStrategy::Native)>
                    "Native (Rust, experimental)"
                </option>
                <option value="NativeAuto" selected=move || matches!(strategy.get(), ConverterStrategy::NativeAuto)>
                    "Native with jar fallback"
                </option>
            </select>
        </section>
    }
}
```

## Per-work override

In the library UI, add a "Reconvert with..." submenu so the user can try
each strategy for a problem work:

```rust
#[derive(serde::Serialize)]
struct ReconvertArgs {
    work_id: u32,
    strategy: ConverterStrategy,
}

#[tauri::command]
pub async fn reconvert_work(
    app: tauri::AppHandle,
    work_id: u32,
    strategy: ConverterStrategy,
    state: State<'_, AppState>,
) -> Result<ConvertedWork, String> {
    // Same as download_and_convert, but skips the download step (txt
    // already on disk) and uses the requested strategy.
    let library = app.path().app_data_dir().map_err(|e| e.to_string())?
        .join("library").join(work_id.to_string());
    let txt_path = library.join("source.txt");
    if !txt_path.exists() {
        return Err("source.txt missing — redownload first".into());
    }

    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let jars = converter::JarPaths::from_resource_dir(&resource_dir);

    let txt = txt_path.clone();
    let epub = tokio::task::spawn_blocking(move || {
        converter::convert(strategy, &jars, &txt, &library)
    })
    .await
    .map_err(|e| format!("join error: {e}"))?
    .map_err(|e| format!("convert error: {e}"))?;

    Ok(ConvertedWork {
        work_id,
        epub_path: epub.to_string_lossy().to_string(),
        txt_path: txt_path.to_string_lossy().to_string(),
    })
}
```

In the library row UI:

```rust
view! {
    <li class="lib-entry">
        <div class="info">{e.title.clone()}</div>
        <div class="actions">
            <button on:click=open_handler>"Open"</button>
            <details class="reconvert">
                <summary>"Reconvert"</summary>
                <button on:click=reconvert(ConverterStrategy::JarJdk21)>"with JDK21 fork"</button>
                <button on:click=reconvert(ConverterStrategy::JarOriginal)>"with original"</button>
                <button on:click=reconvert(ConverterStrategy::Native)>"with native (Rust)"</button>
            </details>
        </div>
    </li>
}
```

## Java compatibility matrix

| AozoraEpub3 version | Java 8 | Java 11 | Java 17 | Java 21 | Java 24 |
|---|---|---|---|---|---|
| Original (hmdev) | ✅ | ✅ | ⚠️ warnings | ⚠️ warnings, occasional crashes | ❌ build errors |
| JDK21 fork | ❌ class file version | ❌ | ✅ | ✅ | ✅ |

Recommendation: tell users to install Temurin 21 (`brew install --cask
temurin@21`). Both versions work on it. Detect version at runtime:

```rust
pub fn detect_java() -> Result<JavaInfo> {
    let bin = which::which("java")?;
    let out = std::process::Command::new(&bin).arg("-version").output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first_line = stderr.lines().next().unwrap_or("");

    // Parse "openjdk version \"21.0.4\" 2024-07-16"
    let major = first_line
        .split('"').nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0);

    Ok(JavaInfo { binary: bin, version_string: first_line.to_string(), major })
}
```

If `major < 17`, surface a UI warning: "Java 17+ recommended; some files
may fail to convert. Install Temurin 21 for best results."

If `major < 8`, refuse to use either jar.

## Maintenance: updating the bundled jars

Both repos release infrequently. Once or twice a year, do:

1. Download latest tarball from each
2. Extract into `src-tauri/resources/aozoraepub3-jdk21/` and
   `src-tauri/resources/aozoraepub3-original/` respectively
3. Run the regression suite (the three reference works should produce
   byte-identical EPUBs to the previous bundled version, modulo a
   `dc:identifier` UUID)
4. If diffs appear, investigate before committing
5. Update `JOURNAL.md`

## License notes

Both versions are GPL-3.0. Bundling them in your app means:

- Your distribution must include the source (or a written offer to provide
  it) for both jars
- Your modifications to the jars (if any — generally there should be none;
  use them as-is) are also GPL-3.0
- Your Rust application code is independent, but if you statically link or
  combine GPL code in a way that creates a "combined work", the whole work
  must be GPL-compatible. Subprocess invocation (which is what we do) is
  generally NOT considered combination — it's separate processes, like
  invoking `git` from a script.

For a personal/open-source app: license everything GPL-3 or compatible.
For closed-source distribution: keep the jars as separate downloadable
artifacts (don't bundle in the same installer), or replace with the native
converter (Phase 6).

## Summary

Bundle both jars. Default strategy: `JarAuto` — try modernized fork, fall
back to original. Settings UI exposes the choice for power users. The
"Reconvert" per-work feature handles edge cases without forcing a global
strategy change. Once Phase 6 native converter is solid, default flips to
`NativeAuto` (native, falling back to JDK21 fork).
