# Phase 3 — Library + Reader

**Goal:** browse imported works in a library view; open a selected work
in either an embedded reader (forked ttu / foliate-js) or external
handoff (Apple Books).

**Time:** one weekend.

This phase is largely v1 phase 3 with the database schema extended for
Phase 4's vocab system. See appendix-b-reader-options.md for the choice
between ttu, foliate-js, and rolling your own.

## Library schema

The v1 phase 3 doc had a single `library` table. Phase 4 will add three
more tables for vocabulary (`vocab`, `vocab_source`, `encounter`) and
two for sources (`source`, `settings`). Define the schema such that
Phase 3 only fills in what it needs, but the migration path to Phase 4
is clean.

```sql
-- Phase 3 schema (only the library + settings tables active)
CREATE TABLE IF NOT EXISTS library (
    work_id INTEGER PRIMARY KEY,
    source_id TEXT NOT NULL UNIQUE,    -- "aozora:773", "web:hash", etc.
    title TEXT NOT NULL,
    author TEXT,
    epub_path TEXT NOT NULL,
    raw_text_path TEXT,                -- if available (Aozora has this)
    added_at INTEGER NOT NULL,
    last_opened_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_library_added ON library(added_at DESC);
CREATE INDEX IF NOT EXISTS idx_library_opened ON library(last_opened_at DESC);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

The `source_id` field is the bridge to Phase 4's source tracking — same
identifier used across `library` and `source` and `vocab_source`.

## Db abstraction

The vocab work in Phase 4 will need a structured DB module, not
ad-hoc queries scattered across commands. Set up the abstraction now:

```rust
// crates/jp-vocab/src/db.rs (created in Phase 3, expanded in Phase 4)

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("library.sqlite");
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        // Run all migrations in order; track current version in user_version
        let current: i32 = self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (target_version, sql) in MIGRATIONS.iter().enumerate() {
            let target = (target_version + 1) as i32;
            if current < target {
                self.conn.execute_batch(sql)?;
                self.conn.pragma_update(None, "user_version", target)?;
            }
        }
        Ok(())
    }

    // library + settings methods (Phase 3)
    // vocab + encounter + source methods (Phase 4)
}

