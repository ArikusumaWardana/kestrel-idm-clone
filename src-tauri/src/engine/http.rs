use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH,
    CONTENT_RANGE, CONTENT_TYPE, COOKIE, ETAG, LAST_MODIFIED, RANGE, REFERER, USER_AGENT,
};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::num::NonZeroU32;
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use crate::contracts::{
    ChecksumStatus, ChunkState, CreateDownloadRequest, DownloadStatus, DownloadTaskMeta,
    DownloaderEngine, EngineKind, ProgressSnapshot,
};
use crate::error::{AppError, AppResult};
use crate::storage::{meta, StoragePaths};

const MIN_CHUNK_SIZE: u64 = 256 * 1024;
const MIN_STEAL_REMAINING: u64 = 512 * 1024;
const PROGRESS_INTERVAL_MS: u64 = 300;

type SharedLimiter = Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>;

#[derive(Clone)]
pub struct DynamicSpeedLimiter {
    inner: Arc<RwLock<Option<(SharedLimiter, u32)>>>,
}

impl DynamicSpeedLimiter {
    pub fn new(limit_bps: Option<u64>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(build_limiter(limit_bps))),
        }
    }

    pub async fn set_limit(&self, limit_bps: Option<u64>) {
        let mut guard = self.inner.write().await;
        *guard = build_limiter(limit_bps);
    }

    pub async fn wait_for_bytes(&self, len: usize) {
        if len == 0 {
            return;
        }
        let limiter = { self.inner.read().await.clone() };
        if let Some((lim, max_burst)) = limiter {
            let mut remaining = len;
            let burst = (max_burst as usize).max(1);
            while remaining > 0 {
                let step = remaining.min(burst);
                if let Some(nz) = NonZeroU32::new(step as u32) {
                    let _ = lim.until_n_ready(nz).await;
                }
                remaining -= step;
            }
        }
    }
}

