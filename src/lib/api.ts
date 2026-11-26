import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AggregateSpeedEvent,
  AppSettings,
  CreateDownloadRequest,
  DownloadTaskMeta,
  HistoryEntry,
  ProgressSnapshot,
  SystemStatus,
  TorrentInspectInfo,
  YtdlpMediaInfo,
} from "./types";

export const api = {
  listDownloads: () => invoke<DownloadTaskMeta[]>("list_downloads"),
  listHistory: () => invoke<HistoryEntry[]>("list_history"),
  removeHistoryEntry: (id: string) =>
    invoke<HistoryEntry[]>("remove_history_entry", { id }),
  clearHistory: () => invoke<void>("clear_history"),
  takePendingBrowserPrompts: () =>
    invoke<CreateDownloadRequest[]>("take_pending_browser_prompts"),
  addDownload: (request: CreateDownloadRequest) =>
    invoke<DownloadTaskMeta>("add_download", { request }),
  probeVideoFormats: (url: string) =>
    invoke<YtdlpMediaInfo>("probe_video_formats", { url }),
  inspectTorrent: (source: string) =>
    invoke<TorrentInspectInfo>("inspect_torrent", { source }),
  pauseDownload: (id: string) => invoke<void>("pause_download", { id }),
  resumeDownload: (id: string) => invoke<void>("resume_download", { id }),
  cancelDownload: (id: string, deletePartial: boolean) =>
    invoke<void>("cancel_download", { id, deletePartial }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<void>("resume_all"),
  setGlobalSpeedLimit: (limitBps: number | null) =>
    invoke<void>("set_global_speed_limit", { limitBps }),
  setTaskSpeedLimit: (id: string, limitBps: number | null) =>
    invoke<void>("set_task_speed_limit", { id, limitBps }),
  requestLinkRefresh: (id: string) =>
    invoke<void>("request_link_refresh", { id }),
  openDownloadFolder: (saveDir: string, filename: string) =>
    invoke<void>("open_download_folder", { saveDir, filename }),
  openMediaPreview: (saveDir: string, filename: string) =>
    invoke<void>("open_media_preview", { saveDir, filename }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) =>
    invoke<AppSettings>("save_settings", { settings }),
  getSystemStatus: () => invoke<SystemStatus>("get_system_status"),
  toggleDropzoneWindow: () => invoke<boolean>("toggle_dropzone_window"),
  openMainWindow: () => invoke<void>("open_main_window"),
  readClipboardText: () => invoke<string>("read_clipboard_text"),
};

export function onDownloadProgress(
  handler: (snap: ProgressSnapshot) => void
): Promise<UnlistenFn> {
  return listen<ProgressSnapshot>("download-progress", (e) => handler(e.payload));
}

export function onDownloadsChanged(handler: () => void): Promise<UnlistenFn> {
  return listen<void>("downloads-changed", () => handler());
}

export function onAggregateSpeed(
  handler: (evt: AggregateSpeedEvent) => void
): Promise<UnlistenFn> {
  return listen<AggregateSpeedEvent>("aggregate-speed", (e) => handler(e.payload));
}

export function onAutoThrottleChanged(
  handler: (active: boolean, reason: string | null) => void
): Promise<UnlistenFn> {
  return listen<[boolean, string | null]>("auto-throttle-changed", (e) =>
    handler(e.payload[0], e.payload[1])
  );
}

export function onBrowserDownloadPrompt(handler: () => void): Promise<UnlistenFn> {
  return listen<void>("browser-download-prompt", () => handler());
}
