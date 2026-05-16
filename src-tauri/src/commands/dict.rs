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
use std::sync::atomic::Ordering;
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
pub async fn scan_dictionary_folder(
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

    // ~150 zip opens + JSON parses adds up; run off-thread so the
    // UI keeps repainting (button state, etc.) during the scan.
    tokio::task::spawn_blocking(move || -> Vec<DictPreview> {
        let zips = collect_zips(&root);
        zips.iter().map(|z| preview_zip(z, &existing)).collect()
    })
    .await
    .map_err(|e| format!("scan task failed: {e}"))
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
///
/// The actual zip parse + sqlite write happens inside
/// `tokio::task::spawn_blocking` so it doesn't tie up the main
/// async runtime thread (a large dictionary takes seconds and was
/// causing the macOS beachball when run inline).
#[tauri::command]
pub async fn import_single_dictionary(
    path: String,
    state: State<'_, AppState>,
) -> Result<ImportOutcome, String> {
    let db = state.dict_db.clone();
    let cancel = state.dict_import_cancel.clone();
    // Reset on every call — a stale cancel from a previous run would
    // otherwise abort the first row of every subsequent import.
    cancel.store(false, Ordering::Release);
    tokio::task::spawn_blocking(move || -> ImportOutcome {
        let zip_path = PathBuf::from(&path);
        match db.import_zip_with_cancel(&zip_path, &cancel) {
            Ok(Some(summary)) => ImportOutcome::Imported {
                path: path.clone(),
                summary,
            },
            Ok(None) => ImportOutcome::Skipped {
                path: path.clone(),
                reason: "already imported".into(),
            },
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("import cancelled") {
                    ImportOutcome::Skipped {
                        path: path.clone(),
                        reason: "cancelled".into(),
                    }
                } else {
                    ImportOutcome::Failed { path: path.clone(), error: msg }
                }
            }
        }
    })
    .await
    .map_err(|e| format!("import task failed: {e}"))
}

/// Flip the cancel flag so the in-flight `import_single_dictionary`
/// will roll back its transaction and return early. Idempotent; the
/// flag is reset at the start of the next import call.
#[tauri::command]
pub fn cancel_dictionary_import(state: State<'_, AppState>) {
    state.dict_import_cancel.store(true, Ordering::Release);
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MoveOutcome {
    Moved { from: String, to: String },
    Failed { from: String, error: String },
}

/// Move a set of `.zip` paths into `target_dir`. Tries `rename`
/// first (cheap when source + target sit on the same volume) and
/// falls back to copy+delete on cross-volume moves. Names that
/// already exist in `target_dir` get a `.1`, `.2`, … suffix before
/// the `.zip` extension so we never overwrite. Returns one outcome
/// per input path.
#[tauri::command]
pub async fn move_dictionary_zips(
    paths: Vec<String>,
    target_dir: String,
) -> Result<Vec<MoveOutcome>, String> {
    let target = PathBuf::from(&target_dir);
    if !target.is_dir() {
        return Err(format!("not a directory: {target_dir}"));
    }

    tokio::task::spawn_blocking(move || -> Vec<MoveOutcome> {
        paths
            .into_iter()
            .map(|p| move_one_zip(Path::new(&p), &target))
            .collect()
    })
    .await
    .map_err(|e| format!("move task failed: {e}"))
}

fn move_one_zip(src: &Path, target_dir: &Path) -> MoveOutcome {
    let from_display = src.to_string_lossy().to_string();
    let file_name = match src.file_name().map(|s| s.to_owned()) {
        Some(n) => n,
        None => {
            return MoveOutcome::Failed {
                from: from_display,
                error: "source has no file name".into(),
            };
        }
    };

    // Pick a non-colliding destination by adding ".N" before the
    // .zip suffix until one is free.
    let mut dest = target_dir.join(&file_name);
    if dest.exists() {
        let stem = src.file_stem().map(|s| s.to_owned()).unwrap_or(file_name);
        for n in 1u32.. {
            let mut candidate = stem.clone();
            candidate.push(format!(".{n}"));
            let with_ext = target_dir.join(&candidate).with_extension("zip");
            if !with_ext.exists() {
                dest = with_ext;
                break;
            }
        }
    }

    let to_display = dest.to_string_lossy().to_string();
    match std::fs::rename(src, &dest) {
        Ok(()) => MoveOutcome::Moved { from: from_display, to: to_display },
        Err(rename_err) => {
            // Cross-volume rename fails on macOS with EXDEV. Fall
            // back to copy + delete.
            match std::fs::copy(src, &dest).and_then(|_| std::fs::remove_file(src)) {
                Ok(()) => MoveOutcome::Moved { from: from_display, to: to_display },
                Err(e) => MoveOutcome::Failed {
                    from: from_display,
                    error: format!("rename: {rename_err}; copy fallback: {e}"),
                },
            }
        }
    }
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