fn build_limiter(limit_bps: Option<u64>) -> Option<(SharedLimiter, u32)> {
    let bps = limit_bps.filter(|&v| v > 0)?;
    let clamped = bps.min(u32::MAX as u64).max(1024) as u32;
    let nz = NonZeroU32::new(clamped)?;
    let quota = Quota::per_second(nz);
    Some((Arc::new(RateLimiter::direct(quota)), clamped))
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub total_bytes: Option<u64>,
    pub supports_range: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub suggested_filename: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeChunk {
    index: usize,
    start: u64,
    end: u64,
    downloaded: u64,
    completed: bool,
    claimed: bool,
}

impl RuntimeChunk {
    fn from_state(s: &ChunkState) -> Self {
        Self {
            index: s.index,
            start: s.start,
            end: s.end,
            downloaded: s.downloaded,
            completed: s.completed,
            claimed: s.completed,
        }
    }

    fn to_state(&self) -> ChunkState {
        ChunkState {
            index: self.index,
            start: self.start,
            end: self.end,
            downloaded: self.downloaded,
            completed: self.completed,
        }
    }

    fn remaining(&self) -> u64 {
        let total = self.end.saturating_sub(self.start) + 1;
        total.saturating_sub(self.downloaded)
    }
}

struct ActiveJob {
    token: CancellationToken,
    meta: Arc<RwLock<DownloadTaskMeta>>,
    chunks: Arc<Mutex<Vec<RuntimeChunk>>>,
    downloaded_atomic: Arc<AtomicU64>,
    speed_bps: Arc<AtomicU64>,
    limiter: DynamicSpeedLimiter,
    handle: Option<JoinHandle<()>>,
}

pub struct HttpEngine {
    paths: StoragePaths,
    client: reqwest::Client,
    global_limiter: DynamicSpeedLimiter,
    jobs: Arc<RwLock<HashMap<Uuid, Arc<Mutex<ActiveJob>>>>>,
    progress_tx: broadcast::Sender<ProgressSnapshot>,
}

impl HttpEngine {
    pub fn new(paths: StoragePaths, global_speed_limit_bps: Option<u64>) -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(32)
            .build()?;
        let (progress_tx, _) = broadcast::channel(256);
        Ok(Self {
            paths,
            client,
            global_limiter: DynamicSpeedLimiter::new(global_speed_limit_bps),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            progress_tx,
        })
    }

    pub fn subscribe_progress(&self) -> broadcast::Receiver<ProgressSnapshot> {
        self.progress_tx.subscribe()
    }

    pub async fn set_global_speed_limit(&self, limit_bps: Option<u64>) {
        self.global_limiter.set_limit(limit_bps).await;
    }

    pub async fn set_task_speed_limit(&self, id: Uuid, limit_bps: Option<u64>) -> AppResult<()> {
        if let Some(job_lock) = self.jobs.read().await.get(&id).cloned() {
            let job = job_lock.lock().await;
            job.limiter.set_limit(limit_bps).await;
            let mut m = job.meta.write().await;
            m.speed_limit_bps = limit_bps;
            m.updated_at = Utc::now();
            meta::save_meta_async(&self.paths, &m).await?;
            return Ok(());
        }
        let mut m = meta::load_meta(&self.paths, id)?;
        m.speed_limit_bps = limit_bps;
        m.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &m).await
    }

    pub async fn suggest_filename(&self, req: &CreateDownloadRequest) -> Option<String> {
        let custom_headers = req.headers.clone().unwrap_or_default();
        let headers = build_request_headers(
            req.cookies.as_deref(),
            req.user_agent.as_deref(),
            req.referer.as_deref(),
            &custom_headers,
        )
        .ok()?;

        if let Ok(Ok(probe)) =
            tokio::time::timeout(Duration::from_secs(3), probe_url(&self.client, &req.url, &headers))
                .await
        {
            if let Some(name) = probe.suggested_filename {
                return Some(sanitize_filename(&name));
            }
        }

        let from_url = filename_from_url(&req.url);
        if Path::new(&from_url).extension().is_some() {
            Some(sanitize_filename(&from_url))
        } else {
            None
        }
    }

    pub async fn create_task(
        &self,
        req: CreateDownloadRequest,
        default_dir: &str,
        default_chunks: usize,
    ) -> AppResult<DownloadTaskMeta> {
        let parsed = Url::parse(&req.url).map_err(|_| {
            AppError::NotDownloadLink("This is not a valid download URL.".to_string())
        })?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(AppError::NotDownloadLink(
                "This is not a download link. Enter an http:// or https:// URL.".to_string(),
            ));
        }
        let id = Uuid::new_v4();
        let custom_headers = req.headers.unwrap_or_default();
        let headers = build_request_headers(
            req.cookies.as_deref(),
            req.user_agent.as_deref(),
            req.referer.as_deref(),
            &custom_headers,
        )?;

        let probe = probe_url(&self.client, &req.url, &headers).await?;
        let filename = resolve_final_filename(
            req.filename.as_deref(),
            probe.suggested_filename.as_deref(),
            &req.url,
        );
        let save_dir = req
            .save_dir
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| default_dir.to_string());

        let requested_chunks = req.max_chunks.unwrap_or(default_chunks).clamp(1, 32);
        let chunks = match (probe.total_bytes, probe.supports_range) {
            (Some(total), true) if total > 0 => split_chunks(total, requested_chunks),
            (Some(total), false) if total > 0 => vec![ChunkState {
                index: 0,
                start: 0,
                end: total - 1,
                downloaded: 0,
                completed: false,
            }],
            _ => Vec::new(),
        };

        let checksum_status = if req
            .expected_checksum
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            ChecksumStatus::Pending
        } else {
            ChecksumStatus::None
        };

        let now = Utc::now();
        let task = DownloadTaskMeta {
            id,
            url: req.url,
            filename,
            save_dir,
            engine: EngineKind::Http,
            status: DownloadStatus::Queued,
            total_bytes: probe.total_bytes,
            downloaded_bytes: 0,
            supports_range: probe.supports_range,
            etag: probe.etag,
            last_modified: probe.last_modified,
            referer: req.referer,
            user_agent: req.user_agent,
            cookies: req.cookies,
            headers: custom_headers,
            chunks,
            speed_limit_bps: req.speed_limit_bps,
            sequential_preview: req.sequential_preview.unwrap_or(false),
            expected_checksum: req.expected_checksum,
            checksum_algorithm: req.checksum_algorithm,
            checksum_status,
            auto_extract: req.auto_extract.unwrap_or(false),
            delete_archive_after_extract: req.delete_archive_after_extract.unwrap_or(false),
            ytdlp_format_id: None,
            torrent_selected_files: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };

        meta::save_meta_async(&self.paths, &task).await?;
        Ok(task)
    }

    pub async fn update_refreshed_link(
        &self,
        id: Uuid,
        new_url: String,
        cookies: Option<String>,
        user_agent: Option<String>,
        referer: Option<String>,
    ) -> AppResult<()> {
        let mut m = meta::load_meta(&self.paths, id)?;
        m.url = new_url;
        if cookies.is_some() {
            m.cookies = cookies;
        }
        if user_agent.is_some() {
            m.user_agent = user_agent;
        }
        if referer.is_some() {
            m.referer = referer;
        }
        m.error_message = None;
        m.status = DownloadStatus::Paused;
        m.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &m).await?;
        self.resume(id).await
    }

    async fn spawn_download_job(&self, mut task: DownloadTaskMeta) -> AppResult<()> {
        let id = task.id;
        let target_path = Path::new(&task.save_dir).join(&task.filename);
        let total_bytes = task.total_bytes;

        let file = tokio::task::spawn_blocking(move || -> AppResult<Arc<std::fs::File>> {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&target_path)?;
            if let Some(total) = total_bytes {
                if f.metadata()?.len() != total {
                    f.set_len(total)?;
                }
            }
            Ok(Arc::new(f))
        })
        .await
        .map_err(|e| AppError::Other(format!("File pre-allocation panicked: {e}")))??;

        task.status = DownloadStatus::Downloading;
        task.error_message = None;
        task.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &task).await?;

        let runtime_chunks: Vec<RuntimeChunk> =
            task.chunks.iter().map(RuntimeChunk::from_state).collect();
        let initial_downloaded: u64 = if runtime_chunks.is_empty() {
            task.downloaded_bytes
        } else {
            runtime_chunks.iter().map(|c| c.downloaded).sum()
        };

        let token = CancellationToken::new();
        let meta_lock = Arc::new(RwLock::new(task.clone()));
        let chunks_lock = Arc::new(Mutex::new(runtime_chunks));
        let downloaded_atomic = Arc::new(AtomicU64::new(initial_downloaded));
        let speed_bps = Arc::new(AtomicU64::new(0));
        let task_limiter = DynamicSpeedLimiter::new(task.speed_limit_bps);

        let job = Arc::new(Mutex::new(ActiveJob {
            token: token.clone(),
            meta: meta_lock.clone(),
            chunks: chunks_lock.clone(),
            downloaded_atomic: downloaded_atomic.clone(),
            speed_bps: speed_bps.clone(),
            limiter: task_limiter.clone(),
            handle: None,
        }));

        self.jobs.write().await.insert(id, job.clone());

        let paths = self.paths.clone();
        let client = self.client.clone();
        let global_limiter = self.global_limiter.clone();
        let jobs_map = self.jobs.clone();
        let progress_tx = self.progress_tx.clone();

        let join_handle = tokio::spawn(async move {
            let agg_token = token.child_token();
            let agg_handle = spawn_progress_aggregator(
                id,
                agg_token.clone(),
                paths.clone(),
                meta_lock.clone(),
                chunks_lock.clone(),
                downloaded_atomic.clone(),
                speed_bps.clone(),
                progress_tx.clone(),
            );

            let outcome = if task.supports_range && !task.chunks.is_empty() {
                run_multi_chunk_workers(
                    client,
                    file,
                    meta_lock.clone(),
                    chunks_lock.clone(),
                    downloaded_atomic.clone(),
                    global_limiter,
                    task_limiter,
                    token.clone(),
                )
                .await
            } else {
                run_single_stream_worker(
                    client,
                    file,
                    meta_lock.clone(),
                    downloaded_atomic.clone(),
                    global_limiter,
                    task_limiter,
                    token.clone(),
                )
                .await
            };

            agg_token.cancel();
            let _ = agg_handle.await;

            let mut final_meta = meta_lock.write().await;
            let final_chunks: Vec<ChunkState> = {
                let guard = chunks_lock.lock().await;
                guard.iter().map(RuntimeChunk::to_state).collect()
            };
            final_meta.chunks = final_chunks;
            final_meta.downloaded_bytes = downloaded_atomic.load(Ordering::Relaxed);
            final_meta.updated_at = Utc::now();

            if token.is_cancelled() {
                if final_meta.status != DownloadStatus::Canceled {
                    final_meta.status = DownloadStatus::Paused;
                }
            } else {
                match outcome {
                    Ok(()) => {
                        let all_done = final_meta.chunks.is_empty()
                            || final_meta.chunks.iter().all(|c| c.completed);
                        if all_done {
                            final_meta.status = DownloadStatus::Completed;
                            final_meta.error_message = None;
                        } else {
                            final_meta.status = DownloadStatus::Failed;
                            final_meta.error_message =
                                Some("Download ended with incomplete chunks".to_string());
                        }
                    }
                    Err(AppError::LinkExpired(code)) => {
                        final_meta.status = DownloadStatus::NeedsRefresh;
                        final_meta.error_message = Some(format!("HTTP {code}: link expired"));
                    }
                    Err(e) => {
                        final_meta.status = DownloadStatus::Failed;
                        final_meta.error_message = Some(e.to_string());
                    }
                }
            }

            let _ = meta::save_meta_async(&paths, &final_meta).await;
            let _ = progress_tx.send(ProgressSnapshot {
                id,
                status: final_meta.status,
                downloaded_bytes: final_meta.downloaded_bytes,
                total_bytes: final_meta.total_bytes,
                speed_bps: 0,
                eta_seconds: None,
                chunks: final_meta.chunks.clone(),
                checksum_status: final_meta.checksum_status,
                error_message: final_meta.error_message.clone(),
            });

            jobs_map.write().await.remove(&id);
        });

        job.lock().await.handle = Some(join_handle);
        Ok(())
    }
}

