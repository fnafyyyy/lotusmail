<script lang="ts">
  // Sprzątanie skrzynki: zbiera newslettery i powiadomienia w grupy po
  // nadawcach, pokazuje, co się nazbierało, i kasuje zaznaczone hurtem.
  //
  // Zaznaczenie jest świadomie puste na starcie - to użytkownik decyduje, co
  // leci, a nie program. Kasowanie przenosi maile do Kosza, więc pomyłkę
  // da się odkręcić po stronie serwera.
  import { fade, scale } from "svelte/transition";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "$lib/api";
  import type { Account, CleanupGroup } from "$lib/types";
  import Icon from "./Icon.svelte";

  let {
    accounts,
    onclose,
    ondone,
  }: {
    accounts: Account[];
    onclose: () => void;
    /** Po skasowaniu - żeby strona przeładowała listy. */
    ondone: (deleted: number) => void;
  } = $props();

  let accountId = $state<number | null>(null);
  let olderThanDays = $state<number | null>(null);
  let onlyUnread = $state(false);
  let groups = $state<CleanupGroup[]>([]);
  let picked = $state<Set<string>>(new Set());
  let scanning = $state(false);
  let deleting = $state(false);
  let done = $state<number | null>(null);
  let error = $state("");
  // Postęp płynie z rdzenia po każdej paczce UID-ów - bez tego kasowanie
  // kilku tysięcy maili to minuty ciszy i wrażenie zawieszenia.
  let progress = $state<{ done: number; total: number } | null>(null);

  onMount(() => {
    const un = listen<{ done: number; total: number }>("cleanup-progress", (e) => {
      progress = e.payload;
    });
    return () => void un.then((fn) => fn());
  });

  const ageOptions: { label: string; days: number | null }[] = [
    { label: "Bez znaczenia", days: null },
    { label: "Starsze niż miesiąc", days: 30 },
    { label: "Starsze niż pół roku", days: 182 },
    { label: "Starsze niż rok", days: 365 },
  ];

  let selectedCount = $derived(
    groups.filter((g) => picked.has(g.fromAddr)).reduce((sum, g) => sum + g.count, 0),
  );
  let totalCount = $derived(groups.reduce((sum, g) => sum + g.count, 0));

  async function scan() {
    scanning = true;
    error = "";
    done = null;
    picked = new Set();
    try {
      groups = await api.cleanupScan({ accountId, olderThanDays, onlyUnread, minCount: 3 });
    } catch (e) {
      error = String(e);
    } finally {
      scanning = false;
    }
  }

  function toggle(addr: string) {
    const next = new Set(picked);
    if (next.has(addr)) next.delete(addr);
    else next.add(addr);
    picked = next;
  }

  function toggleAll() {
    picked = picked.size === groups.length ? new Set() : new Set(groups.map((g) => g.fromAddr));
  }

  /** Zaznacza to, czego nigdy nie otwierano - najbezpieczniejszy wybór. */
  function pickNeverRead() {
    picked = new Set(groups.filter((g) => g.neverRead === g.count).map((g) => g.fromAddr));
  }

  async function remove() {
    const ids = groups.filter((g) => picked.has(g.fromAddr)).flatMap((g) => g.ids);
    if (ids.length === 0) return;
    deleting = true;
    error = "";
    progress = { done: 0, total: ids.length };
    try {
      const removed = await api.cleanupDelete(ids);
      done = removed;
      groups = groups.filter((g) => !picked.has(g.fromAddr));
      picked = new Set();
      ondone(removed);
    } catch (e) {
      error = String(e);
    } finally {
      deleting = false;
      progress = null;
    }
  }

  function fmtDate(ts: number): string {
    return new Date(ts * 1000).toLocaleDateString("pl-PL", {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  }

  const categoryLabel: Record<string, string> = {
    newsletters: "Newsletter",
    notifications: "Powiadomienia",
  };

  scan();
</script>

<div
  class="fixed inset-0 z-30 grid place-items-center bg-ink/30 p-3"
  role="presentation"
  transition:fade={{ duration: 140 }}
  onclick={(e) => e.target === e.currentTarget && onclose()}
>
  <div
    class="panel flex max-h-[84vh] w-full max-w-[680px] flex-col"
    transition:scale={{ start: 0.96, duration: 170 }}
  >
    <header class="flex items-center justify-between border-b border-line px-5 py-3">
      <h2 class="flex items-center gap-2 font-display text-[15px] font-semibold">
        <Icon name="eraser" size={16} class="text-accent" />
        Sprzątanie skrzynki
      </h2>
      <button
        class="rounded-md p-1.5 text-muted hover:bg-rail"
        onclick={onclose}
        aria-label="Zamknij"
      >
        <Icon name="x" size={16} />
      </button>
    </header>

    <!-- Filtry -->
    <div class="flex flex-wrap items-center gap-2 border-b border-line px-5 py-3 text-[13px]">
      {#if accounts.length > 1}
        <select
          bind:value={accountId}
          onchange={scan}
          class="rounded-lg border border-line bg-paper px-2.5 py-1.5 outline-none"
        >
          <option value={null}>Wszystkie konta</option>
          {#each accounts as a (a.id)}
            <option value={a.id}>{a.displayName || a.email}</option>
          {/each}
        </select>
      {/if}
      <select
        bind:value={olderThanDays}
        onchange={scan}
        class="rounded-lg border border-line bg-paper px-2.5 py-1.5 outline-none"
      >
        {#each ageOptions as o (o.label)}
          <option value={o.days}>{o.label}</option>
        {/each}
      </select>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={onlyUnread} onchange={scan} class="accent-accent" />
        Tylko nieprzeczytane
      </label>
      <span class="flex-1"></span>
      <button
        class="rounded-lg px-2.5 py-1.5 font-semibold text-accent hover:bg-rail"
        onclick={scan}
        disabled={scanning}
      >
        {scanning ? "Szukam…" : "Odśwież"}
      </button>
    </div>

    <!-- Lista grup -->
    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-3">
      {#if scanning}
        <p class="py-10 text-center text-sm text-muted">Przeglądam skrzynkę…</p>
      {:else if error}
        <p class="py-10 text-center text-sm text-danger">{error}</p>
      {:else if groups.length === 0}
        <div class="flex flex-col items-center gap-2 py-10 text-center">
          <Icon name="check" size={26} class="text-accent" />
          <p class="text-sm text-muted">
            {done !== null ? "Posprzątane." : "Nie ma czego sprzątać przy tych ustawieniach."}
          </p>
        </div>
      {:else}
        <div class="mb-2 flex items-center gap-3 text-[12.5px] text-muted">
          <button class="font-semibold text-accent hover:underline" onclick={toggleAll}>
            {picked.size === groups.length ? "Odznacz wszystko" : "Zaznacz wszystko"}
          </button>
          <button class="font-semibold text-accent hover:underline" onclick={pickNeverRead}>
            Zaznacz nigdy nieczytane
          </button>
          <span class="flex-1"></span>
          <span class="tabular-nums">{groups.length} nadawców, {totalCount} wiadomości</span>
        </div>

        <div class="space-y-1">
          {#each groups as g (g.fromAddr)}
            {@const on = picked.has(g.fromAddr)}
            <button
              class="flex w-full items-start gap-3 rounded-xl px-3 py-2.5 text-left transition-colors
                     {on ? 'bg-accent-soft' : 'hover:bg-rail'}"
              onclick={() => toggle(g.fromAddr)}
            >
              <span
                class="mt-0.5 grid size-4.5 shrink-0 place-items-center rounded border
                       {on ? 'border-accent bg-accent text-on-accent' : 'border-line'}"
              >
                {#if on}<Icon name="check" size={12} />{/if}
              </span>

              <span class="min-w-0 flex-1">
                <span class="flex items-baseline gap-2">
                  <span class="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-ink">
                    {g.fromName || g.fromAddr}
                  </span>
                  <span class="shrink-0 text-[12px] font-bold tabular-nums text-accent">
                    {g.count}
                  </span>
                </span>
                <span class="block truncate text-[12px] text-muted">{g.fromAddr}</span>
                <span class="mt-1 flex flex-wrap items-center gap-1.5 text-[11.5px] text-muted">
                  <span class="rounded-full bg-rail px-1.75 py-0.25 font-semibold">
                    {categoryLabel[g.category] ?? g.category}
                  </span>
                  {#if g.neverRead === g.count}
                    <span class="rounded-full bg-flag/15 px-1.75 py-0.25 font-semibold text-flag">
                      nigdy nieczytane
                    </span>
                  {:else if g.unread > 0}
                    <span>{g.unread} nieprzeczytanych</span>
                  {/if}
                  <span>{fmtDate(g.oldest)} - {fmtDate(g.newest)}</span>
                </span>
                {#if g.samples.length > 0}
                  <span class="mt-1 block truncate text-[11.5px] text-ink-soft italic">
                    {g.samples.join(" · ")}
                  </span>
                {/if}
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </div>

    {#if deleting && progress}
      {@const percent = progress.total > 0 ? (progress.done / progress.total) * 100 : 0}
      <div class="border-t border-line px-5 pt-3">
        <div class="flex items-baseline justify-between text-[12px] text-muted">
          <span>Przenoszę do Kosza…</span>
          <span class="tabular-nums">{progress.done} z {progress.total}</span>
        </div>
        <div class="mt-1.5 h-1.5 overflow-hidden rounded-full bg-rail">
          <div
            class="h-full rounded-full bg-accent transition-[width] duration-200"
            style="width: {percent}%"
          ></div>
        </div>
      </div>
    {/if}

    <footer class="flex items-center gap-3 border-t border-line px-5 py-3">
      <p class="flex-1 text-[12.5px] text-muted">
        {#if done !== null && picked.size === 0}
          Przeniesiono do Kosza: <span class="font-semibold text-ink">{done}</span>
        {:else if selectedCount > 0}
          Do przeniesienia do Kosza:
          <span class="font-semibold text-ink">{selectedCount}</span> wiadomości
        {:else}
          Zaznacz nadawców, których poczta ma zniknąć.
        {/if}
      </p>
      <button class="rounded-lg px-3 py-1.5 text-sm text-muted hover:bg-rail" onclick={onclose}>
        Zamknij
      </button>
      <button
        class="flex items-center gap-2 rounded-lg bg-danger px-3.5 py-1.5 text-sm font-semibold
               text-white disabled:opacity-40"
        onclick={remove}
        disabled={selectedCount === 0 || deleting}
      >
        <Icon name="trash" size={14} />
        {deleting ? "Kasuję…" : "Przenieś do Kosza"}
      </button>
    </footer>
  </div>
</div>
