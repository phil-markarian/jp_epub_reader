//! Vocab DB: encounter logging, retention, status tracking.
//!
//! Phase 3 sets up the SQLite-backed Library table + a migration runner.
//! Phase 4 will append vocab/encounter/source migrations on top.

pub mod db;
pub mod library;

pub use db::Db;
pub use library::{LibraryEntry, NewLibraryEntry};