#[async_trait]
impl DownloaderEngine for HttpEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Http
    }

    async fn start(&self, id: Uuid) -> AppResult<()> {
        let task = meta::load_meta(&self.paths, id)?;
        self.spawn_download_job(task).await
    }

    async fn pause(&self, id: Uuid) -> AppResult<()> {
        let job_opt = self.jobs.read().await.get(&id).cloned();
        if let Some(job_lock) = job_opt {
            let handle = {
                let mut job = job_lock.lock().await;
                job.token.cancel();
                job.handle.take()
            };
            if let Some(h) = handle {
                let _ = h.await;
            }
            return Ok(());
        }
        let mut m = meta::load_meta(&self.paths, id)?;
        if m.status == DownloadStatus::Queued || m.status == DownloadStatus::Downloading {
            m.status = DownloadStatus::Paused;
            m.updated_at = Utc::now();
            meta::save_meta_async(&self.paths, &m).await?;
        }
        Ok(())
    }

    async fn resume(&self, id: Uuid) -> AppResult<()> {
        if self.jobs.read().await.contains_key(&id) {
            return Ok(());
        }
        let mut task = meta::load_meta(&self.paths, id)?;
        let headers = build_request_headers(
            task.cookies.as_deref(),
            task.user_agent.as_deref(),
            task.referer.as_deref(),
            &task.headers,
        )?;

        match probe_url(&self.client, &task.url, &headers).await {
            Ok(probe) => {
                if let (Some(old_etag), Some(new_etag)) = (&task.etag, &probe.etag) {
                    if old_etag != new_etag {
                        task.status = DownloadStatus::Failed;
                        task.error_message = Some(AppError::SourceChanged.to_string());
                        meta::save_meta_async(&self.paths, &task).await?;
                        return Err(AppError::SourceChanged);
                    }
                }
                if task.total_bytes.is_none() {
                    task.total_bytes = probe.total_bytes;
                }
                for chunk in &mut task.chunks {
                    let expected_len = chunk.end.saturating_sub(chunk.start) + 1;
                    if chunk.downloaded >= expected_len {
                        chunk.completed = true;
                    }
                }
                self.spawn_download_job(task).await
            }
            Err(AppError::LinkExpired(code)) => {
                task.status = DownloadStatus::NeedsRefresh;
                task.error_message = Some(format!("HTTP {code}: link expired"));
                task.updated_at = Utc::now();
                meta::save_meta_async(&self.paths, &task).await?;
                Err(AppError::LinkExpired(code))
            }
            Err(e) => Err(e),
        }
    }

    async fn cancel(&self, id: Uuid, delete_partial: bool) -> AppResult<()> {
        let job_opt = self.jobs.read().await.get(&id).cloned();
        if let Some(job_lock) = job_opt {
            let handle = {
                let mut job = job_lock.lock().await;
                {
                    let mut m = job.meta.write().await;
                    m.status = DownloadStatus::Canceled;
                }
                job.token.cancel();
                job.handle.take()
            };
            if let Some(h) = handle {
                let _ = h.await;
            }
        }
        if let Ok(m) = meta::load_meta(&self.paths, id) {
            if delete_partial {
                let target = PathBuf::from(&m.save_dir).join(&m.filename);
                if target.exists() {
                    let _ = tokio::fs::remove_file(target).await;
                }
            }
            let _ = meta::delete_meta(&self.paths, id);
        }
        Ok(())
    }

    async fn progress_snapshot(&self, id: Uuid) -> AppResult<ProgressSnapshot> {
        if let Some(job_lock) = self.jobs.read().await.get(&id).cloned() {
            let job = job_lock.lock().await;
            let m = job.meta.read().await;
            let chunks: Vec<ChunkState> = job
                .chunks
                .lock()
                .await
                .iter()
                .map(RuntimeChunk::to_state)
                .collect();
            let downloaded = job.downloaded_atomic.load(Ordering::Relaxed);
            let speed = job.speed_bps.load(Ordering::Relaxed);
            let eta = compute_eta(m.total_bytes, downloaded, speed);
            return Ok(ProgressSnapshot {
                id,
                status: m.status,
                downloaded_bytes: downloaded,
                total_bytes: m.total_bytes,
                speed_bps: speed,
                eta_seconds: eta,
                chunks,
                checksum_status: m.checksum_status,
                error_message: m.error_message.clone(),
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

#[allow(clippy::too_many_arguments)]
fn spawn_progress_aggregator(
    id: Uuid,
    token: CancellationToken,
    paths: StoragePaths,
    meta_lock: Arc<RwLock<DownloadTaskMeta>>,
    chunks_lock: Arc<Mutex<Vec<RuntimeChunk>>>,
    downloaded_atomic: Arc<AtomicU64>,
    speed_bps: Arc<AtomicU64>,
    progress_tx: broadcast::Sender<ProgressSnapshot>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(PROGRESS_INTERVAL_MS));
        let mut last_bytes = downloaded_atomic.load(Ordering::Relaxed);
        let mut smoothed_speed: f64 = 0.0;
        let mut tick_count: u64 = 0;

        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = interval.tick() => {
                    tick_count += 1;
                    let current = downloaded_atomic.load(Ordering::Relaxed);
                    let delta = current.saturating_sub(last_bytes);
                    last_bytes = current;
                    let inst_speed = (delta * 1000) / PROGRESS_INTERVAL_MS;
                    if smoothed_speed <= 0.0 {
                        smoothed_speed = inst_speed as f64;
                    } else {
                        smoothed_speed = smoothed_speed * 0.94 + (inst_speed as f64) * 0.06;
                    }
                    let eta_speed = smoothed_speed.round() as u64;
                    speed_bps.store(eta_speed, Ordering::Relaxed);

                    let chunks: Vec<ChunkState> = {
                        let guard = chunks_lock.lock().await;
                        guard.iter().map(RuntimeChunk::to_state).collect()
                    };

                    let snapshot = {
                        let mut m = meta_lock.write().await;
                        m.downloaded_bytes = current;
                        m.chunks = chunks.clone();
                        m.updated_at = Utc::now();
                        if tick_count.is_multiple_of(6) {
                            let _ = meta::save_meta_async(&paths, &m).await;
                        }
                        ProgressSnapshot {
                            id,
                            status: m.status,
                            downloaded_bytes: current,
                            total_bytes: m.total_bytes,
                            speed_bps: eta_speed,
                            eta_seconds: compute_eta(m.total_bytes, current, eta_speed),
                            chunks,
                            checksum_status: m.checksum_status,
                            error_message: m.error_message.clone(),
                        }
                    };
                    let _ = progress_tx.send(snapshot);
                }
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_multi_chunk_workers(
    client: reqwest::Client,
    file: Arc<std::fs::File>,
    meta_lock: Arc<RwLock<DownloadTaskMeta>>,
    chunks_lock: Arc<Mutex<Vec<RuntimeChunk>>>,
    downloaded_atomic: Arc<AtomicU64>,
    global_limiter: DynamicSpeedLimiter,
    task_limiter: DynamicSpeedLimiter,
    token: CancellationToken,
) -> AppResult<()> {
    let (url, headers, worker_count, sequential) = {
        let m = meta_lock.read().await;
        let headers = build_request_headers(
            m.cookies.as_deref(),
            m.user_agent.as_deref(),
            m.referer.as_deref(),
            &m.headers,
        )?;
        let count = m.chunks.len().clamp(1, 32);
        (m.url.clone(), headers, count, m.sequential_preview)
    };

    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let client = client.clone();
        let file = file.clone();
        let url = url.clone();
        let headers = headers.clone();
        let chunks_lock = chunks_lock.clone();
        let downloaded_atomic = downloaded_atomic.clone();
        let global_limiter = global_limiter.clone();
        let task_limiter = task_limiter.clone();
        let token = token.clone();

        handles.push(tokio::spawn(async move {
            loop {
                if token.is_cancelled() {
                    return Ok(());
                }

                let chunk_idx = {
                    let mut guard = chunks_lock.lock().await;
                    pick_next_or_steal_chunk(&mut guard, sequential)
                };

                let Some(idx) = chunk_idx else {
                    return Ok(());
                };

                download_single_chunk(
                    &client,
                    &file,
                    &url,
                    &headers,
                    idx,
                    &chunks_lock,
                    &downloaded_atomic,
                    &global_limiter,
                    &task_limiter,
                    &token,
                )
                .await?;
            }
        }));
    }

    let mut first_err: Option<AppError> = None;
    for h in handles {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() {
                    first_err = Some(e);
                    token.cancel();
                }
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(AppError::Other(format!("Worker panicked: {e}")));
                    token.cancel();
                }
            }
        }
    }

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn download_single_chunk(
    client: &reqwest::Client,
    file: &Arc<std::fs::File>,
    url: &str,
    headers: &HeaderMap,
    chunk_idx: usize,
    chunks_lock: &Arc<Mutex<Vec<RuntimeChunk>>>,
    downloaded_atomic: &Arc<AtomicU64>,
    global_limiter: &DynamicSpeedLimiter,
    task_limiter: &DynamicSpeedLimiter,
    token: &CancellationToken,
) -> AppResult<()> {
    let (mut offset, mut current_end) = {
        let guard = chunks_lock.lock().await;
        let c = &guard[chunk_idx];
        (c.start + c.downloaded, c.end)
    };

    if offset > current_end {
        let mut guard = chunks_lock.lock().await;
        guard[chunk_idx].completed = true;
        return Ok(());
    }

    let mut req_headers = headers.clone();
    let range_val = format!("bytes={offset}-{current_end}");
    req_headers.insert(
        RANGE,
        HeaderValue::from_str(&range_val)
            .map_err(|e| AppError::Other(format!("Invalid range header: {e}")))?,
    );

    let resp = client.get(url).headers(req_headers).send().await?;
    let status = resp.status().as_u16();
    if status == 403 || status == 410 {
        return Err(AppError::LinkExpired(status));
    }
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("HTTP error {status}")));
    }

    let mut stream = resp.bytes_stream();

    while let Some(item) = tokio::select! {
        _ = token.cancelled() => None,
        next = stream.next() => next,
    } {
        let bytes = item?;
        if bytes.is_empty() {
            continue;
        }

        // Refresh current_end in case another worker stole part of our tail.
        current_end = {
            let guard = chunks_lock.lock().await;
            guard[chunk_idx].end
        };

        if offset > current_end {
            break;
        }

        let max_allowed = (current_end - offset + 1) as usize;
        let to_write = bytes.len().min(max_allowed);
        if to_write == 0 {
            break;
        }

        global_limiter.wait_for_bytes(to_write).await;
        task_limiter.wait_for_bytes(to_write).await;

        let slice = bytes[..to_write].to_vec();
        let write_pos = offset;
        let file_clone = file.clone();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut written = 0usize;
            while written < slice.len() {
                let n = file_clone.seek_write(&slice[written..], write_pos + written as u64)?;
                if n == 0 {
                    return Err(AppError::Other("Zero bytes written to disk".to_string()));
                }
                written += n;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::Other(format!("Disk write task panicked: {e}")))??;

        offset += to_write as u64;
        downloaded_atomic.fetch_add(to_write as u64, Ordering::Relaxed);

        let mut guard = chunks_lock.lock().await;
        let c = &mut guard[chunk_idx];
        c.downloaded = offset.saturating_sub(c.start);
        if offset > c.end {
            c.completed = true;
            break;
        }
    }

    if !token.is_cancelled() {
        let mut guard = chunks_lock.lock().await;
        let c = &mut guard[chunk_idx];
        if offset > c.end {
            c.completed = true;
        } else {
            return Err(AppError::Other(format!(
                "Chunk {} ended early at offset {} (expected > {})",
                c.index, offset, c.end
            )));
        }
    }

    Ok(())
}

