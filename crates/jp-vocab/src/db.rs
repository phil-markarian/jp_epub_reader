//! Database handle + migration runner.
//!
//! Migrations are forward-only and tracked in `PRAGMA user_version`.
//! Each entry in `MIGRATIONS` is the SQL applied to advance from
//! `current` to `current + 1`. Add a new migration by appending another
//! `include_str!` line — never edit a previously-released file.

use jp_core::{Error, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Db {
    /// Open (or create) the library DB under `data_dir`. Runs any
    /// pending migrations and enables WAL mode + FK enforcement.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("library.sqlite");
        let conn = Connection::open(&path)
            .map_err(|e| Error::Other(format!("open db {path:?}: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| Error::Other(format!("set wal: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| Error::Other(format!("set fk: {e}")))?;
        let db = Self { conn: Mutex::new(conn), path };
        db.migrate()?;
        Ok(db)
    }

    /// In-memory DB for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::Other(format!("open mem db: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| Error::Other(format!("set fk: {e}")))?;
        let db = Self {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn with_conn<R>(
        &self,
        f: impl FnOnce(&Connection) -> Result<R>,
    ) -> Result<R> {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let current: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| Error::Other(format!("read user_version: {e}")))?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let target = (i + 1) as i32;
            if current < target {
                tracing::info!(target, "applying migration");
                conn.execute_batch(sql)
                    .map_err(|e| Error::Other(format!("migration {target}: {e}")))?;
                conn.pragma_update(None, "user_version", target)
                    .map_err(|e| Error::Other(format!("set user_version: {e}")))?;
            }
        }
        Ok(())
    }
}

const MIGRATIONS: &[&str] = &[
    // v1 — Phase 3: library
    include_str!("migrations/001_initial.sql"),
    // v2 — Phase 3: bookmarks
    include_str!("migrations/002_bookmarks.sql"),
    // v3 — Phase 3: bookmark chapter_index + chapter_total
    include_str!("migrations/003_bookmark_chapter_index.sql"),
    // v4 — Phase 4: vocab + encounter + source (added later)
    // include_str!("migrations/004_vocab.sql"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_runs_all_migrations() {
        let db = Db::open_in_memory().unwrap();
        let v: i32 = db
            .with_conn(|c| {
                c.query_row("PRAGMA user_version", [], |r| r.get(0))
                    .map_err(|e| Error::Other(e.to_string()))
            })
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i32);
    }

    #[test]
    fn library_table_exists() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO library
                 (work_id, source_id, title, author, epub_path, added_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![1u32, "aozora:1", "title", "author", "/tmp/x.epub", 0i64],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
            Ok(())
        })
        .unwrap();
    }
}
