use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use uuid::Uuid;

use crate::contracts::{
    AppSettings, CreateDownloadRequest, DownloadTaskMeta, HistoryEntry, SystemStatus,
    TorrentInspectInfo, YtdlpMediaInfo,
};
use crate::error::{AppError, AppResult};
use crate::manager::DownloadManager;
use crate::tray;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[tauri::command]
pub async fn list_downloads(
    manager: State<'_, Arc<DownloadManager>>,
) -> AppResult<Vec<DownloadTaskMeta>> {
    manager.list_active_downloads().await
}

#[tauri::command]
pub async fn list_history(
    manager: State<'_, Arc<DownloadManager>>,
) -> AppResult<Vec<HistoryEntry>> {
    manager.list_history().await
}

#[tauri::command]
pub async fn remove_history_entry(
    manager: State<'_, Arc<DownloadManager>>,
    id: Uuid,
) -> AppResult<Vec<HistoryEntry>> {
    manager.remove_history_entry(id).await
}

#[tauri::command]
pub async fn clear_history(manager: State<'_, Arc<DownloadManager>>) -> AppResult<()> {
    manager.clear_history().await
}

#[tauri::command]
pub async fn take_pending_browser_prompts(
    manager: State<'_, Arc<DownloadManager>>,
) -> AppResult<Vec<CreateDownloadRequest>> {
    Ok(manager.take_pending_browser_prompts().await)
}

#[tauri::command]
pub async fn add_download(
    manager: State<'_, Arc<DownloadManager>>,
    request: CreateDownloadRequest,
) -> AppResult<DownloadTaskMeta> {
    manager.add_download(request).await
}

#[tauri::command]
pub async fn probe_video_formats(
    manager: State<'_, Arc<DownloadManager>>,
    url: String,
) -> AppResult<YtdlpMediaInfo> {
    manager.ytdlp.probe_formats(&url).await
}

#[tauri::command]
pub async fn inspect_torrent(
    manager: State<'_, Arc<DownloadManager>>,
    source: String,
) -> AppResult<TorrentInspectInfo> {
    manager.torrent.inspect_torrent(&source).await
}

#[tauri::command]
pub async fn pause_download(
    manager: State<'_, Arc<DownloadManager>>,
    id: Uuid,
) -> AppResult<()> {
    manager.pause_download(id).await
}

#[tauri::command]
pub async fn resume_download(
    manager: State<'_, Arc<DownloadManager>>,
    id: Uuid,
) -> AppResult<()> {
    manager.resume_download(id).await
}

#[tauri::command]
pub async fn cancel_download(
    manager: State<'_, Arc<DownloadManager>>,
    id: Uuid,
    delete_partial: bool,
) -> AppResult<()> {
    manager.cancel_download(id, delete_partial).await
}

#[tauri::command]
pub async fn pause_all(manager: State<'_, Arc<DownloadManager>>) -> AppResult<()> {
    manager.pause_all().await
}

#[tauri::command]
pub async fn resume_all(manager: State<'_, Arc<DownloadManager>>) -> AppResult<()> {
    manager.resume_all().await
}

#[tauri::command]
pub async fn set_global_speed_limit(
    manager: State<'_, Arc<DownloadManager>>,
    limit_bps: Option<u64>,
) -> AppResult<()> {
    manager.set_global_speed_limit(limit_bps).await
}

#[tauri::command]
pub async fn set_task_speed_limit(
    manager: State<'_, Arc<DownloadManager>>,
    id: Uuid,
    limit_bps: Option<u64>,
) -> AppResult<()> {
    manager.http.set_task_speed_limit(id, limit_bps).await
}

#[tauri::command]
pub async fn request_link_refresh(
    manager: State<'_, Arc<DownloadManager>>,
    id: Uuid,
) -> AppResult<()> {
    manager.request_link_refresh(id).await
}

#[tauri::command]
pub async fn open_download_folder(save_dir: String, filename: String) -> AppResult<()> {
    let file_path = PathBuf::from(&save_dir).join(&filename);
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        if file_path.exists() {
            Command::new("explorer.exe")
                .arg(format!("/select,{}", file_path.display()))
                .spawn()?;
        } else {
            Command::new("explorer.exe").arg(&save_dir).spawn()?;
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::Other(format!("Open folder failed: {e}")))?
}

#[tauri::command]
pub async fn open_media_preview(save_dir: String, filename: String) -> AppResult<()> {
    let file_path = PathBuf::from(&save_dir).join(&filename);
    if !file_path.exists() {
        return Err(AppError::NotFound(file_path.display().to_string()));
    }
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        Command::new("cmd")
            .args(["/C", "start", "", &file_path.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Other(format!("Preview launch failed: {e}")))?
}

#[tauri::command]
pub async fn get_settings(manager: State<'_, Arc<DownloadManager>>) -> AppResult<AppSettings> {
    Ok(manager.get_settings().await)
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    manager: State<'_, Arc<DownloadManager>>,
    settings: AppSettings,
) -> AppResult<AppSettings> {
    let saved = manager.update_settings(settings).await?;
    let autolaunch = app.autolaunch();
    if saved.autostart_enabled {
        let _ = autolaunch.enable();
    } else {
        let _ = autolaunch.disable();
    }
    Ok(saved)
}

#[tauri::command]
pub async fn get_system_status(
    manager: State<'_, Arc<DownloadManager>>,
) -> AppResult<SystemStatus> {
    Ok(manager.system_status().await)
}

#[tauri::command]
pub async fn toggle_dropzone_window(app: AppHandle) -> AppResult<bool> {
    tray::toggle_dropzone_window(&app)
}

#[tauri::command]
pub async fn open_main_window(app: AppHandle) -> AppResult<()> {
    tray::open_or_focus_main_window(&app)
}

#[tauri::command]
pub async fn read_clipboard_text() -> AppResult<String> {
    tokio::task::spawn_blocking(read_win32_clipboard_text)
        .await
        .map_err(|e| AppError::Other(format!("Clipboard read failed: {e}")))?
}

fn read_win32_clipboard_text() -> AppResult<String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;

    // SAFETY: Standard Win32 clipboard API calls with explicit null checks and CloseClipboard on all paths.
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return Ok(String::new());
        }
        if OpenClipboard(None).is_err() {
            return Ok(String::new());
        }
        let result = (|| -> AppResult<String> {
            let handle: HANDLE = match GetClipboardData(CF_UNICODETEXT) {
                Ok(h) => h,
                Err(_) => return Ok(String::new()),
            };
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal) as *const u16;
            if ptr.is_null() {
                return Ok(String::new());
            }
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(hglobal);
            Ok(text)
        })();
        let _ = CloseClipboard();
        result
    }
}
