//! Shared backend state held by the Tauri app.

use jp_core::settings::Settings;
use jp_dict::Db as DictDb;
use jp_importer::aozora::{AozoraWork, JarPaths, JavaInfo};
use jp_vocab::Db;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub settings: Arc<Settings>,
    /// Cache of parsed works, populated after `refresh_index`.
    pub works: Mutex<Vec<AozoraWork>>,
    pub app_data_dir: PathBuf,
    pub app_cache_dir: PathBuf,
    pub resource_dir: PathBuf,
    /// Detected Java runtime (None if unavailable on PATH).
    pub java: Option<JavaInfo>,
    /// Bundled AozoraEpub3 jar locations.
    pub jars: JarPaths,
    /// Library + (later) vocab database.
    pub db: Arc<Db>,
    /// Yomitan-format dictionary store (dict.sqlite).
    pub dict_db: Arc<DictDb>,
}

impl AppState {
    pub fn new(
        app_data_dir: PathBuf,
        app_cache_dir: PathBuf,
        resource_dir: PathBuf,
    ) -> jp_core::Result<Self> {
        std::fs::create_dir_all(&app_data_dir)?;
        std::fs::create_dir_all(&app_cache_dir)?;
        let settings = Settings::load(app_data_dir.join("settings.json"))?;
        let db = Arc::new(Db::open(&app_data_dir)?);
        let dict_db = Arc::new(DictDb::open(&app_data_dir)?);

        let java = match jp_importer::aozora::java::detect() {
            Ok(j) => {
                tracing::info!(
                    binary = %j.binary.display(),
                    major = j.major,
                    "java detected"
                );
                Some(j)
            }
            Err(e) => {
                tracing::warn!(error=%e, "java not detected; aozora import disabled");
                None
            }
        };

        let jars = JarPaths::from_resource_dir(&resource_dir);
        tracing::info!(
            jdk21 = ?jars.jdk21,
            original = ?jars.original,
            "aozora jar paths"
        );

        let state = Self {
            settings: Arc::new(settings),
            works: Mutex::new(Vec::new()),
            app_data_dir,
            app_cache_dir,
            resource_dir,
            java,
            jars,
            db: db.clone(),
            dict_db,
        };

        // Backfill the DB from any pre-Phase-3 imports (meta.json
        // sidecars or stray .epub files in library/<work_id>/).
        if let Err(e) = backfill_library(&state) {
            tracing::warn!(error=%e, "library backfill failed");
        }

        Ok(state)
    }
}

fn backfill_library(state: &AppState) -> jp_core::Result<()> {
    use jp_vocab::NewLibraryEntry;
    use std::time::UNIX_EPOCH;

    let lib_dir = state.app_data_dir.join("library");
    let Ok(rd) = std::fs::read_dir(&lib_dir) else {
        return Ok(());
    };

    for entry in rd.flatten() {
        let dir = entry.path();
        let Some(work_id) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if state.db.get_library(work_id)?.is_some() {
            continue; // Already in DB.
        }

        // Prefer the existing meta.json sidecar.
        let meta = dir.join("meta.json");
        if let Ok(bytes) = std::fs::read(&meta) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let title = json["title"].as_str().unwrap_or("").to_string();
                let author = json["author"].as_str().map(str::to_owned);
                let epub_path = json["epub_path"].as_str().unwrap_or("").to_string();
                let added_at = json["imported_at"].as_i64().unwrap_or(0);
                if !epub_path.is_empty() && std::path::Path::new(&epub_path).exists() {
                    state.db.upsert_library(&NewLibraryEntry {
                        work_id,
                        source_id: format!("aozora:{work_id}"),
                        title,
                        author,
                        epub_path,
                        raw_text_path: dir
                            .join("source.utf8.txt")
                            .exists()
                            .then(|| dir.join("source.utf8.txt").to_string_lossy().into_owned()),
                        added_at,
                    })?;
                    continue;
                }
            }
        }

        // Fallback: scan for a stray .epub in the dir.
        if let Some(epub) = std::fs::read_dir(&dir).ok().and_then(|rd| {
            rd.flatten().find_map(|e| {
                let p = e.path();
                (p.extension().and_then(|s| s.to_str()) == Some("epub")).then_some(p)
            })
        }) {
            let title = epub
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let added_at = std::fs::metadata(&epub)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            state.db.upsert_library(&NewLibraryEntry {
                work_id,
                source_id: format!("aozora:{work_id}"),
                title,
                author: None,
                epub_path: epub.to_string_lossy().into_owned(),
                raw_text_path: None,
                added_at,
            })?;
        }
    }

    Ok(())
}
