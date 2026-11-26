<script lang="ts">
  import Icons from "./Icons.svelte";

  interface Props {
    message: string | null;
    kind?: "error" | "info";
    onDismiss: () => void;
  }

  let { message, kind = "error", onDismiss }: Props = $props();
</script>

{#if message}
  <div
    role="alert"
    aria-live="assertive"
    class="fixed right-4 bottom-4 z-50 flex max-w-sm items-start gap-2.5 rounded-md border border-[var(--border-color)] px-3.5 py-2.5 text-xs shadow-md"
    style="
      background: {kind === 'info' ? 'var(--ok-bg)' : 'var(--err-bg)'};
      color: {kind === 'info' ? 'var(--ok-text)' : 'var(--err-text)'};
    "
  >
    <Icons name={kind === "info" ? "check" : "alert"} size={15} />
    <span class="flex-1 leading-relaxed font-medium">{message}</span>
    <button
      type="button"
      onclick={onDismiss}
      aria-label="Dismiss notification"
      class="flex h-6 w-6 shrink-0 items-center justify-center rounded hover:bg-black/10"
    >
      <Icons name="close" size={13} />
    </button>
  </div>
{/if}
