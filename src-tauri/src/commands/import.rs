//! Phase 2 — import a resolved Aozora work into an EPUB on disk.

use crate::state::AppState;
use jp_importer::aozora::{AozoraImporter, AozoraInput, AozoraSource, AozoraStrategy};
use jp_importer::{ImportResult, Importer};
use serde::{Deserialize, Serialize};
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

    // Persist a sidecar so the library panel can re-list this entry
    // after restart without re-running the importer.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let entry = LibraryEntry {
        work_id,
        title: result.title.clone(),
        author: result.author.clone(),
        epub_path: result.epub_path.to_string_lossy().into_owned(),
        imported_at: now,
    };
    let meta_path = library.join(work_id.to_string()).join("meta.json");
    if let Ok(bytes) = serde_json::to_vec_pretty(&entry) {
        let _ = std::fs::write(meta_path, bytes);
    }

    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub work_id: u32,
    pub title: String,
    pub author: Option<String>,
    pub epub_path: String,
    /// Unix seconds.
    pub imported_at: i64,
}

#[tauri::command]
pub fn list_library(state: State<'_, AppState>) -> Vec<LibraryEntry> {
    let lib = state.app_data_dir.join("library");
    let mut entries: Vec<LibraryEntry> = Vec::new();
    let Ok(rd) = std::fs::read_dir(&lib) else {
        return entries;
    };
    for e in rd.flatten() {
        let dir = e.path();
        let meta = dir.join("meta.json");
        if let Ok(bytes) = std::fs::read(&meta) {
            if let Ok(le) = serde_json::from_slice::<LibraryEntry>(&bytes) {
                if std::path::Path::new(&le.epub_path).exists() {
                    entries.push(le);
                    continue;
                }
            }
        }
        // No meta — backfill from a stray .epub if one is present.
        if let Some(le) = backfill_entry(&dir) {
            entries.push(le);
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.imported_at));
    entries
}

fn backfill_entry(dir: &std::path::Path) -> Option<LibraryEntry> {
    let work_id: u32 = dir.file_name()?.to_string_lossy().parse().ok()?;
    let epub = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find_map(|e| {
            let p = e.path();
            (p.extension().and_then(|s| s.to_str()) == Some("epub")).then_some(p)
        })?;
    let stem = epub.file_stem()?.to_string_lossy().into_owned();
    let imported_at = std::fs::metadata(&epub)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(LibraryEntry {
        work_id,
        title: stem,
        author: None,
        epub_path: epub.to_string_lossy().into_owned(),
        imported_at,
    })
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

/// Hand a file path off to the OS default application. macOS uses
/// `open`; we'll add Linux/Windows variants once those targets matter.
#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
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
    Ok(())
}
