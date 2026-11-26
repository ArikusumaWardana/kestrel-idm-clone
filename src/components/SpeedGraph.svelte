<script lang="ts">
  import { formatSpeed } from "../lib/format";

  interface Props {
    samples: number[];
    currentSpeedBps: number;
    activeCount: number;
    effectiveLimitBps: number | null;
  }

  let { samples, currentSpeedBps, activeCount, effectiveLimitBps }: Props = $props();

  const width = 400;
  const height = 56;

  let peak = $derived(Math.max(1024, ...samples));
  let points = $derived(
    samples
      .map((v, idx) => {
        const x = samples.length > 1 ? (idx / (samples.length - 1)) * width : width;
        const y = height - Math.min(1, v / peak) * (height - 8) - 4;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ")
  );
  let areaPoints = $derived(`0,${height} ${points} ${width},${height}`);
</script>

<section
  class="flex items-center justify-between gap-4 border-b border-[var(--border-color)] bg-[var(--bg-surface)]/60 px-4 py-2"
  aria-label="Total download speed"
>
  <div class="flex min-w-0 flex-col gap-0.5">
    <span class="text-xs font-medium text-[var(--text-muted)]">
      Total download speed (last 60s)
    </span>
    <div class="flex items-baseline gap-3">
      <span class="tabular-nums text-base font-semibold text-[var(--text-primary)]">
        {formatSpeed(currentSpeedBps)}
      </span>
      <span class="tabular-nums text-xs text-[var(--text-muted)]">
        {activeCount} active
      </span>
      {#if effectiveLimitBps !== null}
        <span class="tabular-nums text-xs text-[var(--text-muted)]">
          Cap: {formatSpeed(effectiveLimitBps)}
        </span>
      {/if}
    </div>
  </div>

  <div class="hidden w-64 shrink-0 sm:block" aria-hidden="true">
    <svg viewBox="0 0 {width} {height}" class="h-12 w-full">
      <polygon points={areaPoints} fill="var(--accent)" opacity="0.14" />
      <polyline
        points={points}
        fill="none"
        stroke="var(--accent)"
        stroke-width="1.75"
        stroke-linejoin="round"
      />
    </svg>
  </div>
</section>
