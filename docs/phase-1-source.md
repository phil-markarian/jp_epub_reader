# Phase 1 — Source: Aozora index + aozorabunko_text repo

**Goal:** browse Aozora's full catalog, search, and resolve any selected
work to a local text file (using the aozorabunko_text repo as the actual
content source).

**Time:** one weekend.

**Big change from v1:** we no longer download per-book zip files from
Aozora's webserver. The aozorabunko_text repo (https://github.com/aozorahack/aozorabunko_text)
mirrors all `.txt` files into a single git repo, kept current daily by CI.
This is more reliable, simpler, and supports an offline-first mode.

## Two data sources, one workflow

1. **Index CSV** from Aozora — metadata (title, author, work_id, copyright,
   reading)
2. **aozorabunko_text repo** — actual text content per work

The CSV gives you "what works exist." The repo gives you "the bytes."

### Index CSV (unchanged from v1)

```
https://www.aozora.gr.jp/index_pages/list_person_all_extended_utf8.zip
```

Columns we care about (see v1 phase 1 doc for full reference):

| Index | Header (Japanese) | Meaning |
|---|---|---|
| 0 | 作品ID | Work ID |
| 1 | 作品名 | Title |
| 2 | 作品名読み | Title reading |
| 4 | 作品著作権フラグ | Copyright flag |
| 15 | 人物ID | Author ID |
| 16/17 | 姓/名 | Author surname/given |
| 18/19 | 姓読み/名読み | Author reading |
| 45 | テキストファイルURL | URL to zip on Aozora's server (we don't use this) |
| 50 | XHTML/HTMLファイルURL | URL to HTML rendering |

**What we no longer use:** column 45's URL. Instead we resolve directly to
the aozorabunko_text repo path.

### aozorabunko_text repo

Two ways to access:

**A. Clone once, sync occasionally** (offline-first):

```bash
git clone --depth 1 https://github.com/aozorahack/aozorabunko_text.git
# ~200MB initial download
# Refresh occasionally via `git pull --depth 1`
```

**B. Fetch individual files on demand** (no big upfront download):

```
https://aozorahack.org/aozorabunko_text/cards/{author_id_padded}/files/{stem}/{stem}.txt
```

Where:
- `author_id_padded` = author ID zero-padded to 6 digits (`81` → `000081`)
- `stem` = the zip filename without `.zip` (e.g., `45630_txt_23610`)

The repo's filename convention: each text lives at `cards/{author_id}/files/{stem}/{stem}.txt`
where `stem` matches the original zip filename. This is documented in their
README.

**Encoding gotcha:** the README warns that files have `text/plain;
charset=utf-8` Content-Type but are actually Shift-JIS. Always decode as
SJIS regardless of what the server says.

## Source resolution

Bridge between "user clicked work_id 773" and "give me the text":

```rust
// crates/jp-core/src/source.rs

#[derive(Debug, Clone)]
pub struct SourceRef {
    pub kind: SourceKind,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    Aozora,
    Web,
    PlainText,
    Ocr,
    Manual,
}
```

```rust
// crates/jp-importer/src/aozora/source.rs

use anyhow::Result;
use std::path::{Path, PathBuf};

pub enum AozoraSource {
    /// Local clone of aozorabunko_text repo
    LocalRepo(PathBuf),
    /// Fetch individual files from aozorahack.org on demand
    Remote { cache_dir: PathBuf },
}

impl AozoraSource {
    /// Resolve a work_id to a path containing the SJIS .txt
    pub async fn resolve(&self, work_id: u32, author_id: u32, stem: &str)
        -> Result<PathBuf>
    {
        match self {
            Self::LocalRepo(root) => {
                let path = root
                    .join("cards")
                    .join(format!("{:06}", author_id))
                    .join("files")
                    .join(stem)
                    .join(format!("{}.txt", stem));
                if !path.exists() {
                    anyhow::bail!("file missing in local repo: {:?}", path);
                }
                Ok(path)
            }
            Self::Remote { cache_dir } => {
                let cached = cache_dir
                    .join(format!("{:06}", author_id))
                    .join(format!("{}.txt", stem));
                if cached.exists() {
                    return Ok(cached);
                }
                let url = format!(
                    "https://aozorahack.org/aozorabunko_text/cards/{:06}/files/{}/{}.txt",
                    author_id, stem, stem
                );
                let bytes = reqwest::get(&url).await?.bytes().await?;
                std::fs::create_dir_all(cached.parent().unwrap())?;
                std::fs::write(&cached, &bytes)?;
                Ok(cached)
            }
        }
    }
}
```

**The `stem` problem:** the aozorabunko_text repo uses zip filenames as
identifiers, but the Aozora CSV gives you a URL like
`https://www.aozora.gr.jp/cards/000081/files/45630_ruby_23610.zip`. You
extract the stem (`45630_ruby_23610`) from the URL.

For some works the CSV has multiple file URLs (one for the original
ruby-annotated version, one for a stripped version). We default to the
ruby version — that's what the aozorabunko_text repo always provides.

## Index loader (mostly v1 with minor updates)

