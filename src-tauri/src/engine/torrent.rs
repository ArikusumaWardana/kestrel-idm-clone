use async_trait::async_trait;
use chrono::Utc;
use librqbit::{
    api::TorrentIdOrHash, limits::LimitsConfig, AddTorrent, AddTorrentOptions, AddTorrentResponse,
    ManagedTorrent, Session, SessionOptions,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, OnceCell, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::contracts::{
    ChecksumStatus, ChunkState, CreateDownloadRequest, DownloadStatus, DownloadTaskMeta,
    DownloaderEngine, EngineKind, ProgressSnapshot, TorrentFileItem, TorrentInspectInfo,
};
use crate::error::{AppError, AppResult};
use crate::storage::{meta, StoragePaths};

type ManagedTorrentHandle = Arc<ManagedTorrent>;

struct ActiveTorrentJob {
    torrent_id: usize,
    handle: ManagedTorrentHandle,
    token: CancellationToken,
    monitor: Option<JoinHandle<()>>,
}

pub struct TorrentEngine {
    paths: StoragePaths,
    default_dir: String,
    global_limit_bps: Arc<AtomicU64>,
    session: OnceCell<Arc<Session>>,
    jobs: Arc<RwLock<HashMap<Uuid, Arc<Mutex<ActiveTorrentJob>>>>>,
    progress_tx: broadcast::Sender<ProgressSnapshot>,
}

impl TorrentEngine {
    pub fn new(paths: StoragePaths, default_dir: String, global_limit_bps: Option<u64>) -> Self {
        let (progress_tx, _) = broadcast::channel(256);
        Self {
            paths,
            default_dir,
            global_limit_bps: Arc::new(AtomicU64::new(global_limit_bps.unwrap_or(0))),
            session: OnceCell::new(),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            progress_tx,
        }
    }

    pub fn set_global_speed_limit(&self, limit_bps: Option<u64>) {
        self.global_limit_bps
            .store(limit_bps.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn subscribe_progress(&self) -> broadcast::Receiver<ProgressSnapshot> {
        self.progress_tx.subscribe()
    }

    async fn get_session(&self) -> AppResult<Arc<Session>> {
        let default_dir = PathBuf::from(&self.default_dir);
        self.session
            .get_or_try_init(|| async {
                tokio::fs::create_dir_all(&default_dir).await?;
                let opts = SessionOptions {
                    persistence: None,
                    fastresume: false,
                    ..Default::default()
                };
                Session::new_with_opts(default_dir, opts)
                    .await
                    .map_err(|e| AppError::Other(format!("Failed to initialize torrent session: {e}")))
            })
            .await
            .map(Arc::clone)
    }

    pub async fn inspect_torrent(&self, source: &str) -> AppResult<TorrentInspectInfo> {
        let session = self.get_session().await?;
        let opts = AddTorrentOptions {
            list_only: true,
            ..Default::default()
        };

        let resp = if source.trim().to_ascii_lowercase().starts_with("magnet:") {
            session
                .add_torrent(AddTorrent::Url(Cow::Borrowed(source)), Some(opts))
                .await
                .map_err(|e| AppError::Other(format!("Magnet inspect failed: {e}")))?
        } else {
            let bytes = tokio::fs::read(source).await?;
            session
                .add_torrent(AddTorrent::TorrentFileBytes(bytes.into()), Some(opts))
                .await
                .map_err(|e| AppError::Other(format!("Torrent file inspect failed: {e}")))?
        };

        match resp {
            AddTorrentResponse::ListOnly(info) => {
                let mut total_bytes = 0u64;
                let files: Vec<TorrentFileItem> = info
                    .info
                    .iter_file_details()
                    .enumerate()
                    .map(|(index, d)| {
                        total_bytes += d.len;
                        TorrentFileItem {
                            index,
                            path: d.filename.to_string(),
                            size_bytes: d.len,
                        }
                    })
                    .collect();
                let name = info
                    .info
                    .name()
                    .map(|n| n.to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| {
                        files
                            .first()
                            .map(|f| f.path.clone())
                            .unwrap_or_else(|| "torrent".to_string())
                    });
                Ok(TorrentInspectInfo {
                    name,
                    info_hash: info.info_hash.as_string(),
                    total_bytes,
                    files,
                })
            }
            AddTorrentResponse::Added(_, handle) | AddTorrentResponse::AlreadyManaged(_, handle) => {
                let stats = handle.stats();
                Ok(TorrentInspectInfo {
                    name: handle.name().unwrap_or_else(|| "torrent".to_string()),
                    info_hash: handle.info_hash().as_string(),
                    total_bytes: stats.total_bytes,
                    files: Vec::new(),
                })
            }
        }
    }

    pub async fn create_task(
        &self,
        req: CreateDownloadRequest,
        default_dir: &str,
    ) -> AppResult<DownloadTaskMeta> {
        let id = Uuid::new_v4();
        let base_save_dir = req
            .save_dir
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| default_dir.to_string());

        let filename = req
            .filename
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| infer_torrent_name(&req.url, id));

        let folder_name = sanitize_folder_name(&filename);
        let save_dir = {
            let base_path = Path::new(&base_save_dir);
            let already_nested = base_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(&folder_name));
            if already_nested {
                base_save_dir
            } else {
                base_path.join(&folder_name).to_string_lossy().to_string()
            }
        };

        let now = Utc::now();
        let task = DownloadTaskMeta {
            id,
            url: req.url,
            filename,
            save_dir,
            engine: EngineKind::Torrent,
            status: DownloadStatus::Queued,
            total_bytes: None,
            downloaded_bytes: 0,
            supports_range: true,
            etag: None,
            last_modified: None,
            referer: None,
            user_agent: None,
            cookies: None,
            headers: HashMap::new(),
            chunks: Vec::new(),
            speed_limit_bps: req.speed_limit_bps,
            sequential_preview: req.sequential_preview.unwrap_or(false),
            expected_checksum: req.expected_checksum,
            checksum_algorithm: req.checksum_algorithm,
            checksum_status: ChecksumStatus::None,
            auto_extract: req.auto_extract.unwrap_or(false),
            delete_archive_after_extract: req.delete_archive_after_extract.unwrap_or(false),
            ytdlp_format_id: None,
            torrent_selected_files: req.torrent_selected_files,
            error_message: None,
            created_at: now,
            updated_at: now,
        };

        meta::save_meta_async(&self.paths, &task).await?;
        Ok(task)
    }

    async fn spawn_torrent(&self, mut task: DownloadTaskMeta) -> AppResult<()> {
        let id = task.id;
        let session = self.get_session().await?;
        tokio::fs::create_dir_all(&task.save_dir).await?;

        let global_lim = self.global_limit_bps.load(Ordering::Relaxed);
        let effective_lim = match (task.speed_limit_bps.filter(|&v| v > 0), global_lim) {
            (Some(t), g) if g > 0 => Some(t.min(g)),
            (Some(t), _) => Some(t),
            (None, g) if g > 0 => Some(g),
            _ => None,
        };
        let download_bps =
            effective_lim.and_then(|v| NonZeroU32::new(v.min(u32::MAX as u64) as u32));

        let opts = AddTorrentOptions {
            overwrite: true,
            only_files: task.torrent_selected_files.clone(),
            output_folder: Some(task.save_dir.clone()),
            ratelimits: LimitsConfig {
                download_bps,
                upload_bps: None,
            },
            ..Default::default()
        };

        let resp = if task.url.trim().to_ascii_lowercase().starts_with("magnet:") {
            session
                .add_torrent(AddTorrent::Url(Cow::Owned(task.url.clone())), Some(opts))
                .await
                .map_err(|e| AppError::Other(format!("Failed to add magnet: {e}")))?
        } else {
            let bytes = tokio::fs::read(&task.url).await?;
            session
                .add_torrent(AddTorrent::TorrentFileBytes(bytes.into()), Some(opts))
                .await
                .map_err(|e| AppError::Other(format!("Failed to add torrent file: {e}")))?
        };

        let (torrent_id, handle) = match resp {
            AddTorrentResponse::Added(tid, h) | AddTorrentResponse::AlreadyManaged(tid, h) => {
                (tid, h)
            }
            AddTorrentResponse::ListOnly(_) => {
                return Err(AppError::Other("Unexpected list-only response".into()));
            }
        };

        if let Some(real_name) = handle.name() {
            if !real_name.trim().is_empty() {
                task.filename = real_name;
            }
        }
        task.status = DownloadStatus::Downloading;
        task.error_message = None;
        task.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &task).await?;

        let token = CancellationToken::new();
        let monitor = self.spawn_monitor(id, handle.clone(), token.clone());
        let job = Arc::new(Mutex::new(ActiveTorrentJob {
            torrent_id,
            handle,
            token,
            monitor: Some(monitor),
        }));
        self.jobs.write().await.insert(id, job);
        Ok(())
    }

    fn spawn_monitor(
        &self,
        id: Uuid,
        handle: ManagedTorrentHandle,
        token: CancellationToken,
    ) -> JoinHandle<()> {
        let paths = self.paths.clone();
        let progress_tx = self.progress_tx.clone();
        let jobs = self.jobs.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(350));
            let mut smoothed_speed: f64 = 0.0;
            let mut ticks = 0u64;

            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = interval.tick() => {
                        ticks += 1;
                        let stats = handle.stats();
                        let total = if stats.total_bytes > 0 { Some(stats.total_bytes) } else { None };
                        let downloaded = stats.progress_bytes;
                        let raw_speed_bps = stats
                            .live
                            .as_ref()
                            .map(|l| l.download_speed.as_bytes())
                            .unwrap_or(0);
                        if smoothed_speed <= 0.0 {
                            smoothed_speed = raw_speed_bps as f64;
                        } else {
                            smoothed_speed = smoothed_speed * 0.94 + (raw_speed_bps as f64) * 0.06;
                        }
                        let speed_bps = smoothed_speed.round() as u64;
                        let eta_seconds = total.and_then(|t| {
                            if speed_bps > 0 && downloaded < t {
                                Some((t - downloaded) / speed_bps)
                            } else {
                                None
                            }
                        });

                        let chunks = build_torrent_chunks(downloaded, stats.total_bytes);

                        if let Some(err) = stats.error {
                            if let Ok(mut m) = meta::load_meta(&paths, id) {
                                m.status = DownloadStatus::Failed;
                                m.error_message = Some(err.clone());
                                m.downloaded_bytes = downloaded;
                                m.total_bytes = total;
                                m.updated_at = Utc::now();
                                let _ = meta::save_meta_async(&paths, &m).await;
                            }
                            let _ = progress_tx.send(ProgressSnapshot {
                                id,
                                status: DownloadStatus::Failed,
                                downloaded_bytes: downloaded,
                                total_bytes: total,
                                speed_bps: 0,
                                eta_seconds: None,
                                chunks,
                                checksum_status: ChecksumStatus::None,
                                error_message: Some(err),
                            });
                            jobs.write().await.remove(&id);
                            break;
                        }

                        if stats.finished && stats.total_bytes > 0 {
                            if let Ok(mut m) = meta::load_meta(&paths, id) {
                                m.status = DownloadStatus::Completed;
                                m.downloaded_bytes = downloaded;
                                m.total_bytes = total;
                                m.chunks = chunks.clone();
                                m.updated_at = Utc::now();
                                let _ = meta::save_meta_async(&paths, &m).await;
                            }
                            let _ = progress_tx.send(ProgressSnapshot {
                                id,
                                status: DownloadStatus::Completed,
                                downloaded_bytes: downloaded,
                                total_bytes: total,
                                speed_bps: 0,
                                eta_seconds: None,
                                chunks,
                                checksum_status: ChecksumStatus::None,
                                error_message: None,
                            });
                            jobs.write().await.remove(&id);
                            break;
                        }

                        if ticks.is_multiple_of(6) {
                            if let Ok(mut m) = meta::load_meta(&paths, id) {
                                m.downloaded_bytes = downloaded;
                                m.total_bytes = total;
                                m.chunks = chunks.clone();
                                m.updated_at = Utc::now();
                                let _ = meta::save_meta_async(&paths, &m).await;
                            }
                        }

                        let _ = progress_tx.send(ProgressSnapshot {
                            id,
                            status: DownloadStatus::Downloading,
                            downloaded_bytes: downloaded,
                            total_bytes: total,
                            speed_bps,
                            eta_seconds,
                            chunks,
                            checksum_status: ChecksumStatus::None,
                            error_message: None,
                        });
                    }
                }
            }
        })
    }
}