async fn run_single_stream_worker(
    client: reqwest::Client,
    file: Arc<std::fs::File>,
    meta_lock: Arc<RwLock<DownloadTaskMeta>>,
    downloaded_atomic: Arc<AtomicU64>,
    global_limiter: DynamicSpeedLimiter,
    task_limiter: DynamicSpeedLimiter,
    token: CancellationToken,
) -> AppResult<()> {
    let (url, headers) = {
        let m = meta_lock.read().await;
        let headers = build_request_headers(
            m.cookies.as_deref(),
            m.user_agent.as_deref(),
            m.referer.as_deref(),
            &m.headers,
        )?;
        (m.url.clone(), headers)
    };

    let resp = client.get(&url).headers(headers).send().await?;
    let status = resp.status().as_u16();
    if status == 403 || status == 410 {
        return Err(AppError::LinkExpired(status));
    }
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("HTTP error {status}")));
    }

    let mut stream = resp.bytes_stream();
    let mut offset: u64 = 0;

    while let Some(item) = tokio::select! {
        _ = token.cancelled() => None,
        next = stream.next() => next,
    } {
        let bytes = item?;
        if bytes.is_empty() {
            continue;
        }
        let len = bytes.len();
        global_limiter.wait_for_bytes(len).await;
        task_limiter.wait_for_bytes(len).await;

        let buf = bytes.to_vec();
        let write_pos = offset;
        let file_clone = file.clone();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut written = 0usize;
            while written < buf.len() {
                let n = file_clone.seek_write(&buf[written..], write_pos + written as u64)?;
                if n == 0 {
                    return Err(AppError::Other("Zero bytes written to disk".to_string()));
                }
                written += n;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::Other(format!("Disk write task panicked: {e}")))??;

        offset += len as u64;
        downloaded_atomic.store(offset, Ordering::Relaxed);
    }

    if token.is_cancelled() {
        return Err(AppError::Other("Paused".to_string()));
    }

    let mut m = meta_lock.write().await;
    if m.total_bytes.is_none() {
        m.total_bytes = Some(offset);
    }
    Ok(())
}

