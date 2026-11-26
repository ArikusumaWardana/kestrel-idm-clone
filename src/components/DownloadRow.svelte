<script lang="ts">
  import Icons from "./Icons.svelte";
  import { formatBytes, formatEta, formatPercent, formatSpeed } from "../lib/format";
  import type { DownloadTaskMeta, ProgressSnapshot } from "../lib/types";

  interface Props {
    task: DownloadTaskMeta;
    snapshot: ProgressSnapshot | undefined;
    onPause: (id: string) => void;
    onResume: (id: string) => void;
    onRefreshLink: (id: string) => void;
    onPreview: (saveDir: string, filename: string) => void;
    onOpenFolder: (saveDir: string, filename: string) => void;
    onCancel: (id: string) => void;
  }

  let {
    task,
    snapshot,
    onPause,
    onResume,
    onRefreshLink,
    onPreview,
    onOpenFolder,
    onCancel,
  }: Props = $props();

  let status = $derived(snapshot?.status ?? task.status);
  let downloaded = $derived(snapshot?.downloadedBytes ?? task.downloadedBytes);
  let total = $derived(snapshot?.totalBytes ?? task.totalBytes);
  let speed = $derived(snapshot?.speedBps ?? 0);
  let eta = $derived(snapshot?.etaSeconds ?? null);
  let chunks = $derived(snapshot?.chunks ?? task.chunks);
  let checksumStatus = $derived(snapshot?.checksumStatus ?? task.checksumStatus);
  let errorMessage = $derived(snapshot?.errorMessage ?? task.errorMessage);
  let pct = $derived(formatPercent(downloaded, total));

  let isMedia = $derived(/\.(mp4|mkv|webm|mov|avi|mp3|flac|wav|m4a)$/i.test(task.filename));

  function statusLabel(s: string): string {
    switch (s) {
      case "downloading":
        return "Downloading";
      case "paused":
        return "Paused";
      case "queued":
        return "Queued";
      case "verifying":
        return "Verifying checksum";
      case "extracting":
        return "Extracting";
      case "completed":
        return "Completed";
      case "needs_refresh":
        return "Link expired (refresh needed)";
      case "failed":
        return "Failed";
      case "canceled":
        return "Canceled";
      default:
        return s;
    }
  }
</script>

<article class="rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-3">
  <div class="flex items-start justify-between gap-3">
    <div class="min-w-0 flex-1">
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="truncate text-sm font-semibold text-[var(--text-primary)]" title={task.filename}>
          {task.filename}
        </h2>
        <span class="rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-1.5 py-0.5 text-[11px] font-medium uppercase text-[var(--text-muted)]">
          {task.engine}
        </span>
        {#if checksumStatus === "valid"}
          <span class="inline-flex items-center gap-1 rounded bg-[var(--ok-bg)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--ok-text)]">
            <Icons name="check" size={12} />
            Checksum valid
          </span>
        {:else if checksumStatus === "mismatch"}
          <span class="inline-flex items-center gap-1 rounded bg-[var(--err-bg)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--err-text)]">
            <Icons name="alert" size={12} />
            Checksum mismatch
          </span>
        {/if}
      </div>
      <p class="mt-0.5 truncate text-xs text-[var(--text-muted)]" title={task.url}>
        {task.url}
      </p>
    </div>

    <div class="flex shrink-0 items-center gap-1">
      {#if status === "downloading" || status === "queued"}
        <button
          type="button"
          onclick={() => onPause(task.id)}
          aria-label="Pause download"
          class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] bg-[var(--bg-surface)] text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
        >
          <Icons name="pause" size={14} />
        </button>
      {:else if status === "paused" || status === "failed"}
        <button
          type="button"
          onclick={() => onResume(task.id)}
          aria-label="Resume download"
          class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] bg-[var(--bg-surface)] text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
        >
          <Icons name="play" size={14} />
        </button>
      {:else if status === "needs_refresh"}
        <button
          type="button"
          onclick={() => onRefreshLink(task.id)}
          aria-label="Refresh expired link from browser"
          class="flex h-8 items-center gap-1 rounded px-2.5 text-xs font-medium text-[var(--accent-text)]"
          style="background: var(--accent);"
        >
          <Icons name="refresh" size={13} />
          <span>Refresh Link</span>
        </button>
      {/if}

      {#if isMedia && downloaded > 0}
        <button
          type="button"
          onclick={() => onPreview(task.saveDir, task.filename)}
          aria-label="Preview media file"
          class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] bg-[var(--bg-surface)] text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
        >
          <Icons name="film" size={14} />
        </button>
      {/if}

      <button
        type="button"
        onclick={() => onOpenFolder(task.saveDir, task.filename)}
        aria-label="Open containing folder"
        class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] bg-[var(--bg-surface)] text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
      >
        <Icons name="folder" size={14} />
      </button>

      <button
        type="button"
        onclick={() => onCancel(task.id)}
        aria-label="Cancel and remove download"
        class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] bg-[var(--bg-surface)] text-[var(--err-text)] hover:bg-[var(--err-bg)]"
      >
        <Icons name="trash" size={14} />
      </button>
    </div>
  </div>

  <!-- Chunk visualizer -->
  <div class="mt-2.5">
    {#if chunks.length > 0}
      <div
        class="flex h-2 w-full gap-0.5 overflow-hidden rounded bg-[var(--bg-subtle)]"
        role="progressbar"
        aria-valuenow={Math.round(pct)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label="{task.filename} progress"
      >
        {#each chunks as chunk (chunk.index)}
          {@const span = Math.max(1, chunk.end - chunk.start + 1)}
          {@const chunkPct = Math.min(100, (chunk.downloaded / span) * 100)}
          <div class="relative h-full flex-1 bg-[var(--bg-subtle)]">
            <div
              class="h-full"
              style="width: {chunkPct.toFixed(1)}%; background: {chunk.completed ? 'var(--ok-text)' : 'var(--accent)'};"
            ></div>
          </div>
        {/each}
      </div>
    {:else}
      <div
        class="h-2 w-full overflow-hidden rounded bg-[var(--bg-subtle)]"
        role="progressbar"
        aria-valuenow={Math.round(pct)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label="{task.filename} progress"
      >
        <div
          class="h-full"
          style="width: {pct.toFixed(1)}%; background: var(--accent);"
        ></div>
      </div>
    {/if}
  </div>

  <div class="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--text-muted)] tabular-nums">
    <div class="flex flex-wrap items-center gap-3">
      <span class="font-medium text-[var(--text-primary)]">{statusLabel(status)}</span>
      <span>{formatBytes(downloaded)} / {formatBytes(total)} ({pct.toFixed(1)}%)</span>
      {#if chunks.length > 1}
        <span>{chunks.length} chunks</span>
      {/if}
    </div>

    <div class="flex items-center gap-3">
      {#if status === "downloading"}
        <span>{formatSpeed(speed)}</span>
        {#if eta}
          <span>ETA {formatEta(eta)}</span>
        {/if}
      {/if}
    </div>
  </div>

  {#if errorMessage}
    <p class="mt-2 rounded bg-[var(--err-bg)] px-2.5 py-1.5 text-xs text-[var(--err-text)]" role="alert">
      {errorMessage}
    </p>
  {/if}
</article>
