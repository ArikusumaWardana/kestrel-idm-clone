pub mod automation;
pub mod commands;
pub mod contracts;
pub mod engine;
pub mod error;
pub mod manager;
pub mod server;
pub mod sidecars;
pub mod storage;
pub mod tray;

use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_autostart::ManagerExt;
use uuid::Uuid;

use crate::automation::auto_throttle;
use crate::contracts::{AggregateSpeedEvent, DownloadStatus};
use crate::manager::DownloadManager;
use crate::storage::StoragePaths;

pub fn run() {
    let portable_mode = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("portable.flag").exists()))
        .unwrap_or(false);

    let paths = match StoragePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to resolve storage paths: {e}");
            return;
        }
    };

    let manager = match DownloadManager::new(paths, portable_mode) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to initialize download manager: {e}");
            return;
        }
    };

    let start_minimized = std::env::args().any(|a| a == "--minimized");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let launched_minimized = args.iter().any(|a| a == "--minimized");
            if !launched_minimized {
                let _ = tray::open_or_focus_main_window(app);
            }
        }))
        .manage(Arc::clone(&manager))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_downloads,
            commands::list_history,
            commands::remove_history_entry,
            commands::clear_history,
            commands::take_pending_browser_prompts,
            commands::add_download,
            commands::probe_video_formats,
            commands::inspect_torrent,
            commands::pause_download,
            commands::resume_download,
            commands::cancel_download,
            commands::pause_all,
            commands::resume_all,
            commands::set_global_speed_limit,
            commands::set_task_speed_limit,
            commands::request_link_refresh,
            commands::open_download_folder,
            commands::open_media_preview,
            commands::get_settings,
            commands::save_settings,
            commands::get_system_status,
            commands::toggle_dropzone_window,
            commands::open_main_window,
            commands::read_clipboard_text,
        ])
        .setup({
            let mgr = Arc::clone(&manager);
            move |app| {
                let app_handle = app.handle().clone();
                if let Err(e) = tray::setup_tray(&app_handle, Arc::clone(&mgr)) {
                    eprintln!("Tray setup failed: {e}");
                }

                if let Some(main_win) = app.get_webview_window("main") {
                    if start_minimized {
                        let _ = main_win.destroy();
                    } else {
                        tray::apply_window_effects(&main_win);
                        tray::attach_destroy_on_close(&main_win);
                    }
                }

                // Sync autostart setting on startup
                let mgr_boot = Arc::clone(&mgr);
                let app_boot = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let cfg = mgr_boot.get_settings().await;
                    let autolaunch = app_boot.autolaunch();
                    if cfg.autostart_enabled {
                        let _ = autolaunch.enable();
                    } else {
                        let _ = autolaunch.disable();
                    }
                    if let Err(e) = server::spawn_bridge_server(Arc::clone(&mgr_boot)).await {
                        eprintln!("Failed to bind localhost bridge: {e}");
                    }
                });

                spawn_ui_event_forwarder(app_handle.clone(), Arc::clone(&mgr));
                spawn_auto_throttle_task(app_handle, mgr);
                Ok(())
            }
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Fatal Tauri build error: {e}");
            return;
        }
    };

    app.run(|_app_handle, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}

fn spawn_ui_event_forwarder(app: tauri::AppHandle, manager: Arc<DownloadManager>) {
    let mut progress_rx = manager.progress_tx.subscribe();
    let mut changed_rx = manager.state_changed_tx.subscribe();
    let mut prompt_rx = manager.browser_prompt_tx.subscribe();

    tauri::async_runtime::spawn(async move {
        let mut speeds: HashMap<Uuid, (u64, DownloadStatus)> = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_millis(350));

        loop {
            tokio::select! {
                res = progress_rx.recv() => {
                    if let Ok(snap) = res {
                        if snap.status == DownloadStatus::Downloading {
                            speeds.insert(snap.id, (snap.speed_bps, snap.status));
                        } else {
                            speeds.remove(&snap.id);
                        }
                        if app.get_webview_window("main").is_some() {
                            let _ = app.emit("download-progress", &snap);
                        }
                    }
                }
                res = changed_rx.recv() => {
                    if res.is_ok() && app.get_webview_window("main").is_some() {
                        let _ = app.emit("downloads-changed", ());
                    }
                }
                res = prompt_rx.recv() => {
                    if res.is_ok() {
                        let _ = tray::open_or_focus_main_window(&app);
                        if app.get_webview_window("main").is_some() {
                            let _ = app.emit("browser-download-prompt", ());
                        }
                    }
                }
                _ = tick.tick() => {
                    if app.get_webview_window("main").is_some() || app.get_webview_window("dropzone").is_some() {
                        let total_speed_bps: u64 = speeds.values().map(|(s, _)| *s).sum();
                        let active_count = speeds.len();
                        let _ = app.emit("aggregate-speed", AggregateSpeedEvent {
                            total_speed_bps,
                            active_count,
                            timestamp_ms: Utc::now().timestamp_millis(),
                        });
                    }
                }
            }
        }
    });
}

fn spawn_auto_throttle_task(app: tauri::AppHandle, manager: Arc<DownloadManager>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        let mut last_active = false;
        let mut last_reason: Option<String> = None;

        loop {
            interval.tick().await;
            let cfg = manager.get_settings().await;
            let trigger = if cfg.auto_throttle_enabled {
                auto_throttle::detect_throttle_trigger(
                    cfg.auto_throttle_processes,
                    cfg.auto_throttle_on_fullscreen,
                )
                .await
                .unwrap_or(None)
            } else {
                None
            };

            let active = trigger.is_some();
            if active != last_active || trigger != last_reason {
                last_active = active;
                last_reason = trigger.clone();
                manager.apply_auto_throttle(active, trigger.clone()).await;
                if app.get_webview_window("main").is_some() {
                    let _ = app.emit("auto-throttle-changed", (active, trigger));
                }
            }
        }
    });
}
