# Phase 8 — Web Importer

**Goal:** paste a URL, get an EPUB. The reader doesn't care that the
source was a web page; vocab tracking, mining, and review surfaces work
the same way.

**Time:** one weekend.

## Pipeline

```
URL
  ↓ reqwest GET (with reasonable User-Agent)
HTML + headers
  ↓ encoding detection (Content-Type / <meta charset> / heuristics)
HTML String
  ↓ Readability extraction (readable-readability or similar)
Cleaned HTML (article-like)
  ↓ HTML → EPUB via rbook
EPUB on disk
```

## Crate addition

```
crates/jp-importer/src/web/
├─ mod.rs
├─ fetch.rs          # HTTP fetch + encoding detection
├─ readability.rs    # article extraction
└─ to_epub.rs        # cleaned HTML → EPUB
```

## Dependencies

```toml
[dependencies]
readable-readability = "0.4"     # Mozilla Readability port
scraper = "0.20"                 # HTML parsing
url = "2"
```

`readable-readability` is a Rust port of Mozilla Readability. It does
the "extract the article body, drop nav/ads/comments" extraction. Quality
is good for typical news sites; weaker for blog platforms with unusual
markup.

## Importer impl

```rust
// crates/jp-importer/src/web/mod.rs

use crate::{Importer, ImportResult};

pub struct WebImporter {
    pub user_agent: String,
}

pub struct WebInput {
    pub url: String,
}

impl Importer for WebImporter {
    type Input = WebInput;

    async fn import(
        &self,
        input: WebInput,
        out_dir: &Path,
    ) -> Result<ImportResult> {
        // 1. Fetch
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .timeout(Duration::from_secs(30))
            .build()?;
        let resp = client.get(&input.url).send().await?;
        let final_url = resp.url().clone();
        let bytes = resp.bytes().await?;

        // 2. Encoding detection (most JP sites are UTF-8 or SJIS)
        let html = decode_html(&bytes);

        // 3. Readability extraction
        let parsed = readable_readability::Readability::new()
            .parse(&html);
        let title = parsed.title;
        let cleaned_html = parsed.content;

        // 4. Sanitize cleaned HTML — strip scripts, iframes, style attrs
        let safe_html = sanitize(&cleaned_html);

        // 5. Build EPUB
        let url_hash = hash_url(&final_url);
        let work_dir = out_dir.join(format!("web-{}", url_hash));
        std::fs::create_dir_all(&work_dir)?;
        let epub_path = work_dir.join("source.epub");
        build_epub(&epub_path, &title, &safe_html, &final_url)?;

        // 6. Save raw text sidecar (for tokenizer in Phase 5+)
        let plaintext = strip_tags(&safe_html);
        let txt_path = work_dir.join("source.txt");
        std::fs::write(&txt_path, &plaintext)?;

        Ok(ImportResult {
            epub_path,
            source_id: format!("web:{}", url_hash),
            title,
            author: None,    // could parse author meta tag
            raw_text_path: Some(txt_path),
        })
    }
}

fn decode_html(bytes: &[u8]) -> String {
    // Try UTF-8 first
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // Fall back to SJIS, then EUC-JP, then ISO-2022-JP
    for enc in [encoding_rs::SHIFT_JIS, encoding_rs::EUC_JP] {
        let (cow, _, had_errors) = enc.decode(bytes);
        if !had_errors { return cow.into_owned(); }
    }
    // Last resort: lossy UTF-8
    String::from_utf8_lossy(bytes).into_owned()
}

fn sanitize(html: &str) -> String {
    // Use ammonia or a hand-rolled allowlist sanitizer.
    // Strip: <script>, <iframe>, <object>, <embed>, on* attributes,
    //        javascript: URLs, style attributes (keep class)
    ammonia::clean(html)
}

fn build_epub(out: &Path, title: &str, html: &str, source_url: &Url) -> Result<()> {
    // Use rbook (same as Phase 7). Key differences from Aozora:
    //  - horizontal text by default (no tategaki)
    //  - one chapter, not multiple
    //  - include source URL as <meta dc:source>
    //  - keep CSS minimal but readable
    todo!()
}
```

## Sanitize aggressively

Web HTML is the most adversarial input we ingest. Even after Readability
extraction, the cleaned HTML can contain:

- `<script>` tags trying to phone home
- `onclick`/`onerror` handlers
- `javascript:` URLs in `<a href>`
- CSS expressions
- Embedded SVGs with scripting
- `<iframe>` to third-party content
- `data:` URIs hiding payloads

Use `ammonia` crate with a strict allowlist. Allowed: paragraph
structure, headings, links (http/https only), images (http/https/data
without scripts), lists, basic emphasis, ruby (if Japanese pages have
it). Disallow everything else.

## CSP for web-sourced reader windows

Reader windows for web-sourced EPUBs get an extra-tight CSP:

```html
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; img-src 'self' https: data:; style-src 'self' 'unsafe-inline'; script-src 'none';">
```

`script-src 'none'` is the critical one. The injected lookup-popup script
runs in a separate world via Tauri's initialization_script, so it
isn't blocked by the page's CSP.

## Tauri command

```rust
#[tauri::command]
pub async fn import_url(
    app: tauri::AppHandle,
    url: String,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    // Validate URL early
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("Only http/https URLs supported".into());
    }

    let library = app.path().app_data_dir().unwrap().join("library");
    let importer = WebImporter {
        user_agent: format!("aozora-tool/{} (https://github.com/...)",
                            env!("CARGO_PKG_VERSION")),
    };

    let result = importer
        .import(WebInput { url }, &library)
        .await
        .map_err(|e| e.to_string())?;

    // Register in library + source DB
    state.db.lock().unwrap().add_library_entry(&result)?;
    Ok(result)
}
```

## UI

Add a "Save URL" button in the main window (or toolbar). Pastes URL,
shows a progress indicator, shows the imported article in the library
when done.

## Acceptance criteria

- [ ] Imports a typical Japanese news article (Asahi, NHK, Yomiuri)
      cleanly
- [ ] Imports a typical blog post (note.com, hatena, syosetu)
- [ ] Strips ads, navigation, and comment sections
- [ ] Preserves ruby/furigana when present in the source
- [ ] EPUB opens in reader, lookups + mining work the same as Aozora
- [ ] URL-derived `source_id` is stable across re-imports of the same
      URL
- [ ] CSP blocks script execution in the imported content
- [ ] Sanitization strips `<script>` even when nested or
      attribute-encoded
- [ ] Network capability scoped to https/http only; localhost-only
      domains blocked

## Common Phase 8 problems

**Readability extraction is empty:** some sites use unusual markup.
Fall back to "use full body" if extraction returns < 200 chars.

**Encoding detection picks wrong encoding:** add `<meta charset>`
detection from the HTML head before falling back to heuristics. Some
old SJIS pages don't declare encoding correctly.

**Vertical text intent lost:** Aozora is always vertical; web pages are
mostly horizontal. EPUB defaults are horizontal here. If a user wants
vertical for everything, add a per-import or global toggle.

**Image references are remote URLs:** the EPUB references
`https://...` for images. They'll only work with network. Decision:
fetch + embed (privacy + offline; bigger EPUB), or leave as URLs (smaller
EPUB; needs network). Default to fetch + embed.

**Login-walled content:** out of scope. We only fetch public pages; no
cookie management, no auth.

## What's next

Phase 9: OCR overlay.
