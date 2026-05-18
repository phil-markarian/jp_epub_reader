-- Phase 5.5 — remember which .zip a dictionary was imported from so
-- the "Reimport" button can find the same file without rescanning
-- the user's whole dict folder. Nullable so existing rows from
-- pre-v3 imports keep working; for those the reimport command falls
-- back to a folder scan keyed by dictionary name.

ALTER TABLE dictionary ADD COLUMN source_path TEXT;
