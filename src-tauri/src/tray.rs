use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::error::{AppError, AppResult};
use crate::manager::DownloadManager;

const MENU_OPEN: &str = "tray_open";
const MENU_PAUSE_ALL: &str = "tray_pause_all";
const MENU_RESUME_ALL: &str = "tray_resume_all";
const MENU_TOGGLE_LIMIT: &str = "tray_toggle_limit";
const MENU_TOGGLE_DROPZONE: &str = "tray_toggle_dropzone";
const MENU_EXIT: &str = "tray_exit";

pub fn apply_window_effects(window: &WebviewWindow) {
    if window_vibrancy::apply_mica(window, None).is_err() {
        let _ = window_vibrancy::apply_acrylic(window, Some((24, 24, 27, 200)));
    }
}

pub fn attach_destroy_on_close(window: &WebviewWindow) {
    let win_clone = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win_clone.destroy();
        }
    });
}

pub fn open_or_focus_main_window(app: &AppHandle) -> AppResult<()> {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_always_on_top(true);
        let _ = win.set_focus();
        let _ = win.set_always_on_top(false);
        return Ok(());
    }

    let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Kestrel")
        .inner_size(1080.0, 700.0)
        .min_inner_size(820.0, 540.0)
        .transparent(true)
        .decorations(true)
        .center()
        .build()
        .map_err(|e| AppError::Other(format!("Failed to create main window: {e}")))?;

    apply_window_effects(&win);
    attach_destroy_on_close(&win);
    let _ = win.show();
    let _ = win.set_always_on_top(true);
    let _ = win.set_focus();
    let _ = win.set_always_on_top(false);
    Ok(())
}

pub fn toggle_dropzone_window(app: &AppHandle) -> AppResult<bool> {
    if let Some(win) = app.get_webview_window("dropzone") {
        win.destroy()
            .map_err(|e| AppError::Other(format!("Failed to close dropzone: {e}")))?;
        return Ok(false);
    }

    let win = WebviewWindowBuilder::new(
        app,
        "dropzone",
        WebviewUrl::App("index.html?window=dropzone".into()),
    )
    .title("Drop Zone")
    .inner_size(264.0, 144.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(false)
    .shadow(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .build()
    .map_err(|e| AppError::Other(format!("Failed to create dropzone window: {e}")))?;

    let _ = win;
    Ok(true)
}

pub fn setup_tray(app: &AppHandle, manager: Arc<DownloadManager>) -> AppResult<()> {
    let open_item = MenuItem::with_id(app, MENU_OPEN, "Open Kestrel", true, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))?;
    let pause_item = MenuItem::with_id(app, MENU_PAUSE_ALL, "Pause All", true, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))?;
    let resume_item = MenuItem::with_id(app, MENU_RESUME_ALL, "Resume All", true, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))?;
    let limit_item =
        MenuItem::with_id(app, MENU_TOGGLE_LIMIT, "Toggle Speed Limit", true, None::<&str>)
            .map_err(|e| AppError::Other(e.to_string()))?;
    let dropzone_item = MenuItem::with_id(
        app,
        MENU_TOGGLE_DROPZONE,
        "Toggle Floating Drop Zone",
        true,
        None::<&str>,
    )
    .map_err(|e| AppError::Other(e.to_string()))?;
    let exit_item = MenuItem::with_id(app, MENU_EXIT, "Exit", true, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))?;

    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &pause_item,
            &resume_item,
            &limit_item,
            &dropzone_item,
            &exit_item,
        ],
    )
    .map_err(|e| AppError::Other(e.to_string()))?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Kestrel")
        .show_menu_on_left_click(false)
        .on_menu_event({
            let mgr = Arc::clone(&manager);
            move |app_handle, event| {
                let id = event.id().as_ref();
                match id {
                    MENU_OPEN => {
                        let app_c = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = open_or_focus_main_window(&app_c);
                        });
                    }
                    MENU_PAUSE_ALL => {
                        let mgr = Arc::clone(&mgr);
                        tauri::async_runtime::spawn(async move {
                            let _ = mgr.pause_all().await;
                        });
                    }
                    MENU_RESUME_ALL => {
                        let mgr = Arc::clone(&mgr);
                        tauri::async_runtime::spawn(async move {
                            let _ = mgr.resume_all().await;
                        });
                    }
                    MENU_TOGGLE_LIMIT => {
                        let mgr = Arc::clone(&mgr);
                        let app_c = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = mgr.toggle_speed_limit().await;
                            let _ = app_c.emit("downloads-changed", ());
                        });
                    }
                    MENU_TOGGLE_DROPZONE => {
                        let app_c = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = toggle_dropzone_window(&app_c);
                        });
                    }
                    MENU_EXIT => {
                        let mgr = Arc::clone(&mgr);
                        let app_c = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = mgr.pause_all().await;
                            app_c.exit(0);
                        });
                    }
                    _ => {}
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app_c = tray.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = open_or_focus_main_window(&app_c);
                });
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder
        .build(app)
        .map_err(|e| AppError::Other(format!("Failed to build tray icon: {e}")))?;

    Ok(())
}