fn pick_next_or_steal_chunk(chunks: &mut Vec<RuntimeChunk>, sequential: bool) -> Option<usize> {
    if sequential && !chunks.is_empty() {
        if !chunks[0].completed && !chunks[0].claimed {
            chunks[0].claimed = true;
            return Some(0);
        }
        let last = chunks.len() - 1;
        if !chunks[last].completed && !chunks[last].claimed {
            chunks[last].claimed = true;
            return Some(last);
        }
    }

    if let Some((idx, _)) = chunks
        .iter()
        .enumerate()
        .find(|(_, c)| !c.completed && !c.claimed)
    {
        chunks[idx].claimed = true;
        return Some(idx);
    }

    // Dynamic work-stealing: split the slowest active chunk in half.
    let victim_idx = chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.completed && c.remaining() >= MIN_STEAL_REMAINING)
        .max_by_key(|(_, c)| c.remaining())
        .map(|(idx, _)| idx)?;

    let victim = &mut chunks[victim_idx];
    let current_pos = victim.start + victim.downloaded;
    let remaining = victim.end.saturating_sub(current_pos) + 1;
    let split_len = remaining / 2;
    if split_len < MIN_CHUNK_SIZE {
        return None;
    }

    let old_end = victim.end;
    let victim_new_end = current_pos + split_len - 1;
    let new_start = victim_new_end + 1;
    victim.end = victim_new_end;

    let new_idx = chunks.len();
    chunks.push(RuntimeChunk {
        index: new_idx,
        start: new_start,
        end: old_end,
        downloaded: 0,
        completed: false,
        claimed: true,
    });
    Some(new_idx)
}

