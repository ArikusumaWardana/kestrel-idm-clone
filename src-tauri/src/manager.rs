use chrono::Utc;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use uuid::Uuid;

use crate::automation::{auto_throttle, checksum, extract, routing};
use crate::contracts::{
    AppSettings, ChecksumStatus, CreateDownloadRequest, DownloadStatus, DownloadTaskMeta,
    DownloaderEngine, EngineKind, HistoryEntry, ProgressSnapshot, SystemStatus, WsMessage,
};
use crate::engine::http::HttpEngine;
use crate::engine::torrent::TorrentEngine;
use crate::engine::ytdlp::YtdlpEngine;
use crate::error::{AppError, AppResult};
use crate::sidecars;
use crate::storage::{history, meta, settings, StoragePaths};

pub struct DownloadManager {
    pub paths: StoragePaths,
    pub portable_mode: bool,
    pub settings: Arc<RwLock<AppSettings>>,
    pub http: Arc<HttpEngine>,
    pub ytdlp: Arc<YtdlpEngine>,
    pub torrent: Arc<TorrentEngine>,
    pub progress_tx: broadcast::Sender<ProgressSnapshot>,
    pub ws_out_tx: broadcast::Sender<WsMessage>,
    pub state_changed_tx: broadcast::Sender<()>,
    pub browser_prompt_tx: broadcast::Sender<()>,
    pub pending_browser_prompts: Mutex<VecDeque<CreateDownloadRequest>>,
    pub extension_clients: Arc<AtomicUsize>,
    pub auto_throttle_active: Arc<AtomicBool>,
    pub auto_throttle_reason: Arc<RwLock<Option<String>>>,
    pub effective_limit_bps: Arc<AtomicU64>,
    pub speed_limit_paused: Arc<AtomicBool>,
}

impl DownloadManager {
    pub fn new(paths: StoragePaths, portable_mode: bool) -> AppResult<Arc<Self>> {
        let loaded_settings = settings::load_or_init_settings(&paths)?;
        let global_limit = loaded_settings.global_speed_limit_bps;

        let http = Arc::new(HttpEngine::new(paths.clone(), global_limit)?);
        let ytdlp = Arc::new(YtdlpEngine::new(paths.clone(), global_limit));
        let torrent = Arc::new(TorrentEngine::new(
            paths.clone(),
            loaded_settings.default_download_dir.clone(),
            global_limit,
        ));

        let (progress_tx, _) = broadcast::channel(512);
        let (ws_out_tx, _) = broadcast::channel(64);
        let (state_changed_tx, _) = broadcast::channel(64);
        let (browser_prompt_tx, _) = broadcast::channel(32);

        // Reconcile any tasks left in transient active states from an ungraceful exit.
        if let Ok(metas) = meta::list_active_metas(&paths) {
            for mut m in metas {
                if matches!(
                    m.status,
                    DownloadStatus::Downloading
                        | DownloadStatus::Verifying
                        | DownloadStatus::Extracting
                ) {
                    m.status = DownloadStatus::Paused;
                    m.updated_at = Utc::now();
                    let _ = meta::save_meta(&paths, &m);
                }
            }
        }

        let manager = Arc::new(Self {
            paths,
            portable_mode,
            settings: Arc::new(RwLock::new(loaded_settings)),
            http,
            ytdlp,
            torrent,
            progress_tx,
            ws_out_tx,
            state_changed_tx,
            browser_prompt_tx,
            pending_browser_prompts: Mutex::new(VecDeque::new()),
            extension_clients: Arc::new(AtomicUsize::new(0)),
            auto_throttle_active: Arc::new(AtomicBool::new(false)),
            auto_throttle_reason: Arc::new(RwLock::new(None)),
            effective_limit_bps: Arc::new(AtomicU64::new(global_limit.unwrap_or(0))),
            speed_limit_paused: Arc::new(AtomicBool::new(false)),
        });

        manager.spawn_engine_forwarder(manager.http.subscribe_progress());
        manager.spawn_engine_forwarder(manager.ytdlp.subscribe_progress());
        manager.spawn_engine_forwarder(manager.torrent.subscribe_progress());
        Ok(manager)
    }

