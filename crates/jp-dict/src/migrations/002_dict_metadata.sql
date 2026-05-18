-- Phase 5.5 — extra dictionary metadata for the details panel.
-- Yomitan index.json carries title/revision/format plus optional
-- description/attribution/url; the first three we already store on
-- the dictionary row. Stash the latter three here, plus a
-- user_notes column for free-form annotations the user types in
-- (e.g. "use only for kokugo lookups", "this one has bad EN
-- glosses"). All new columns are nullable so existing rows from
-- imports done before this migration keep working — they'll just
-- show empty details until re-imported.

ALTER TABLE dictionary ADD COLUMN description TEXT;
ALTER TABLE dictionary ADD COLUMN attribution TEXT;
ALTER TABLE dictionary ADD COLUMN url TEXT;
ALTER TABLE dictionary ADD COLUMN user_notes TEXT;
