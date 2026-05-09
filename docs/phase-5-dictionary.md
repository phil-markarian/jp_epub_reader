# Phase 5 — Dictionary Engine + Lookup Popup

**Goal:** Yomitan-format dictionary engine that imports any compatible
`.zip`. Hover/click in reader → popup → lookup → 4-button mining.
Every lookup writes to the vocab DB.

**Time:** 2-3 weekends.

## Big picture

In v1 this phase was JMdict-only. In v2 it's the full Yomitan-format
ecosystem: Jitendex, Meikyou, Shinmeikai, frequency lists, pitch accent
dictionaries, monolingual dictionaries — anything in Yomitan's `.zip`
format. Users drop in their existing Yomitan dictionary collection and
it just works.

Four substantial pieces, build in order:

1. **Dictionary import** — parse Yomitan zip → SQLite
2. **Tokenizer** — find word boundaries
3. **Deinflection** — port Yomitan's rules
4. **Popup** — UI on top of all the above, wired to the vocab DB

## 1. Yomitan dictionary format

See appendix-e-yomitan-format.md for full details. Brief version:

A Yomitan `.zip` contains:

```
index.json                           # name, format version, revision
term_bank_1.json                     # entries — paginated
term_bank_2.json
...
term_meta_bank_1.json                # frequency, pitch accent
kanji_bank_1.json                    # kanji info (some dicts)
kanji_meta_bank_1.json               # kanji frequency
tag_bank_1.json                      # tag definitions
```

Term bank entries are arrays:

```json
[
    "食べる",                          // expression
    "たべる",                          // reading
    "v1 vt",                          // POS / tags
    "",                                // rules (deinflection chain)
    100,                               // popularity score
    [
        "to eat",
        "to live on (e.g. one's salary)",
        "to take into one's mouth"
    ],
    1234567,                           // sequence number
    "common"                           // term tags
]
```

Definitions can be plain strings or "structured content" (a tree of
typed objects representing rich HTML — links, lists, images).

## Dictionary import schema

```sql
-- Phase 5 migration: 003_dict.sql
CREATE TABLE dictionary (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    revision TEXT,
    format_version INTEGER NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    imported_at INTEGER NOT NULL
);

CREATE TABLE term (
    id INTEGER PRIMARY KEY,
    dict_id INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    expression TEXT NOT NULL,
    reading TEXT NOT NULL,
    pos TEXT,                       -- space-separated tags
    rules TEXT,                     -- deinflection rule classes
    score INTEGER NOT NULL DEFAULT 0,
    sequence INTEGER,
    term_tags TEXT,
    glossary TEXT NOT NULL          -- JSON array of strings or structured content
);
CREATE INDEX idx_term_expr ON term(expression);
CREATE INDEX idx_term_read ON term(reading);
CREATE INDEX idx_term_dict ON term(dict_id);

CREATE TABLE term_meta (
    dict_id INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    expression TEXT NOT NULL,
    mode TEXT NOT NULL,             -- 'freq' | 'pitch' | 'ipa'
    data TEXT NOT NULL              -- JSON
);
CREATE INDEX idx_term_meta_expr ON term_meta(expression, mode);

CREATE TABLE kanji (
    id INTEGER PRIMARY KEY,
    dict_id INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    character TEXT NOT NULL,
    onyomi TEXT,
    kunyomi TEXT,
    tags TEXT,
    meanings TEXT NOT NULL,         -- JSON array
    stats TEXT                      -- JSON object
);
CREATE INDEX idx_kanji_char ON kanji(dict_id, character);

CREATE TABLE tag (
    dict_id INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    category TEXT,
    description TEXT,
    score INTEGER,
    PRIMARY KEY (dict_id, name)
);
```

## Importer