fn build_torrent_chunks(downloaded: u64, total: u64) -> Vec<ChunkState> {
    if total == 0 {
        return Vec::new();
    }
    let segments = 8usize;
    let seg_len = (total / segments as u64).max(1);
    let mut chunks = Vec::with_capacity(segments);
    let mut cursor = 0u64;
    let mut remaining_dl = downloaded;

    for i in 0..segments {
        let end = if i == segments - 1 {
            total - 1
        } else {
            (cursor + seg_len - 1).min(total - 1)
        };
        let len = end.saturating_sub(cursor) + 1;
        let dl = remaining_dl.min(len);
        remaining_dl = remaining_dl.saturating_sub(dl);
        chunks.push(ChunkState {
            index: i,
            start: cursor,
            end,
            downloaded: dl,
            completed: dl >= len,
        });
        cursor = end + 1;
        if cursor >= total {
            break;
        }
    }
    chunks
}

#[async_trait]
impl DownloaderEngine for TorrentEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Torrent
    }

    async fn start(&self, id: Uuid) -> AppResult<()> {
        let task = meta::load_meta(&self.paths, id)?;
        self.spawn_torrent(task).await
    }

    async fn pause(&self, id: Uuid) -> AppResult<()> {
        let job_opt = self.jobs.read().await.get(&id).cloned();
        if let Some(job_lock) = job_opt {
            let session = self.get_session().await?;
            let (handle, token, monitor) = {
                let mut job = job_lock.lock().await;
                job.token.cancel();
                (job.handle.clone(), job.token.clone(), job.monitor.take())
            };
            let _ = token;
            if let Some(m) = monitor {
                let _ = m.await;
            }
            let _ = session.pause(&handle).await;
        }
        let mut m = meta::load_meta(&self.paths, id)?;
        m.status = DownloadStatus::Paused;
        m.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &m).await
    }

    async fn resume(&self, id: Uuid) -> AppResult<()> {
        if let Some(job_lock) = self.jobs.read().await.get(&id).cloned() {
            let session = self.get_session().await?;
            let mut job = job_lock.lock().await;
            session
                .unpause(&job.handle)
                .await
                .map_err(|e| AppError::Other(format!("Failed to resume torrent: {e}")))?;
            let token = CancellationToken::new();
            let monitor = self.spawn_monitor(id, job.handle.clone(), token.clone());
            job.token = token;
            job.monitor = Some(monitor);
            let mut m = meta::load_meta(&self.paths, id)?;
            m.status = DownloadStatus::Downloading;
            m.updated_at = Utc::now();
            meta::save_meta_async(&self.paths, &m).await?;
            return Ok(());
        }
        let task = meta::load_meta(&self.paths, id)?;
        self.spawn_torrent(task).await
    }

    async fn cancel(&self, id: Uuid, delete_partial: bool) -> AppResult<()> {
        let job_opt = self.jobs.write().await.remove(&id);
        if let Some(job_lock) = job_opt {
            let session = self.get_session().await?;
            let (tid, monitor) = {
                let mut job = job_lock.lock().await;
                job.token.cancel();
                (job.torrent_id, job.monitor.take())
            };
            if let Some(m) = monitor {
                let _ = m.await;
            }
            let _ = session
                .delete(TorrentIdOrHash::Id(tid), delete_partial)
                .await;
        }
        let _ = meta::delete_meta(&self.paths, id);
        Ok(())
    }

    async fn progress_snapshot(&self, id: Uuid) -> AppResult<ProgressSnapshot> {
        if let Some(job_lock) = self.jobs.read().await.get(&id).cloned() {
            let job = job_lock.lock().await;
            let stats = job.handle.stats();
            let total = if stats.total_bytes > 0 { Some(stats.total_bytes) } else { None };
            let speed_bps = stats
                .live
                .as_ref()
                .map(|l| l.download_speed.as_bytes())
                .unwrap_or(0);
            return Ok(ProgressSnapshot {
                id,
                status: DownloadStatus::Downloading,
                downloaded_bytes: stats.progress_bytes,
                total_bytes: total,
                speed_bps,
                eta_seconds: None,
                chunks: build_torrent_chunks(stats.progress_bytes, stats.total_bytes),
                checksum_status: ChecksumStatus::None,
                error_message: stats.error,
            });
        }
        let m = meta::load_meta(&self.paths, id)?;
        Ok(ProgressSnapshot {
            id,
            status: m.status,
            downloaded_bytes: m.downloaded_bytes,
            total_bytes: m.total_bytes,
            speed_bps: 0,
            eta_seconds: None,
            chunks: m.chunks,
            checksum_status: m.checksum_status,
            error_message: m.error_message,
        })
    }
}

fn infer_torrent_name(source: &str, id: Uuid) -> String {
    let trimmed = source.trim();
    if trimmed.to_ascii_lowercase().starts_with("magnet:") {
        if let Ok(parsed) = url::Url::parse(trimmed) {
            if let Some((_, dn)) = parsed.query_pairs().find(|(k, _)| k == "dn") {
                let dn_str = dn.trim();
                if !dn_str.is_empty() {
                    return dn_str.to_string();
                }
            }
        }
        return format!("torrent-{}", &id.to_string()[..8]);
    }
    Path::new(trimmed)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("torrent-{}", &id.to_string()[..8]))
}

fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "torrent-download".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_and_sanitizes_magnet_folder_name() {
        let id = Uuid::new_v4();
        let name = infer_torrent_name(
            "magnet:?xt=urn:btih:123456&dn=Ubuntu+24.04%3A+Desktop",
            id,
        );
        assert_eq!(name, "Ubuntu 24.04: Desktop");
        assert_eq!(sanitize_folder_name(&name), "Ubuntu 24.04_ Desktop");
    }
}
