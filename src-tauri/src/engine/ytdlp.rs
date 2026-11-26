use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::contracts::{
    ChecksumStatus, CreateDownloadRequest, DownloadStatus, DownloadTaskMeta, DownloaderEngine,
    EngineKind, ProgressSnapshot, YtdlpFormat, YtdlpMediaInfo,
};
use crate::error::{AppError, AppResult};
use crate::sidecars;
use crate::storage::{meta, StoragePaths};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROGRESS_INTERVAL_MS: u64 = 350;

#[derive(Deserialize)]
struct RawYtdlpInfo {
    title: Option<String>,
    uploader: Option<String>,
    duration: Option<f64>,
    #[serde(default)]
    formats: Vec<RawYtdlpFormat>,
    #[serde(default)]
    subtitles: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RawYtdlpFormat {
    format_id: Option<String>,
    ext: Option<String>,
    resolution: Option<String>,
    width: Option<u64>,
    height: Option<u64>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    format_note: Option<String>,
    vcodec: Option<String>,
    acodec: Option<String>,
}

struct ActiveYtdlpJob {
    token: CancellationToken,
    meta: Arc<RwLock<DownloadTaskMeta>>,
    downloaded_atomic: Arc<AtomicU64>,
    total_atomic: Arc<AtomicU64>,
    speed_atomic: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

pub struct YtdlpEngine {
    paths: StoragePaths,
    global_limit_bps: Arc<AtomicU64>,
    jobs: Arc<RwLock<HashMap<Uuid, Arc<Mutex<ActiveYtdlpJob>>>>>,
    progress_tx: broadcast::Sender<ProgressSnapshot>,
}

impl YtdlpEngine {
    pub fn new(paths: StoragePaths, global_limit_bps: Option<u64>) -> Self {
        let (progress_tx, _) = broadcast::channel(256);
        Self {
            paths,
            global_limit_bps: Arc::new(AtomicU64::new(global_limit_bps.unwrap_or(0))),
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

    pub async fn probe_formats(&self, url: &str) -> AppResult<YtdlpMediaInfo> {
        let bin = sidecars::resolve_sidecar("yt-dlp").ok_or_else(|| AppError::MissingBinary {
            binary: "yt-dlp".to_string(),
            hint: "Place yt-dlp-x86_64-pc-windows-msvc.exe in src-tauri/binaries/".to_string(),
        })?;

        let mut cmd = Command::new(bin);
        cmd.args(["-J", "--no-playlist", "--no-warnings", url])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Other(format!(
                "yt-dlp probe failed: {}",
                err.trim()
            )));
        }

        let raw: RawYtdlpInfo = serde_json::from_slice(&output.stdout)?;
        let title = raw
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "video".to_string());

        let formats: Vec<YtdlpFormat> = raw
            .formats
            .into_iter()
            .filter_map(|f| {
                let format_id = f.format_id?;
                let ext = f.ext.unwrap_or_else(|| "mp4".to_string());
                let has_video = f.vcodec.as_deref().is_some_and(|v| v != "none");
                let has_audio = f.acodec.as_deref().is_some_and(|a| a != "none");
                if !has_video && !has_audio {
                    return None;
                }
                let resolution = f.resolution.unwrap_or_else(|| {
                    if let (Some(w), Some(h)) = (f.width, f.height) {
                        format!("{w}x{h}")
                    } else if has_audio && !has_video {
                        "audio only".to_string()
                    } else {
                        "unknown".to_string()
                    }
                });
                Some(YtdlpFormat {
                    format_id,
                    ext,
                    resolution,
                    filesize: f.filesize.or(f.filesize_approx),
                    note: f.format_note.unwrap_or_default(),
                    has_video,
                    has_audio,
                })
            })
            .collect();

        let mut subtitles: Vec<String> = raw.subtitles.keys().cloned().collect();
        subtitles.sort();

        Ok(YtdlpMediaInfo {
            title,
            uploader: raw.uploader,
            duration_seconds: raw.duration.map(|d| d.max(0.0) as u64),
            formats,
            subtitles,
        })
    }

