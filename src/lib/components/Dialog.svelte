<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";

  let {
    title,
    message = "",
    placeholder = "",
    initialValue = null,
    confirmLabel = "OK",
    danger = false,
    onconfirm,
    oncancel,
  }: {
    title: string;
    message?: string;
    placeholder?: string;
    /** Podanie wartości zamienia okno w pole tekstowe (zamiast potwierdzenia). */
    initialValue?: string | null;
    confirmLabel?: string;
    danger?: boolean;
    onconfirm: (value: string) => void;
    oncancel: () => void;
  } = $props();

  // Kopia robocza wartości - celowo tylko wartość początkowa propa.
  // svelte-ignore state_referenced_locally
  let value = $state(initialValue ?? "");
  let input: HTMLInputElement | undefined = $state();
  let isPrompt = $derived(initialValue !== null);
  let ready = $derived(!isPrompt || value.trim().length > 0);

  $effect(() => {
    input?.focus();
  });

  function confirm() {
    if (ready) onconfirm(value.trim());
  }
</script>

<div
  class="fixed inset-0 z-50 grid place-items-center bg-ink/40 p-3"
  role="presentation"
  transition:fade={{ duration: 120 }}
  onclick={(e) => e.target === e.currentTarget && oncancel()}
>
  <div
    class="panel w-full max-w-[420px] p-5"
    transition:scale={{ start: 0.96, duration: 150 }}
    role="dialog"
    aria-modal="true"
    aria-label={title}
  >
    <div class="flex items-start gap-3">
      <span
        class="grid size-9 shrink-0 place-items-center rounded-xl
               {danger ? 'bg-danger/10 text-danger' : 'bg-accent-soft text-accent'}"
      >
        <Icon name={danger ? "trash" : "folder"} size={17} />
      </span>
      <div class="min-w-0 flex-1">
        <h2 class="tight font-display text-[15px] font-bold">{title}</h2>
        {#if message}
          <p class="mt-1 text-[13px] leading-relaxed text-muted text-pretty">{message}</p>
        {/if}
      </div>
    </div>

    {#if isPrompt}
      <input
        bind:this={input}
        bind:value
        {placeholder}
        class="mt-4 w-full rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none
               placeholder:text-muted focus:border-accent"
        onkeydown={(e) => {
          if (e.key === "Enter") confirm();
          if (e.key === "Escape") oncancel();
        }}
      />
    {/if}

    <div class="mt-5 flex justify-end gap-2">
      <button
        class="rounded-full px-4 py-2 text-sm font-semibold text-ink-soft hover:bg-rail"
        onclick={oncancel}
      >
        Anuluj
      </button>
      <button
        class="rounded-full px-4 py-2 text-sm font-semibold text-on-accent transition-opacity
               hover:opacity-90 disabled:opacity-40 {danger ? 'bg-danger text-white' : 'bg-accent'}"
        disabled={!ready}
        onclick={confirm}
      >
        {confirmLabel}
      </button>
    </div>
  </div>
</div>

<svelte:window onkeydown={(e) => e.key === "Escape" && oncancel()} />
