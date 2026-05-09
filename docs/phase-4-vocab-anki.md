# Phase 4 — Vocab DB + AnkiConnect

**Goal:** local SQLite database tracking every word ever seen, where
seen, in what state. AnkiConnect client for mining cards. Handlebars
template engine for card formatting.

**Time:** one weekend.

This is the most important phase of v2. Everything from Phase 5 onward
feeds into or reads from these tables.

## Schema

Four tables. Migration file `crates/jp-vocab/src/migrations/002_vocab.sql`:

```sql
-- One row per unique word (headword + reading)
CREATE TABLE vocab (
    id              INTEGER PRIMARY KEY,
    headword        TEXT NOT NULL,
    reading         TEXT NOT NULL,
    pos             TEXT,                              -- POS from dict
    status          TEXT NOT NULL DEFAULT 'seen'
                    CHECK(status IN ('seen','tracked','learning','known','ignored')),
    first_seen_at   INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL,
    encounter_count INTEGER NOT NULL DEFAULT 0,        -- monotonic
    anki_note_id    INTEGER,                           -- nullable
    notes           TEXT,                              -- user free-text
    UNIQUE(headword, reading)
);
CREATE INDEX idx_vocab_status ON vocab(status);
CREATE INDEX idx_vocab_headword ON vocab(headword);
CREATE INDEX idx_vocab_last_seen ON vocab(last_seen_at);

-- Permanent record of (word, source) — never trimmed by retention
CREATE TABLE vocab_source (
    vocab_id        INTEGER NOT NULL REFERENCES vocab(id) ON DELETE CASCADE,
    source_type     TEXT NOT NULL,
    source_ref      TEXT NOT NULL,
    first_seen_at   INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL,
    count_in_source INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (vocab_id, source_type, source_ref)
);
CREATE INDEX idx_vocab_source_word ON vocab_source(vocab_id);
CREATE INDEX idx_vocab_source_src ON vocab_source(source_type, source_ref);

-- Sentence-level encounters; subject to retention policy
CREATE TABLE encounter (
    id              INTEGER PRIMARY KEY,
    vocab_id        INTEGER NOT NULL REFERENCES vocab(id) ON DELETE CASCADE,
    surface         TEXT NOT NULL,
    sentence        TEXT NOT NULL,
    source_type     TEXT NOT NULL,
    source_ref      TEXT NOT NULL,
    occurred_at     INTEGER NOT NULL
);
CREATE INDEX idx_enc_vocab ON encounter(vocab_id);
CREATE INDEX idx_enc_source ON encounter(source_type, source_ref);
CREATE INDEX idx_enc_time ON encounter(occurred_at);

-- Source metadata: titles, token counts, opened-at timestamps
CREATE TABLE source (
    type            TEXT NOT NULL,
    ref             TEXT NOT NULL,
    title           TEXT NOT NULL,
    author          TEXT,
    total_tokens    INTEGER,
    unique_tokens   INTEGER,
    first_opened_at INTEGER,
    last_opened_at  INTEGER,
    PRIMARY KEY (type, ref)
);

-- Optional FTS over sentences for context queries
CREATE VIRTUAL TABLE encounter_fts USING fts5(
    sentence,
    content='encounter',
    content_rowid='id',
    tokenize="trigram"
);

-- Keep FTS in sync with encounter table
CREATE TRIGGER encounter_ai AFTER INSERT ON encounter BEGIN
    INSERT INTO encounter_fts(rowid, sentence) VALUES (new.id, new.sentence);
END;
CREATE TRIGGER encounter_ad AFTER DELETE ON encounter BEGIN
    INSERT INTO encounter_fts(encounter_fts, rowid, sentence)
        VALUES ('delete', old.id, old.sentence);
END;
```

See appendix-f-vocab-db.md for the schema rationale and retention
algorithm details.

## Status state machine

```
       ┌────────────────────────────────────────────┐
       │                                            │
       │     (auto on first lookup)                 │
       ▼                                            │
   ┌───────┐  user: track   ┌─────────┐            │
   │ seen  │───────────────▶│ tracked │────────┐   │
   └───┬───┘                └─────────┘        │   │
       │                                       │   │
       │ user: ignore         user: mine to    │   │
       │                      Anki             │   │
       │                                       ▼   │
       │                                   ┌─────────┐
       └─────────────────▶┌───────────┐    │learning │
                          │ ignored   │    └────┬────┘
                          └───────────┘         │
                                                │ user: mark known
                                                │ (or Anki marks done)
                                                ▼
                                            ┌───────┐
                                            │ known │
                                            └───────┘
```

