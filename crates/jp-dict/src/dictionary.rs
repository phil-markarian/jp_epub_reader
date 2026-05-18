//! `dictionary` table CRUD: listing and deletion. Import lives in
//! `crate::import` since it also writes the per-bank tables.

use crate::Db;
use jp_core::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dictionary {
    pub id: i64,
    pub name: String,
    pub revision: Option<String>,
    pub format_version: i32,
    pub priority: i32,
    pub enabled: bool,
    pub imported_at: i64,
    /// Number of `term` rows that belong to this dictionary. Filled in
    /// by `list_dictionaries`; do not assume it's free to recompute.
    pub term_count: i64,
    /// Auto-populated from index.json during import.
    pub description: Option<String>,
    pub attribution: Option<String>,
    pub url: Option<String>,
    /// Free-form notes the user types in the details panel.
    pub user_notes: Option<String>,
    /// On-disk path to the zip we imported this dictionary from,
    /// captured at import time. Used by the "Reimport" button to
    /// find the source without rescanning the user's dict folder.
    /// None for rows imported before migration v3.
    pub source_path: Option<String>,
}

impl Db {
    pub fn list_dictionaries(&self) -> Result<Vec<Dictionary>> {
        self.with_conn(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT d.id, d.name, d.revision, d.format_version,
                            d.priority, d.enabled, d.imported_at,
                            (SELECT COUNT(*) FROM term WHERE dict_id = d.id),
                            d.description, d.attribution, d.url, d.user_notes,
                            d.source_path
                     FROM dictionary d
                     ORDER BY d.priority DESC, d.imported_at ASC",
                )
                .map_err(|e| Error::Other(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(Dictionary {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        revision: r.get(2)?,
                        format_version: r.get(3)?,
                        priority: r.get(4)?,
                        enabled: r.get::<_, i64>(5)? != 0,
                        imported_at: r.get(6)?,
                        term_count: r.get(7)?,
                        description: r.get(8)?,
                        attribution: r.get(9)?,
                        url: r.get(10)?,
                        user_notes: r.get(11)?,
                        source_path: r.get(12)?,
                    })
                })
                .map_err(|e| Error::Other(e.to_string()))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| Error::Other(e.to_string()))?);
            }
            Ok(out)
        })
    }

    pub fn delete_dictionary(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM dictionary WHERE id = ?", rusqlite::params![id])
                .map_err(|e| Error::Other(format!("delete dictionary: {e}")))?;
            Ok(())
        })
    }

    /// Wipe every dictionary. CASCADE drops their terms/kanji/meta/
    /// tags. Returns the number of rows deleted so the caller can
    /// confirm.
    pub fn delete_all_dictionaries(&self) -> Result<usize> {
        self.with_conn(|c| {
            let n = c
                .execute("DELETE FROM dictionary", [])
                .map_err(|e| Error::Other(format!("delete all: {e}")))?;
            Ok(n)
        })
    }

    /// Return just the on-disk source zip for a single dictionary,
    /// or None if it has none recorded (pre-migration v3 row, or
    /// imported through a code path that never set it).
    pub fn dictionary_source_path(&self, id: i64) -> Result<Option<String>> {
        self.with_conn(|c| {
            let path: Option<String> = c
                .query_row(
                    "SELECT source_path FROM dictionary WHERE id = ?",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .map_err(|e| Error::Other(format!("source_path: {e}")))?;
            Ok(path)
        })
    }

    pub fn set_dictionary_notes(&self, id: i64, notes: Option<&str>) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE dictionary SET user_notes = ? WHERE id = ?",
                rusqlite::params![notes, id],
            )
            .map_err(|e| Error::Other(format!("set notes: {e}")))?;
            Ok(())
        })
    }

    pub fn set_dictionary_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE dictionary SET enabled = ? WHERE id = ?",
                rusqlite::params![if enabled { 1 } else { 0 }, id],
            )
            .map_err(|e| Error::Other(format!("set enabled: {e}")))?;
            Ok(())
        })
    }

    /// Re-number priorities so the first id in `ordered_ids` has the
    /// highest, the last has the lowest. Run as a single transaction
    /// — partial updates would leave the list inconsistent.
    pub fn reorder_dictionaries(&self, ordered_ids: &[i64]) -> Result<()> {
        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction()
                .map_err(|e| Error::Other(format!("begin reorder tx: {e}")))?;
            // Top of list gets the highest priority. Use the slice
            // length as the top so we never collide with rows that
            // happen not to be in `ordered_ids` (those keep their
            // existing priorities, which will sort below us).
            let top = ordered_ids.len() as i64;
            for (i, id) in ordered_ids.iter().enumerate() {
                let priority = top - i as i64;
                tx.execute(
                    "UPDATE dictionary SET priority = ? WHERE id = ?",
                    rusqlite::params![priority, id],
                )
                .map_err(|e| Error::Other(format!("update priority: {e}")))?;
            }
            tx.commit()
                .map_err(|e| Error::Other(format!("commit reorder: {e}")))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.list_dictionaries().unwrap().is_empty());
    }
}