pub fn split_chunks(total_bytes: u64, max_chunks: usize) -> Vec<ChunkState> {
    if total_bytes == 0 {
        return Vec::new();
    }
    let max_by_size = (total_bytes / MIN_CHUNK_SIZE).max(1) as usize;
    let count = max_chunks.clamp(1, 32).min(max_by_size);
    let base = total_bytes / count as u64;
    let rem = total_bytes % count as u64;

    let mut result = Vec::with_capacity(count);
    let mut cursor = 0u64;
    for i in 0..count {
        let extra = if (i as u64) < rem { 1 } else { 0 };
        let len = base + extra;
        let end = cursor + len - 1;
        result.push(ChunkState {
            index: i,
            start: cursor,
            end,
            downloaded: 0,
            completed: false,
        });
        cursor = end + 1;
    }
    result
}

pub async fn probe_url(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
) -> AppResult<ProbeResult> {
    let head_resp = client.head(url).headers(headers.clone()).send().await;
    if let Ok(resp) = head_resp {
        let code = resp.status().as_u16();
        if code == 403 || code == 410 {
            return Err(AppError::LinkExpired(code));
        }
        if resp.status().is_success() {
            let hdrs = resp.headers();
            ensure_downloadable_headers(hdrs)?;
            let total_bytes = hdrs
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            let supports_range = hdrs
                .get(ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));
            let etag = hdrs
                .get(ETAG)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let last_modified = hdrs
                .get(LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let suggested_filename = parse_content_disposition(hdrs);

            if total_bytes.is_some() || supports_range {
                return Ok(ProbeResult {
                    total_bytes,
                    supports_range,
                    etag,
                    last_modified,
                    suggested_filename,
                });
            }
        }
    }

    // Fallback: GET bytes=0-0 for servers that reject HEAD or omit Content-Length on HEAD.
    let mut range_headers = headers.clone();
    range_headers.insert(RANGE, HeaderValue::from_static("bytes=0-0"));
    let resp = client.get(url).headers(range_headers).send().await?;
    let code = resp.status().as_u16();
    if code == 403 || code == 410 {
        return Err(AppError::LinkExpired(code));
    }
    if !resp.status().is_success() {
        return Err(AppError::Other(format!("Probe failed with HTTP {code}")));
    }

    let hdrs = resp.headers();
    ensure_downloadable_headers(hdrs)?;
    let supports_range = code == 206;
    let total_bytes = parse_content_range_total(hdrs).or_else(|| {
        hdrs.get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });
    let etag = hdrs
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let last_modified = hdrs
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let suggested_filename = parse_content_disposition(hdrs);

    Ok(ProbeResult {
        total_bytes,
        supports_range,
        etag,
        last_modified,
        suggested_filename,
    })
}

fn ensure_downloadable_headers(headers: &HeaderMap) -> AppResult<()> {
    let is_attachment = headers
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("attachment"));
    if is_attachment {
        return Ok(());
    }

    if let Some(ct) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
        let lower = ct.trim().to_ascii_lowercase();
        if lower.starts_with("text/html") || lower.starts_with("application/xhtml+xml") {
            return Err(AppError::NotDownloadLink(
                "This URL points to a web page (HTML), not a download link. Use the Video (yt-dlp) tab for media pages.".to_string(),
            ));
        }
    }

    Ok(())
}

fn parse_content_range_total(headers: &HeaderMap) -> Option<u64> {
    let val = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let slash_pos = val.rfind('/')?;
    let total_str = val[slash_pos + 1..].trim();
    if total_str == "*" {
        return None;
    }
    total_str.parse::<u64>().ok()
}