    pub async fn create_task(
        &self,
        req: CreateDownloadRequest,
        default_dir: &str,
    ) -> AppResult<DownloadTaskMeta> {
        if sidecars::resolve_sidecar("yt-dlp").is_none() {
            return Err(AppError::MissingBinary {
                binary: "yt-dlp".to_string(),
                hint: "Place yt-dlp-x86_64-pc-windows-msvc.exe in src-tauri/binaries/".to_string(),
            });
        }

        let id = Uuid::new_v4();
        let save_dir = req
            .save_dir
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| default_dir.to_string());
        let filename = req
            .filename
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| "video.mp4".to_string());

        let now = Utc::now();
        let task = DownloadTaskMeta {
            id,
            url: req.url,
            filename,
            save_dir,
            engine: EngineKind::Ytdlp,
            status: DownloadStatus::Queued,
            total_bytes: None,
            downloaded_bytes: 0,
            supports_range: true,
            etag: None,
            last_modified: None,
            referer: req.referer,
            user_agent: req.user_agent,
            cookies: req.cookies,
            headers: req.headers.unwrap_or_default(),
            chunks: Vec::new(),
            speed_limit_bps: req.speed_limit_bps,
            sequential_preview: req.sequential_preview.unwrap_or(false),
            expected_checksum: req.expected_checksum,
            checksum_algorithm: req.checksum_algorithm,
            checksum_status: crate::contracts::ChecksumStatus::None,
            auto_extract: req.auto_extract.unwrap_or(false),
            delete_archive_after_extract: req.delete_archive_after_extract.unwrap_or(false),
            ytdlp_format_id: req.ytdlp_format_id,
            torrent_selected_files: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };

        meta::save_meta_async(&self.paths, &task).await?;
        Ok(task)
    }

    async fn spawn_job(&self, mut task: DownloadTaskMeta) -> AppResult<()> {
        let bin = sidecars::resolve_sidecar("yt-dlp").ok_or_else(|| AppError::MissingBinary {
            binary: "yt-dlp".to_string(),
            hint: "Place yt-dlp-x86_64-pc-windows-msvc.exe in src-tauri/binaries/".to_string(),
        })?;

        let id = task.id;
        task.status = DownloadStatus::Downloading;
        task.error_message = None;
        task.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &task).await?;

        let token = CancellationToken::new();
        let meta_lock = Arc::new(RwLock::new(task.clone()));
        let downloaded_atomic = Arc::new(AtomicU64::new(task.downloaded_bytes));
        let total_atomic = Arc::new(AtomicU64::new(task.total_bytes.unwrap_or(0)));
        let speed_atomic = Arc::new(AtomicU64::new(0));

        let job = Arc::new(Mutex::new(ActiveYtdlpJob {
            token: token.clone(),
            meta: meta_lock.clone(),
            downloaded_atomic: downloaded_atomic.clone(),
            total_atomic: total_atomic.clone(),
            speed_atomic: speed_atomic.clone(),
            handle: None,
        }));
        self.jobs.write().await.insert(id, job.clone());

        let paths = self.paths.clone();
        let jobs_map = self.jobs.clone();
        let progress_tx = self.progress_tx.clone();
        let global_limit = self.global_limit_bps.load(Ordering::Relaxed);

        let join_handle = tokio::spawn(async move {
            let agg_token = token.child_token();
            let agg_handle = spawn_ytdlp_aggregator(
                id,
                agg_token.clone(),
                paths.clone(),
                meta_lock.clone(),
                downloaded_atomic.clone(),
                total_atomic.clone(),
                speed_atomic.clone(),
                progress_tx.clone(),
            );

            let res = run_ytdlp_process(
                bin,
                &task,
                if global_limit > 0 { Some(global_limit) } else { None },
                &downloaded_atomic,
                &total_atomic,
                &speed_atomic,
                &token,
            )
            .await;

            agg_token.cancel();
            let _ = agg_handle.await;

            let mut m = meta_lock.write().await;
            m.downloaded_bytes = downloaded_atomic.load(Ordering::Relaxed);
            let t = total_atomic.load(Ordering::Relaxed);
            if t > 0 {
                m.total_bytes = Some(t);
            }
            m.updated_at = Utc::now();

            if token.is_cancelled() {
                if m.status != DownloadStatus::Canceled {
                    m.status = DownloadStatus::Paused;
                }
            } else {
                match res {
                    Ok(final_name) => {
                        if let Some(name) = final_name {
                            m.filename = name;
                        }
                        let final_path = PathBuf::from(&m.save_dir).join(&m.filename);
                        if let Ok(file_meta) = tokio::fs::metadata(&final_path).await {
                            let len = file_meta.len();
                            if len > 0 {
                                m.downloaded_bytes = len;
                                m.total_bytes = Some(len);
                            }
                        }
                        m.status = DownloadStatus::Completed;
                        m.error_message = None;
                    }
                    Err(e) => {
                        m.status = DownloadStatus::Failed;
                        m.error_message = Some(e.to_string());
                    }
                }
            }

            let _ = meta::save_meta_async(&paths, &m).await;
            let _ = progress_tx.send(ProgressSnapshot {
                id,
                status: m.status,
                downloaded_bytes: m.downloaded_bytes,
                total_bytes: m.total_bytes,
                speed_bps: 0,
                eta_seconds: None,
                chunks: Vec::new(),
                checksum_status: ChecksumStatus::None,
                error_message: m.error_message.clone(),
            });
            jobs_map.write().await.remove(&id);
        });

        job.lock().await.handle = Some(join_handle);
        Ok(())
    }
}

