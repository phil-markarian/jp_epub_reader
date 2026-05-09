pub mod commands;
pub mod state;

use crate::state::AppState;
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
            let state = AppState::new(app_data_dir, app_cache_dir)?;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
