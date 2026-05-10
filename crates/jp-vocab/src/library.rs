//! Library table CRUD.

use crate::db::Db;
use jp_core::{Error, Result};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub work_id: u32,
    pub source_id: String,
    pub title: String,
    pub author: Option<String>,
    pub epub_path: String,
    pub raw_text_path: Option<String>,
    pub added_at: i64,
    pub last_opened_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewLibraryEntry {
    pub work_id: u32,
    pub source_id: String,
    pub title: String,
    pub author: Option<String>,
    pub epub_path: String,
    pub raw_text_path: Option<String>,
    pub added_at: i64,
}

impl Db {
    pub fn upsert_library(&self, entry: &NewLibraryEntry) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO library
                    (work_id, source_id, title, author, epub_path,
                     raw_text_path, added_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(work_id) DO UPDATE SET
                    source_id = excluded.source_id,
                    title = excluded.title,
                    author = excluded.author,
                    epub_path = excluded.epub_path,
                    raw_text_path = excluded.raw_text_path,
                    added_at = excluded.added_at",
                params![
                    entry.work_id,
                    entry.source_id,
                    entry.title,
                    entry.author,
                    entry.epub_path,
                    entry.raw_text_path,
                    entry.added_at,
                ],
            )
            .map_err(|e| Error::Other(format!("upsert library: {e}")))?;
            Ok(())
        })
    }

    pub fn list_library(&self) -> Result<Vec<LibraryEntry>> {
        self.with_conn(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT work_id, source_id, title, author, epub_path,
                            raw_text_path, added_at, last_opened_at
                     FROM library
                     ORDER BY added_at DESC, work_id ASC",
                )
                .map_err(|e| Error::Other(e.to_string()))?;
            let iter = stmt
                .query_map([], row_to_entry)
                .map_err(|e| Error::Other(e.to_string()))?;
            let mut out = Vec::new();
            for row in iter {
                out.push(row.map_err(|e| Error::Other(e.to_string()))?);
            }
            Ok(out)
        })
    }

    pub fn get_library(&self, work_id: u32) -> Result<Option<LibraryEntry>> {
        self.with_conn(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT work_id, source_id, title, author, epub_path,
                            raw_text_path, added_at, last_opened_at
                     FROM library WHERE work_id = ?",
                )
                .map_err(|e| Error::Other(e.to_string()))?;
            match stmt.query_row(params![work_id], row_to_entry) {
                Ok(entry) => Ok(Some(entry)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(Error::Other(e.to_string())),
            }
        })
    }

    pub fn delete_library(&self, work_id: u32) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM library WHERE work_id = ?", params![work_id])
                .map_err(|e| Error::Other(e.to_string()))?;
            Ok(())
        })
    }

    pub fn touch_library(&self, work_id: u32, when: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE library SET last_opened_at = ? WHERE work_id = ?",
                params![when, work_id],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
            Ok(())
        })
    }
}

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<LibraryEntry> {
    Ok(LibraryEntry {
        work_id: row.get(0)?,
        source_id: row.get(1)?,
        title: row.get(2)?,
        author: row.get(3)?,
        epub_path: row.get(4)?,
        raw_text_path: row.get(5)?,
        added_at: row.get(6)?,
        last_opened_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> NewLibraryEntry {
        NewLibraryEntry {
            work_id: 773,
            source_id: "aozora:773".into(),
            title: "こころ".into(),
            author: Some("夏目漱石".into()),
            epub_path: "/tmp/kokoro.epub".into(),
            raw_text_path: Some("/tmp/kokoro.utf8.txt".into()),
            added_at: 1_700_000_000,
        }
    }

    #[test]
    fn upsert_and_list() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_library(&fixture()).unwrap();
        let rows = db.list_library().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "こころ");
        assert_eq!(rows[0].author.as_deref(), Some("夏目漱石"));
    }

    #[test]
    fn upsert_overwrites_same_work_id() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_library(&fixture()).unwrap();
        let mut updated = fixture();
        updated.title = "こころ (revised)".into();
        db.upsert_library(&updated).unwrap();
        let rows = db.list_library().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "こころ (revised)");
    }

    #[test]
    fn delete() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_library(&fixture()).unwrap();
        db.delete_library(773).unwrap();
        assert_eq!(db.list_library().unwrap().len(), 0);
    }

    #[test]
    fn touch_sets_last_opened() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_library(&fixture()).unwrap();
        db.touch_library(773, 9_999).unwrap();
        let entry = db.get_library(773).unwrap().unwrap();
        assert_eq!(entry.last_opened_at, Some(9_999));
    }
}
