-- Phase 3 — Bookmarks: record the flat chapter index (and total)
-- captured from the in-memory chapter cache at creation time. The
-- bookmark drawer renders "Chapter X of Y" from this rather than
-- estimating from section_index, which is much too coarse for
-- multi-chapter sections (Kokoro's 下 section alone has 58
-- chapters).

ALTER TABLE bookmark ADD COLUMN chapter_index INTEGER;
ALTER TABLE bookmark ADD COLUMN chapter_total INTEGER;