```rust
// crates/jp-dict/src/import.rs

pub fn import_zip(db: &mut Db, zip_path: &Path) -> Result<DictId> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // 1. Read index.json
    let index_str = read_to_string(&mut archive, "index.json")?;
    let index: DictIndex = serde_json::from_str(&index_str)?;

    // 2. Insert dictionary row
    let dict_id = db.insert_dictionary(&index)?;

    // 3. Iterate every file in the zip:
    //    - term_bank_*.json → into `term`
    //    - term_meta_bank_*.json → into `term_meta`
    //    - kanji_bank_*.json → into `kanji`
    //    - kanji_meta_bank_*.json → into `term_meta` (mode='kanji_freq')
    //    - tag_bank_*.json → into `tag`
    let tx = db.conn.transaction()?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if name.starts_with("term_bank_") && name.ends_with(".json") {
            import_term_bank(&tx, dict_id, &mut entry)?;
        } else if name.starts_with("term_meta_bank_") {
            import_term_meta_bank(&tx, dict_id, &mut entry)?;
        } else if name.starts_with("kanji_bank_") {
            import_kanji_bank(&tx, dict_id, &mut entry)?;
        } else if name.starts_with("tag_bank_") {
            import_tag_bank(&tx, dict_id, &mut entry)?;
        }
    }
    tx.commit()?;
    Ok(dict_id)
}

fn import_term_bank(tx: &Transaction, dict_id: i64, reader: &mut impl Read) -> Result<()> {
    // Yomitan's term banks are arrays of arrays. Stream-parse if possible
    // for memory; small dicts can deserialize as Vec<TermEntry>.
    let entries: Vec<serde_json::Value> = serde_json::from_reader(reader)?;
    let mut stmt = tx.prepare(
        "INSERT INTO term (dict_id, expression, reading, pos, rules, score, glossary, sequence, term_tags)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    )?;
    for arr in entries {
        let arr = arr.as_array().ok_or_else(|| anyhow!("expected array"))?;
        // Index into the fixed positions; see Yomitan docs
        let expression = arr[0].as_str().unwrap_or("").to_string();
        let reading = arr[1].as_str().unwrap_or("").to_string();
        let pos = arr[2].as_str().unwrap_or("").to_string();
        let rules = arr[3].as_str().unwrap_or("").to_string();
        let score = arr[4].as_i64().unwrap_or(0);
        // arr[5] is the glossary (most varied: array of strings or structured)
        let glossary = serde_json::to_string(&arr[5])?;
        let sequence = arr.get(6).and_then(|v| v.as_i64());
        let term_tags = arr.get(7).and_then(|v| v.as_str()).unwrap_or("").to_string();

        stmt.execute(params![dict_id, expression, reading, pos, rules, score, glossary, sequence, term_tags])?;
    }
    Ok(())
}
```

## 2. Tokenizer

Same options as v1 phase 5: vibrato (recommended) or longest-match.

`crates/jp-tokenizer/`:

```rust
pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}

pub struct Token {
    pub surface: String,
    pub feature: String,
    pub start: usize,
    pub end: usize,
    pub base_form: Option<String>,    // mecab gives this; longest-match doesn't
}

pub struct VibratoTokenizer { /* ... */ }
pub struct LongestMatchTokenizer { /* ... */ }
```

For Phase 5, ship longest-match by default (no model bundle). Add
vibrato as an opt-in upgrade if accuracy isn't enough.

## 3. Deinflection

Port Yomitan's `deinflect.json`. ~300 lines of rules-as-data.

```rust
// crates/jp-dict/src/deinflect.rs

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Rule {
    #[serde(rename = "kanaIn")]    pub from: String,
    #[serde(rename = "kanaOut")]   pub to: String,
    #[serde(rename = "rulesIn")]   pub from_rules: Vec<String>,
    #[serde(rename = "rulesOut")]  pub to_rules: Vec<String>,
}

pub struct Deinflector {
    rules: Vec<(String, Rule)>,
}

impl Deinflector {
    pub fn from_json(json: &str) -> Result<Self> { /* parse map */ }

    pub fn deinflect(&self, word: &str) -> Vec<Candidate> {
        // BFS up to depth 5; each step tries every rule's `from` ending
        // and recurses with the substituted form. Track applied rules.
        // Filter at lookup time by matching candidate's `to_rules` against
        // the dict entry's POS rules.
    }
}

pub struct Candidate {
    pub form: String,
    pub allowed_rules: Vec<String>,    // POS classes valid for this form
    pub applied_chain: Vec<String>,    // for display: "polite past → past → plain"
}
```

