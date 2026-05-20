-- Phase 5.5 — Lookup-mode routing.
--
-- `kind` lets the lookup pipeline send a kanji-hover only to
-- dicts that actually have kanji entries, a word-hover only to
-- term-bearing dicts, etc. Auto-populated at import time:
--   word      — has term_bank_* (default for most kokugo / bilingual)
--   kanji     — has kanji_bank_* but no term_bank_*
--   frequency — only term_meta_bank_* with mode=freq
--   pitch     — only term_meta_bank_* with mode=pitch
--   name      — JMnedict-style proper-noun dict (manual tag for now)
--   grammar   — grammar-focused dict (manual tag)
--   other     — fallback when heuristics can't classify
--
-- Pre-migration rows have NULL kind. The lookup queries treat
-- NULL as "matches any mode" so old dicts still surface until the
-- user re-imports or runs apply_catalog_to_dictionaries (which
-- now back-fills kind too).

ALTER TABLE dictionary ADD COLUMN kind TEXT;
