<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import AddDownloadModal from "./components/AddDownloadModal.svelte";
  import DownloadRow from "./components/DownloadRow.svelte";
  import DropZoneWindow from "./components/DropZoneWindow.svelte";
  import Icons from "./components/Icons.svelte";
  import SettingsPanel, { type ThemeMode } from "./components/SettingsPanel.svelte";
  import Sidebar, { type NavTab } from "./components/Sidebar.svelte";
  import SpeedGraph from "./components/SpeedGraph.svelte";
  import Toast from "./components/Toast.svelte";
  import Toolbar from "./components/Toolbar.svelte";
  import {
    api,
    onAggregateSpeed,
    onAutoThrottleChanged,
    onBrowserDownloadPrompt,
    onDownloadProgress,
    onDownloadsChanged,
  } from "./lib/api";
  import { formatBytes, formatTimestamp } from "./lib/format";
  import type {
    AppSettings,
    CreateDownloadRequest,
    DownloadTaskMeta,
    HistoryEntry,
    ProgressSnapshot,
    SystemStatus,
  } from "./lib/types";

  const isDropZone =
    typeof window !== "undefined" &&
    new URLSearchParams(window.location.search).get("window") === "dropzone";

  let activeTab = $state<NavTab>("all");
  let sidebarCollapsed = $state(false);
  let searchQuery = $state("");
  let modalOpen = $state(false);
  let modalPrefill = $state<CreateDownloadRequest | null>(null);

  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let toastMessage = $state<string | null>(null);
  let toastKind = $state<"error" | "info">("error");
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  function showToast(msg: string, kind: "error" | "info" = "error") {
    toastMessage = msg;
    toastKind = kind;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toastMessage = null;
    }, 5000);
  }

  let tasks = $state<DownloadTaskMeta[]>([]);
  let snapshots = $state<Record<string, ProgressSnapshot>>({});
  let historyList = $state<HistoryEntry[]>([]);
  let settings = $state<AppSettings | null>(null);
  let systemStatus = $state<SystemStatus | null>(null);

  let speedSamples = $state<number[]>(Array.from({ length: 60 }, () => 0));
  let currentSpeedBps = $state(0);
  let activeCount = $state(0);

  let themeMode = $state<ThemeMode>("system");

  function applyTheme(mode: ThemeMode) {
    themeMode = mode;
    if (typeof document !== "undefined") {
      document.documentElement.dataset.theme = mode;
      localStorage.setItem("idm_theme", mode);
    }
  }

  async function refreshAll() {
    try {
      const [active, hist, cfg, sys] = await Promise.all([
        api.listDownloads(),
        api.listHistory(),
        api.getSettings(),
        api.getSystemStatus(),
      ]);
      tasks = active;
      historyList = hist;
      settings = cfg;
      systemStatus = sys;
      loadError = null;
    } catch (err) {
      loadError = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  let filteredTasks = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    return tasks.filter((t) => {
      const st = snapshots[t.id]?.status ?? t.status;
      if (activeTab === "active" && st !== "downloading" && st !== "queued" && st !== "verifying" && st !== "extracting") {
        return false;
      }
      if (activeTab === "paused" && st !== "paused" && st !== "failed" && st !== "needs_refresh") {
        return false;
      }
      if (!q) return true;
      return (
        t.filename.toLowerCase().includes(q) ||
        t.url.toLowerCase().includes(q)
      );
    });
  });

  let filteredHistory = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return historyList;
    return historyList.filter(
      (h) =>
        h.filename.toLowerCase().includes(q) ||
        h.url.toLowerCase().includes(q)
    );
  });

  let counts = $derived.by(() => {
    let active = 0;
    let paused = 0;
    for (const t of tasks) {
      const st = snapshots[t.id]?.status ?? t.status;
      if (st === "downloading" || st === "queued" || st === "verifying" || st === "extracting") {
        active += 1;
      } else if (st === "paused" || st === "failed" || st === "needs_refresh") {
        paused += 1;
      }
    }
    return {
      all: tasks.length,
      active,
      paused,
      history: historyList.length,
    };
  });

  let speedLimitKbps = $derived(
    settings?.globalSpeedLimitBps
      ? String(Math.round(settings.globalSpeedLimitBps / 1024))
      : ""
  );

  async function runAction(fn: () => Promise<unknown>) {
    actionError = null;
    try {
      await fn();
    } catch (err) {
      actionError =
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: unknown }).message)
          : String(err);
    }
  }

  function handleWindowKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      modalOpen = true;
    }
  }

  async function drainBrowserPrompts() {
    try {
      const pending = await api.takePendingBrowserPrompts();
      if (pending.length > 0) {
        modalPrefill = pending[0];
        modalOpen = true;
      }
    } catch {
      // Ignore if backend not ready
    }
  }

  onMount(() => {
    const savedTheme = (localStorage.getItem("idm_theme") as ThemeMode | null) ?? "system";
    applyTheme(savedTheme);

    if (isDropZone) return;

    void refreshAll();
    void drainBrowserPrompts();

    const unlisteners: Array<() => void> = [];
    onBrowserDownloadPrompt(() => {
      void drainBrowserPrompts();
    }).then((fn) => unlisteners.push(fn));
    onDownloadProgress((snap) => {
      snapshots[snap.id] = snap;
      if (snap.status === "completed") {
        const t = tasks.find((item) => item.id === snap.id);
        if (t) {
          showToast(`Completed: ${t.filename} (saved to ${t.saveDir})`, "info");
        }
      }
    }).then((fn) => unlisteners.push(fn));

    onDownloadsChanged(() => {
      void refreshAll();
    }).then((fn) => unlisteners.push(fn));

    onAggregateSpeed((evt) => {
      currentSpeedBps = evt.totalSpeedBps;
      activeCount = evt.activeCount;
      speedSamples = [...speedSamples.slice(1), evt.totalSpeedBps];
    }).then((fn) => unlisteners.push(fn));

    onAutoThrottleChanged((active, reason) => {
      if (systemStatus) {
        systemStatus = {
          ...systemStatus,
          autoThrottleActive: active,
          autoThrottleReason: reason,
        };
      }
      void api.getSystemStatus().then((s) => (systemStatus = s));
    }).then((fn) => unlisteners.push(fn));

    listen<string>("ui-toast", (e) => {
      showToast(e.payload);
    }).then((fn) => unlisteners.push(fn));

    const statusTimer = setInterval(() => {
      void api.getSystemStatus().then((s) => (systemStatus = s));
    }, 5000);

    return () => {
      clearInterval(statusTimer);
      for (const fn of unlisteners) fn();
    };
  });
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if isDropZone}
  <DropZoneWindow />
{:else}
  <div class="app-shell flex h-screen w-screen overflow-hidden text-sm">
    <Sidebar
      {activeTab}
      collapsed={sidebarCollapsed}
      {counts}
      {systemStatus}
      onSelectTab={(tab) => (activeTab = tab)}
      onToggleCollapse={() => (sidebarCollapsed = !sidebarCollapsed)}
    />

    <div class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <Toolbar
        {searchQuery}
        {speedLimitKbps}
        hasActiveOrQueued={counts.active > 0}
        hasPaused={counts.paused > 0}
        onSearchChange={(v) => (searchQuery = v)}
        onAddClick={() => (modalOpen = true)}
        onPauseAll={() => runAction(() => api.pauseAll().then(refreshAll))}
        onResumeAll={() => runAction(() => api.resumeAll().then(refreshAll))}
        onApplySpeedLimit={(kbps) =>
          runAction(() =>
            api
              .setGlobalSpeedLimit(kbps ? kbps * 1024 : null)
              .then(refreshAll)
          )}
        onToggleDropZone={() => runAction(() => api.toggleDropzoneWindow())}
      />

      <SpeedGraph
        samples={speedSamples}
        {currentSpeedBps}
        {activeCount}
        effectiveLimitBps={systemStatus?.effectiveSpeedLimitBps ?? null}
      />

      {#if actionError}
        <div
          role="alert"
          class="flex items-center justify-between border-b border-[var(--border-color)] bg-[var(--err-bg)] px-4 py-2 text-xs text-[var(--err-text)]"
        >
          <span>{actionError}</span>
          <button
            type="button"
            onclick={() => (actionError = null)}
            aria-label="Dismiss error"
            class="rounded p-1 hover:bg-black/10"
          >
            <Icons name="close" size={12} />
          </button>
        </div>
      {/if}

      <main class="min-h-0 flex-1 overflow-y-auto">
        {#if loading}
          <div class="flex h-full items-center justify-center p-8 text-xs text-[var(--text-muted)]">
            Loading downloads and configuration...
          </div>
        {:else if loadError}
          <div class="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
            <p class="text-xs text-[var(--err-text)]" role="alert">
              Failed to load backend state: {loadError}
            </p>
            <button
              type="button"
              onclick={refreshAll}
              class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-4 text-xs font-medium hover:bg-[var(--bg-hover)]"
            >
              Retry
            </button>
          </div>
        {:else if activeTab === "settings"}
          <SettingsPanel
            {settings}
            {systemStatus}
            {themeMode}
            onThemeChange={applyTheme}
            onSaved={(updated) => {
              settings = updated;
              void api.getSystemStatus().then((s) => (systemStatus = s));
            }}
          />
        {:else if activeTab === "history"}
          <div class="p-4">
            <div class="mb-3 flex items-center justify-between">
              <h1 class="text-sm font-semibold">Completed History</h1>
              {#if historyList.length > 0}
                <button
                  type="button"
                  onclick={() => runAction(() => api.clearHistory().then(refreshAll))}
                  class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-3 text-xs font-medium hover:bg-[var(--bg-hover)]"
                >
                  Clear History
                </button>
              {/if}
            </div>

            {#if filteredHistory.length === 0}
              <div class="rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-8 text-center text-xs text-[var(--text-muted)]">
                No completed downloads recorded yet. Finished files appear here automatically.
              </div>
            {:else}
              <div class="space-y-2">
                {#each filteredHistory as entry (entry.id)}
                  <article class="flex items-center justify-between gap-3 rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-3 text-xs">
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-2">
                        <span class="truncate font-semibold text-[var(--text-primary)]">{entry.filename}</span>
                        <span class="rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-1.5 py-0.5 text-[10px] uppercase text-[var(--text-muted)]">
                          {entry.engine}
                        </span>
                      </div>
                      <div class="mt-0.5 truncate text-[var(--text-muted)]">
                        {formatBytes(entry.totalBytes)} · {formatTimestamp(entry.completedAt)} · {entry.saveDir}
                      </div>
                    </div>
                    <div class="flex shrink-0 items-center gap-1">
                      <button
                        type="button"
                        onclick={() => runAction(() => api.openDownloadFolder(entry.saveDir, entry.filename))}
                        aria-label="Open folder for {entry.filename}"
                        class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] hover:bg-[var(--bg-hover)]"
                      >
                        <Icons name="folder" size={14} />
                      </button>
                      <button
                        type="button"
                        onclick={() => runAction(() => api.removeHistoryEntry(entry.id).then(refreshAll))}
                        aria-label="Remove {entry.filename} from history"
                        class="flex h-8 w-8 items-center justify-center rounded border border-[var(--border-color)] text-[var(--err-text)] hover:bg-[var(--err-bg)]"
                      >
                        <Icons name="trash" size={14} />
                      </button>
                    </div>
                  </article>
                {/each}
              </div>
            {/if}
          </div>
        {:else}
          <div class="p-4">
            <h1 class="sr-only">Download Queue</h1>
            {#if filteredTasks.length === 0}
              <div class="flex flex-col items-center justify-center gap-3 rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-10 text-center">
                <p class="text-xs text-[var(--text-muted)]">
                  {tasks.length === 0
                    ? "No active or paused downloads. Add a URL, video, or torrent to start."
                    : "No downloads match the current filter."}
                </p>
                {#if tasks.length === 0}
                  <button
                    type="button"
                    onclick={() => (modalOpen = true)}
                    class="h-9 rounded px-4 text-xs font-medium text-[var(--accent-text)]"
                    style="background: var(--accent);"
                  >
                    Add Download
                  </button>
                {/if}
              </div>
            {:else}
              <div class="space-y-2.5">
                {#each filteredTasks as task (task.id)}
                  <DownloadRow
                    {task}
                    snapshot={snapshots[task.id]}
                    onPause={(id) => runAction(() => api.pauseDownload(id).then(refreshAll))}
                    onResume={(id) => runAction(() => api.resumeDownload(id).then(refreshAll))}
                    onRefreshLink={(id) => runAction(() => api.requestLinkRefresh(id))}
                    onPreview={(dir, file) => runAction(() => api.openMediaPreview(dir, file))}
                    onOpenFolder={(dir, file) => runAction(() => api.openDownloadFolder(dir, file))}
                    onCancel={(id) => runAction(() => api.cancelDownload(id, true).then(refreshAll))}
                  />
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </main>
    </div>
  </div>

  <AddDownloadModal
    openModal={modalOpen}
    defaultChunks={settings?.defaultChunks ?? 8}
    prefill={modalPrefill}
    onClose={() => {
      modalOpen = false;
      modalPrefill = null;
    }}
    onCreated={refreshAll}
    onError={(msg) => showToast(msg)}
  />

  <Toast message={toastMessage} kind={toastKind} onDismiss={() => (toastMessage = null)} />
{/if}
