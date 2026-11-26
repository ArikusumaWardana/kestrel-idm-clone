<script lang="ts">
  import Icons from "./Icons.svelte";

  interface Props {
    searchQuery: string;
    speedLimitKbps: string;
    hasActiveOrQueued: boolean;
    hasPaused: boolean;
    onSearchChange: (val: string) => void;
    onAddClick: () => void;
    onPauseAll: () => void;
    onResumeAll: () => void;
    onApplySpeedLimit: (kbps: number | null) => void;
    onToggleDropZone: () => void;
  }

  let {
    searchQuery,
    speedLimitKbps,
    hasActiveOrQueued,
    hasPaused,
    onSearchChange,
    onAddClick,
    onPauseAll,
    onResumeAll,
    onApplySpeedLimit,
    onToggleDropZone,
  }: Props = $props();

  let limitInput = $state("");

  $effect(() => {
    limitInput = speedLimitKbps;
  });

  function handleLimitSubmit(e: SubmitEvent) {
    e.preventDefault();
    const parsed = Number(limitInput.trim());
    if (!limitInput.trim() || Number.isNaN(parsed) || parsed <= 0) {
      onApplySpeedLimit(null);
    } else {
      onApplySpeedLimit(Math.round(parsed));
    }
  }
</script>

<header class="flex flex-wrap items-center justify-between gap-2 border-b border-[var(--border-color)] bg-[var(--bg-surface)]/80 px-4 py-2.5">
  <div class="flex flex-wrap items-center gap-2">
    <button
      type="button"
      onclick={onAddClick}
      class="flex h-9 items-center gap-1.5 rounded px-3 text-sm font-medium text-[var(--accent-text)]"
      style="background: var(--accent);"
    >
      <Icons name="plus" size={15} />
      <span>Add Download</span>
    </button>

    <button
      type="button"
      onclick={onPauseAll}
      disabled={!hasActiveOrQueued}
      class="flex h-9 items-center gap-1.5 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs font-medium text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
    >
      <Icons name="pause" size={14} />
      <span>Pause All</span>
    </button>

    <button
      type="button"
      onclick={onResumeAll}
      disabled={!hasPaused}
      class="flex h-9 items-center gap-1.5 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs font-medium text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
    >
      <Icons name="play" size={14} />
      <span>Resume All</span>
    </button>

    <button
      type="button"
      onclick={onToggleDropZone}
      class="flex h-9 items-center gap-1.5 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs font-medium text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
    >
      <Icons name="dropzone" size={14} />
      <span>Drop Zone</span>
    </button>
  </div>

  <div class="flex flex-wrap items-center gap-2">
    <form onsubmit={handleLimitSubmit} class="flex items-center gap-1.5">
      <label for="toolbar-speed-limit" class="text-xs text-[var(--text-muted)]">
        Limit (KB/s)
      </label>
      <input
        id="toolbar-speed-limit"
        type="number"
        min="0"
        placeholder="Unlimited"
        bind:value={limitInput}
        class="h-8 w-24 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs text-[var(--text-primary)] tabular-nums"
      />
      <button
        type="submit"
        class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs font-medium text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
      >
        Set
      </button>
    </form>

    <div class="flex items-center">
      <label for="toolbar-search" class="sr-only">Filter downloads</label>
      <input
        id="toolbar-search"
        type="search"
        value={searchQuery}
        oninput={(e) => onSearchChange((e.currentTarget as HTMLInputElement).value)}
        placeholder="Filter by file or URL..."
        class="h-8 w-44 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs text-[var(--text-primary)]"
      />
    </div>
  </div>
</header>
