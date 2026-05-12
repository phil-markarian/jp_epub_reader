-- Phase 5 — Yomitan dictionary store.
--
-- Mirrors the schema documented in docs/phase-5-dictionary.md. One row
-- per installed dictionary; per-dict terms / kanji / metadata / tags
-- cascade on deletion. The glossary column carries the raw JSON of the
-- term-entry array element (we'll parse it for rendering later, in
-- subpiece 4); term_meta.data does the same for frequency / pitch /
-- IPA payloads.

CREATE TABLE IF NOT EXISTS dictionary (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL UNIQUE,
    revision        TEXT,
    format_version  INTEGER NOT NULL,
    priority        INTEGER NOT NULL DEFAULT 0,
    enabled         INTEGER NOT NULL DEFAULT 1,
    imported_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS term (
    id          INTEGER PRIMARY KEY,
    dict_id     INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    expression  TEXT    NOT NULL,
    reading     TEXT    NOT NULL,
    pos         TEXT,
    rules       TEXT,
    score       INTEGER NOT NULL DEFAULT 0,
    sequence    INTEGER,
    term_tags   TEXT,
    glossary    TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_term_expr ON term(expression);
CREATE INDEX IF NOT EXISTS idx_term_read ON term(reading);
CREATE INDEX IF NOT EXISTS idx_term_dict ON term(dict_id);

CREATE TABLE IF NOT EXISTS term_meta (
    dict_id     INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    expression  TEXT    NOT NULL,
    mode        TEXT    NOT NULL,
    data        TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_term_meta_expr ON term_meta(expression, mode);

CREATE TABLE IF NOT EXISTS kanji (
    id          INTEGER PRIMARY KEY,
    dict_id     INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    character   TEXT    NOT NULL,
    onyomi      TEXT,
    kunyomi     TEXT,
    tags        TEXT,
    meanings    TEXT    NOT NULL,
    stats       TEXT
);
CREATE INDEX IF NOT EXISTS idx_kanji_char ON kanji(dict_id, character);

CREATE TABLE IF NOT EXISTS tag (
    dict_id     INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    category    TEXT,
    description TEXT,
    score       INTEGER,
    PRIMARY KEY (dict_id, name)
);
