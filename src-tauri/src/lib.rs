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
        .plugin(tauri_plugin_dialog::init())
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
            commands::import::delete_library_entry,
            commands::import::open_reader_window,
            commands::import::get_library_entry,
            commands::import::read_epub_bytes,
            commands::import::start_window_dragging,
            commands::import::toggle_window_maximize,
            commands::bookmark::add_bookmark,
            commands::bookmark::list_bookmarks,
            commands::bookmark::update_bookmark_note,
            commands::bookmark::delete_bookmark,
            commands::dict::scan_dictionary_folder,
            commands::dict::import_single_dictionary,
            commands::dict::cancel_dictionary_import,
            commands::dict::move_dictionary_zips,
            commands::dict::list_dictionaries,
            commands::dict::delete_dictionary,
            commands::dict::delete_all_dictionaries,
            commands::dict::reimport_dictionary,
            commands::dict::set_dictionary_enabled,
            commands::dict::set_dictionary_notes,
            commands::dict::reorder_dictionaries,
            commands::dict::get_dict_last_folder,
            commands::dict::clear_dict_last_folder,
            commands::lookup::dict_lookup,
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
