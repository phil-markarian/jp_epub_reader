//! Phase 5 subpiece 1 — dictionary import + listing commands.
//!
//! Folder-based batch import: pick a directory in the dialog, every
//! `.zip` under it (one level OR via the standard subfolder layout
//! used by the shoui collection) gets parsed into `dict.sqlite`.
//! Per-zip progress emits the `dictionary-import-progress` event so
//! the frontend can render a counter.

use crate::state::AppState;
use jp_dict::{Dictionary, ImportSummary};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

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

#[derive(Debug, Clone, Serialize)]
pub struct ImportProgress<'a> {
    current: usize,
    total: usize,
    path: &'a str,
    status: &'a str,
}

#[tauri::command]
pub fn import_dictionary_folder(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ImportOutcome>, String> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("not a directory: {path}"));
    }

    let zips = collect_zips(&root);
    let total = zips.len();
    let mut outcomes = Vec::with_capacity(total);

    for (i, zip_path) in zips.iter().enumerate() {
        let display = zip_path.to_string_lossy().to_string();
        let _ = app.emit(
            "dictionary-import-progress",
            ImportProgress {
                current: i + 1,
                total,
                path: &display,
                status: "starting",
            },
        );

        let outcome = match state.dict_db.import_zip(zip_path) {
            Ok(Some(summary)) => ImportOutcome::Imported {
                path: display.clone(),
                summary,
            },
            Ok(None) => ImportOutcome::Skipped {
                path: display.clone(),
                reason: "already imported".into(),
            },
            Err(e) => ImportOutcome::Failed {
                path: display.clone(),
                error: e.to_string(),
            },
        };

        let status = match &outcome {
            ImportOutcome::Imported { .. } => "imported",
            ImportOutcome::Skipped { .. } => "skipped",
            ImportOutcome::Failed { .. } => "failed",
        };
        let _ = app.emit(
            "dictionary-import-progress",
            ImportProgress {
                current: i + 1,
                total,
                path: &display,
                status,
            },
        );

        outcomes.push(outcome);
    }

    Ok(outcomes)
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

/// Walk `root` (one level deep + immediate subdirectories) and collect
/// every `.zip` file. Matches the shoui collection's layout where
/// `.zip` files sit inside category folders (`Bilingual/`, `Grammar/`,
/// etc.) under a single root, but also handles a flat folder of zips.
fn collect_zips(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_zips_from_dir(root, &mut out);
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                push_zips_from_dir(&path, &mut out);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn push_zips_from_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("zip"))
                == Some(true)
        {
            out.push(path);
        }
    }
}
