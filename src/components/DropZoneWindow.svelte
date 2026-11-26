<script lang="ts">
  import { emit } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { api, onAggregateSpeed } from "../lib/api";
  import { formatSpeed } from "../lib/format";
  import Icons from "./Icons.svelte";

  let speedBps = $state(0);
  let activeCount = $state(0);
  let statusMsg = $state("Drop URL or .torrent");
  let statusError = $state(false);
  let dragging = $state(false);

  async function queueSource(source: string) {
    const trimmed = source.trim();
    if (!trimmed) return;
    statusError = false;
    statusMsg = "Queuing...";
    try {
      await api.addDownload({ url: trimmed });
      statusMsg = "Download started";
    } catch (err) {
      const msg =
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: unknown }).message)
          : err instanceof Error
            ? err.message
            : "Not a valid download link";
      statusError = true;
      statusMsg = "Not a download link";
      void emit("ui-toast", msg);
    }
  }

  function handleHtmlDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    const text =
      e.dataTransfer?.getData("text/uri-list") ||
      e.dataTransfer?.getData("text/plain") ||
      "";
    if (text) {
      void queueSource(text.split(/\r?\n/)[0]);
    }
  }

  async function handlePasteClick() {
    try {
      const text = await api.readClipboardText();
      if (text.trim()) {
        await queueSource(text);
      } else {
        statusMsg = "Clipboard empty";
      }
    } catch {
      statusMsg = "Clipboard unavailable";
    }
  }

  onMount(() => {
    let unlistenSpeed: (() => void) | undefined;
    let unlistenDrop: (() => void) | undefined;

    onAggregateSpeed((evt) => {
      speedBps = evt.totalSpeedBps;
      activeCount = evt.activeCount;
    }).then((fn) => (unlistenSpeed = fn));

    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over") {
          dragging = true;
        } else if (event.payload.type === "leave") {
          dragging = false;
        } else if (event.payload.type === "drop") {
          dragging = false;
          const torrentPath = event.payload.paths.find((p) =>
            p.toLowerCase().endsWith(".torrent")
          );
          if (torrentPath) {
            void queueSource(torrentPath);
          }
        }
      })
      .then((fn) => (unlistenDrop = fn));

    return () => {
      unlistenSpeed?.();
      unlistenDrop?.();
    };
  });
</script>

<main
  class="flex h-screen w-screen flex-col justify-between overflow-hidden rounded-lg border bg-[var(--bg-surface)] p-2.5 text-xs text-[var(--text-primary)]"
  style="border-color: {dragging ? 'var(--accent)' : 'var(--border-color)'};"
  ondragover={(e) => {
    e.preventDefault();
    dragging = true;
  }}
  ondragleave={() => (dragging = false)}
  ondrop={handleHtmlDrop}
>
  <div class="flex items-center justify-between gap-2" data-tauri-drag-region>
    <button
      type="button"
      onclick={() => api.openMainWindow()}
      class="truncate text-left font-semibold text-[var(--text-primary)] hover:underline"
    >
      Kestrel
    </button>
    <button
      type="button"
      onclick={() => getCurrentWindow().destroy()}
      aria-label="Close Drop Zone"
      class="flex h-8 w-8 items-center justify-center rounded text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
    >
      <Icons name="close" size={14} />
    </button>
  </div>

  <div class="tabular-nums">
    <div class="text-sm font-semibold text-[var(--text-primary)]">
      {formatSpeed(speedBps)}
    </div>
    <div
      class="truncate text-[11px]"
      style="color: {statusError ? 'var(--err-text)' : 'var(--text-muted)'};"
      title={statusMsg}
    >
      {activeCount > 0 ? `${activeCount} active` : statusMsg}
    </div>
  </div>

  <div class="flex gap-2">
    <button
      type="button"
      onclick={handlePasteClick}
      class="h-11 flex-1 rounded px-2.5 text-xs font-medium text-[var(--accent-text)]"
      style="background: var(--accent);"
    >
      Paste URL
    </button>
    <button
      type="button"
      onclick={() => api.openMainWindow()}
      class="h-11 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-3 text-xs font-medium hover:bg-[var(--bg-hover)]"
    >
      Open
    </button>
  </div>
</main>
