# Appendix C — Security Checklist

A living document. Audit before each phase ships, before each release,
and any time the threat surface changes.

## Per-phase audit

### Phase 0 (setup)

- [ ] `Cargo.lock` committed
- [ ] `cargo deny check` passes
- [ ] `deny.toml` rejects unknown registries and unknown git sources
- [ ] No `[patch.crates-io]` overrides without explicit justification
- [ ] CI runs `cargo audit` (or equivalent advisory check) on every PR

### Phase 1 (Aozora source)

- [ ] All HTTPS to `aozora.gr.jp` and `aozorahack.org`; no HTTP fallback
- [ ] Index download verifies decompression doesn't escape working dir
      (zip slip)
- [ ] CSV schema sanity check fires loudly if columns shift
- [ ] If using local repo path: validated as a directory containing
      `cards/`; symlinks resolved, not followed past the root
- [ ] Cached files have unique paths derived from work_id, not
      attacker-supplied filenames

### Phase 2 (Aozora importer / jar)

- [ ] Both bundled jars verified by sha256 at first run; mismatched jars
      refuse to run
- [ ] Java subprocess runs with restricted args; no shell interpolation
- [ ] EPUB output validated before being added to library
      (`epub_looks_valid`)
- [ ] Imported EPUB filename derived from work_id, not Aozora-provided
      title
- [ ] Per-import working directory cleaned up on failure

### Phase 3 (library + reader)

- [ ] Library SQLite uses parameterized queries everywhere; zero string
      concatenation in SQL
- [ ] Reader window has its own capability scope (cannot trigger
      downloads, cannot modify settings)
- [ ] Reader windows cannot navigate to arbitrary URLs (CSP locks
      script-src)
- [ ] Click-capture injection runs in initialization_script, not in
      page-controlled scope
- [ ] Path validation on `epub_path`: must live under
      `$APPDATA/library/`
- [ ] Sentence extraction handles malformed UTF-8 / unterminated runs
      without panicking

### Phase 4 (vocab DB + Anki)

- [ ] All DB writes through `record_encounter` / `set_status` /
      `link_anki_note` — no scattered SQL in commands
- [ ] FTS index sync verified (insert + delete triggers fire)
- [ ] Retention algorithm covered by test: never deletes the
      first-ever encounter
- [ ] Retention runs as background task, not as trigger
- [ ] Status transitions follow the state machine (test invalid
      transitions are blocked)
- [ ] AnkiConnect: only POST to `127.0.0.1:8765`; reject
      `localhost`-mismatched responses
- [ ] Handlebars templates: user-supplied templates can't escape into
      the engine's helpers (sandboxed by default in `handlebars` crate)
- [ ] Card content sanitized: HTML-escape user-typed notes before
      template render
- [ ] AnkiConnect timeout enforced (Anki not running shouldn't hang the
      app)

### Phase 5 (dictionary + popup)

- [ ] Dictionary `.zip` import: validate file paths inside zip don't
      contain `..` or absolute paths (zip slip)
- [ ] `index.json` schema validated before insertion
- [ ] Term entries with malformed structure logged and skipped, not
      crash the import
- [ ] Structured-content rendering: no script execution, no fetch from
      arbitrary URLs
- [ ] Lookup query parameterized; no SQL injection via crafted text
- [ ] Deinflection rules are static (`include_str!`); not user-loadable
- [ ] Popup positioning won't render off-screen content
- [ ] LookupResult sanitized before send to frontend (escape tags in
      structured content)

### Phase 6 (review surfaces)

- [ ] Word-of-day window has restricted capabilities
- [ ] Weekly HTML report renders user-supplied source titles via
      escaping (XSS via crafted source title)
- [ ] Tray menu doesn't accept user-supplied content as menu item
      labels (only stats)
- [ ] Notification content is safe text only (macOS handles its own
      escaping but log raw content for audit)
- [ ] Generated HTML files written under `$APPCACHE/`, not `$APPDATA/`,
      so they're cleanable

### Phase 7 (native converter)

- [ ] Same path validation as Phase 2
- [ ] Chuki parser doesn't trust unbounded nesting (limit depth)
- [ ] Gaiji table loaded from `include_str!` of vetted file
- [ ] Output XHTML escaped at every text-content boundary

### Phase 8 (web importer)

- [ ] URL validated as http/https only; no `file:` / `javascript:` /
      `data:`
