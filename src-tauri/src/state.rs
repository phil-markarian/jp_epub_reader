//! Shared backend state held by the Tauri app.

use jp_core::settings::Settings;
use jp_importer::aozora::AozoraWork;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub settings: Arc<Settings>,
    /// Cache of parsed works, populated after `refresh_index`.
    pub works: Mutex<Vec<AozoraWork>>,
    pub app_data_dir: PathBuf,
    pub app_cache_dir: PathBuf,
}

impl AppState {
    pub fn new(
        app_data_dir: PathBuf,
        app_cache_dir: PathBuf,
    ) -> jp_core::Result<Self> {
        std::fs::create_dir_all(&app_data_dir)?;
        std::fs::create_dir_all(&app_cache_dir)?;
        let settings = Settings::load(app_data_dir.join("settings.json"))?;
        Ok(Self {
            settings: Arc::new(settings),
            works: Mutex::new(Vec::new()),
            app_data_dir,
            app_cache_dir,
        })
    }
}