const MIGRATIONS: &[&str] = &[
    // v1: library + settings
    include_str!("migrations/001_initial.sql"),
    // v2: vocab tables (added in Phase 4)
    // include_str!("migrations/002_vocab.sql"),
];
```

The migration system means Phase 4 just appends a new SQL file; nothing
rewires.

## Reader integration paths

V1 phase 3 had two paths (3a external, 3b embedded ttu). Phase 6 of v2
needs the reader to capture word-level click events to feed encounters
into the vocab DB. This narrows the embedded-reader options:

| Reader | Captures clicks? | Phase 6 fit |
|---|---|---|
| Apple Books (external) | No | Doesn't work — encounters can't flow back |
| ttu in browser (external + Yomitan) | No (in your app) | Doesn't work for vocab tracking |
| **ttu embedded (Tauri webview)** | Yes via injected JS | Works |
| **foliate-js embedded** | Yes via custom integration | Works |
| **Rolling your own** (Leptos + EPUB.js) | Yes natively | Works, more effort |

For Phase 6 onward to track encounters automatically, you need an
embedded reader. The two viable choices:

1. **ttu embedded with `loadBookFromBytes` patch** — fastest to v1 (see
   v1 appendix-b). AGPL-3.0. Yomitan-extension features lost (we have
   our own dictionary in Phase 5).
2. **foliate-js embedded** — BSD-3 license (compatible with everything),
   smaller scope (you build the reader chrome in Leptos). More work but
   cleaner license.

Either works. If you're going GPL across the board (consistent with
yomitan + AozoraEpub3), ttu is fine. If you might want closed-source
distribution someday, foliate-js.

You can also offer external handoff for users who want their existing
Yomitan workflow — but those reads won't flow into your vocab DB. UI
should make this tradeoff visible.

## Reader window — capture clicks for Phase 5+

Each reader window opens with an injected JS bridge that captures
clicks/hovers and forwards to the dictionary lookup. Sketched:

```javascript
// Injected at reader window creation time via initialization_script
(function() {
    document.addEventListener('mouseup', async (e) => {
        if (!e.shiftKey) return;
        const sel = window.getSelection();
        if (!sel || sel.isCollapsed) return;

        const range = sel.getRangeAt(0);
        const node = range.startContainer;
        const text = node.textContent || '';
        const offset = range.startOffset;

        // Phase 5 will replace this with full lookup-popup behavior
        const result = await window.__TAURI__.core.invoke('lookup', {
            text, offset,
            sourceType: 'aozora',
            sourceRef: window.__JP_SOURCE_REF__,    // injected per-window
            sentence: extractSentence(node, offset),
        });
        if (result) showPopup(result);
    });

    function extractSentence(node, offset) {
        const text = node.textContent || '';
        // Find sentence boundaries (。 ！ ？ + Japanese punct)
        const sentEnd = text.indexOf('。', offset);
        const sentStart = Math.max(
            text.lastIndexOf('。', offset),
            text.lastIndexOf('！', offset),
            text.lastIndexOf('？', offset),
            -1,
        ) + 1;
        return text.substring(sentStart, sentEnd === -1 ? text.length : sentEnd + 1);
    }
})();
```

The `lookup` command is stubbed in Phase 3 (returns null) and gets its
real implementation in Phase 5.

The `sentence` field is what gets stored in the encounter table when the
user clicks "track" or "mine" on the popup. Capturing it at click time
means we always have context, even if the user later loses their place.

## Window labels and capabilities

Reader windows use label pattern `reader-{work_id}`:

```rust
WebviewWindowBuilder::new(
    &app,
    format!("reader-{}", work_id),
    /* ... */
)
```

Capability `src-tauri/capabilities/reader.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "reader",
  "description": "Reader window — restricted permissions",
  "windows": ["reader-*"],
  "permissions": [
    "core:default",
    {
      "identifier": "fs:scope",
      "allow": [{ "path": "$APPDATA/library/**" }]
    },
    {
      "identifier": "http:default",
      "allow": [{ "url": "http://127.0.0.1:8765/*" }]
    }
  ]
}
```

Reader windows can read EPUBs from app data dir + talk to AnkiConnect
(once Phase 4 lands). Cannot trigger downloads, modify settings, or
reach the network beyond AnkiConnect.

## Library UI

Two-tab layout (Search / Library) using Leptos signals.

Library row actions:
- **Open** (in embedded reader)
- **Open externally** (Apple Books / system default)
- **Reconvert with...** (per-strategy override from Phase 2)
- **Remove** (with confirmation; deletes local files + DB row)

Phase 4 will add:
- **Stats for this work** (% words known, lookups in last week)
- **Open in review mode** (shows only words you've encountered/tracked)

Layout is signal-driven; adding stats columns later is a one-line change
to the result row component.

## Acceptance criteria

- [ ] Library survives restart (SQLite persistence)
- [ ] Three test works appear after Phase 2 imports
- [ ] "Open" launches an embedded reader window with the EPUB loaded
- [ ] "Open externally" launches Apple Books
- [ ] Reader window has reduced capability set (verify by trying to
      call a main-window-only command — should fail)
- [ ] Click+shift in reader fires the `lookup` command (returns null
      in Phase 3, but the wiring is in place)
- [ ] Sentence extraction from click position works for Aozora
      vertical text
- [ ] DB migrations run cleanly: clean-slate install gets latest
      schema; existing DB upgrades from any prior version

## Common Phase 3 problems

See v1 phase 3 doc; same issues apply.

**New: sentence extraction misses sentences spanning multiple text
nodes.** For some EPUB layouts, a sentence is split across `<p>`
elements. The simple `indexOf('。')` doesn't cross node boundaries.
Mitigation for Phase 3: accept truncation at node boundaries; Phase 5
can implement walking the DOM tree if it becomes a problem.

## What's next

Phase 4: vocab DB + AnkiConnect. The reader's `lookup` stub gets wired
to a real handler that records encounters. The 4-button popup arrives in
Phase 5 once the dictionary engine is in place.
