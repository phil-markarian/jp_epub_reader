//! Phase 3 bookmark commands. Tied to the reader window: clicking the
//! bookmark control captures the current Foliate location, persists it,
//! and the drawer pulls the list back via `list_bookmarks`.

use crate::state::AppState;
use jp_vocab::{Bookmark, NewBookmark};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn add_bookmark(
    work_id: u32,
    cfi: Option<String>,
    section_index: Option<u32>,
    fraction: Option<f64>,
    chapter: Option<String>,
    chapter_index: Option<u32>,
    chapter_total: Option<u32>,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Bookmark, String> {
    let bm = NewBookmark {
        work_id,
        cfi,
        section_index,
        fraction,
        chapter,
        chapter_index,
        chapter_total,
        note: note.unwrap_or_default(),
    };
    state.db.add_bookmark(&bm, now_seconds()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_bookmarks(
    work_id: u32,
    state: State<'_, AppState>,
) -> Result<Vec<Bookmark>, String> {
    state.db.list_bookmarks(work_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_bookmark_note(
    id: i64,
    note: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .update_bookmark_note(id, &note, now_seconds())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_bookmark(id: i64, state: State<'_, AppState>) -> Result<(), String> {
    state.db.delete_bookmark(id).map_err(|e| e.to_string())
}
