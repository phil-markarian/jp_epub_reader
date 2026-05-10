//! Phase 2/3 — import a resolved Aozora work into an EPUB on disk and
//! register it in the library DB.

use crate::state::AppState;
use jp_importer::aozora::{AozoraImporter, AozoraInput, AozoraSource, AozoraStrategy};
use jp_importer::{ImportResult, Importer};
use jp_vocab::{LibraryEntry, NewLibraryEntry};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const SETTING_REPO_PATH: &str = "aozora.repo_path";
const SETTING_STRATEGY: &str = "aozora.strategy";

#[derive(Debug, Clone, Serialize)]
pub struct ImporterStatus {
    pub java: Option<JavaSummary>,
    pub jdk21_bundled: bool,
    pub original_bundled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JavaSummary {
    pub version_string: String,
    pub major: u32,
    pub binary: String,
}

#[tauri::command]
pub fn get_importer_status(state: State<'_, AppState>) -> ImporterStatus {
    ImporterStatus {
        java: state.java.as_ref().map(|j| JavaSummary {
            version_string: j.version_string.clone(),
            major: j.major,
            binary: j.binary.to_string_lossy().into_owned(),
        }),
        jdk21_bundled: state.jars.jdk21.is_some(),
        original_bundled: state.jars.original.is_some(),
    }
}

#[tauri::command]
pub async fn import_aozora_work(
    work_id: u32,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let java = state
        .java
        .as_ref()
        .ok_or("Java not detected — install Temurin 21")?
        .clone();

    let work = state
        .works
        .lock()
        .unwrap()
        .iter()
        .find(|w| w.work_id == work_id)
        .cloned()
        .ok_or("work not in index — refresh first")?;

    let repo_path = state.settings.get_string(SETTING_REPO_PATH);
    let cache_dir = state.app_cache_dir.join("aozora-text");
    let source = match repo_path {
        Some(p) => AozoraSource::LocalRepo(PathBuf::from(p)),
        None => AozoraSource::Remote { cache_dir },
    };

    let strategy: AozoraStrategy = state
        .settings
        .get(SETTING_STRATEGY)
        .unwrap_or_default();

    let importer = AozoraImporter {
        source,
        jars: state.jars.clone(),
        java,
        strategy,
    };

    let library = state.app_data_dir.join("library");
    let result = importer
        .import(AozoraInput { work }, &library)
        .await
        .map_err(|e| e.to_string())?;

    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    state
        .db
        .upsert_library(&NewLibraryEntry {
            work_id,
            source_id: result.source_id.clone(),
            title: result.title.clone(),
            author: result.author.clone(),
            epub_path: result.epub_path.to_string_lossy().into_owned(),
            raw_text_path: result
                .raw_text_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            added_at,
        })
        .map_err(|e| e.to_string())?;

    Ok(result)
}

#[tauri::command]
pub fn set_aozora_strategy(
    strategy: AozoraStrategy,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .settings
        .set(SETTING_STRATEGY, &strategy)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_aozora_strategy(state: State<'_, AppState>) -> AozoraStrategy {
    state
        .settings
        .get(SETTING_STRATEGY)
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_library(state: State<'_, AppState>) -> Result<Vec<LibraryEntry>, String> {
    state.db.list_library().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_library_entry(
    work_id: u32,
    delete_files: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(entry) = state.db.get_library(work_id).map_err(|e| e.to_string())? {
        if delete_files {
            // Delete the per-work directory rather than just the EPUB so
            // the source.txt + utf8 sidecar go too. Best-effort.
            let dir = state.app_data_dir.join("library").join(work_id.to_string());
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            } else if let Ok(p) = std::path::PathBuf::from(&entry.epub_path).canonicalize() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    state
        .db
        .delete_library(work_id)
        .map_err(|e| e.to_string())
}

/// Hand a file path off to the OS default application. macOS uses
/// `open`; we'll add Linux/Windows variants once those targets matter.
#[tauri::command]
pub fn open_path(
    path: String,
    work_id: Option<u32>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("path not found: {path}"));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&p)
            .spawn()
            .map_err(|e| format!("open: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&p)
            .spawn()
            .map_err(|e| format!("xdg-open: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| format!("start: {e}"))?;
    }
    if let Some(id) = work_id {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = state.db.touch_library(id, now);
    }
    Ok(())
}