- [ ] User-Agent identifies the app (don't impersonate browsers)
- [ ] Fetch timeout enforced; oversized responses (> 10MB) rejected
- [ ] `ammonia` sanitizer with explicit allowlist (don't rely on
      `clean()` defaults; configure)
- [ ] Imported EPUB embedded with strict CSP: `script-src 'none'`,
      `default-src 'none'` minimum
- [ ] Embedded images: validate Content-Type before embedding;
      reject `image/svg+xml` (scripting risk) or sanitize SVG
- [ ] Source URL stored in DB, displayed in UI for transparency
- [ ] Robots.txt is *not* checked — we're an end-user agent, not a
      crawler. But: rate-limit per host (1 req/sec) to avoid hammering
      anything

### Phase 9 (OCR overlay)

- [ ] Screen Recording permission prompt shown with clear copy in
      Info.plist
- [ ] Permission denied state handled gracefully (clipboard fallback)
- [ ] Captured image bytes never persisted; held in memory for OCR only
- [ ] OCR output text validated as UTF-8 before storage
- [ ] Overlay window: skip taskbar, prevent screen-sharing leaks
- [ ] Region selector closes on Escape; doesn't block input forever
- [ ] Overlay window capability is narrow (no fs:scope beyond library)
- [ ] Hotkey registration: deduplication if user binds the same combo
      to multiple actions

### Phase 10 (LLM)

- [ ] Network capability allowlist contains only localhost / 127.0.0.1
- [ ] User-configured base_url validated: must parse as URL, must be
      http(s), host must be `localhost` or `127.0.0.1`
- [ ] Default port allowlist documented (11434, 1234, 8080); other
      ports require explicit settings change
- [ ] LLM responses sanitized before display (HTML escape, strip
      control chars)
- [ ] LLM responses *not* logged (privacy: could contain user-OCR'd
      content)
- [ ] Audit log records timing/feature/model only, no prompts/responses
- [ ] LLM-generated content in card templates: html_escape applied by
      handlebars
- [ ] Per-feature toggles default to off; user must opt in
- [ ] Endpoint timeout enforced (model running away on a long generation
      shouldn't hang the UI)
- [ ] No outbound HTTP traffic when LLM is disabled (network capability
      allowlist still enforced; but verify by inspecting network monitor)

## Cross-cutting checks

### Supply chain

- [ ] `cargo deny check` runs in CI on every PR
- [ ] Pinned versions in workspace `[dependencies]`; review every
      `Cargo.lock` diff
- [ ] No `[patch]` overrides in production
- [ ] Vendored Java jars (Phase 2): SHA-256 checked in CI before bundle
- [ ] Vendored ttu/foliate-js (Phase 3): same; commit `dist/` rebuilds
      reproducible
- [ ] Vendored deinflect rules (Phase 5): same
- [ ] Bundled chuki gaiji table (Phase 7): same
- [ ] No npm in production. Only build-time, only for ttu fork. Lockfile
      committed if used.

### Network surface

Per-window capability allowlists:

| Window | Allowed destinations |
|---|---|
| main | `https://aozora.gr.jp`, `https://aozorahack.org`, AnkiConnect, LLM (if enabled) |
| reader-* | AnkiConnect only, LLM (if enabled & feature on) |
| word-of-day-* | AnkiConnect only |
| ocr-overlay, ocr-region-selector | AnkiConnect only, LLM (if enabled) |

- [ ] Each capability JSON manually reviewed
- [ ] No window grants `http:default` without explicit URL allowlist
- [ ] DevTools disabled in release builds

### Data at rest

- [ ] SQLite at `$APPDATA/library.sqlite`, WAL mode, foreign_keys on
- [ ] No secrets stored — we don't have any (no API keys, no passwords)
- [ ] Backups of the DB are user's responsibility; document the path
- [ ] Library files under `$APPDATA/library/` are filesystem-readable
      by user only (default umask)

### IPC / commands

- [ ] Every `#[tauri::command]` validates inputs before use
- [ ] Path inputs canonicalized + checked for traversal
- [ ] Numeric inputs (work_id, vocab_id, etc.) bounded to plausible
      ranges
- [ ] String inputs (search queries, headwords) length-bounded
- [ ] Vocab DB IDs validated to exist before use (no bare lookups by
      attacker-supplied ID)

### Content rendering

- [ ] Any user-typed content in card fields, vocab notes, source titles
      rendered with HTML escaping
- [ ] Reader windows render EPUB content; CSP enforces `script-src 'none'`
      on the document level (Tauri's init script runs before CSP applies)
- [ ] OCR text treated as untrusted input (sanitize before display)
- [ ] LLM output treated as untrusted input (sanitize before display
      and before storage)
- [ ] Web importer output sanitized via `ammonia` allowlist

### Updates / upgrades

- [ ] DB migrations idempotent; resumable if killed mid-migration
- [ ] App version recorded in DB so we know how to migrate
- [ ] Ship with a "data export" command before any destructive
      migration

## Threat model recap

Defended against:

- Supply-chain attacks via crates.io + npm — `cargo deny`, no npm in
  prod, lockfile review
- Malicious EPUBs from Aozora corpus or web — sandbox + CSP + path
  validation
- Malicious dictionaries — zip slip protection + structure validation
- Malicious LLM output — sanitization + escaping + audit log
- Malicious web pages (Phase 8) — Readability + ammonia + tight CSP +
  CSP allowlist
- Path traversal via crafted IPC inputs — input validation
- SQL injection — parameterized queries everywhere, no exceptions
- XSS in user-facing reports — HTML escape on every variable

Not defended against:

- Aozora.gr.jp itself being compromised and serving malicious content
- aozorabunko_text repo being compromised
- Crates.io publisher account compromise (cargo deny is best-effort)
- User running an attacker-supplied LLM model (the model owns the LLM
  output; we sanitize but can't claim defense)
- User installing an attacker-supplied dictionary `.zip` from a hostile
  source — dictionary content can include misleading definitions but
  can't escape the renderer if zip-slip + structure validation pass
- Attackers with code execution as the user (we can't out-defend the OS)
- Determined adversaries with physical access
- Side-channel attacks on the OS sandbox

## Pre-release checklist

Before publishing any release:

- [ ] All phase audits passing
- [ ] `cargo deny check` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] No `unwrap()` / `expect()` in command handlers (use `?`)
- [ ] No `dbg!()` / `println!()` in release code
- [ ] No `TODO` / `FIXME` in security-sensitive code paths
- [ ] Acceptance criteria for the latest phase ticked off
- [ ] JOURNAL.md updated with release notes

## When to revisit this checklist

- Adding a new Tauri command → re-audit IPC validation
- Adding a new window type → write a new capability JSON, audit
  network/fs scope
- Adding a new external content source (new importer) → audit fetch +
  parse + sanitize
- Adding a new LLM feature → audit prompt template + output handling
- Bumping a major dep version → re-run cargo deny + manual diff review
