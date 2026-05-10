-- Phase 3 — Library schema.
--
-- The vocab/encounter/source tables that the user-facing review surfaces
-- depend on are added in a separate migration during Phase 4. Don't add
-- them here even if it would be convenient — the migration runner
-- expects each released file to be append-only.

CREATE TABLE IF NOT EXISTS library (
    work_id        INTEGER PRIMARY KEY,
    source_id      TEXT    NOT NULL UNIQUE,
    title          TEXT    NOT NULL,
    author         TEXT,
    epub_path      TEXT    NOT NULL,
    raw_text_path  TEXT,
    added_at       INTEGER NOT NULL,
    last_opened_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_library_added  ON library(added_at DESC);
CREATE INDEX IF NOT EXISTS idx_library_opened ON library(last_opened_at DESC);