- `seen` → automatic on first encounter. Doesn't surface in reviews.
- `tracked` → user marked "I want to remember this exists." Reviews surface it.
- `learning` → has an Anki card. Anki handles SRS; we record the link.
- `known` → user-marked mastered. Filtered from reviews.
- `ignored` → don't surface; useful for proper nouns, names.

Transitions are user-initiated except for `seen` (automatic).

## Encounter recording API

```rust
// crates/jp-vocab/src/lib.rs

pub struct VocabDb { /* wraps Db */ }

#[derive(Debug, Clone)]
pub struct EncounterInput {
    pub headword: String,
    pub reading: String,
    pub pos: Option<String>,
    pub surface: String,
    pub sentence: String,
    pub source_type: String,    // "aozora" | "ocr" | "web" | "manual"
    pub source_ref: String,
    pub occurred_at: i64,       // unix seconds
}

impl VocabDb {
    /// Record a single encounter. Creates vocab/source rows as needed.
    /// Updates encounter_count + last_seen on vocab.
    /// Updates count_in_source + last_seen on vocab_source.
    /// Inserts a new encounter row (subject to later trimming).
    pub fn record_encounter(&mut self, e: &EncounterInput) -> Result<i64> {
        let tx = self.conn.transaction()?;

        // Upsert vocab row
        let vocab_id: i64 = tx.query_row(
            "INSERT INTO vocab
                (headword, reading, pos, first_seen_at, last_seen_at, encounter_count)
             VALUES (?1, ?2, ?3, ?4, ?4, 1)
             ON CONFLICT (headword, reading) DO UPDATE SET
                last_seen_at = excluded.last_seen_at,
                encounter_count = encounter_count + 1,
                pos = COALESCE(vocab.pos, excluded.pos)
             RETURNING id",
            params![e.headword, e.reading, e.pos, e.occurred_at],
            |row| row.get(0),
        )?;

        // Upsert vocab_source row — never trimmed
        tx.execute(
            "INSERT INTO vocab_source
                (vocab_id, source_type, source_ref, first_seen_at, last_seen_at, count_in_source)
             VALUES (?1, ?2, ?3, ?4, ?4, 1)
             ON CONFLICT (vocab_id, source_type, source_ref) DO UPDATE SET
                last_seen_at = excluded.last_seen_at,
                count_in_source = count_in_source + 1",
            params![vocab_id, e.source_type, e.source_ref, e.occurred_at],
        )?;

        // Insert encounter row
        let enc_id: i64 = tx.query_row(
            "INSERT INTO encounter
                (vocab_id, surface, sentence, source_type, source_ref, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             RETURNING id",
            params![
                vocab_id, e.surface, e.sentence,
                e.source_type, e.source_ref, e.occurred_at,
            ],
            |row| row.get(0),
        )?;

        // Bump source.last_opened_at
        tx.execute(
            "INSERT INTO source (type, ref, title, last_opened_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (type, ref) DO UPDATE SET
                last_opened_at = excluded.last_opened_at",
            params![e.source_type, e.source_ref, /* title from caller */ "", e.occurred_at],
        )?;

        tx.commit()?;
        Ok(vocab_id)
    }

    /// Status transitions
    pub fn set_status(&self, vocab_id: i64, status: Status) -> Result<()> { /* ... */ }
    pub fn link_anki_note(&self, vocab_id: i64, note_id: i64) -> Result<()> { /* ... */ }

    /// Stats queries
    pub fn count_words_seen(&self, exclude_ignored: bool) -> Result<i64> { /* ... */ }
    pub fn words_for_source(&self, source_type: &str, source_ref: &str)
        -> Result<Vec<VocabRow>> { /* ... */ }
    pub fn known_pct_for_source(&self, source_type: &str, source_ref: &str)
        -> Result<f64> { /* ... */ }
}
```

## Retention algorithm

Run as a periodic background task, not a trigger. See
appendix-f-vocab-db.md for full rationale.

