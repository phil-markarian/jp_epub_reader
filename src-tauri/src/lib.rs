pub mod commands;
pub mod state;

use crate::state::AppState;
use std::path::PathBuf;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let app_cache_dir = app.path().app_cache_dir()?;
            let resource_dir = resolve_resource_dir(app)?;
            let state = AppState::new(app_data_dir, app_cache_dir, resource_dir)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::source::refresh_index,
            commands::source::search_works,
            commands::source::set_aozora_repo_path,
            commands::source::clear_aozora_repo_path,
            commands::source::get_aozora_source_status,
            commands::source::resolve_work,
            commands::import::get_importer_status,
            commands::import::import_aozora_work,
            commands::import::get_aozora_strategy,
            commands::import::set_aozora_strategy,
            commands::import::open_path,
            commands::import::list_library,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// In dev `resource_dir()` returns the build target directory which doesn't
/// contain our staged resources. Fall back to `<src-tauri>/resources` —
/// the directory we actually checked the jars into. Production bundles
/// land via tauri.conf.json's `bundle.resources` glob, so resource_dir()
/// is correct there.
fn resolve_resource_dir(app: &tauri::App) -> tauri::Result<PathBuf> {
    let bundled = app.path().resource_dir()?;
    let candidate = bundled.join("resources");
    if candidate.join("aozoraepub3-original").exists()
        || candidate.join("aozoraepub3-jdk21").exists()
    {
        return Ok(candidate);
    }
    // Dev fallback: src-tauri/resources/ relative to the manifest.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    Ok(dev)
}
