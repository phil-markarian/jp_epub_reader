//! Shared backend state held by the Tauri app.

use jp_core::settings::Settings;
use jp_importer::aozora::{AozoraWork, JarPaths, JavaInfo};
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

        Ok(Self {
            settings: Arc::new(settings),
            works: Mutex::new(Vec::new()),
            app_data_dir,
            app_cache_dir,
            resource_dir,
            java,
            jars,
        })
    }
}
