<script lang="ts">
  interface Props {
    name:
      | "plus"
      | "pause"
      | "play"
      | "folder"
      | "trash"
      | "refresh"
      | "settings"
      | "history"
      | "download"
      | "film"
      | "close"
      | "dropzone"
      | "sidebar"
      | "check"
      | "alert"
      | "copy";
    size?: number;
  }

  let { name, size = 16 }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="square"
  stroke-linejoin="miter"
  aria-hidden="true"
  class="shrink-0"
>
  {#if name === "plus"}
    <path d="M8 3v10M3 8h10" />
  {:else if name === "pause"}
    <path d="M5.5 3.5v9M10.5 3.5v9" />
  {:else if name === "play"}
    <path d="M4.5 3.5L12 8l-7.5 4.5z" fill="currentColor" stroke="none" />
  {:else if name === "folder"}
    <path d="M2 4.5h4l1.5 1.5H14v7.5H2z" />
  {:else if name === "trash"}
    <path d="M3 4.5h10M6 4.5V3h4v1.5M4.5 4.5l.5 8.5h6l.5-8.5" />
  {:else if name === "refresh"}
    <path d="M13 8a5 5 0 1 1-1.3-3.4M13 2.5v2.5h-2.5" />
  {:else if name === "settings"}
    <circle cx="8" cy="8" r="2.2" />
    <path d="M8 2v1.5M8 12.5V14M2 8h1.5M12.5 8H14M3.8 3.8l1.1 1.1M11.1 11.1l1.1 1.1M12.2 3.8l-1.1 1.1M4.9 11.1l-1.1 1.1" />
  {:else if name === "history"}
    <circle cx="8" cy="8" r="5.5" />
    <path d="M8 5v3.5l2.5 1.5" />
  {:else if name === "download"}
    <path d="M8 2.5v7.5M5 7.5L8 10.5l3-3M3 13h10" />
  {:else if name === "film"}
    <rect x="2" y="3" width="12" height="10" />
    <path d="M5 3v10M11 3v10M2 6h3M2 10h3M11 6h3M11 10h3" />
  {:else if name === "close"}
    <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />
  {:else if name === "dropzone"}
    <rect x="2.5" y="2.5" width="11" height="11" stroke-dasharray="2 2" />
    <path d="M8 5v6M5.5 8.5L8 11l2.5-2.5" />
  {:else if name === "sidebar"}
    <rect x="2" y="3" width="12" height="10" />
    <path d="M6 3v10" />
  {:else if name === "check"}
    <path d="M3.5 8.5L6.5 11.5L12.5 4.5" />
  {:else if name === "alert"}
    <path d="M8 2.5L14 13H2zM8 6.5v3M8 11.2v.4" />
  {:else if name === "copy"}
    <rect x="5" y="5" width="8" height="8" />
    <path d="M3 11V3h8" />
  {/if}
</svg>