Embed `deinflect.json` via `include_str!` so it ships with the binary
(~25KB). Ports of Yomitan's deinflection have been done multiple times
(rikaikun, jisho, etc.); reference them if you get stuck.

## 4. Lookup engine

```rust
// crates/jp-dict/src/lookup.rs

pub struct Dict {
    db: Arc<Mutex<Db>>,
    deinflector: Deinflector,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LookupResult {
    pub query: String,            // dictionary form found
    pub original: String,         // what was actually clicked
    pub deinflection: Vec<String>,// chain of inflections applied
    pub entries: Vec<DictEntry>,  // grouped by (expression, reading)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DictEntry {
    pub expression: String,
    pub reading: String,
    pub senses: Vec<Sense>,       // each sense from each enabled dict
    pub frequency: Option<u32>,
    pub pitch_accent: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Sense {
    pub dict_name: String,
    pub pos: Vec<String>,
    pub gloss: serde_json::Value,  // structured content or strings
    pub tags: Vec<String>,
}

impl Dict {
    pub fn lookup(&self, text: &str, offset: usize) -> Result<Option<LookupResult>> {
        // Try progressively shorter substrings starting at offset.
        // For each, deinflect and look up across all enabled dicts.
        let chars: Vec<(usize, char)> = text[offset..].char_indices().collect();
        for end_idx in (1..=chars.len().min(16)).rev() {
            let end_byte = /* end_idx → byte offset */;
            let substring = &text[offset..end_byte];

            for cand in self.deinflector.deinflect(substring) {
                let entries = self.find(&cand.form, &cand.allowed_rules)?;
                if !entries.is_empty() {
                    return Ok(Some(LookupResult {
                        query: cand.form,
                        original: substring.to_string(),
                        deinflection: cand.applied_chain,
                        entries,
                    }));
                }
            }
        }
        Ok(None)
    }

    fn find(&self, form: &str, allowed_rules: &[String]) -> Result<Vec<DictEntry>> {
        // Query `term` across all enabled dictionaries:
        //   SELECT ... FROM term JOIN dictionary
        //   WHERE term.expression = ?1 AND dictionary.enabled = 1
        //   ORDER BY dictionary.priority DESC
        //
        // Group results by (expression, reading) to merge senses across
        // dicts. Filter senses whose POS doesn't intersect allowed_rules
        // (deinflection accuracy filter, like Yomitan).
        //
        // Augment with frequency + pitch accent from term_meta.
    }
}
```

## Lookup popup UI

The popup is a Leptos component rendered as an absolutely-positioned div
in the reader window. It receives a `LookupResult` and shows:

- Header: `query` form (with reading), deinflection chain if any
- Frequency rank (small badge if available)
- Pitch accent (small mark)
- Per-dict sections: each dictionary's senses, with POS tags and gloss
- 4 action buttons at the bottom

```rust
#[component]
fn LookupPopup(result: ReadSignal<Option<LookupResult>>) -> impl IntoView {
    view! {
        {move || result.get().map(|r| view! {
            <div class="lookup-popup">
                <div class="header">
                    <ruby>
                        {r.entries[0].expression.clone()}
                        <rt>{r.entries[0].reading.clone()}</rt>
                    </ruby>
                    {if !r.deinflection.is_empty() {
                        view! { <span class="deinfl">"← " {r.deinflection.join(" ← ")}</span> }.into_any()
                    } else { ().into_any() }}
                </div>

                <For each={move || r.entries.clone()} key=|e| e.expression.clone() let:entry>
                    <DictEntryView entry={entry} />
                </For>

                <div class="actions">
                    <button on:click=move |_| set_status(r.entries[0].clone(), "tracked")>"Track"</button>
                    <button on:click=move |_| mine_to_anki(r.entries[0].clone())>"Mine"</button>
                    <button on:click=move |_| set_status(r.entries[0].clone(), "known")>"Known"</button>
                    <button on:click=move |_| set_status(r.entries[0].clone(), "ignored")>"Ignore"</button>
                </div>
            </div>
        })}
    }
}
```