fn parse_content_disposition(headers: &HeaderMap) -> Option<String> {
    let val = headers.get(CONTENT_DISPOSITION)?.to_str().ok()?;
    let mut fallback: Option<String> = None;

    for part in val.split(';') {
        let trimmed = part.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("filename*=") {
            let raw = trimmed["filename*=".len()..].trim().trim_matches('"');
            let encoded = raw
                .split_once("''")
                .map(|(_, rest)| rest)
                .unwrap_or(raw);
            let decoded = percent_decode_utf8(encoded);
            let cleaned = decoded.trim();
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        } else if lower.starts_with("filename=") && fallback.is_none() {
            let cleaned = trimmed["filename=".len()..].trim().trim_matches('"').trim();
            if !cleaned.is_empty() {
                fallback = Some(cleaned.to_string());
            }
        }
    }
    fallback
}

fn percent_decode_utf8(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn build_request_headers(
    cookies: Option<&str>,
    user_agent: Option<&str>,
    referer: Option<&str>,
    extra: &HashMap<String, String>,
) -> AppResult<HeaderMap> {
    let mut map = HeaderMap::new();
    for (k, v) in extra {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            map.insert(name, val);
        }
    }
    if let Some(c) = cookies.filter(|s| !s.trim().is_empty()) {
        if let Ok(v) = HeaderValue::from_str(c) {
            map.insert(COOKIE, v);
        }
    }
    let ua = user_agent
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Kestrel/0.1");
    if let Ok(v) = HeaderValue::from_str(ua) {
        map.insert(USER_AGENT, v);
    }
    if let Some(r) = referer.filter(|s| !s.trim().is_empty()) {
        if let Ok(v) = HeaderValue::from_str(r) {
            map.insert(REFERER, v);
        }
    }
    Ok(map)
}

fn resolve_final_filename(custom: Option<&str>, detected: Option<&str>, url: &str) -> String {
    let detected_name = detected
        .filter(|s| !s.trim().is_empty())
        .map(sanitize_filename)
        .unwrap_or_else(|| sanitize_filename(&filename_from_url(url)));

    if let Some(custom_raw) = custom.filter(|s| !s.trim().is_empty()) {
        let custom_clean = sanitize_filename(custom_raw);
        let custom_has_ext = Path::new(&custom_clean)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| !e.is_empty());
        if !custom_has_ext {
            if let Some(ext) = Path::new(&detected_name)
                .extension()
                .and_then(|e| e.to_str())
                .filter(|e| !e.is_empty())
            {
                return format!("{custom_clean}.{ext}");
            }
        }
        return custom_clean;
    }

    detected_name
}

fn filename_from_url(raw_url: &str) -> String {
    if let Ok(parsed) = Url::parse(raw_url) {
        if let Some(mut segs) = parsed.path_segments() {
            if let Some(last) = segs.next_back() {
                if !last.is_empty() {
                    return last.to_string();
                }
            }
        }
    }
    "download.bin".to_string()
}

fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "download.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn compute_eta(total: Option<u64>, downloaded: u64, speed_bps: u64) -> Option<u64> {
    let total = total?;
    if speed_bps == 0 || downloaded >= total {
        return None;
    }
    Some((total - downloaded) / speed_bps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::State,
        http::{header, HeaderMap as AxumHeaders, StatusCode},
        response::Response,
        routing::get,
        Router,
    };
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    #[test]
    fn split_chunks_covers_entire_file_contiguously() {
        let total = 10_000_007u64;
        let chunks = split_chunks(total, 8);
        assert_eq!(chunks.len(), 8);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks.last().unwrap().end, total - 1);
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].end + 1, pair[1].start);
        }
        let sum: u64 = chunks.iter().map(|c| c.end - c.start + 1).sum();
        assert_eq!(sum, total);
    }

    #[test]
    fn work_stealing_splits_slowest_chunk() {
        let mut chunks = vec![
            RuntimeChunk {
                index: 0,
                start: 0,
                end: 1024 * 1024 - 1,
                downloaded: 1024 * 1024,
                completed: true,
                claimed: true,
            },
            RuntimeChunk {
                index: 1,
                start: 1024 * 1024,
                end: 3 * 1024 * 1024 - 1,
                downloaded: 0,
                completed: false,
                claimed: true,
            },
        ];

        let stolen = pick_next_or_steal_chunk(&mut chunks, false);
        assert_eq!(stolen, Some(2));
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[1].end + 1, chunks[2].start);
        assert_eq!(chunks[2].end, 3 * 1024 * 1024 - 1);
    }

    #[test]
    fn sequential_preview_picks_first_and_last_chunks_first() {
        let states = split_chunks(4 * 1024 * 1024, 4);
        let mut chunks: Vec<RuntimeChunk> =
            states.iter().map(RuntimeChunk::from_state).collect();
        assert_eq!(pick_next_or_steal_chunk(&mut chunks, true), Some(0));
        assert_eq!(pick_next_or_steal_chunk(&mut chunks, true), Some(3));
        assert_eq!(pick_next_or_steal_chunk(&mut chunks, true), Some(1));
        assert_eq!(pick_next_or_steal_chunk(&mut chunks, true), Some(2));
    }

    #[tokio::test]
    async fn dynamic_speed_limiter_handles_burst_larger_than_quota() {
        let limiter = DynamicSpeedLimiter::new(Some(2048));
        limiter.wait_for_bytes(3072).await;
        limiter.set_limit(None).await;
        limiter.wait_for_bytes(16384).await;
    }

    async fn range_handler(
        State(payload): State<Arc<Vec<u8>>>,
        headers: AxumHeaders,
    ) -> Response {
        let total = payload.len();
        if let Some(range_hdr) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
            if let Some(spec) = range_hdr.strip_prefix("bytes=") {
                let parts: Vec<&str> = spec.split('-').collect();
                if parts.len() == 2 {
                    let start: usize = parts[0].parse().unwrap_or(0);
                    let end: usize = parts[1].parse().unwrap_or(total - 1).min(total - 1);
                    if start <= end && end < total {
                        let slice = payload[start..=end].to_vec();
                        return Response::builder()
                            .status(StatusCode::PARTIAL_CONTENT)
                            .header(header::ACCEPT_RANGES, "bytes")
                            .header(header::ETAG, "\"test-etag-1\"")
                            .header(
                                header::CONTENT_RANGE,
                                format!("bytes {start}-{end}/{total}"),
                            )
                            .header(header::CONTENT_LENGTH, slice.len().to_string())
                            .body(Body::from(slice))
                            .unwrap();
                    }
                }
            }
        }
        Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::ETAG, "\"test-etag-1\"")
            .header(header::CONTENT_LENGTH, total.to_string())
            .body(Body::from((*payload).clone()))
            .unwrap()
    }

    #[tokio::test]
    async fn http_download_pause_and_resume_matches_sha256() {
        let size = 2 * 1024 * 1024 + 12345;
        let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let expected_hash = hex::encode(Sha256::digest(&payload));

        let app = Router::new()
            .route("/file.bin", get(range_handler).head(range_handler))
            .with_state(Arc::new(payload));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let dir = tempdir().unwrap();
        let paths = StoragePaths::from_base(dir.path().join("state")).unwrap();
        let save_dir = dir.path().join("downloads");
        let engine = HttpEngine::new(paths.clone(), None).unwrap();

        let task = engine
            .create_task(
                CreateDownloadRequest {
                    url: format!("http://{addr}/file.bin"),
                    filename: Some("file.bin".to_string()),
                    save_dir: Some(save_dir.to_string_lossy().to_string()),
                    engine: None,
                    cookies: None,
                    user_agent: None,
                    referer: None,
                    headers: None,
                    max_chunks: Some(4),
                    speed_limit_bps: Some(256 * 1024),
                    sequential_preview: None,
                    expected_checksum: None,
                    checksum_algorithm: None,
                    auto_extract: None,
                    delete_archive_after_extract: None,
                    ytdlp_format_id: None,
                    torrent_selected_files: None,
                },
                &save_dir.to_string_lossy(),
                4,
            )
            .await
            .unwrap();

        engine.start(task.id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        engine.pause(task.id).await.unwrap();

        let paused_meta = meta::load_meta(&paths, task.id).unwrap();
        assert_eq!(paused_meta.status, DownloadStatus::Paused);

        engine.set_task_speed_limit(task.id, None).await.unwrap();
        engine.resume(task.id).await.unwrap();
        for _ in 0..80 {
            let m = meta::load_meta(&paths, task.id).unwrap();
            if m.status == DownloadStatus::Completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let final_meta = meta::load_meta(&paths, task.id).unwrap();
        assert_eq!(final_meta.status, DownloadStatus::Completed);

        let downloaded = std::fs::read(save_dir.join("file.bin")).unwrap();
        assert_eq!(downloaded.len(), size);
        let actual_hash = hex::encode(Sha256::digest(&downloaded));
        assert_eq!(actual_hash, expected_hash);
    }

    #[tokio::test]
    async fn rejects_html_webpage_as_not_download_link() {
        async fn html_handler() -> Response {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CONTENT_LENGTH, "25")
                .body(Body::from("<html><body>Hi</body></html>"))
                .unwrap()
        }

        let app = Router::new().route("/page", get(html_handler).head(html_handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let dir = tempdir().unwrap();
        let paths = StoragePaths::from_base(dir.path().join("state")).unwrap();
        let engine = HttpEngine::new(paths, None).unwrap();

        let err = engine
            .create_task(
                CreateDownloadRequest {
                    url: format!("http://{addr}/page"),
                    filename: None,
                    save_dir: None,
                    engine: None,
                    cookies: None,
                    user_agent: None,
                    referer: None,
                    headers: None,
                    max_chunks: None,
                    speed_limit_bps: None,
                    sequential_preview: None,
                    expected_checksum: None,
                    checksum_algorithm: None,
                    auto_extract: None,
                    delete_archive_after_extract: None,
                    ytdlp_format_id: None,
                    torrent_selected_files: None,
                },
                &dir.path().to_string_lossy(),
                4,
            )
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::NotDownloadLink(_)));
    }

    #[test]
    fn preserves_detected_extension_when_custom_filename_omits_it() {
        assert_eq!(
            resolve_final_filename(
                Some("tes-tes"),
                Some("Silent Partner - Dog Park.mp3"),
                "https://example.com/audio?id=123",
            ),
            "tes-tes.mp3"
        );
        assert_eq!(
            resolve_final_filename(
                Some("custom.wav"),
                Some("Silent Partner - Dog Park.mp3"),
                "https://example.com/audio?id=123",
            ),
            "custom.wav"
        );
    }

    #[test]
    fn parses_content_disposition_standard_and_rfc5987() {
        let mut hdrs = HeaderMap::new();
        hdrs.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static(
                "attachment; filename=\"fallback.mp3\"; filename*=UTF-8''Silent%20Partner%20-%20Dog%20Park.mp3",
            ),
        );
        assert_eq!(
            parse_content_disposition(&hdrs).as_deref(),
            Some("Silent Partner - Dog Park.mp3")
        );
    }
}