#[async_trait]
impl DownloaderEngine for YtdlpEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Ytdlp
    }

    async fn start(&self, id: Uuid) -> AppResult<()> {
        let task = meta::load_meta(&self.paths, id)?;
        self.spawn_job(task).await
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
        m.status = DownloadStatus::Paused;
        m.updated_at = Utc::now();
        meta::save_meta_async(&self.paths, &m).await
    }

    async fn resume(&self, id: Uuid) -> AppResult<()> {
        if self.jobs.read().await.contains_key(&id) {
            return Ok(());
        }
        let task = meta::load_meta(&self.paths, id)?;
        self.spawn_job(task).await
    }

    async fn cancel(&self, id: Uuid, delete_partial: bool) -> AppResult<()> {
        let job_opt = self.jobs.read().await.get(&id).cloned();
        if let Some(job_lock) = job_opt {
            let handle = {
                let mut job = job_lock.lock().await;
                job.meta.write().await.status = DownloadStatus::Canceled;
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
            let dl = job.downloaded_atomic.load(Ordering::Relaxed);
            let total = job.total_atomic.load(Ordering::Relaxed);
            let speed = job.speed_atomic.load(Ordering::Relaxed);
            return Ok(ProgressSnapshot {
                id,
                status: m.status,
                downloaded_bytes: dl,
                total_bytes: if total > 0 { Some(total) } else { m.total_bytes },
                speed_bps: speed,
                eta_seconds: None,
                chunks: Vec::new(),
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
            chunks: Vec::new(),
            checksum_status: m.checksum_status,
            error_message: m.error_message,
        })
    }
}

async fn run_ytdlp_process(
    bin: PathBuf,
    task: &DownloadTaskMeta,
    global_limit_bps: Option<u64>,
    downloaded_atomic: &AtomicU64,
    total_atomic: &AtomicU64,
    speed_atomic: &AtomicU64,
    token: &CancellationToken,
) -> AppResult<Option<String>> {
    tokio::fs::create_dir_all(&task.save_dir).await?;
    let template = PathBuf::from(&task.save_dir).join("%(title)s.%(ext)s");

    let mut cmd = Command::new(bin);
    cmd.arg("--newline")
        .arg("--no-simulate")
        .arg("--progress")
        .arg("--continue")
        .arg("--no-warnings")
        .arg("--progress-template")
        .arg("download:IDM_PROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s")
        .arg("--print")
        .arg("after_move:IDM_FILE|%(filepath)s")
        .arg("-o")
        .arg(&template);

    if let Some(ffmpeg) = sidecars::resolve_sidecar("ffmpeg") {
        cmd.arg("--ffmpeg-location").arg(&ffmpeg);
    }
    if let Some(fmt) = task.ytdlp_format_id.as_deref().filter(|s| !s.is_empty()) {
        cmd.arg("-f").arg(fmt);
    }
    let effective_limit = match (task.speed_limit_bps.filter(|&v| v > 0), global_limit_bps.filter(|&v| v > 0)) {
        (Some(t), Some(g)) => Some(t.min(g)),
        (Some(t), None) => Some(t),
        (None, Some(g)) => Some(g),
        (None, None) => None,
    };
    if let Some(limit) = effective_limit {
        cmd.arg("-r").arg(limit.to_string());
    }
    cmd.arg(&task.url)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Other("Failed to capture yt-dlp stdout".into()))?;
    let stderr = child.stderr.take();

    let mut reader = BufReader::new(stdout).lines();
    let mut final_filename: Option<String> = None;

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Ok(None);
            }
            line_res = reader.next_line() => {
                match line_res? {
                    Some(line) => {
                        if let Some(rest) = line.strip_prefix("IDM_PROG|") {
                            let parts: Vec<&str> = rest.split('|').collect();
                            if parts.len() >= 4 {
                                if let Ok(dl) = parts[0].trim().parse::<f64>() {
                                    downloaded_atomic.store(dl.max(0.0) as u64, Ordering::Relaxed);
                                }
                                let total = parts[1]
                                    .trim()
                                    .parse::<f64>()
                                    .ok()
                                    .or_else(|| parts[2].trim().parse::<f64>().ok());
                                if let Some(t) = total {
                                    total_atomic.store(t.max(0.0) as u64, Ordering::Relaxed);
                                }
                                if let Ok(sp) = parts[3].trim().parse::<f64>() {
                                    speed_atomic.store(sp.max(0.0) as u64, Ordering::Relaxed);
                                }
                            }
                        } else if let Some(path_str) = line.strip_prefix("IDM_FILE|") {
                            if let Some(name) = PathBuf::from(path_str.trim()).file_name() {
                                final_filename = Some(name.to_string_lossy().to_string());
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        let mut err_msg = format!("yt-dlp exited with {status}");
        if let Some(mut se) = stderr {
            use tokio::io::AsyncReadExt;
            let mut buf = String::new();
            let _ = se.read_to_string(&mut buf).await;
            if !buf.trim().is_empty() {
                err_msg = buf.trim().lines().last().unwrap_or(&err_msg).to_string();
            }
        }
        return Err(AppError::Other(err_msg));
    }

    Ok(final_filename)
}

#[allow(clippy::too_many_arguments)]
fn spawn_ytdlp_aggregator(
    id: Uuid,
    token: CancellationToken,
    paths: StoragePaths,
    meta_lock: Arc<RwLock<DownloadTaskMeta>>,
    downloaded_atomic: Arc<AtomicU64>,
    total_atomic: Arc<AtomicU64>,
    speed_atomic: Arc<AtomicU64>,
    progress_tx: broadcast::Sender<ProgressSnapshot>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(PROGRESS_INTERVAL_MS));
        let mut smoothed_speed: f64 = 0.0;
        let mut ticks: u64 = 0;

        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = interval.tick() => {
                    ticks += 1;
                    let dl = downloaded_atomic.load(Ordering::Relaxed);
                    let total_val = total_atomic.load(Ordering::Relaxed);
                    let raw_speed = speed_atomic.load(Ordering::Relaxed);
                    if smoothed_speed <= 0.0 {
                        smoothed_speed = raw_speed as f64;
                    } else {
                        smoothed_speed = smoothed_speed * 0.94 + (raw_speed as f64) * 0.06;
                    }
                    let speed = smoothed_speed.round() as u64;

                    let snapshot = {
                        let mut m = meta_lock.write().await;
                        m.downloaded_bytes = dl;
                        if total_val > 0 {
                            m.total_bytes = Some(total_val);
                        }
                        m.updated_at = Utc::now();
                        if ticks.is_multiple_of(6) {
                            let _ = meta::save_meta_async(&paths, &m).await;
                        }
                        let eta = m.total_bytes.and_then(|t| {
                            if speed > 0 && dl < t { Some((t - dl) / speed) } else { None }
                        });
                        ProgressSnapshot {
                            id,
                            status: m.status,
                            downloaded_bytes: dl,
                            total_bytes: m.total_bytes,
                            speed_bps: speed,
                            eta_seconds: eta,
                            chunks: Vec::new(),
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
