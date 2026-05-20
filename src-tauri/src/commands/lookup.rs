//! Phase 5.4 — dictionary lookup commands for the reader popup.
//!
//! Three flavors:
//!   * `dict_lookup`         — word mode: forward text from the
//!     cursor, prefix-scan against word-kind dicts.
//!   * `dict_lookup_context` — same as word but with a larger
//!     scan window so sentence-level idioms / set phrases hit.
//!   * `dict_lookup_kanji`   — single-character hover, queries
//!     the kanji table against kanji-kind dicts.

use crate::state::AppState;
use jp_dict::{KanjiHit, LookupHit};
use tauri::State;

const DEFAULT_MAX_SCAN_LEN: usize = 16;
const DEFAULT_CONTEXT_SCAN_LEN: usize = 48;
/// Dict kinds the word + context modes consider. Pre-migration
/// rows (kind IS NULL) also match — handled at the SQL layer.
const WORD_KINDS: &[&str] = &["word", "name", "grammar"];

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
    tokio::task::spawn_blocking(move || dict_db.lookup_kinds(&text, cap, WORD_KINDS))
        .await
        .map_err(|e| format!("lookup task failed: {e}"))?
        .map_err(|e| e.to_string())
}

/// Wider scan for sentence-level lookups. The frontend's context
/// extractor already trims to a sentence boundary, so the cap is
/// mostly a sanity guard against a runaway scan.
#[tauri::command]
pub async fn dict_lookup_context(
    text: String,
    max_scan_len: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<LookupHit>, String> {
    let dict_db = state.dict_db.clone();
    let cap = max_scan_len.unwrap_or(DEFAULT_CONTEXT_SCAN_LEN);
    tokio::task::spawn_blocking(move || dict_db.lookup_kinds(&text, cap, WORD_KINDS))
        .await
        .map_err(|e| format!("context lookup task failed: {e}"))?
        .map_err(|e| e.to_string())
}

/// Single-character kanji lookup. Frontend sends the codepoint
/// under the cursor; we slice the first char (guards against
/// receiving a longer string) and query the kanji table.
#[tauri::command]
pub async fn dict_lookup_kanji(
    character: String,
    state: State<'_, AppState>,
) -> Result<Option<KanjiHit>, String> {
    let dict_db = state.dict_db.clone();
    let Some(ch) = character.chars().next() else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || dict_db.lookup_kanji(ch))
        .await
        .map_err(|e| format!("kanji lookup task failed: {e}"))?
        .map_err(|e| e.to_string())
}
