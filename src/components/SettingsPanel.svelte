<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import type { AppSettings, RoutingRule, SystemStatus } from "../lib/types";
  import Icons from "./Icons.svelte";

  export type ThemeMode = "system" | "light" | "dark";

  interface Props {
    settings: AppSettings | null;
    systemStatus: SystemStatus | null;
    themeMode: ThemeMode;
    onThemeChange: (mode: ThemeMode) => void;
    onSaved: (updated: AppSettings) => void;
  }

  let { settings, systemStatus, themeMode, onThemeChange, onSaved }: Props = $props();

  let draft = $state<AppSettings | null>(null);
  let limitKbps = $state("");
  let processesText = $state("");
  let showToken = $state(false);
  let copiedToken = $state(false);
  let saving = $state(false);
  let feedback = $state<{ ok: boolean; text: string } | null>(null);

  let newRuleName = $state("");
  let newRuleDomain = $state("");
  let newRuleRegex = $state("");
  let newRuleExts = $state("");
  let newRuleDir = $state("");

  $effect(() => {
    if (settings) {
      draft = structuredClone($state.snapshot(settings));
      limitKbps = settings.globalSpeedLimitBps
        ? String(Math.round(settings.globalSpeedLimitBps / 1024))
        : "";
      processesText = settings.autoThrottleProcesses.join(", ");
    }
  });

  async function pickDefaultDir() {
    if (!draft) return;
    const picked = await open({ directory: true, multiple: false, title: "Default download folder" });
    if (typeof picked === "string") {
      draft.defaultDownloadDir = picked;
    }
  }

  async function pickRuleDir() {
    const picked = await open({ directory: true, multiple: false, title: "Rule destination folder" });
    if (typeof picked === "string") {
      newRuleDir = picked;
    }
  }

  function addRule() {
    if (!draft) return;
    if (!newRuleName.trim() || !newRuleDir.trim()) {
      feedback = { ok: false, text: "Rule name and target folder are required." };
      return;
    }
    const extensions = newRuleExts
      .split(",")
      .map((s) => s.trim().replace(/^\./, ""))
      .filter(Boolean);
    const rule: RoutingRule = {
      name: newRuleName.trim(),
      domainPattern: newRuleDomain.trim() || null,
      filenameRegex: newRuleRegex.trim() || null,
      extensions,
      targetDir: newRuleDir.trim(),
    };
    draft.routingRules = [...draft.routingRules, rule];
    newRuleName = "";
    newRuleDomain = "";
    newRuleRegex = "";
    newRuleExts = "";
    newRuleDir = "";
    feedback = null;
  }

  function removeRule(idx: number) {
    if (!draft) return;
    draft.routingRules = draft.routingRules.filter((_, i) => i !== idx);
  }

  async function copyToken() {
    if (!draft) return;
    await navigator.clipboard.writeText(draft.localAuthToken);
    copiedToken = true;
    setTimeout(() => (copiedToken = false), 2000);
  }

  async function handleSave(e: SubmitEvent) {
    e.preventDefault();
    if (!draft) return;
    saving = true;
    feedback = null;

    const kbps = Number(limitKbps.trim());
    const globalSpeedLimitBps =
      limitKbps.trim() && !Number.isNaN(kbps) && kbps > 0
        ? Math.round(kbps * 1024)
        : null;

    const payload: AppSettings = {
      ...draft,
      defaultChunks: Math.min(32, Math.max(1, Number(draft.defaultChunks) || 8)),
      maxConcurrentDownloads: Math.min(10, Math.max(1, Number(draft.maxConcurrentDownloads) || 3)),
      autoThrottlePercent: Math.min(100, Math.max(5, Number(draft.autoThrottlePercent) || 20)),
      globalSpeedLimitBps,
      autoThrottleProcesses: processesText
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
    };

    try {
      const saved = await api.saveSettings(payload);
      onSaved(saved);
      feedback = { ok: true, text: "Settings saved to settings.toml." };
    } catch (err) {
      feedback = {
        ok: false,
        text: err instanceof Error ? err.message : String(err),
      };
    } finally {
      saving = false;
    }
  }
</script>

