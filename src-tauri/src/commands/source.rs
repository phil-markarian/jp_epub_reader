//! Tauri commands for the Aozora source layer (Phase 1).

use crate::state::AppState;
use jp_importer::aozora::{self, AozoraSource, AozoraWork};
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

const SETTING_REPO_PATH: &str = "aozora.repo_path";

/// Download (or use cached) index, parse, and stash in state. Returns the
/// number of works loaded.
#[tauri::command]
pub async fn refresh_index(state: State<'_, AppState>) -> Result<usize, String> {
    let cache_dir = state.app_cache_dir.clone();

    let csv_path = aozora::index::ensure_index(&cache_dir)
        .await
        .map_err(|e| e.to_string())?;
    let works = aozora::parse_index(&csv_path).map_err(|e| e.to_string())?;
    let count = works.len();
    *state.works.lock().unwrap() = works;
    Ok(count)
}

/// Case-insensitive substring search across title/yomi/author/yomi/romaji,
/// scored so exact-title hits surface first. Returns a page of `limit`
/// rows starting at `offset`, plus the total match count.
#[tauri::command]
pub fn search_works(
    query: String,
    offset: usize,
    limit: usize,
    only_public_domain: bool,
    state: State<'_, AppState>,
) -> SearchPage {
    let needle = query.trim().to_lowercase();
    let works = state.works.lock().unwrap();

    if needle.is_empty() {
        let mut filtered: Vec<&AozoraWork> = works
            .iter()
            .filter(|w| !only_public_domain || !w.copyright_active)
            .collect();
        filtered.sort_by_key(|w| w.work_id);
        let total = filtered.len();
        let rows: Vec<AozoraWork> = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();
        return SearchPage { rows, total };
    }

    let mut scored: Vec<(u32, &AozoraWork)> = works
        .iter()
        .filter_map(|w| {
            if only_public_domain && w.copyright_active {
                return None;
            }
            let s = relevance_score(w, &needle)?;
            Some((s, w))
        })
        .collect();

    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.work_id.cmp(&b.1.work_id)));
    let total = scored.len();
    let rows: Vec<AozoraWork> = scored
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, w)| w.clone())
        .collect();
    SearchPage { rows, total }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchPage {
    pub rows: Vec<AozoraWork>,
    pub total: usize,
}

/// Returns Some(score) if any field matches the needle. Lower is better.
fn relevance_score(w: &AozoraWork, needle: &str) -> Option<u32> {
    // Exact work_id match wins outright (lets users paste 773 → kokoro).
    if w.work_id.to_string() == needle {
        return Some(0);
    }
    let title_lc = w.title.to_lowercase();
    if title_lc == needle {
        return Some(1);
    }
    if title_lc.starts_with(needle) {
        return Some(2);
    }
    let title_yomi_lc = w.title_yomi.to_lowercase();
    if title_yomi_lc == needle {
        return Some(3);
    }
    if title_yomi_lc.starts_with(needle) {
        return Some(4);
    }
    if title_lc.contains(needle) {
        return Some(5);
    }
    if title_yomi_lc.contains(needle) {
        return Some(6);
    }
    let author_lc = w.author.to_lowercase();
    let author_yomi_lc = w.author_yomi.to_lowercase();
    let romaji_lc = w.author_romaji.to_lowercase();
    if author_lc.contains(needle)
        || author_yomi_lc.contains(needle)
        || romaji_lc.contains(needle)
    {
        return Some(7);
    }
    None
}

/// Persist a path to a local clone of `aozorabunko_text`. Validates the
/// directory looks like the right repo (has a `cards/` subdirectory).
#[tauri::command]
pub fn set_aozora_repo_path(
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.join("cards").is_dir() {
        return Err(format!(
            "not an aozorabunko_text checkout (no cards/ subdir): {path}"
        ));
    }
    state
        .settings
        .set(SETTING_REPO_PATH, &path)
        .map_err(|e| e.to_string())
}

/// Clear the local-repo setting; resolution falls back to remote.
#[tauri::command]
pub fn clear_aozora_repo_path(state: State<'_, AppState>) -> Result<(), String> {
    state
        .settings
        .unset(SETTING_REPO_PATH)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct AozoraSourceStatus {
    pub mode: &'static str,
    pub repo_path: Option<String>,
    pub works_loaded: usize,
}

#[tauri::command]
pub fn get_aozora_source_status(state: State<'_, AppState>) -> AozoraSourceStatus {
    let repo_path = state.settings.get_string(SETTING_REPO_PATH);
    let works_loaded = state.works.lock().unwrap().len();
    AozoraSourceStatus {
        mode: if repo_path.is_some() { "local" } else { "remote" },
        repo_path,
        works_loaded,
    }
}

/// Resolve a work to a SJIS-encoded `.txt` file path. Tries local repo
/// first if configured, falls back to remote-with-cache.
#[tauri::command]
pub async fn resolve_work(
    author_id: u32,
    stem: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let repo_path = state.settings.get_string(SETTING_REPO_PATH);
    let cache_dir = state.app_cache_dir.join("aozora-text");

    let primary = match &repo_path {
        Some(p) => AozoraSource::LocalRepo(PathBuf::from(p)),
        None => AozoraSource::Remote {
            cache_dir: cache_dir.clone(),
        },
    };

    let path = match primary.resolve(author_id, &stem).await {
        Ok(p) => p,
        Err(e) if repo_path.is_some() => {
            tracing::warn!(error=%e, "local resolve failed; falling back to remote");
            AozoraSource::Remote { cache_dir }
                .resolve(author_id, &stem)
                .await
                .map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };

    Ok(path.to_string_lossy().into_owned())
}