```rust
// crates/jp-vocab/src/retention.rs

const DEFAULT_KEEP_PER_WORD: i64 = 20;

pub fn trim_encounters(db: &mut VocabDb, keep_per_word: i64) -> Result<usize> {
    let conn = &db.conn;

    // For each vocab row with > keep_per_word encounters, delete the
    // ones that don't survive the diversity-aware policy:
    //
    // Survivors = (first ever) ∪ (most recent per source)
    //           ∪ (top N most recent overall)
    //
    // Implemented as one DELETE-WHERE-NOT-IN per word with > keep encounters.

    let candidates: Vec<i64> = conn.prepare(
        "SELECT vocab_id FROM encounter
         GROUP BY vocab_id
         HAVING COUNT(*) > ?1"
    )?.query_map([keep_per_word], |r| r.get(0))?
       .filter_map(|r| r.ok()).collect();

    let mut total_deleted = 0;
    for vocab_id in candidates {
        let deleted = conn.execute(
            "DELETE FROM encounter
             WHERE vocab_id = ?1 AND id NOT IN (
                -- absolute first
                SELECT id FROM encounter WHERE vocab_id = ?1
                ORDER BY occurred_at ASC LIMIT 1
                UNION
                -- most recent per (source_type, source_ref)
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (
                        PARTITION BY source_type, source_ref
                        ORDER BY occurred_at DESC
                    ) AS rn
                    FROM encounter WHERE vocab_id = ?1
                ) WHERE rn = 1
                UNION
                -- N most recent overall
                SELECT id FROM encounter WHERE vocab_id = ?1
                ORDER BY occurred_at DESC LIMIT ?2
             )",
            params![vocab_id, keep_per_word],
        )?;
        total_deleted += deleted;
    }

    Ok(total_deleted)
}
```

Schedule:

```rust
// In src-tauri/src/lib.rs setup hook
let db_handle = state.vocab.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        let _ = tokio::task::spawn_blocking(move || {
            let mut db = db_handle.lock().unwrap();
            retention::trim_encounters(&mut db, 20)
        }).await;
    }
});
```

