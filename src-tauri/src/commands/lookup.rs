//! Phase 5.4 — dictionary lookup command for the reader popup.
//!
//! The frontend's text scanner (in reader-init.js) sends a slice of
//! the page text starting at the click/hover position; this command
//! runs the deinflector + dict query and returns ranked hits.

use crate::state::AppState;
use jp_dict::LookupHit;
use tauri::State;

const DEFAULT_MAX_SCAN_LEN: usize = 16;

#[tauri::command]
pub async fn dict_lookup(
    text: String,
    max_scan_len: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<LookupHit>, String> {
    let dict_db = state.dict_db.clone();
    let cap = max_scan_len.unwrap_or(DEFAULT_MAX_SCAN_LEN);
    // Run off-thread: the deinflector pass + sqlite queries are
    // sync and can each take a few ms; keeping the UI thread free
    // means hover-triggered lookups stay snappy.
    tokio::task::spawn_blocking(move || dict_db.lookup(&text, cap))
        .await
        .map_err(|e| format!("lookup task failed: {e}"))?
        .map_err(|e| e.to_string())
}