```rust
// crates/jp-importer/src/aozora/index.rs

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AozoraWork {
    pub work_id: u32,
    pub author_id: u32,
    pub title: String,
    pub title_yomi: String,
    pub author: String,
    pub author_yomi: String,
    pub copyright_active: bool,
    pub stem: Option<String>,           // zip filename without extension
}

const INDEX_URL: &str =
    "https://www.aozora.gr.jp/index_pages/list_person_all_extended_utf8.zip";
const FRESHNESS_DAYS: u64 = 7;

pub async fn ensure_index(cache_dir: &Path) -> Result<PathBuf> {
    // Identical to v1: download zip, extract CSV, cache for 7 days.
    // See v1 phase 1 doc for full implementation.
    todo!()
}

pub fn parse_index(csv_path: &Path) -> Result<Vec<AozoraWork>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(csv_path)?;

    let mut works = Vec::with_capacity(20_000);
    let mut saw_kokoro = false;

    for record in rdr.records() {
        let r = record?;

        let work_id: u32 = r.get(0).unwrap_or("0").parse().unwrap_or(0);
        let title = r.get(1).unwrap_or("").to_string();
        let title_yomi = r.get(2).unwrap_or("").to_string();
        let copyright_active = r.get(4).unwrap_or("") == "あり";
        let author_id: u32 = r.get(15).unwrap_or("0").parse().unwrap_or(0);
        let surname = r.get(16).unwrap_or("");
        let given = r.get(17).unwrap_or("");
        let surname_yomi = r.get(18).unwrap_or("");
        let given_yomi = r.get(19).unwrap_or("");
        let text_url = r.get(45).filter(|s| !s.is_empty());

        // Extract stem from URL like:
        //   https://www.aozora.gr.jp/cards/000081/files/45630_ruby_23610.zip
        let stem = text_url.and_then(|url| {
            url.rsplit('/').next()
                .and_then(|n| n.strip_suffix(".zip"))
                .map(String::from)
        });

        if work_id == 773 && title.contains("こころ") {
            saw_kokoro = true;
        }

        works.push(AozoraWork {
            work_id, author_id,
            title, title_yomi,
            author: format!("{}{}", surname, given),
            author_yomi: format!("{}{}", surname_yomi, given_yomi),
            copyright_active,
            stem,
        });
    }

    if !saw_kokoro {
        return Err(anyhow!(
            "Schema sanity check failed: work_id 773 / こころ not found. \
             Aozora may have changed CSV columns."
        ));
    }

    Ok(works)
}
```

## Tauri commands

Frontend talks to backend through three commands:

```rust
// src-tauri/src/commands/source.rs

#[tauri::command]
pub async fn refresh_index(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    // Same as v1
}

#[tauri::command]
pub fn search_works(
    query: String,
    limit: usize,
    only_public_domain: bool,
    state: State<'_, AppState>,
) -> Vec<AozoraWork> {
    // Same as v1
}

/// User has chosen "use local repo" — set the path to their checkout
#[tauri::command]
pub fn set_aozora_repo_path(
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if !p.join("cards").is_dir() {
        return Err(format!("not an aozorabunko_text checkout: {}", path));
    }
    state.settings.lock().unwrap()
        .set("aozora.repo_path", &path)
        .map_err(|e| e.to_string())
}

/// Return current source mode + diagnostics
#[tauri::command]
pub fn get_aozora_source_status(
    state: State<'_, AppState>,
) -> AozoraSourceStatus {
    let settings = state.settings.lock().unwrap();
    let repo_path = settings.get("aozora.repo_path");
    AozoraSourceStatus {
        mode: if repo_path.is_some() { "local" } else { "remote" }.into(),
        repo_path,
        repo_size: /* du -sh on the repo if local */,
    }
}
```

## Frontend — settings + search

Add a new "Sources" section to settings:

- Radio button: "Use local repo" vs "Fetch on demand"
- If local: file picker for the repo path, "Clone now" button (runs
  `git clone --depth 1 ...` via shell command), "Refresh" button
- Diagnostics: total works in repo, last sync time

Search UI: same as v1 but result rows now show whether the work is
available locally (faster) or will require a download (slower).

## Capability declaration

`src-tauri/capabilities/main.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main",
  "description": "Main window — library + settings",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open",
    {
      "identifier": "fs:scope",
      "allow": [
        { "path": "$APPDATA/**" },
        { "path": "$APPCACHE/**" }
      ]
    },
    {
      "identifier": "http:default",
      "allow": [
        { "url": "https://www.aozora.gr.jp/*" },
        { "url": "https://aozorahack.org/*" }
      ]
    }
  ]
}
```

## Acceptance criteria

- [ ] Index downloads on first launch, caches to `app_cache_dir`
- [ ] Search returns Sōseki for "夏目" / "natsume"
- [ ] Search returns Rashomon (127), Ningen Shikkaku (301), Kokoro (773)
- [ ] Settings UI lets user choose local repo vs remote
- [ ] In remote mode, fetching a work file produces a cached SJIS .txt
- [ ] In local mode, resolving Kokoro returns the path inside the cloned
      repo
- [ ] Public-domain toggle filters correctly
- [ ] Schema sanity check fires loudly if Aozora changes CSV columns

## Common Phase 1 problems

**Stem extraction fails on some works:** older works sometimes have
unusual URL formats. Log and skip — those works won't be downloadable
without manual intervention. Almost always affects works that have been
withdrawn or have weird metadata.

**Local repo doesn't have a file you expect:** the repo runs daily but
isn't always perfectly in sync. Fall back to remote fetch if local
resolution fails:

```rust
let path = match local.resolve(...).await {
    Ok(p) => p,
    Err(_) => remote.resolve(...).await?,
};
```

**Repo clone is huge:** ~200MB compressed, larger when checked out.
Offer `git clone --depth 1` only — full history is irrelevant for our
purposes.

**Encoding errors when reading repo files:** they're SJIS, not UTF-8.
Always decode through `encoding_rs::SHIFT_JIS`. The Content-Type header
on the web version lies.

## What's next

Phase 2: importer — turn the resolved SJIS .txt into an EPUB using the
bundled AozoraEpub3 jars.