Plus run at startup and explicitly when the user opens a vocabulary
review view (since they're about to look at the data).

## AnkiConnect client

Carry over from v1 phase 4: HTTP client to `localhost:8765`, JSON-RPC
wrapped in `request(action, params)`. Lives in `crates/jp-anki/`.

```
crates/jp-anki/src/
├─ lib.rs
├─ client.rs       # AnkiClient, AnkiError
├─ note.rs         # NewNote, AudioAttachment
└─ template.rs     # handlebars-based card formatting (NEW)
```

## Handlebars template engine

This is what's new in v2 vs v1's Anki integration. Yomitan uses
handlebars templates to format card fields, with helpers like
`{{kanji-stats}}`, `{{glossary-no-dictionary}}`, `{{cloze-prefix}}`.
Steal the model.

```rust
// crates/jp-anki/src/template.rs

use handlebars::{Handlebars, Helper, RenderContext, RenderError, Output, Context, ScopedJson};
use serde::Serialize;

pub struct CardFormatter {
    hb: Handlebars<'static>,
}

#[derive(Serialize)]
pub struct CardContext {
    pub expression: String,         // headword (kanji form if avail.)
    pub reading: String,            // kana
    pub glossary: Vec<String>,      // dict definitions
    pub sentence: String,
    pub source_title: String,
    pub source_url: Option<String>,
    pub frequency: Option<u32>,     // from frequency dict
    pub pitch_accent: Option<String>,
    pub audio_url: Option<String>,
    pub timestamp: i64,
}

impl CardFormatter {
    pub fn new() -> Self {
        let mut hb = Handlebars::new();
        hb.set_strict_mode(false);
        hb.register_helper("furigana", Box::new(furigana_helper));
        hb.register_helper("cloze", Box::new(cloze_helper));
        hb.register_helper("first", Box::new(first_helper));
        Self { hb }
    }

    pub fn render(&self, template: &str, ctx: &CardContext) -> Result<String, RenderError> {
        self.hb.render_template(template, ctx)
    }

    /// Render a full field map for an Anki note
    pub fn render_fields(
        &self,
        field_templates: &[(String, String)],   // (anki_field_name, template_string)
        ctx: &CardContext,
    ) -> Result<HashMap<String, String>, RenderError> {
        field_templates.iter()
            .map(|(name, tmpl)| Ok((name.clone(), self.hb.render_template(tmpl, ctx)?)))
            .collect()
    }
}

// Helper: wrap kanji with reading as <ruby>kanji<rt>reading</rt></ruby>
fn furigana_helper(h: &Helper, _: &Handlebars, _: &Context, _: &mut RenderContext, out: &mut dyn Output) -> Result<(), RenderError> {
    let expr = h.param(0).map(|v| v.value().as_str().unwrap_or("")).unwrap_or("");
    let read = h.param(1).map(|v| v.value().as_str().unwrap_or("")).unwrap_or("");
    out.write(&format!("<ruby>{}<rt>{}</rt></ruby>", html_escape(expr), html_escape(read)))?;
    Ok(())
}

// Helper: cloze around a substring
fn cloze_helper(...) { /* ... */ }

// Helper: first item from a list
fn first_helper(...) { /* ... */ }
```

A user template for the "Sentence" field might look like:

```
{{sentence}}<br><small>— {{source_title}}</small>
```

Or for "Word":

```
{{furigana expression reading}}
```

Or for "Definition":

```
<ol>{{#each glossary}}<li>{{this}}</li>{{/each}}</ol>
```

Settings UI lets the user paste templates for each Anki field. Defaults
are sensible Yomitan-style.

## Settings persistence

```rust
// stored in `settings` table as JSON
pub struct AnkiSettings {
    pub deck: String,
    pub model: String,
    pub field_templates: Vec<(String, String)>,    // (field_name, handlebars_template)
    pub default_tags: Vec<String>,
}
```

## Tauri commands

```rust
// Vocab DB
#[tauri::command] fn vocab_set_status(vocab_id: i64, status: String) -> Result<(), String>;
#[tauri::command] fn vocab_get(vocab_id: i64) -> Result<VocabRow, String>;
#[tauri::command] fn vocab_search(query: String, limit: usize) -> Vec<VocabRow>;
#[tauri::command] fn vocab_stats() -> Result<VocabStats, String>;
#[tauri::command] fn vocab_recent_encounters(vocab_id: i64) -> Vec<Encounter>;
#[tauri::command] fn vocab_set_notes(vocab_id: i64, notes: String) -> Result<(), String>;

// Source stats
#[tauri::command] fn source_known_pct(source_type: String, source_ref: String) -> Result<f64, String>;
#[tauri::command] fn source_lookup_density(source_type: String, source_ref: String) -> Result<f64, String>;

// Anki
#[tauri::command] async fn anki_status() -> AnkiStatus;
#[tauri::command] async fn anki_decks() -> Result<Vec<String>, String>;
#[tauri::command] async fn anki_models() -> Result<Vec<String>, String>;
#[tauri::command] async fn anki_model_fields(model: String) -> Result<Vec<String>, String>;
#[tauri::command] async fn anki_mine(vocab_id: i64) -> Result<i64, String>;  // returns note_id
#[tauri::command] fn get_anki_settings() -> Result<AnkiSettings, String>;
#[tauri::command] fn save_anki_settings(settings: AnkiSettings) -> Result<(), String>;
```

The `anki_mine` command is the high-level operation: takes a vocab_id,
loads the row + encounters, builds a `CardContext`, renders all field
templates, sends to AnkiConnect, and updates the vocab row's status to
`learning` and `anki_note_id` on success.

## Acceptance criteria

- [ ] All four tables exist; FTS index on `encounter_fts`
- [ ] `record_encounter` inserts/updates correctly across vocab,
      vocab_source, encounter, source
- [ ] Inserting 1000 encounters of the same word leaves
      `vocab.encounter_count = 1000` after retention but only ~20 rows
      in `encounter`
- [ ] After retention, `vocab_source` still has all distinct
      (source_type, source_ref) pairs
- [ ] FTS query "find encounters containing 食べる" returns hits
- [ ] Status transitions follow the state machine; invalid transitions
      (e.g., `seen → known` directly) are blocked
- [ ] AnkiConnect status reflects whether Anki is running
- [ ] Mining a word renders all field templates correctly and creates a
      note in Anki
- [ ] Anki note_id is recorded back into vocab.anki_note_id
- [ ] Settings panel persists Anki deck/model/field templates across
      restarts
- [ ] Vocab stats panel shows: total seen, total tracked, total
      learning, total known, and per-source breakdown

## Common Phase 4 problems

**Retention deletes too aggressively:** verify the diversity policy by
inserting controlled test data. Test case: 30 encounters across 5
sources, run retention with keep=10. Expected: 1 (first ever) + 5 (one
per source) + up-to-10 (recent overall, deduped against the others).

**FTS index out of sync:** if you `DELETE FROM encounter` directly in
SQL without going through the trigger path, FTS leaks. Always go through
the API. There's a `INSERT INTO encounter_fts(...) VALUES('rebuild')`
escape hatch if it does drift.

**`record_encounter` is slow on first run:** without WAL mode or
transactions, hundreds of inserts take seconds. Verify
`PRAGMA journal_mode=WAL` and that all multi-statement operations are
wrapped in `tx.commit()`.

**Handlebars template injection:** if a card field template contains
literal handlebars syntax that the user actually wants, escape with `\{{ \}}`.
Document this in the settings UI help text.

## What's next

Phase 5: Yomitan-format dictionary engine. Lookups will populate the
vocab tables on every popup hover. The 4-button popup (track / mine /
ignore / known) wires UI to the status transitions defined here.