{#if !draft}
  <p class="p-4 text-xs text-[var(--text-muted)]">Loading settings...</p>
{:else}
  <form onsubmit={handleSave} class="space-y-5 p-4 text-xs">
    <section class="rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-4">
      <h2 class="text-sm font-semibold">General</h2>
      <div class="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div class="sm:col-span-2">
          <label for="set-dir" class="mb-1 block font-medium text-[var(--text-muted)]">
            Default download folder
          </label>
          <div class="flex gap-2">
            <input
              id="set-dir"
              type="text"
              bind:value={draft.defaultDownloadDir}
              class="h-9 flex-1 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs"
            />
            <button
              type="button"
              onclick={pickDefaultDir}
              class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 font-medium hover:bg-[var(--bg-hover)]"
            >
              Browse
            </button>
          </div>
        </div>

        <div>
          <label for="set-chunks" class="mb-1 block font-medium text-[var(--text-muted)]">
            Default chunks per file (1 to 32)
          </label>
          <input
            id="set-chunks"
            type="number"
            min="1"
            max="32"
            bind:value={draft.defaultChunks}
            class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs tabular-nums"
          />
        </div>

        <div>
          <label for="set-theme" class="mb-1 block font-medium text-[var(--text-muted)]">
            Appearance
          </label>
          <select
            id="set-theme"
            value={themeMode}
            onchange={(e) => onThemeChange((e.currentTarget as HTMLSelectElement).value as ThemeMode)}
            class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs"
          >
            <option value="system">Follow Windows</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>

        <label class="flex items-center gap-2 sm:col-span-2">
          <input type="checkbox" bind:checked={draft.autostartEnabled} />
          <span>Start minimized in System Tray when Windows signs in</span>
        </label>
      </div>
    </section>

    <section class="rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-4">
      <h2 class="text-sm font-semibold">Bandwidth &amp; Auto-Throttle</h2>
      <div class="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div>
          <label for="set-limit" class="mb-1 block font-medium text-[var(--text-muted)]">
            Global speed limit (KB/s, blank = unlimited)
          </label>
          <input
            id="set-limit"
            type="number"
            min="0"
            bind:value={limitKbps}
            placeholder="Unlimited"
            class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs tabular-nums"
          />
        </div>

        <div>
          <label for="set-pct" class="mb-1 block font-medium text-[var(--text-muted)]">
            Auto-throttle speed target (% of limit / 5 MB/s baseline)
          </label>
          <input
            id="set-pct"
            type="number"
            min="5"
            max="100"
            bind:value={draft.autoThrottlePercent}
            class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs tabular-nums"
          />
        </div>

        <div class="sm:col-span-2">
          <label for="set-procs" class="mb-1 block font-medium text-[var(--text-muted)]">
            Gaming / meeting processes (comma-separated)
          </label>
          <input
            id="set-procs"
            type="text"
            bind:value={processesText}
            class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs"
          />
        </div>

        <label class="flex items-center gap-2">
          <input type="checkbox" bind:checked={draft.autoThrottleEnabled} />
          <span>Enable Smart Auto-Throttle</span>
        </label>

        <label class="flex items-center gap-2">
          <input type="checkbox" bind:checked={draft.autoThrottleOnFullscreen} />
          <span>Throttle on Windows fullscreen / D3D apps</span>
        </label>
      </div>
    </section>

    <section class="rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-4">
      <h2 class="text-sm font-semibold">Smart Folder Routing</h2>
      <div class="mt-3 space-y-2">
        {#if draft.routingRules.length === 0}
          <p class="text-[var(--text-muted)]">No routing rules configured. All files use default folder.</p>
        {:else}
          {#each draft.routingRules as rule, idx}
            <div class="flex items-center justify-between gap-2 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)]/40 px-3 py-2">
              <div class="min-w-0">
                <div class="font-medium text-[var(--text-primary)]">{rule.name}</div>
                <div class="truncate text-[11px] text-[var(--text-muted)]">
                  {#if rule.domainPattern}Domain: {rule.domainPattern} · {/if}
                  {#if rule.filenameRegex}Regex: {rule.filenameRegex} · {/if}
                  {#if rule.extensions.length > 0}Ext: {rule.extensions.join(", ")} · {/if}
                  Folder: {rule.targetDir}
                </div>
              </div>
              <button
                type="button"
                onclick={() => removeRule(idx)}
                aria-label="Remove rule {rule.name}"
                class="flex h-7 w-7 shrink-0 items-center justify-center rounded border border-[var(--border-color)] text-[var(--err-text)] hover:bg-[var(--err-bg)]"
              >
                <Icons name="trash" size={13} />
              </button>
            </div>
          {/each}
        {/if}

        <div class="mt-3 grid grid-cols-1 gap-2 border-t border-[var(--border-color)] pt-3 sm:grid-cols-2">
          <input
            type="text"
            aria-label="Rule name"
            bind:value={newRuleName}
            placeholder="Rule name (e.g. ISOs)"
            class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs"
          />
          <input
            type="text"
            aria-label="Domain pattern"
            bind:value={newRuleDomain}
            placeholder="Domain (optional, e.g. *.github.com)"
            class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs"
          />
          <input
            type="text"
            aria-label="Filename regex"
            bind:value={newRuleRegex}
            placeholder="Filename regex (optional)"
            class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs"
          />
          <input
            type="text"
            aria-label="Extensions"
            bind:value={newRuleExts}
            placeholder="Extensions (e.g. iso, img)"
            class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs"
          />
          <div class="flex gap-1.5 sm:col-span-2">
            <input
              type="text"
              aria-label="Target folder"
              bind:value={newRuleDir}
              placeholder="Target folder path"
              class="h-8 flex-1 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2 text-xs"
            />
            <button
              type="button"
              onclick={pickRuleDir}
              class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 font-medium hover:bg-[var(--bg-hover)]"
            >
              Browse
            </button>
            <button
              type="button"
              onclick={addRule}
              class="h-8 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 font-medium hover:bg-[var(--bg-hover)]"
            >
              Add rule
            </button>
          </div>
        </div>
      </div>
    </section>

    <section class="rounded-md border border-[var(--border-color)] bg-[var(--bg-surface)] p-4">
      <h2 class="text-sm font-semibold">Browser Extension Pairing &amp; Sidecars</h2>
      <div class="mt-3 space-y-3">
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div>
            <label for="set-port" class="mb-1 block font-medium text-[var(--text-muted)]">
              Localhost port (127.0.0.1)
            </label>
            <input
              id="set-port"
              type="number"
              min="1024"
              max="65535"
              bind:value={draft.bridgePort}
              class="h-9 w-full rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 text-xs tabular-nums"
            />
          </div>
          <div class="sm:col-span-2">
            <label for="set-token" class="mb-1 block font-medium text-[var(--text-muted)]">
              Local pairing token (paste into Brave extension popup)
            </label>
            <div class="flex gap-1.5">
              <input
                id="set-token"
                type={showToken ? "text" : "password"}
                readonly
                value={draft.localAuthToken}
                class="h-9 flex-1 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)]/50 px-2.5 font-mono text-xs"
              />
              <button
                type="button"
                onclick={() => (showToken = !showToken)}
                class="h-9 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 hover:bg-[var(--bg-hover)]"
              >
                {showToken ? "Hide" : "Show"}
              </button>
              <button
                type="button"
                onclick={copyToken}
                class="flex h-9 items-center gap-1 rounded border border-[var(--border-color)] bg-[var(--bg-surface)] px-2.5 hover:bg-[var(--bg-hover)]"
              >
                <Icons name="copy" size={13} />
                <span>{copiedToken ? "Copied" : "Copy"}</span>
              </button>
            </div>
          </div>
        </div>

        {#if systemStatus}
          <div class="flex flex-wrap gap-2 border-t border-[var(--border-color)] pt-3">
            <span class="rounded px-2 py-1 font-medium {systemStatus.sidecars.ytdlp ? 'bg-[var(--ok-bg)] text-[var(--ok-text)]' : 'bg-[var(--warn-bg)] text-[var(--warn-text)]'}">
              yt-dlp: {systemStatus.sidecars.ytdlp ? "Ready" : "Missing"}
            </span>
            <span class="rounded px-2 py-1 font-medium {systemStatus.sidecars.ffmpeg ? 'bg-[var(--ok-bg)] text-[var(--ok-text)]' : 'bg-[var(--warn-bg)] text-[var(--warn-text)]'}">
              ffmpeg: {systemStatus.sidecars.ffmpeg ? "Ready" : "Missing"}
            </span>
            <span class="rounded px-2 py-1 font-medium {systemStatus.sidecars.sevenZip ? 'bg-[var(--ok-bg)] text-[var(--ok-text)]' : 'bg-[var(--warn-bg)] text-[var(--warn-text)]'}">
              7z: {systemStatus.sidecars.sevenZip ? "Ready" : "Missing"}
            </span>
            <span class="rounded bg-[var(--bg-subtle)] px-2 py-1 text-[var(--text-muted)]">
              Storage: {systemStatus.portableMode ? "Portable (./data)" : "AppData"}
            </span>
          </div>
        {/if}
      </div>
    </section>

    {#if feedback}
      <p
        role="status"
        class="rounded px-3 py-2 text-xs font-medium {feedback.ok ? 'bg-[var(--ok-bg)] text-[var(--ok-text)]' : 'bg-[var(--err-bg)] text-[var(--err-text)]'}"
      >
        {feedback.text}
      </p>
    {/if}

    <div class="flex justify-end">
      <button
        type="submit"
        disabled={saving}
        class="h-9 rounded px-5 text-xs font-medium text-[var(--accent-text)]"
        style="background: var(--accent);"
      >
        {saving ? "Saving..." : "Save Settings"}
      </button>
    </div>
  </form>
{/if}