    fn spawn_engine_forwarder(
        self: &Arc<Self>,
        mut rx: broadcast::Receiver<ProgressSnapshot>,
    ) {
        let mgr = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(snap) => {
                        let _ = mgr.progress_tx.send(snap.clone());
                        if snap.status == DownloadStatus::Completed {
                            let mgr_done = Arc::clone(&mgr);
                            tauri::async_runtime::spawn(async move {
                                mgr_done.handle_completed_task(snap.id).await;
                            });
                        } else if matches!(
                            snap.status,
                            DownloadStatus::Paused
                                | DownloadStatus::Failed
                                | DownloadStatus::NeedsRefresh
                                | DownloadStatus::Canceled
                        ) {
                            let _ = mgr.state_changed_tx.send(());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub async fn handle_completed_task(&self, id: Uuid) {
        let Ok(mut m) = meta::load_meta(&self.paths, id) else {
            return;
        };

        if let (Some(expected), Some(algo)) = (&m.expected_checksum, m.checksum_algorithm) {
            if !expected.trim().is_empty() {
                m.status = DownloadStatus::Verifying;
                m.updated_at = Utc::now();
                let _ = meta::save_meta_async(&self.paths, &m).await;
                let _ = self.emit_snapshot_from_meta(&m);

                let file_path = std::path::PathBuf::from(&m.save_dir).join(&m.filename);
                match checksum::verify_file_checksum(file_path, expected.clone(), algo).await {
                    Ok(ChecksumStatus::Valid) => {
                        m.checksum_status = ChecksumStatus::Valid;
                    }
                    Ok(ChecksumStatus::Mismatch) => {
                        m.checksum_status = ChecksumStatus::Mismatch;
                        m.status = DownloadStatus::Failed;
                        m.error_message =
                            Some("Checksum verification failed: file corrupted".to_string());
                        m.updated_at = Utc::now();
                        let _ = meta::save_meta_async(&self.paths, &m).await;
                        let _ = self.emit_snapshot_from_meta(&m);
                        let _ = self.state_changed_tx.send(());
                        return;
                    }
                    Ok(other) => {
                        m.checksum_status = other;
                    }
                    Err(e) => {
                        m.status = DownloadStatus::Failed;
                        m.error_message = Some(format!("Checksum error: {e}"));
                        m.updated_at = Utc::now();
                        let _ = meta::save_meta_async(&self.paths, &m).await;
                        let _ = self.emit_snapshot_from_meta(&m);
                        let _ = self.state_changed_tx.send(());
                        return;
                    }
                }
            }
        }

        if m.auto_extract {
            m.status = DownloadStatus::Extracting;
            m.updated_at = Utc::now();
            let _ = meta::save_meta_async(&self.paths, &m).await;
            let _ = self.emit_snapshot_from_meta(&m);

            let archive_path = std::path::PathBuf::from(&m.save_dir).join(&m.filename);
            let seven_zip = sidecars::resolve_sidecar("7z");
            if let Err(e) =
                extract::extract_archive(archive_path, m.delete_archive_after_extract, seven_zip)
                    .await
            {
                m.status = DownloadStatus::Failed;
                m.error_message = Some(e.to_string());
                m.updated_at = Utc::now();
                let _ = meta::save_meta_async(&self.paths, &m).await;
                let _ = self.emit_snapshot_from_meta(&m);
                let _ = self.state_changed_tx.send(());
                return;
            }
        }

        m.status = DownloadStatus::Completed;
        m.updated_at = Utc::now();
        let entry = HistoryEntry {
            id: m.id,
            url: m.url.clone(),
            filename: m.filename.clone(),
            save_dir: m.save_dir.clone(),
            engine: m.engine,
            status: DownloadStatus::Completed,
            total_bytes: m.total_bytes.or(Some(m.downloaded_bytes)),
            checksum_status: m.checksum_status,
            error_message: None,
            created_at: m.created_at,
            completed_at: Some(Utc::now()),
        };

        let _ = self.emit_snapshot_from_meta(&m);
        let paths = self.paths.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = history::upsert_history_entry(&paths, entry);
            let _ = meta::delete_meta(&paths, id);
        })
        .await;
        let _ = self.state_changed_tx.send(());
    }

    fn emit_snapshot_from_meta(&self, m: &DownloadTaskMeta) -> AppResult<()> {
        let _ = self.progress_tx.send(ProgressSnapshot {
            id: m.id,
            status: m.status,
            downloaded_bytes: m.downloaded_bytes,
            total_bytes: m.total_bytes,
            speed_bps: 0,
            eta_seconds: None,
            chunks: m.chunks.clone(),
            checksum_status: m.checksum_status,
            error_message: m.error_message.clone(),
        });
        Ok(())
    }

    pub async fn queue_browser_prompt(&self, mut req: CreateDownloadRequest) -> CreateDownloadRequest {
        let cfg = self.settings.read().await.clone();
        if req.filename.as_deref().is_none_or(|f| f.trim().is_empty())
            && (req.url.starts_with("http://") || req.url.starts_with("https://"))
        {
            if let Some(suggested) = self.http.suggest_filename(&req).await {
                req.filename = Some(suggested);
            }
        }
        if req.save_dir.as_deref().is_none_or(|d| d.trim().is_empty()) {
            let hint_name = req.filename.as_deref().unwrap_or("");
            let routed = routing::resolve_save_dir(
                &req.url,
                hint_name,
                &cfg.routing_rules,
                &cfg.default_download_dir,
            );
            req.save_dir = Some(routed);
        }
        {
            let mut queue = self.pending_browser_prompts.lock().await;
            queue.push_back(req.clone());
        }
        let _ = self.browser_prompt_tx.send(());
        req
    }

    pub async fn take_pending_browser_prompts(&self) -> Vec<CreateDownloadRequest> {
        let mut queue = self.pending_browser_prompts.lock().await;
        queue.drain(..).collect()
    }

    pub async fn add_download(&self, mut req: CreateDownloadRequest) -> AppResult<DownloadTaskMeta> {
        let cfg = self.settings.read().await.clone();
        let kind = req.engine.unwrap_or_else(|| detect_engine_kind(&req));

        if req.save_dir.as_deref().is_none_or(|d| d.trim().is_empty()) {
            let hint_name = req.filename.as_deref().unwrap_or("");
            let routing_name = if kind == EngineKind::Ytdlp && !hint_name.contains('.') {
                format!("{hint_name}.mp4")
            } else {
                hint_name.to_string()
            };
            let routed = routing::resolve_save_dir(
                &req.url,
                &routing_name,
                &cfg.routing_rules,
                &cfg.default_download_dir,
            );
            req.save_dir = Some(routed);
        }

        match kind {
            EngineKind::Http => {
                let mut task = self
                    .http
                    .create_task(req.clone(), &cfg.default_download_dir, cfg.default_chunks)
                    .await?;
                if req.save_dir.is_none() || req.filename.is_none() {
                    let rerouted = routing::resolve_save_dir(
                        &task.url,
                        &task.filename,
                        &cfg.routing_rules,
                        &cfg.default_download_dir,
                    );
                    if rerouted != task.save_dir && req.save_dir.is_none() {
                        task.save_dir = rerouted;
                        meta::save_meta_async(&self.paths, &task).await?;
                    }
                }
                self.http.start(task.id).await?;
                let _ = self.state_changed_tx.send(());
                Ok(task)
            }
            EngineKind::Ytdlp => {
                let task = self
                    .ytdlp
                    .create_task(req, &cfg.default_download_dir)
                    .await?;
                self.ytdlp.start(task.id).await?;
                let _ = self.state_changed_tx.send(());
                Ok(task)
            }
            EngineKind::Torrent => {
                let task = self
                    .torrent
                    .create_task(req, &cfg.default_download_dir)
                    .await?;
                self.torrent.start(task.id).await?;
                let _ = self.state_changed_tx.send(());
                Ok(task)
            }
        }
    }

    pub async fn pause_download(&self, id: Uuid) -> AppResult<()> {
        let m = meta::load_meta(&self.paths, id)?;
        match m.engine {
            EngineKind::Http => self.http.pause(id).await?,
            EngineKind::Ytdlp => self.ytdlp.pause(id).await?,
            EngineKind::Torrent => self.torrent.pause(id).await?,
        }
        let _ = self.state_changed_tx.send(());
        Ok(())
    }

    pub async fn resume_download(&self, id: Uuid) -> AppResult<()> {
        let m = meta::load_meta(&self.paths, id)?;
        match m.engine {
            EngineKind::Http => self.http.resume(id).await?,
            EngineKind::Ytdlp => self.ytdlp.resume(id).await?,
            EngineKind::Torrent => self.torrent.resume(id).await?,
        }
        let _ = self.state_changed_tx.send(());
        Ok(())
    }

    pub async fn cancel_download(&self, id: Uuid, delete_partial: bool) -> AppResult<()> {
        if let Ok(m) = meta::load_meta(&self.paths, id) {
            match m.engine {
                EngineKind::Http => self.http.cancel(id, delete_partial).await?,
                EngineKind::Ytdlp => self.ytdlp.cancel(id, delete_partial).await?,
                EngineKind::Torrent => self.torrent.cancel(id, delete_partial).await?,
            }
        }
        let _ = self.state_changed_tx.send(());
        Ok(())
    }

    pub async fn pause_all(&self) -> AppResult<()> {
        let metas = meta::list_active_metas(&self.paths)?;
        for m in metas {
            if matches!(
                m.status,
                DownloadStatus::Downloading | DownloadStatus::Queued
            ) {
                let _ = self.pause_download(m.id).await;
            }
        }
        Ok(())
    }

    pub async fn resume_all(&self) -> AppResult<()> {
        let metas = meta::list_active_metas(&self.paths)?;
        for m in metas {
            if m.status == DownloadStatus::Paused || m.status == DownloadStatus::Queued {
                let _ = self.resume_download(m.id).await;
            }
        }
        Ok(())
    }

    pub async fn request_link_refresh(&self, id: Uuid) -> AppResult<()> {
        let m = meta::load_meta(&self.paths, id)?;
        if self.extension_clients.load(Ordering::Relaxed) == 0 {
            return Err(AppError::Other(
                "No browser extension connected on WebSocket bridge".to_string(),
            ));
        }
        let _ = self.ws_out_tx.send(WsMessage::RefreshLinkRequest {
            download_id: m.id,
            url: m.url,
            referer: m.referer,
        });
        Ok(())
    }

    pub async fn apply_refreshed_link(
        &self,
        id: Uuid,
        new_url: String,
        cookies: Option<String>,
        user_agent: Option<String>,
        referer: Option<String>,
    ) -> AppResult<()> {
        self.http
            .update_refreshed_link(id, new_url, cookies, user_agent, referer)
            .await?;
        let _ = self.state_changed_tx.send(());
        Ok(())
    }

    pub async fn list_active_downloads(&self) -> AppResult<Vec<DownloadTaskMeta>> {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || meta::list_active_metas(&paths))
            .await
            .map_err(|e| AppError::Other(format!("List active failed: {e}")))?
    }

