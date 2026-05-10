//! Phase 2 — import a resolved Aozora work into an EPUB on disk.

use crate::state::AppState;
use jp_importer::aozora::{AozoraImporter, AozoraInput, AozoraSource, AozoraStrategy};
use jp_importer::{ImportResult, Importer};
use serde::Serialize;
use std::path::PathBuf;
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
    importer
        .import(AozoraInput { work }, &library)
        .await
        .map_err(|e| e.to_string())
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
