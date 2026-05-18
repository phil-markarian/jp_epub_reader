//! Database handle + migration runner for the Yomitan dictionary store.
//!
//! Same pattern as `jp-vocab::db`: PRAGMA user_version + a forward-only
//! `MIGRATIONS` array of `include_str!` SQL files. Always release new
//! migrations by appending; never edit a previously-released file.

use jp_core::{Error, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Db {
    /// Open (or create) `dict.sqlite` under `data_dir`. Runs any
    /// pending migrations and enables WAL + FK enforcement.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("dict.sqlite");
        let conn = Connection::open(&path)
            .map_err(|e| Error::Other(format!("open dict db {path:?}: {e}")))?;
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
            .map_err(|e| Error::Other(format!("open mem dict db: {e}")))?;
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

    pub(crate) fn with_conn_mut<R>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R>,
    ) -> Result<R> {
        let mut conn = self.conn.lock().unwrap();
        f(&mut conn)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let current: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| Error::Other(format!("read user_version: {e}")))?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let target = (i + 1) as i32;
            if current < target {
                tracing::info!(target, "applying jp-dict migration");
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
    // v1 — Phase 5 subpiece 1: dictionary store
    include_str!("migrations/001_initial.sql"),
    // v2 — Phase 5.5: dictionary description / attribution / url /
    //                 user_notes for the details panel
    include_str!("migrations/002_dict_metadata.sql"),
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
}
