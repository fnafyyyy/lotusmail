<script lang="ts">
  // Sterowanie oknem zamiast natywnego paska tytułu (decorations: false).
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  const win = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    win.isMaximized().then((v) => (maximized = v));
    const un = win.onResized(() => win.isMaximized().then((v) => (maximized = v)));
    return () => {
      un.then((fn) => fn());
    };
  });
</script>

<div class="flex shrink-0 items-center">
  <button
    class="grid h-8.5 w-11 place-items-center text-muted hover:bg-surface hover:text-ink"
    onclick={() => win.minimize()}
    aria-label="Minimalizuj"
    title="Minimalizuj"
  >
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <rect x="0" y="4.5" width="10" height="1" fill="currentColor" />
    </svg>
  </button>
  <button
    class="grid h-8.5 w-11 place-items-center text-muted hover:bg-surface hover:text-ink"
    onclick={() => win.toggleMaximize()}
    aria-label={maximized ? "Przywróć" : "Maksymalizuj"}
    title={maximized ? "Przywróć" : "Maksymalizuj"}
  >
    {#if maximized}
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" aria-hidden="true">
        <rect x="0.5" y="2.5" width="7" height="7" />
        <path d="M2.5 2.5V0.5h7v7h-2" />
      </svg>
    {:else}
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" aria-hidden="true">
        <rect x="0.5" y="0.5" width="9" height="9" />
      </svg>
    {/if}
  </button>
  <button
    class="grid h-8.5 w-11 place-items-center text-muted hover:bg-[#c42b1c] hover:text-white"
    onclick={() => win.close()}
    aria-label="Zamknij"
    title="Zamknij"
  >
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" aria-hidden="true">
      <path d="m0.5 0.5 9 9M9.5 0.5l-9 9" />
    </svg>
  </button>
</div>