## Wiring lookup → vocab DB

The reader's injected JS captures click events and calls a `lookup`
command. That command:

1. Calls `Dict::lookup`
2. **Records an encounter automatically** (status defaults to `seen`)
3. Returns the lookup result for popup rendering

```rust
#[tauri::command]
pub async fn lookup(
    text: String,
    offset: usize,
    source_type: String,
    source_ref: String,
    sentence: String,
    state: State<'_, AppState>,
) -> Result<Option<LookupResult>, String> {
    let dict = state.dict.lock().unwrap();
    let result = dict.lookup(&text, offset).map_err(|e| e.to_string())?;

    // Record encounter automatically — every lookup writes to vocab
    if let Some(ref r) = result {
        let entry = &r.entries[0];
        let now = chrono::Utc::now().timestamp();
        let mut vocab = state.vocab.lock().unwrap();
        vocab.record_encounter(&EncounterInput {
            headword: entry.expression.clone(),
            reading: entry.reading.clone(),
            pos: entry.senses.first().map(|s| s.pos.join(" ")),
            surface: r.original.clone(),
            sentence,
            source_type,
            source_ref,
            occurred_at: now,
        }).map_err(|e| e.to_string())?;
    }

    Ok(result)
}
```

So: every popup that appears = one encounter recorded. Status defaults
to `seen`. The 4 buttons promote to `tracked` / `learning` / `known` /
`ignored`.

## Settings UI

Dictionary management panel:

- List of imported dictionaries (name, revision, term count, enabled
  toggle)
- "Add dictionary" button → file picker for `.zip` → import
- Drag-to-reorder priority (affects display order in popup)
- "Remove" button per dict (with confirmation)
- Total disk usage indicator

## Acceptance criteria

- [ ] Yomitan-format `.zip` imports without errors (test: JMdict, an
      EPWING-converted dict, Jitendex, a frequency list)
- [ ] Lookup of `食べる` returns the JMdict entry
- [ ] Lookup of `食べました` deinflects → returns the same entry, with
      deinflection chain shown
- [ ] Lookup of `走らなければならなかった` deinflects to `走る`
- [ ] Substring fallback: clicking inside `日本語` finds `日本語` first,
      then `日本`, then `日`
- [ ] Frequency badges appear when a frequency dict is enabled
- [ ] Pitch accent marks appear when a pitch dict is enabled
- [ ] Multi-dict display: lookup with multiple enabled dicts shows
      senses from each, in priority order
- [ ] Every popup creates exactly one encounter row
- [ ] 4 buttons correctly transition status
- [ ] "Mine" button sends a card to Anki and links the note_id back
- [ ] Lookup latency < 50ms p95 on a 200k-entry dictionary

## Common Phase 5 problems

**Slow imports for large dictionaries:** wrap in transaction (we do).
Also batch INSERTs — pre-build the `?,?,?` placeholders for 1000 rows
and execute as one statement. Drop indexes during import, recreate at
end.

**Yomitan structured content rendering:** definitions can be nested
HTML-like trees. For Phase 5, render the simple cases (paragraphs,
lists, links). Punt on the exotic ones (collapsible regions, tables) —
fall back to JSON-toString of the structured node.

**Deinflection picks wrong form:** when multiple candidates match,
filter by POS rules (the `rulesOut` field in deinflect rules vs the dict
entry's `rules` field). Sort surviving candidates by depth (fewer
applications wins).

**Lookup popup positioning:** in vertical text, the popup's natural
position (below the click) can fall off-screen. Implement boundary
detection: if `bottom > window.innerHeight - 100`, position above the
click instead.

**FTS sentence search returns junk for short Japanese:** trigram FTS
requires ≥3 characters per query token. For 1-2 char queries, fall back
to LIKE.

## What's next

Phase 6: review surfaces. Now that we have months of vocab data feeding
in, build the menu bar, daily notification, word-of-day, and weekly HTML
report.
