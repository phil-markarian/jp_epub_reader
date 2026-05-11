//! Bookmark CRUD. A bookmark is a saved location inside a library
//! work plus a free-form note. Optional CFI + section_index/fraction
//! covers both Foliate's CFI-based navigation and the numeric fallback.

use crate::db::Db;
use jp_core::{Error, Result};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub work_id: u32,
    pub cfi: Option<String>,
    pub section_index: Option<u32>,
    pub fraction: Option<f64>,
    pub chapter: Option<String>,
    pub note: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewBookmark {
    pub work_id: u32,
    pub cfi: Option<String>,
    pub section_index: Option<u32>,
    pub fraction: Option<f64>,
    pub chapter: Option<String>,
    pub note: String,
}

impl Db {
    pub fn add_bookmark(&self, bm: &NewBookmark, now: i64) -> Result<Bookmark> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO bookmark
                    (work_id, cfi, section_index, fraction, chapter, note, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    bm.work_id,
                    bm.cfi,
                    bm.section_index,
                    bm.fraction,
                    bm.chapter,
                    bm.note,
                    now,
                    now,
                ],
            )
            .map_err(|e| Error::Other(format!("insert bookmark: {e}")))?;
            let id = c.last_insert_rowid();
            Ok(Bookmark {
                id,
                work_id: bm.work_id,
                cfi: bm.cfi.clone(),
                section_index: bm.section_index,
                fraction: bm.fraction,
                chapter: bm.chapter.clone(),
                note: bm.note.clone(),
                created_at: now,
                updated_at: now,
            })
        })
    }

    pub fn list_bookmarks(&self, work_id: u32) -> Result<Vec<Bookmark>> {
        self.with_conn(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT id, work_id, cfi, section_index, fraction, chapter, note,
                            created_at, updated_at
                     FROM bookmark
                     WHERE work_id = ?
                     ORDER BY COALESCE(section_index, 0) ASC,
                              COALESCE(fraction, 0) ASC,
                              created_at ASC",
                )
                .map_err(|e| Error::Other(e.to_string()))?;
            let iter = stmt
                .query_map(params![work_id], row_to_bookmark)
                .map_err(|e| Error::Other(e.to_string()))?;
            let mut out = Vec::new();
            for row in iter {
                out.push(row.map_err(|e| Error::Other(e.to_string()))?);
            }
            Ok(out)
        })
    }

    pub fn update_bookmark_note(&self, id: i64, note: &str, now: i64) -> Result<()> {
        self.with_conn(|c| {
            let n = c
                .execute(
                    "UPDATE bookmark SET note = ?, updated_at = ? WHERE id = ?",
                    params![note, now, id],
                )
                .map_err(|e| Error::Other(e.to_string()))?;
            if n == 0 {
                return Err(Error::not_found(format!("bookmark {id}")));
            }
            Ok(())
        })
    }

    pub fn delete_bookmark(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM bookmark WHERE id = ?", params![id])
                .map_err(|e| Error::Other(e.to_string()))?;
            Ok(())
        })
    }
}

fn row_to_bookmark(row: &Row<'_>) -> rusqlite::Result<Bookmark> {
    Ok(Bookmark {
        id: row.get(0)?,
        work_id: row.get(1)?,
        cfi: row.get(2)?,
        section_index: row.get(3)?,
        fraction: row.get(4)?,
        chapter: row.get(5)?,
        note: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::NewLibraryEntry;

    fn seed_library(db: &Db) -> u32 {
        db.upsert_library(&NewLibraryEntry {
            work_id: 773,
            source_id: "aozora:773".into(),
            title: "こころ".into(),
            author: Some("夏目漱石".into()),
            epub_path: "/tmp/k.epub".into(),
            raw_text_path: None,
            added_at: 1_700_000_000,
        })
        .unwrap();
        773
    }

    fn fixture(work_id: u32) -> NewBookmark {
        NewBookmark {
            work_id,
            cfi: Some("epubcfi(/6/4!/4/2/4,/1:0,/1:24)".into()),
            section_index: Some(3),
            fraction: Some(0.42),
            chapter: Some("先生と私".into()),
            note: "interesting passage".into(),
        }
    }

    #[test]
    fn add_and_list() {
        let db = Db::open_in_memory().unwrap();
        let work_id = seed_library(&db);
        let b = db.add_bookmark(&fixture(work_id), 1_700_000_001).unwrap();
        assert!(b.id > 0);
        let rows = db.list_bookmarks(work_id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].chapter.as_deref(), Some("先生と私"));
        assert_eq!(rows[0].note, "interesting passage");
    }

    #[test]
    fn update_note() {
        let db = Db::open_in_memory().unwrap();
        let work_id = seed_library(&db);
        let b = db.add_bookmark(&fixture(work_id), 1_700_000_001).unwrap();
        db.update_bookmark_note(b.id, "revised", 1_700_000_100)
            .unwrap();
        let rows = db.list_bookmarks(work_id).unwrap();
        assert_eq!(rows[0].note, "revised");
        assert_eq!(rows[0].updated_at, 1_700_000_100);
    }

    #[test]
    fn delete() {
        let db = Db::open_in_memory().unwrap();
        let work_id = seed_library(&db);
        let b = db.add_bookmark(&fixture(work_id), 1_700_000_001).unwrap();
        db.delete_bookmark(b.id).unwrap();
        assert!(db.list_bookmarks(work_id).unwrap().is_empty());
    }

    #[test]
    fn cascade_when_library_row_deleted() {
        let db = Db::open_in_memory().unwrap();
        let work_id = seed_library(&db);
        db.add_bookmark(&fixture(work_id), 1_700_000_001).unwrap();
        db.delete_library(work_id).unwrap();
        // bookmark row should be gone via ON DELETE CASCADE.
        assert!(db.list_bookmarks(work_id).unwrap().is_empty());
    }
}
