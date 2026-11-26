export type EngineKind = "http" | "ytdlp" | "torrent";

export type DownloadStatus =
  | "queued"
  | "downloading"
  | "paused"
  | "verifying"
  | "extracting"
  | "completed"
  | "failed"
  | "canceled"
  | "needs_refresh";

export type ChecksumAlgorithm = "sha256" | "md5";

export type ChecksumStatus = "none" | "pending" | "valid" | "mismatch";

export interface ChunkState {
  index: number;
  start: number;
  end: number;
  downloaded: number;
  completed: boolean;
}

export interface ProgressSnapshot {
  id: string;
  status: DownloadStatus;
  downloadedBytes: number;
  totalBytes: number | null;
  speedBps: number;
  etaSeconds: number | null;
  chunks: ChunkState[];
  checksumStatus: ChecksumStatus;
  errorMessage: string | null;
}

export interface DownloadTaskMeta {
  id: string;
  url: string;
  filename: string;
  saveDir: string;
  engine: EngineKind;
  status: DownloadStatus;
  totalBytes: number | null;
  downloadedBytes: number;
  supportsRange: boolean;
  etag: string | null;
  lastModified: string | null;
  referer: string | null;
  userAgent: string | null;
  cookies: string | null;
  headers: Record<string, string>;
  chunks: ChunkState[];
  speedLimitBps: number | null;
  sequentialPreview: boolean;
  expectedChecksum: string | null;
  checksumAlgorithm: ChecksumAlgorithm | null;
  checksumStatus: ChecksumStatus;
  autoExtract: boolean;
  deleteArchiveAfterExtract: boolean;
  ytdlpFormatId: string | null;
  torrentSelectedFiles: number[] | null;
  errorMessage: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface HistoryEntry {
  id: string;
  url: string;
  filename: string;
  saveDir: string;
  engine: EngineKind;
  status: DownloadStatus;
  totalBytes: number | null;
  checksumStatus: ChecksumStatus;
  errorMessage: string | null;
  createdAt: string;
  completedAt: string | null;
}

export interface CreateDownloadRequest {
  url: string;
  filename?: string | null;
  saveDir?: string | null;
  engine?: EngineKind | null;
  cookies?: string | null;
  userAgent?: string | null;
  referer?: string | null;
  headers?: Record<string, string> | null;
  maxChunks?: number | null;
  speedLimitBps?: number | null;
  sequentialPreview?: boolean | null;
  expectedChecksum?: string | null;
  checksumAlgorithm?: ChecksumAlgorithm | null;
  autoExtract?: boolean | null;
  deleteArchiveAfterExtract?: boolean | null;
  ytdlpFormatId?: string | null;
  torrentSelectedFiles?: number[] | null;
}

export interface RoutingRule {
  name: string;
  domainPattern: string | null;
  filenameRegex: string | null;
  extensions: string[];
  targetDir: string;
}

export interface AppSettings {
  defaultDownloadDir: string;
  defaultChunks: number;
  maxConcurrentDownloads: number;
  globalSpeedLimitBps: number | null;
  bridgePort: number;
  localAuthToken: string;
  autostartEnabled: boolean;
  autoThrottleEnabled: boolean;
  autoThrottlePercent: number;
  autoThrottleProcesses: string[];
  autoThrottleOnFullscreen: boolean;
  routingRules: RoutingRule[];
}

export interface YtdlpFormat {
  formatId: string;
  ext: string;
  resolution: string;
  filesize: number | null;
  note: string;
  hasVideo: boolean;
  hasAudio: boolean;
}

export interface YtdlpMediaInfo {
  title: string;
  uploader: string | null;
  durationSeconds: number | null;
  formats: YtdlpFormat[];
  subtitles: string[];
}

export interface TorrentFileItem {
  index: number;
  path: string;
  sizeBytes: number;
}

export interface TorrentInspectInfo {
  name: string;
  infoHash: string;
  totalBytes: number;
  files: TorrentFileItem[];
}

export interface SidecarStatus {
  ytdlp: boolean;
  ffmpeg: boolean;
  sevenZip: boolean;
}

export interface SystemStatus {
  bridgePort: number;
  extensionConnectedCount: number;
  autoThrottleActive: boolean;
  autoThrottleReason: string | null;
  effectiveSpeedLimitBps: number | null;
  speedLimitPaused: boolean;
  portableMode: boolean;
  sidecars: SidecarStatus;
}

export interface AggregateSpeedEvent {
  totalSpeedBps: number;
  activeCount: number;
  timestampMs: number;
}
