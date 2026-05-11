-- Phase 3 — Bookmarks.
--
-- Each row records a saved location inside a library work. The
-- `cfi` column is a foliate-js EPUB CFI string when available; we
-- also keep section_index and fraction so a numeric jump still works
-- if the CFI fails to resolve (e.g. after a re-import).

CREATE TABLE IF NOT EXISTS bookmark (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id       INTEGER NOT NULL REFERENCES library(work_id) ON DELETE CASCADE,
    cfi           TEXT,
    section_index INTEGER,
    fraction      REAL,
    chapter       TEXT,
    note          TEXT NOT NULL DEFAULT '',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_bookmark_work
    ON bookmark(work_id, created_at DESC);