    pub async fn list_history(&self) -> AppResult<Vec<HistoryEntry>> {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || history::load_history(&paths))
            .await
            .map_err(|e| AppError::Other(format!("List history failed: {e}")))?
    }

    pub async fn remove_history_entry(&self, id: Uuid) -> AppResult<Vec<HistoryEntry>> {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || history::remove_history_entry(&paths, id))
            .await
            .map_err(|e| AppError::Other(format!("Remove history failed: {e}")))?
    }

    pub async fn clear_history(&self) -> AppResult<()> {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || history::save_history(&paths, &[]))
            .await
            .map_err(|e| AppError::Other(format!("Clear history failed: {e}")))?
    }

    pub async fn get_settings(&self) -> AppSettings {
        self.settings.read().await.clone()
    }

    pub async fn update_settings(&self, new_settings: AppSettings) -> AppResult<AppSettings> {
        let paths = self.paths.clone();
        let to_save = new_settings.clone();
        tokio::task::spawn_blocking(move || settings::save_settings(&paths, &to_save))
            .await
            .map_err(|e| AppError::Other(format!("Save settings failed: {e}")))??;

        {
            let mut guard = self.settings.write().await;
            *guard = new_settings.clone();
        }
        self.recompute_effective_speed_limit().await;
        Ok(new_settings)
    }

    pub async fn set_global_speed_limit(&self, limit_bps: Option<u64>) -> AppResult<()> {
        {
            let mut guard = self.settings.write().await;
            guard.global_speed_limit_bps = limit_bps;
            let paths = self.paths.clone();
            let snapshot = guard.clone();
            tokio::task::spawn_blocking(move || settings::save_settings(&paths, &snapshot))
                .await
                .map_err(|e| AppError::Other(format!("Save speed limit failed: {e}")))??;
        }
        self.speed_limit_paused.store(false, Ordering::Relaxed);
        self.recompute_effective_speed_limit().await;
        Ok(())
    }

    pub async fn toggle_speed_limit(&self) -> bool {
        let prev = self.speed_limit_paused.fetch_xor(true, Ordering::Relaxed);
        let now_paused = !prev;
        self.recompute_effective_speed_limit().await;
        now_paused
    }

    pub async fn apply_auto_throttle(&self, active: bool, reason: Option<String>) {
        self.auto_throttle_active.store(active, Ordering::Relaxed);
        *self.auto_throttle_reason.write().await = reason;
        self.recompute_effective_speed_limit().await;
    }

    pub async fn recompute_effective_speed_limit(&self) {
        let cfg = self.settings.read().await.clone();
        let limit_disabled = self.speed_limit_paused.load(Ordering::Relaxed);
        let throttle_active = self.auto_throttle_active.load(Ordering::Relaxed);

        let effective = if limit_disabled {
            None
        } else {
            auto_throttle::compute_effective_limit(
                cfg.global_speed_limit_bps,
                throttle_active && cfg.auto_throttle_enabled,
                cfg.auto_throttle_percent,
                5 * 1024 * 1024,
            )
        };

        self.effective_limit_bps
            .store(effective.unwrap_or(0), Ordering::Relaxed);
        self.http.set_global_speed_limit(effective).await;
        self.ytdlp.set_global_speed_limit(effective);
        self.torrent.set_global_speed_limit(effective);
    }

    pub async fn system_status(&self) -> SystemStatus {
        let cfg = self.settings.read().await.clone();
        let eff = self.effective_limit_bps.load(Ordering::Relaxed);
        SystemStatus {
            bridge_port: cfg.bridge_port,
            extension_connected_count: self.extension_clients.load(Ordering::Relaxed),
            auto_throttle_active: self.auto_throttle_active.load(Ordering::Relaxed),
            auto_throttle_reason: self.auto_throttle_reason.read().await.clone(),
            effective_speed_limit_bps: if eff > 0 { Some(eff) } else { None },
            speed_limit_paused: self.speed_limit_paused.load(Ordering::Relaxed),
            portable_mode: self.portable_mode,
            sidecars: sidecars::detect_sidecars(),
        }
    }
}

fn detect_engine_kind(req: &CreateDownloadRequest) -> EngineKind {
    let lower = req.url.trim().to_ascii_lowercase();
    if lower.starts_with("magnet:") || lower.ends_with(".torrent") {
        EngineKind::Torrent
    } else if req
        .ytdlp_format_id
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        EngineKind::Ytdlp
    } else {
        EngineKind::Http
    }
}
