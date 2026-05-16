//! Phase 5 subpiece 1 — dictionary import + listing commands.
//!
//! Folder-based batch import: pick a directory in the dialog, every
//! `.zip` under it (one level OR via the standard subfolder layout
//! used by the shoui collection) gets parsed into `dict.sqlite`.
//! Per-zip progress emits the `dictionary-import-progress` event so
//! the frontend can render a counter.

use crate::state::AppState;
use jp_dict::{peek_index, Dictionary, ImportSummary};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::State;

const SETTING_DICT_LAST_FOLDER: &str = "dict.last_folder";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ImportOutcome {
    Imported {
        path: String,
        summary: ImportSummary,
    },
    Skipped {
        path: String,
        reason: String,
    },
    Failed {
        path: String,
        error: String,
    },
}

/// Lightweight per-zip preview returned by `scan_dictionary_folder`.
/// Reads only the zip's `index.json` — no term bank parsing — so
/// scanning a 147-zip collection takes a couple seconds rather than
/// the many minutes a full import would take.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictPreview {
    pub path: String,
    pub name: Option<String>,
    pub revision: Option<String>,
    pub format_version: Option<i32>,
    pub status: DictPreviewStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DictPreviewStatus {
    /// Format 3, not already imported — safe to select.
    Ready,
    /// Same name is already in the dictionary table.
    AlreadyImported,
    /// index.json reports format != 3.
    UnsupportedFormat,
    /// Couldn't open the zip, missing index.json, or invalid JSON.
    Broken,
}

#[tauri::command]
pub fn scan_dictionary_folder(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<DictPreview>, String> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("not a directory: {path}"));
    }

    // Remember this folder so the next app launch can auto-rescan
    // and show what's already imported. Best-effort; we don't fail
    // the scan if settings write fails.
    let _ = state.settings.set(SETTING_DICT_LAST_FOLDER, &path);

    // Snapshot existing dictionary names once so the per-zip preview
    // can flag duplicates without a DB query per file.
    let existing: HashSet<String> = state
        .dict_db
        .list_dictionaries()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|d| d.name)
        .collect();

    let zips = collect_zips(&root);
    let mut out = Vec::with_capacity(zips.len());
    for zip_path in &zips {
        out.push(preview_zip(zip_path, &existing));
    }
    Ok(out)
}

/// Returns the folder path the user last scanned (or imported from),
/// or None if they've never picked one. The frontend calls this on
/// mount so it can auto-rescan and surface "already imported"
/// statuses without making the user re-pick the folder each session.
#[tauri::command]
pub fn get_dict_last_folder(state: State<'_, AppState>) -> Option<String> {
    state.settings.get_string(SETTING_DICT_LAST_FOLDER)
}

/// Clear the saved dict folder. Useful if the folder gets renamed /
/// deleted and the auto-rescan keeps erroring.
#[tauri::command]
pub fn clear_dict_last_folder(state: State<'_, AppState>) -> Result<(), String> {
    state
        .settings
        .unset(SETTING_DICT_LAST_FOLDER)
        .map_err(|e| e.to_string())
}

fn preview_zip(zip_path: &Path, existing: &HashSet<String>) -> DictPreview {
    let display = zip_path.to_string_lossy().to_string();
    match peek_index(zip_path) {
        Ok(idx) => {
            let already = existing.contains(&idx.title);
            let status = if already {
                DictPreviewStatus::AlreadyImported
            } else if idx.format != 3 {
                DictPreviewStatus::UnsupportedFormat
            } else {
                DictPreviewStatus::Ready
            };
            DictPreview {
                path: display,
                name: Some(idx.title),
                revision: idx.revision,
                format_version: Some(idx.format),
                status,
                error: None,
            }
        }
        Err(e) => DictPreview {
            path: display,
            name: None,
            revision: None,
            format_version: None,
            status: DictPreviewStatus::Broken,
            error: Some(e.to_string()),
        },
    }
}

/// Import a single Yomitan zip and return its outcome. The frontend
/// drives the batch — calling this once per selected path — which
/// gives the JS event loop a chance to repaint between zips so the
/// UI never appears to hang. Replaces the previous batch command.
#[tauri::command]
pub fn import_single_dictionary(
    path: String,
    state: State<'_, AppState>,
) -> Result<ImportOutcome, String> {
    let zip_path = PathBuf::from(&path);
    let outcome = match state.dict_db.import_zip(&zip_path) {
        Ok(Some(summary)) => ImportOutcome::Imported {
            path: path.clone(),
            summary,
        },
        Ok(None) => ImportOutcome::Skipped {
            path: path.clone(),
            reason: "already imported".into(),
        },
        Err(e) => ImportOutcome::Failed {
            path: path.clone(),
            error: e.to_string(),
        },
    };
    Ok(outcome)
}

#[tauri::command]
pub fn list_dictionaries(
    state: State<'_, AppState>,
) -> Result<Vec<Dictionary>, String> {
    state.dict_db.list_dictionaries().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_dictionary(
    id: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.dict_db.delete_dictionary(id).map_err(|e| e.to_string())
}

/// Recursively walk `root` and collect every `.zip` file at any depth.
/// Handles flat folders (drop a bunch of zips in one place), the shoui
/// collection's category layout (`root/Bilingual/*.zip`), and
/// arbitrarily deeper nests (`root/Monolingual/Series/*.zip`).
///
/// Skips hidden entries (anything starting with `.`) and the
/// `__MACOSX` cruft that macOS adds to zips. Does NOT follow
/// symlinks to avoid infinite loops on misconfigured trees.
fn collect_zips(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_for_zips(root, &mut out, 0);
    out.sort();
    out.dedup();
    out
}

fn walk_for_zips(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    // Belt-and-suspenders: hard cap recursion in case someone creates
    // a circular junction. 32 levels is well past any sane dict tree.
    if depth > 32 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "__MACOSX" {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_for_zips(&path, out, depth + 1);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("zip"))
                == Some(true)
        {
            out.push(path);
        }
    }
}
