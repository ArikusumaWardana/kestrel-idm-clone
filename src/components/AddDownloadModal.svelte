<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import { formatBytes } from "../lib/format";
  import type {
    ChecksumAlgorithm,
    CreateDownloadRequest,
    EngineKind,
    TorrentInspectInfo,
    YtdlpMediaInfo,
  } from "../lib/types";
  import Icons from "./Icons.svelte";

  interface Props {
    openModal: boolean;
    defaultChunks: number;
    prefill?: CreateDownloadRequest | null;
    onClose: () => void;
    onCreated: () => void;
    onError?: (msg: string) => void;
  }

  let {
    openModal,
    defaultChunks,
    prefill = null,
    onClose,
    onCreated,
    onError,
  }: Props = $props();

  let mode = $state<EngineKind>("http");
  let url = $state("");
  let filename = $state("");
  let saveDir = $state("");
  let maxChunks = $state(8);
  let speedLimitKbps = $state("");
  let sequentialPreview = $state(false);
  let checksumAlgo = $state<ChecksumAlgorithm>("sha256");
  let expectedChecksum = $state("");
  let autoExtract = $state(false);
  let deleteArchiveAfterExtract = $state(false);

  let probing = $state(false);
  let submitting = $state(false);
  let errorMsg = $state<string | null>(null);

  let videoInfo = $state<YtdlpMediaInfo | null>(null);
  let selectedFormatId = $state("");

  let torrentInfo = $state<TorrentInspectInfo | null>(null);
  let selectedFileIndices = $state<number[]>([]);

  let urlInputEl = $state<HTMLInputElement | null>(null);
  let browserCookies = $state<string | null>(null);
  let browserUserAgent = $state<string | null>(null);
  let browserReferer = $state<string | null>(null);
  let browserHeaders = $state<Record<string, string> | null>(null);

  $effect(() => {
    if (openModal) {
      maxChunks = defaultChunks;
      errorMsg = null;
      if (prefill) {
        mode = prefill.engine ?? "http";
        url = prefill.url ?? "";
        filename = prefill.filename ?? "";
        saveDir = prefill.saveDir ?? "";
        browserCookies = prefill.cookies ?? null;
        browserUserAgent = prefill.userAgent ?? null;
        browserReferer = prefill.referer ?? null;
        browserHeaders = prefill.headers ?? null;
      }
      queueMicrotask(() => urlInputEl?.focus());
    }
  });

  function resetForm() {
    url = "";
    filename = "";
    saveDir = "";
    browserCookies = null;
    browserUserAgent = null;
    browserReferer = null;
    browserHeaders = null;
    speedLimitKbps = "";
    sequentialPreview = false;
    expectedChecksum = "";
    autoExtract = false;
    deleteArchiveAfterExtract = false;
    videoInfo = null;
    selectedFormatId = "";
    torrentInfo = null;
    selectedFileIndices = [];
    errorMsg = null;
  }

  async function handleBrowseFolder() {
    const picked = await open({ directory: true, multiple: false, title: "Select save folder" });
    if (typeof picked === "string") {
      saveDir = picked;
    }
  }

  async function handleBrowseTorrent() {
    const picked = await open({
      multiple: false,
      title: "Select .torrent file",
      filters: [{ name: "Torrent files", extensions: ["torrent"] }],
    });
    if (typeof picked === "string") {
      mode = "torrent";
      url = picked;
      await handleInspectTorrent();
    }
  }

  async function handleProbeVideo() {
    if (!url.trim()) {
      errorMsg = "Enter a video URL before probing formats.";
      return;
    }
    probing = true;
    errorMsg = null;
    videoInfo = null;
    try {
      const info = await api.probeVideoFormats(url.trim());
      videoInfo = info;
      if (!filename.trim() && info.title) {
        filename = info.title;
      }
      selectedFormatId = "bestvideo+bestaudio/best";
    } catch (err) {
      const msg = formatErr(err);
      errorMsg = msg;
      onError?.(msg);
    } finally {
      probing = false;
    }
  }

  async function handleInspectTorrent() {
    if (!url.trim()) {
      errorMsg = "Enter a magnet URI or select a .torrent file.";
      return;
    }
    probing = true;
    errorMsg = null;
    torrentInfo = null;
    try {
      const info = await api.inspectTorrent(url.trim());
      torrentInfo = info;
      if (!filename.trim() && info.name) {
        filename = info.name;
      }
      selectedFileIndices = info.files.map((f) => f.index);
    } catch (err) {
      const msg = formatErr(err);
      errorMsg = msg;
      onError?.(msg);
    } finally {
      probing = false;
    }
  }

  function toggleTorrentFile(idx: number) {
    if (selectedFileIndices.includes(idx)) {
      selectedFileIndices = selectedFileIndices.filter((i) => i !== idx);
    } else {
      selectedFileIndices = [...selectedFileIndices, idx];
    }
  }

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (!url.trim()) {
      errorMsg = "Enter a URL, magnet link, or .torrent path.";
      return;
    }
    submitting = true;
    errorMsg = null;

    const kbps = Number(speedLimitKbps.trim());
    const speedLimitBps =
      speedLimitKbps.trim() && !Number.isNaN(kbps) && kbps > 0
        ? Math.round(kbps * 1024)
        : null;

    const request: CreateDownloadRequest = {
      url: url.trim(),
      filename: filename.trim() || null,
      saveDir: saveDir.trim() || null,
      engine: mode,
      cookies: browserCookies,
      userAgent: browserUserAgent,
      referer: browserReferer,
      headers: browserHeaders,
      maxChunks: mode === "http" ? maxChunks : null,
      speedLimitBps,
      sequentialPreview,
      expectedChecksum: expectedChecksum.trim() || null,
      checksumAlgorithm: expectedChecksum.trim() ? checksumAlgo : null,
      autoExtract,
      deleteArchiveAfterExtract: autoExtract ? deleteArchiveAfterExtract : false,
      ytdlpFormatId: mode === "ytdlp" ? selectedFormatId || "bestvideo+bestaudio/best" : null,
      torrentSelectedFiles:
        mode === "torrent" && torrentInfo && selectedFileIndices.length > 0
          ? selectedFileIndices
          : null,
    };

    try {
      await api.addDownload(request);
      resetForm();
      onCreated();
      onClose();
    } catch (err) {
      const msg = formatErr(err);
      errorMsg = msg;
      onError?.(msg);
    } finally {
      submitting = false;
    }
  }

  function formatErr(err: unknown): string {
    if (typeof err === "object" && err !== null && "message" in err) {
      return String((err as { message: unknown }).message);
    }
    return String(err);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (openModal && e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if openModal}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div
      class="flex max-h-[90vh] w-full max-w-xl flex-col overflow-hidden rounded-lg border border-[var(--border-color)] bg-[var(--bg-surface)] shadow-lg"
      role="dialog"
      aria-modal="true"
      aria-labelledby="add-modal-title"
    >
      <div class="flex items-center justify-between border-b border-[var(--border-color)] px-4 py-3">
        <h2 id="add-modal-title" class="text-sm font-semibold">New Download</h2>
        <button
          type="button"
          onclick={onClose}
          aria-label="Close dialog"
          class="flex h-8 w-8 items-center justify-center rounded text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
        >
          <Icons name="close" size={15} />
        </button>
      </div>

      <form onsubmit={handleSubmit} class="flex-1 space-y-3 overflow-y-auto p-4 text-xs">
        {#if prefill}
          <p class="rounded bg-[var(--ok-bg)] px-3 py-2 text-xs font-medium text-[var(--ok-text)]">
            Intercepted from browser. Review file name and save location, then click Start Download.
          </p>
        {/if}

        <div role="tablist" aria-label="Download source type" class="flex gap-1.5 border-b border-[var(--border-color)] pb-2">
          <button
            type="button"
            role="tab"
            aria-selected={mode === "http"}
            onclick={() => (mode = "http")}
            class="rounded px-3 py-1.5 text-xs font-medium"
            style="
              background: {mode === 'http' ? 'var(--accent)' : 'var(--bg-subtle)'};
              color: {mode === 'http' ? 'var(--accent-text)' : 'var(--text-primary)'};
            "
          >
            Direct HTTP
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={mode === "ytdlp"}
            onclick={() => (mode = "ytdlp")}
            class="rounded px-3 py-1.5 text-xs font-medium"
            style="
              background: {mode === 'ytdlp' ? 'var(--accent)' : 'var(--bg-subtle)'};
              color: {mode === 'ytdlp' ? 'var(--accent-text)' : 'var(--text-primary)'};
            "
          >
            Video (yt-dlp)
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={mode === "torrent"}
            onclick={() => (mode = "torrent")}
            class="rounded px-3 py-1.5 text-xs font-medium"
            style="
              background: {mode === 'torrent' ? 'var(--accent)' : 'var(--bg-subtle)'};
              color: {mode === 'torrent' ? 'var(--accent-text)' : 'var(--text-primary)'};
            "
          >
            Torrent / Magnet
          </button>
        </div>

        <div>
          <label for="dl-url" class="mb-1 block font-medium text-[var(--text-muted)]">
            {mode === "torrent" ? "Magnet link or .torrent file path" : "Download URL"}
          </label>
          <div class="flex gap-2">
            <input
              id="dl-url"
              bind:this={urlInputEl}
              bind:value={url}
              type="text"
              placeholder={mode === "torrent" ? "magnet:?xt=urn:btih:... or C:\\path\\file.torrent" : "https://example.com/file.zip"}
              class="h-9 flex-1 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs text-[var(--text-primary)]"
              required
            />
            {#if mode === "ytdlp"}
              <button
                type="button"
                onclick={handleProbeVideo}
                disabled={probing}
                class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 font-medium hover:bg-[var(--bg-hover)]"
              >
                {probing ? "Probing..." : "Fetch formats"}
              </button>
            {:else if mode === "torrent"}
              <button
                type="button"
                onclick={handleBrowseTorrent}
                class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 font-medium hover:bg-[var(--bg-hover)]"
              >
                Browse .torrent
              </button>
              <button
                type="button"
                onclick={handleInspectTorrent}
                disabled={probing}
                class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 font-medium hover:bg-[var(--bg-hover)]"
              >
                {probing ? "Inspecting..." : "Inspect"}
              </button>
            {/if}
          </div>
        </div>

        {#if mode === "ytdlp" && videoInfo}
          <div class="rounded border border-[var(--border-color)] bg-[var(--bg-subtle)]/50 p-2.5">
            <div class="font-medium text-[var(--text-primary)]">{videoInfo.title}</div>
            <label for="ytdlp-format" class="mt-2 mb-1 block text-[var(--text-muted)]">
              Quality / Format ({videoInfo.formats.length} available)
            </label>
            <select
              id="ytdlp-format"
              bind:value={selectedFormatId}
              class="h-8 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs text-[var(--text-primary)]"
            >
              <optgroup label="Video + Audio (auto merge via ffmpeg)">
                <option value="bestvideo+bestaudio/best">Best available (Video + Audio, auto merge)</option>
                <option value="bestvideo[height<=1080]+bestaudio/best[height<=1080]">1080p (Video + Audio, auto merge)</option>
                <option value="bestvideo[height<=720]+bestaudio/best[height<=720]">720p (Video + Audio, auto merge)</option>
                <option value="bestvideo[height<=480]+bestaudio/best[height<=480]">480p (Video + Audio, auto merge)</option>
                <option value="bestaudio/best">Audio only (Best)</option>
              </optgroup>
              <optgroup label="Individual streams">
                {#each videoInfo.formats as fmt}
                  {@const streamTag = fmt.hasVideo && fmt.hasAudio ? "video+audio" : fmt.hasVideo ? "video only" : "audio only"}
                  <option value={fmt.formatId}>
                    {fmt.resolution} ({fmt.ext}, {streamTag}) {fmt.filesize ? formatBytes(fmt.filesize) : ""} {fmt.note}
                  </option>
                {/each}
              </optgroup>
            </select>
          </div>
        {/if}

        {#if mode === "torrent" && torrentInfo}
          <div class="rounded border border-[var(--border-color)] bg-[var(--bg-subtle)]/50 p-2.5">
            <div class="flex items-center justify-between font-medium text-[var(--text-primary)]">
              <span>{torrentInfo.name}</span>
              <span class="tabular-nums">{formatBytes(torrentInfo.totalBytes)}</span>
            </div>
            {#if torrentInfo.files.length > 0}
              <div class="mt-2 max-h-32 space-y-1 overflow-y-auto border-t border-[var(--border-color)] pt-2">
                {#each torrentInfo.files as f}
                  <label class="flex items-center justify-between gap-2">
                    <span class="flex items-center gap-2 truncate">
                      <input
                        type="checkbox"
                        checked={selectedFileIndices.includes(f.index)}
                        onchange={() => toggleTorrentFile(f.index)}
                      />
                      <span class="truncate">{f.path}</span>
                    </span>
                    <span class="shrink-0 tabular-nums text-[var(--text-muted)]">
                      {formatBytes(f.sizeBytes)}
                    </span>
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        {/if}

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div>
            <label for="dl-filename" class="mb-1 block font-medium text-[var(--text-muted)]">
              File name (optional)
            </label>
            <input
              id="dl-filename"
              bind:value={filename}
              type="text"
              placeholder="Auto-detect from server"
              class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs text-[var(--text-primary)]"
            />
          </div>

          <div>
            <label for="dl-savedir" class="mb-1 block font-medium text-[var(--text-muted)]">
              Save folder (blank = smart routing)
            </label>
            <div class="flex gap-1.5">
              <input
                id="dl-savedir"
                bind:value={saveDir}
                type="text"
                placeholder="Smart routing / default"
                class="h-9 flex-1 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs text-[var(--text-primary)]"
              />
              <button
                type="button"
                onclick={handleBrowseFolder}
                class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 font-medium hover:bg-[var(--bg-hover)]"
              >
                Browse
              </button>
            </div>
          </div>
        </div>

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          {#if mode === "http"}
            <div>
              <label for="dl-chunks" class="mb-1 block font-medium text-[var(--text-muted)]">
                Parallel chunks (1 to 32)
              </label>
              <input
                id="dl-chunks"
                bind:value={maxChunks}
                type="number"
                min="1"
                max="32"
                class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs text-[var(--text-primary)] tabular-nums"
              />
            </div>
          {/if}

          <div>
            <label for="dl-limit" class="mb-1 block font-medium text-[var(--text-muted)]">
              Task speed limit (KB/s, optional)
            </label>
            <input
              id="dl-limit"
              bind:value={speedLimitKbps}
              type="number"
              min="0"
              placeholder="Use global limit"
              class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs text-[var(--text-primary)] tabular-nums"
            />
          </div>
        </div>

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div>
            <label for="dl-algo" class="mb-1 block font-medium text-[var(--text-muted)]">
              Checksum type
            </label>
            <select
              id="dl-algo"
              bind:value={checksumAlgo}
              class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs text-[var(--text-primary)]"
            >
              <option value="sha256">SHA-256</option>
              <option value="md5">MD5</option>
            </select>
          </div>
          <div class="sm:col-span-2">
            <label for="dl-checksum" class="mb-1 block font-medium text-[var(--text-muted)]">
              Expected hash (optional)
            </label>
            <input
              id="dl-checksum"
              bind:value={expectedChecksum}
              type="text"
              placeholder="Paste expected SHA-256 or MD5 hex"
              class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 font-mono text-xs text-[var(--text-primary)]"
            />
          </div>
        </div>

        <div class="flex flex-wrap gap-4 pt-1">
          <label class="flex items-center gap-2">
            <input type="checkbox" bind:checked={sequentialPreview} />
            <span>Sequential preview (media playback while downloading)</span>
          </label>
          <label class="flex items-center gap-2">
            <input type="checkbox" bind:checked={autoExtract} />
            <span>Auto-extract archive (.zip, .7z, .rar)</span>
          </label>
          {#if autoExtract}
            <label class="flex items-center gap-2">
              <input type="checkbox" bind:checked={deleteArchiveAfterExtract} />
              <span>Delete archive after extraction</span>
            </label>
          {/if}
        </div>

        {#if errorMsg}
          <p class="rounded bg-[var(--err-bg)] px-3 py-2 text-xs text-[var(--err-text)]" role="alert">
            {errorMsg}
          </p>
        {/if}

        <div class="flex justify-end gap-2 border-t border-[var(--border-color)] pt-3">
          <button
            type="button"
            onclick={onClose}
            class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-4 text-xs font-medium hover:bg-[var(--bg-hover)]"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={submitting}
            class="h-9 rounded px-4 text-xs font-medium text-[var(--accent-text)]"
            style="background: var(--accent);"
          >
            {submitting ? "Starting..." : "Start Download"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}
