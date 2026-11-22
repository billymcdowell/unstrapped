//! Phase 7: menu-bar tray, window hide-on-close, launch at login, auto-sync timer, overdue warnings.

use crate::commands::AppState;
use crate::model::AssessmentListItem;
use crate::strap::Hooks;
use chrono::TimeZone;
use std::sync::Arc;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_notification::NotificationExt as _;

pub const AUTO_SYNC_INTERVAL: Duration = Duration::from_secs(20 * 60);
/// Sync automatically when the last successful sync is older than this.
pub const AUTO_SYNC_AFTER_MS: i64 = 4 * 3_600_000;
const DAY_MS: i64 = 86_400_000;

pub fn apply_autostart(app: &AppHandle, on: bool) {
    let m = app.autolaunch();
    let current = m.is_enabled().unwrap_or(false);
    if on && !current {
        let _ = m.enable();
    } else if !on && current {
        let _ = m.disable();
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn last_sync_text(last: Option<i64>) -> String {
    match last {
        None => "Last sync: never".into(),
        Some(ms) => {
            let ago = (chrono::Utc::now().timestamp_millis() - ms).max(0) / 60_000;
            let local = chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.format("%a %H:%M").to_string()).unwrap_or_default();
            let rel = if ago < 1 {
                "just now".to_string()
            } else if ago < 60 {
                format!("{ago} min ago")
            } else if ago < 48 * 60 {
                format!("{} h ago", ago / 60)
            } else {
                format!("{} days ago", ago / 1440)
            };
            format!("Last sync: {rel} ({local})")
        }
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Connect (if needed), sync, and disconnect again if this call opened the connection.
pub async fn auto_sync(state: Arc<AppState>, force: bool) -> Result<(), String> {
    if state.strap.is_syncing() {
        return Ok(());
    }
    if !force
        && let Some(last) = state.db.last_sync_ms()
            && chrono::Utc::now().timestamp_millis() - last < AUTO_SYNC_AFTER_MS {
                return Ok(());
            }
    let opened = !state.strap.is_connected().await;
    let kind = state.db.settings().transport;
    // Background syncs only reconnect to a strap the user has already paired from the Strap
    // screen. A fresh install never starts scanning (or prompts for Bluetooth) on its own.
    if opened && !force && state.db.strap_id(&kind).is_none() {
        return Ok(());
    }
    let _nap = crate::nap::NapGuard::begin("Syncing the WHOOP strap");
    if opened {
        state.strap.connect(&kind).await?;
    }
    let res = state.strap.sync_now().await;
    if opened {
        let _ = state.strap.disconnect().await;
    }
    res
}

fn check_overdue(app: &AppHandle, state: &AppState) {
    let Some(last) = state.db.last_sync_ms() else { return };
    let days = (chrono::Utc::now().timestamp_millis() - last) / DAY_MS;
    let today = crate::compute::today_string();
    let (level, title, body) = if days >= 10 {
        (10, "Sync your strap now", format!("It has been {days} days. The strap only keeps about 14 days, so your oldest data is about to be overwritten."))
    } else if days >= 7 {
        (7, "Strap needs a sync", format!("It has been {days} days since the last sync. Open Unstrapped near your strap to download your data."))
    } else {
        return;
    };
    let key = format!("overdue_notified_{level}");
    if state.db.get_state(&key).as_deref() == Some(today.as_str()) {
        return;
    }
    let _ = state.db.set_state(&key, &today);
    notify(app, title, &body);
}

pub fn setup(app: &tauri::App, state: Arc<AppState>) -> tauri::Result<()> {
    let handle = app.handle().clone();

    // ---- tray
    let last_item = MenuItem::with_id(app, "last_sync", last_sync_text(state.db.last_sync_ms()), false, None::<&str>)?;
    let sync_item = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
    let open_item = MenuItem::with_id(app, "open", "Open Unstrapped", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, Some("Cmd+Q"))?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&last_item, &sep, &sync_item, &open_item, &sep, &quit_item])?;
    let st_for_menu = state.clone();
    let mut tray = TrayIconBuilder::with_id("main").menu(&menu).tooltip("Unstrapped").show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(move |app, ev| match ev.id().as_ref() {
        "sync" => {
            let st = st_for_menu.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = auto_sync(st, true).await {
                    notify(&app, "Sync failed", &e);
                }
            });
        }
        "open" => show_main_window(app),
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;

    // keep the "Last sync" line fresh (also updated right after each sync)
    {
        let item = last_item.clone();
        let st = state.clone();
        state.strap.set_hooks(Hooks {
            on_synced: Some(Arc::new({
                let item = last_item.clone();
                move |ms| {
                    let _ = item.set_text(last_sync_text(Some(ms)));
                }
            })),
            on_assessment: Some(Arc::new({
                let h = handle.clone();
                move |a: &AssessmentListItem| {
                    let (t, b) = match a.kind.as_str() {
                        "weekly" => ("Your weekly assessment is ready", "See how last week went in Strap."),
                        "monthly" => ("Your monthly assessment is ready", "See how last month went in Strap."),
                        _ => ("Your year in review is ready", "Open Strap to look back on the year."),
                    };
                    notify(&h, t, b);
                }
            })),
        });
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let _ = item.set_text(last_sync_text(st.db.last_sync_ms()));
            }
        });
    }

    // ---- launch at login follows the stored setting
    apply_autostart(&handle, state.db.settings().launch_at_login);

    // ---- auto-sync every 20 minutes + overdue warnings hourly
    {
        let st = state.clone();
        let app = handle.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(45)).await;
            loop {
                if st.db.settings().auto_sync {
                    let _ = auto_sync(st.clone(), false).await;
                }
                tokio::time::sleep(AUTO_SYNC_INTERVAL).await;
                let _ = &app;
            }
        });
        let st = state.clone();
        let app = handle.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                check_overdue(&app, &st);
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
    }
    Ok(())
}
