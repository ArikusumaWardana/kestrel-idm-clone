use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Http,
    Ytdlp,
    Torrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Verifying,
    Extracting,
    Completed,
    Failed,
    Canceled,
    NeedsRefresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Sha256,
    Md5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumStatus {
    None,
    Pending,
    Valid,
    Mismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkState {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTaskMeta {
    pub id: Uuid,
    pub url: String,
    pub filename: String,
    pub save_dir: String,
    pub engine: EngineKind,
    pub status: DownloadStatus,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub supports_range: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub cookies: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub chunks: Vec<ChunkState>,
    pub speed_limit_bps: Option<u64>,
    pub sequential_preview: bool,
    pub expected_checksum: Option<String>,
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
    pub checksum_status: ChecksumStatus,
    pub auto_extract: bool,
    pub delete_archive_after_extract: bool,
    #[serde(default)]
    pub ytdlp_format_id: Option<String>,
    #[serde(default)]
    pub torrent_selected_files: Option<Vec<usize>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub id: Uuid,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub chunks: Vec<ChunkState>,
    pub checksum_status: ChecksumStatus,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: Uuid,
    pub url: String,
    pub filename: String,
    pub save_dir: String,
    pub engine: EngineKind,
    pub status: DownloadStatus,
    pub total_bytes: Option<u64>,
    pub checksum_status: ChecksumStatus,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDownloadRequest {
    pub url: String,
    pub filename: Option<String>,
    pub save_dir: Option<String>,
    pub engine: Option<EngineKind>,
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub max_chunks: Option<usize>,
    pub speed_limit_bps: Option<u64>,
    pub sequential_preview: Option<bool>,
    pub expected_checksum: Option<String>,
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
    pub auto_extract: Option<bool>,
    pub delete_archive_after_extract: Option<bool>,
    pub ytdlp_format_id: Option<String>,
    pub torrent_selected_files: Option<Vec<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRule {
    pub name: String,
    pub domain_pattern: Option<String>,
    pub filename_regex: Option<String>,
    pub extensions: Vec<String>,
    pub target_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub default_download_dir: String,
    pub default_chunks: usize,
    pub max_concurrent_downloads: usize,
    pub global_speed_limit_bps: Option<u64>,
    pub bridge_port: u16,
    pub local_auth_token: String,
    pub autostart_enabled: bool,
    pub auto_throttle_enabled: bool,
    pub auto_throttle_percent: u8,
    pub auto_throttle_processes: Vec<String>,
    pub auto_throttle_on_fullscreen: bool,
    pub routing_rules: Vec<RoutingRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtdlpFormat {
    pub format_id: String,
    pub ext: String,
    pub resolution: String,
    pub filesize: Option<u64>,
    pub note: String,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtdlpMediaInfo {
    pub title: String,
    pub uploader: Option<String>,
    pub duration_seconds: Option<u64>,
    pub formats: Vec<YtdlpFormat>,
    pub subtitles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFileItem {
    pub index: usize,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentInspectInfo {
    pub name: String,
    pub info_hash: String,
    pub total_bytes: u64,
    pub files: Vec<TorrentFileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarStatus {
    pub ytdlp: bool,
    pub ffmpeg: bool,
    pub seven_zip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub bridge_port: u16,
    pub extension_connected_count: usize,
    pub auto_throttle_active: bool,
    pub auto_throttle_reason: Option<String>,
    pub effective_speed_limit_bps: Option<u64>,
    pub speed_limit_paused: bool,
    pub portable_mode: bool,
    pub sidecars: SidecarStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateSpeedEvent {
    pub total_speed_bps: u64,
    pub active_count: usize,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    Ping,
    Pong {
        active_downloads: usize,
    },
    RefreshLinkRequest {
        download_id: Uuid,
        url: String,
        referer: Option<String>,
    },
    RefreshLinkResponse {
        download_id: Uuid,
        new_url: String,
        cookies: Option<String>,
        user_agent: Option<String>,
        referer: Option<String>,
    },
    DownloadAccepted {
        download_id: Uuid,
        filename: String,
    },
}

#[async_trait]
pub trait DownloaderEngine: Send + Sync {
    fn kind(&self) -> EngineKind;
    async fn start(&self, id: Uuid) -> AppResult<()>;
    async fn pause(&self, id: Uuid) -> AppResult<()>;
    async fn resume(&self, id: Uuid) -> AppResult<()>;
    async fn cancel(&self, id: Uuid, delete_partial: bool) -> AppResult<()>;
    async fn progress_snapshot(&self, id: Uuid) -> AppResult<ProgressSnapshot>;
}
