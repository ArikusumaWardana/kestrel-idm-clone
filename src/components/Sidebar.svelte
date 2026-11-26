<script lang="ts">
  import Icons from "./Icons.svelte";
  import type { SystemStatus } from "../lib/types";

  export type NavTab = "all" | "active" | "paused" | "history" | "settings";

  interface Props {
    activeTab: NavTab;
    collapsed: boolean;
    counts: {
      all: number;
      active: number;
      paused: number;
      history: number;
    };
    systemStatus: SystemStatus | null;
    onSelectTab: (tab: NavTab) => void;
    onToggleCollapse: () => void;
  }

  let {
    activeTab,
    collapsed,
    counts,
    systemStatus,
    onSelectTab,
    onToggleCollapse,
  }: Props = $props();

  const items: Array<{
    id: NavTab;
    label: string;
    icon: "download" | "play" | "pause" | "history" | "settings";
  }> = [
    { id: "all", label: "Queue", icon: "download" },
    { id: "active", label: "Active", icon: "play" },
    { id: "paused", label: "Paused", icon: "pause" },
    { id: "history", label: "History", icon: "history" },
    { id: "settings", label: "Settings", icon: "settings" },
  ];

  function countFor(id: NavTab): number | null {
    if (id === "all") return counts.all;
    if (id === "active") return counts.active;
    if (id === "paused") return counts.paused;
    if (id === "history") return counts.history;
    return null;
  }
</script>

<aside
  class="flex shrink-0 flex-col border-r border-[var(--border-color)] bg-[var(--bg-surface)]/80"
  style="width: {collapsed ? '56px' : '208px'}"
  aria-label="Primary navigation"
>
  <div class="flex h-12 items-center justify-between border-b border-[var(--border-color)] px-3">
    {#if !collapsed}
      <div class="flex items-center gap-2">
        <img src="/logo.png" alt="Kestrel logo" width="20" height="20" class="h-5 w-5 shrink-0" />
        <span class="text-sm font-semibold tracking-tight">Kestrel</span>
      </div>
    {/if}
    <button
      type="button"
      onclick={onToggleCollapse}
      aria-expanded={!collapsed}
      aria-label={collapsed ? "Expand navigation" : "Collapse navigation"}
      class="flex h-8 w-8 items-center justify-center rounded border border-transparent text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
    >
      <Icons name="sidebar" size={16} />
    </button>
  </div>

  <nav class="flex flex-1 flex-col gap-1 p-2" aria-label="Views">
    {#each items as item}
      {@const isActive = activeTab === item.id}
      {@const count = countFor(item.id)}
      <button
        type="button"
        onclick={() => onSelectTab(item.id)}
        aria-label={collapsed ? item.label : undefined}
        aria-current={isActive ? "page" : undefined}
        class="flex h-9 items-center justify-between rounded px-2.5 text-sm font-medium transition-colors"
        style="
          background: {isActive ? 'var(--bg-subtle)' : 'transparent'};
          color: {isActive ? 'var(--text-primary)' : 'var(--text-muted)'};
          border-left: 3px solid {isActive ? 'var(--accent)' : 'transparent'};
        "
      >
        <span class="flex items-center gap-2.5 overflow-hidden">
          <Icons name={item.icon} size={15} />
          {#if !collapsed}
            <span class="truncate">{item.label}</span>
          {/if}
        </span>
        {#if !collapsed && count !== null}
          <span class="tabular-nums text-xs text-[var(--text-muted)]">{count}</span>
        {/if}
      </button>
    {/each}
  </nav>

  {#if !collapsed && systemStatus}
    <div class="border-t border-[var(--border-color)] p-3 text-xs text-[var(--text-muted)]">
      <div class="flex items-center justify-between">
        <span>Browser bridge</span>
        <span class="font-medium text-[var(--text-primary)]">
          {systemStatus.extensionConnectedCount > 0
            ? `${systemStatus.extensionConnectedCount} connected`
            : "Waiting"}
        </span>
      </div>
      {#if systemStatus.autoThrottleActive}
        <div class="mt-1.5 rounded bg-[var(--warn-bg)] px-2 py-1 text-[11px] font-medium text-[var(--warn-text)]">
          Auto-throttle: {systemStatus.autoThrottleReason ?? "Active"}
        </div>
      {/if}
    </div>
  {/if}
</aside>
