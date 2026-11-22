//! Unstrapped: Tauri backend. Bluetooth (or emulator) transport, SQLite storage, history sync, scores, UI commands.

pub mod assess;
pub mod background;
pub mod commands;
pub mod compute;
pub mod db;
pub mod emitter;
pub mod journal;
pub mod model;
pub mod nap;
pub mod reports;
pub mod screens;
pub mod strap;
pub mod sync;
pub mod transport;

#[cfg(test)]
mod tests;

use commands::AppState;
use std::sync::Arc;
use tauri::Manager;

/// Builds before the rename stored data under the old bundle identifier. Move that folder into place
/// once, so existing databases carry over. Does nothing when the new folder already exists.
fn migrate_legacy_data_dir(dir: &std::path::Path) {
    const LEGACY_ID: &str = "com.williammcdowell.whoop-mac";
    let Some(parent) = dir.parent() else { return };
    let legacy = parent.join(LEGACY_ID);
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, dir);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            migrate_legacy_data_dir(&dir);
            let db = db::Db::open(&dir.join("strap.sqlite3")).map_err(|e| format!("could not open the database: {e}"))?;
            let em: Arc<dyn emitter::Emitter> = Arc::new(emitter::TauriEmitter(app.handle().clone()));
            let strap = strap::StrapManager::new(db.clone(), em, strap::default_factory());
            let state = Arc::new(AppState { db, strap });
            app.manage(state.clone());
            background::setup(app, state)?;
            if std::env::args().any(|a| a == "--hidden")
                && let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            Ok(())
        })
        // Closing the window hides it; the app keeps running in the menu bar.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_home,
            commands::get_sleep,
            commands::get_recovery,
            commands::get_strain,
            commands::get_activity,
            commands::add_activity,
            commands::rename_activity,
            commands::get_health,
            commands::get_metric_detail,
            commands::get_stress,
            commands::get_trend,
            commands::get_journal,
            commands::get_journal_library,
            commands::set_journal_behaviors,
            commands::set_journal_answer,
            commands::get_insights,
            commands::list_assessments,
            commands::get_assessment,
            commands::get_healthspan,
            commands::get_raw_day,
            commands::get_raw_night,
            commands::get_raw_trends,
            commands::export_csv,
            commands::get_strap_status,
            commands::connect_strap,
            commands::disconnect_strap,
            commands::sync_now,
            commands::rebuild_decoded,
            commands::get_settings,
            commands::set_settings,
            commands::emulator_control,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        // Clicking the Dock icon brings the hidden window back.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            background::show_main_window(handle);
        }
        let _ = (handle, event);
    });
}
