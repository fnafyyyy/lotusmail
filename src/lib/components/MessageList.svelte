<script lang="ts">
  import { fade, scale, slide } from "svelte/transition";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import type { Category, CategoryCounts, MessageSummary, SortKey } from "$lib/types";
  import { fmtDate, fmtDateYear, initials, isUnread } from "$lib/api";
  import { theme } from "$lib/theme.svelte";
  import Icon from "./Icon.svelte";

  let {
    messages,
    selectedId,
    holdId,
    showCategories,
    category,
    counts,
    sort,
    searchQuery,
    title,
    onopen,
    oncategory,
    onsort,
    onaction,
    onsearchappend,
    onsearchserver,
    serverSearching = false,
    loadingMore = false,
    hasMore = false,
    onloadmore,
  }: {
    messages: MessageSummary[];
    selectedId: number | null;
    holdId: number | null;
    showCategories: boolean;
    category: Category;
    /** Nieprzeczytane w każdej zakładce - liczniki na przyciskach. */
    counts: CategoryCounts;
    sort: SortKey;
    searchQuery: string;
    title: string;
    onopen: (m: MessageSummary) => void;
    oncategory: (c: Category) => void;
    onsort: (s: SortKey) => void;
    onaction: (m: MessageSummary, action: string, arg?: number) => void;
    onsearchappend: (operator: string) => void;
    /// Dociąga z serwera maile pasujące do zapytania, których nie ma lokalnie.
    onsearchserver: () => void;
    serverSearching?: boolean;
    /** Trwa dociąganie kolejnej strony. */
    loadingMore?: boolean;
    /** Czy na serwerze zostało coś jeszcze. */
    hasMore?: boolean;
    /** Przewijanie dobiło do końca - czas na kolejną stronę. */
    onloadmore: () => void;
  } = $props();

  // Zaznaczenie zmienione klawiaturą musi zostać w widoku listy.
  let listEl: HTMLDivElement | undefined = $state();
  $effect(() => {
    const id = selectedId;
    if (id == null || !listEl) return;
    const el = listEl.querySelector(`[data-mid="${id}"]`);
    el?.scrollIntoView({ block: "nearest" });
  });

  // Szybkie filtry dopisywane do zapytania jednym kliknięciem.
  const searchChips = [
    { op: "jest:nieprzeczytane", label: "Nieprzeczytane" },
    { op: "ma:zalacznik", label: "Z załącznikiem" },
    { op: "jest:oflagowane", label: "Oflagowane" },
  ];

  // Przeniesienie wiadomości między sekcjami: krótkie wygaszenie w miejscu
  // startu i pojawienie się w celu. Wcześniejszy crossfade morfował wysoką
  // kartę w wąski wiersz, co przycinało się przy jednoczesnym ładowaniu treści.
  const MOVE_IN = { duration: 140, delay: 60 };
  const MOVE_OUT = { duration: 110 };
  const FLIP = { duration: 200, easing: cubicOut };

  let newOpen = $state(true);
  let seenOpen = $state(true);

  // Przy kilkudziesięciu nowych wiadomościach nagłówek „Przejrzane" lądował
  // poza ekranem i wyglądało to, jakby sekcji nie było - dlatego „Nowe"
  // pokazuje na start tylko początek listy. „Przejrzane" kończy listę, więc
  // niczego nie zasłania i obcinanie go nie miałoby sensu; po prostu się
  // przewija.
  const NEW_PREVIEW = 6;
  let newAll = $state(false);


  const tabs: { id: Category; label: string }[] = [
    { id: "primary", label: "Główne" },
    { id: "newsletters", label: "Newslettery" },
    { id: "notifications", label: "Powiadomienia" },
  ];

  // Kolejność listy. `short` to etykieta na przycisku, `label` w menu.
  const sortOptions: { id: SortKey; label: string; short: string }[] = [
    { id: "date_desc", label: "Data - najnowsze", short: "Data" },
    { id: "date_asc", label: "Data - najstarsze", short: "Najstarsze" },
    { id: "unread", label: "Nieprzeczytane na górze", short: "Nieprzeczytane" },
    { id: "from", label: "Nadawca (A-Z)", short: "Nadawca" },
    { id: "subject", label: "Temat (A-Z)", short: "Temat" },
    { id: "attachments", label: "Z załącznikiem na górze", short: "Załączniki" },
  ];
  let sortOpen = $state(false);

  // Stały, powtarzalny kolor awatara wyliczany z adresu nadawcy.
  const avatarHues = [356, 25, 95, 160, 200, 245, 285, 320];
  function avatarColor(addr: string, dark: boolean): string {
    let h = 0;
    for (const ch of addr) h = (h * 31 + ch.charCodeAt(0)) | 0;
    const hue = avatarHues[Math.abs(h) % avatarHues.length];
    return dark ? `oklch(0.74 0.12 ${hue})` : `oklch(0.62 0.13 ${hue})`;
  }

  // W wynikach wyszukiwania maile bywają sprzed lat - pokazujemy pełną datę.
  let searching = $derived(searchQuery.trim().length > 0);
  let dateOf = $derived(searching ? fmtDateYear : fmtDate);

  // Grupowanie jak w Sparku: otwarta wiadomość (holdId) zostaje w „Nowe",
  // dopóki nie klikniesz następnej.
  let unseen = $derived(messages.filter((m) => isUnread(m) || m.id === holdId));
  let seen = $derived(messages.filter((m) => !isUnread(m) && m.id !== holdId));
  let grouped = $derived(showCategories);

  let unseenShown = $derived(newAll ? unseen : unseen.slice(0, NEW_PREVIEW));

  // Menu kontekstowe (prawy przycisk myszy) na wiadomości.
  let ctx = $state<{ x: number; y: number; m: MessageSummary } | null>(null);

  function openContext(e: MouseEvent, m: MessageSummary) {
    e.preventDefault();
    ctx = {
      x: Math.min(e.clientX, window.innerWidth - 230),
      y: Math.min(e.clientY, window.innerHeight - 260),
      m,
    };
  }

  function ctxAction(action: string, arg?: number) {
    if (!ctx) return;
    onaction(ctx.m, action, arg);
    ctx = null;
  }

  function snoozeTomorrow(): number {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    d.setHours(9, 0, 0, 0);
    return Math.floor(d.getTime() / 1000);
  }
</script>

<section class="panel flex h-full w-full flex-col">
  <div class="flex flex-wrap items-center gap-1.5 px-3.5 pt-3.5 pb-2.5">
    {#if searching}
      <span class="text-xs text-muted">Znaleziono: {messages.length}</span>
      <button
        class="flex items-center gap-1.5 rounded-full bg-accent-soft px-2.5 py-0.75 text-[11px]
               font-semibold text-accent hover:opacity-80 disabled:opacity-50"
        title="Przeszukaj całą skrzynkę na serwerze, także maile jeszcze niepobrane"
        disabled={serverSearching}
        onclick={onsearchserver}
      >
        <Icon name={serverSearching ? "refresh" : "search"} size={11} class={serverSearching ? "animate-spin" : ""} />
        {serverSearching ? "Szukam…" : "Szukaj na serwerze"}
      </button>
      <span class="flex-1"></span>
      {#each searchChips as chip (chip.op)}
        <button
          class="rounded-full bg-rail px-2.5 py-0.75 text-[11px] font-semibold text-ink-soft
                 transition-colors hover:text-ink"
          title="Dopisz do zapytania: {chip.op}"
          onclick={() => onsearchappend(chip.op)}
        >
          {chip.label}
        </button>
      {/each}
    {:else if showCategories}
      {#each tabs as tab (tab.id)}
        <button
          class="flex h-6.5 items-center gap-1.5 rounded-full pl-3 text-xs font-semibold
                 {counts[tab.id] > 0 ? 'pr-1.5' : 'pr-3'}
                 {category === tab.id
            ? 'bg-accent text-on-accent'
            : 'bg-rail text-ink-soft hover:text-ink'}"
          onclick={() => oncategory(tab.id)}
        >
          {tab.label}
          {#if counts[tab.id] > 0}
            <span
              class="grid h-4.5 min-w-4.5 place-items-center rounded-full px-1 text-[10.5px]
                     font-bold tabular-nums
                     {category === tab.id ? 'bg-on-accent/20' : 'bg-accent text-on-accent'}"
              title="{counts[tab.id]} nieprzeczytanych"
            >
              {counts[tab.id] > 99 ? "99+" : counts[tab.id]}
            </span>
          {/if}
        </button>
      {/each}
    {:else}
      <span class="text-[11px] font-bold tracking-[0.09em] text-muted uppercase">{title}</span>
    {/if}

    {#if !searching}
      <span class="flex-1"></span>
      <div class="relative">
        <button
          class="flex h-6.5 items-center gap-1.25 rounded-full bg-rail px-2.5 text-[11px]
                 font-semibold text-ink-soft transition-colors hover:text-ink"
          title="Kolejność wiadomości"
          onclick={(e) => {
            e.stopPropagation();
            sortOpen = !sortOpen;
          }}
        >
          <Icon name="sort" size={12} />
          {sortOptions.find((o) => o.id === sort)?.short ?? "Data"}
        </button>
        {#if sortOpen}
          <div
            class="panel absolute top-8 right-0 z-30 w-52 p-1 ring-1 ring-line"
            transition:fade={{ duration: 100 }}
          >
            {#each sortOptions as option (option.id)}
              <button
                class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px]
                       hover:bg-rail {sort === option.id ? 'font-semibold text-accent' : ''}"
                onclick={() => {
                  sortOpen = false;
                  onsort(option.id);
                }}
              >
                <Icon
                  name="check"
                  size={12}
                  class={sort === option.id ? "" : "opacity-0"}
                />
                {option.label}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  {#snippet moreRow(hidden: number, expand: () => void)}
    <button
      class="mb-1 flex w-full items-center gap-2 rounded-lg px-3.5 py-1.5 text-left text-[12px]
             font-semibold text-muted transition-colors hover:bg-rail hover:text-ink"
      onclick={expand}
    >
      <Icon name="chevronDown" size={12} />
      Pokaż pozostałe ({hidden})
    </button>
  {/snippet}

  {#snippet groupHeader(
    label: string,
    count: number,
    open: boolean,
    accent: boolean,
    ontoggle: () => void,
  )}
    <button class="flex w-full items-center gap-2 px-3.5 pt-2 pb-1.5 text-left" onclick={ontoggle}>
      <Icon
        name="chevronDown"
        size={12}
        class="transition-transform duration-200 {open ? '' : '-rotate-90'} {accent
          ? 'text-accent'
          : 'text-muted'}"
      />
      <span
        class="text-[11px] font-bold tracking-[0.09em] uppercase {accent
          ? 'text-accent'
          : 'text-muted'}"
      >
        {label}
      </span>
      <span
        class="rounded-full px-1.75 text-[11px] font-bold tabular-nums
               {accent ? 'bg-accent text-on-accent' : 'bg-rail text-ink-soft'}"
      >
        {count}
      </span>
    </button>
  {/snippet}

  <div
    class="msg-list min-h-0 flex-1 overflow-y-auto px-2 pb-2"
    bind:this={listEl}
    onscroll={(e) => {
      // Zapas 400 px, żeby kolejna strona zdążyła dojść, zanim dojedziesz
      // do samego dołu - inaczej lista szarpałaby przy każdym doładowaniu.
      const el = e.currentTarget;
      if (el.scrollHeight - el.scrollTop - el.clientHeight < 400) onloadmore();
    }}
  >
    {#if messages.length === 0}
      <div class="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
        <Icon name="inbox" size={28} class="text-line" />
        <p class="text-sm text-muted">
          {searchQuery.trim() ? "Brak wyników dla tego zapytania." : "Ta lista jest pusta."}
        </p>
      </div>
    {:else if grouped}
      {@render groupHeader("Nowe", unseen.length, newOpen, true, () => (newOpen = !newOpen))}
      {#if newOpen}
        <div transition:slide={{ duration: 180 }}>
          {#each unseenShown as m (m.id)}
            <div in:fade={MOVE_IN} out:fade={MOVE_OUT} animate:flip={FLIP}>
              {@render card(m)}
            </div>
          {/each}
          {#if unseen.length > unseenShown.length}
            {@render moreRow(unseen.length - unseenShown.length, () => (newAll = true))}
          {/if}
        </div>
      {/if}

      {@render groupHeader("Przejrzane", seen.length, seenOpen, false, () => (seenOpen = !seenOpen))}
      {#if seenOpen}
        <div transition:slide={{ duration: 180 }}>
          {#each seen as m (m.id)}
            <div in:fade={MOVE_IN} out:fade={MOVE_OUT} animate:flip={FLIP}>
              {@render compact(m)}
            </div>
          {/each}
        </div>
      {/if}
    {:else}
      {#each messages as m (m.id)}
        {@render compact(m)}
      {/each}
    {/if}

    <!-- Stopka listy mówi wprost, na czym stoimy: czy coś jeszcze idzie,
         czy zostało do dociągnięcia, czy to już koniec. Bez tego dojechanie
         do dna wyglądało tak samo jak zawieszenie. -->
    {#if messages.length > 0}
      <div class="py-3 text-center text-[12px] text-muted">
        {#if loadingMore}
          <span class="flex items-center justify-center gap-2">
            <Icon name="refresh" size={12} class="animate-spin text-accent" />
            Dociągam starsze…
          </span>
        {:else if hasMore}
          <button
            class="rounded-lg px-3 py-1.5 font-semibold text-accent transition-colors hover:bg-rail"
            onclick={onloadmore}
          >
            Pokaż starsze
          </button>
        {:else}
          To już wszystko
        {/if}
      </div>
    {/if}
  </div>

  {#snippet card(m: MessageSummary)}
    <button
      class="mb-0.75 flex w-full gap-2.75 rounded-xl px-3 py-2.75 text-left transition-colors
             {selectedId === m.id ? 'bg-accent-soft' : 'hover:bg-rail'}"
      data-mid={m.id}
      onclick={() => onopen(m)}
      oncontextmenu={(e) => openContext(e, m)}
      draggable="true"
      ondragstart={(e) => e.dataTransfer?.setData("text/lotus-id", String(m.id))}
    >
      <span
        class="grid size-8.5 shrink-0 place-items-center rounded-[11px] text-xs font-bold
               {theme.dark ? 'text-[#08120f]' : 'text-white'}"
        style="background:{avatarColor(m.fromAddr, theme.dark)}"
      >
        {initials(m.fromName, m.fromAddr)}
      </span>
      <div class="min-w-0 flex-1">
        <div class="flex items-baseline gap-2">
          <span class="min-w-0 flex-1 truncate text-[13.5px] font-bold text-ink">
            {m.fromName || m.fromAddr}
          </span>
          <span class="shrink-0 text-[11.5px] tabular-nums text-muted">{dateOf(m.date)}</span>
        </div>
        <div class="mt-px flex items-center gap-1.5">
          <span class="min-w-0 flex-1 truncate text-[13px] font-semibold text-ink">
            {m.subject || "(bez tematu)"}
          </span>
          {#if m.threadCount > 1}
            <span
              class="flex shrink-0 items-center gap-0.75 rounded-full bg-rail px-1.5 text-[10.5px]
                     font-bold tabular-nums text-ink-soft"
              title="{m.threadCount} wiadomości w konwersacji"
            >
              <Icon name="reply" size={9} />
              {m.threadCount}
            </span>
          {/if}
          {#if m.isFlagged}<Icon name="flag" size={12} class="shrink-0 text-flag" />{/if}
          {#if m.snoozedUntil}<Icon name="moon" size={12} class="shrink-0 text-accent" />{/if}
        </div>
        <p class="mt-0.75 line-clamp-2 text-[12.5px] leading-snug text-muted">{m.preview}</p>
        {#if m.hasAttachments}
          <span
            class="mt-1.75 inline-flex items-center gap-1.25 rounded-md bg-surface px-2 py-0.75
                   text-[11.5px] text-ink-soft shadow-[var(--chip-shadow)]"
          >
            <Icon name="paperclip" size={11} />
            Załącznik
          </span>
        {/if}
      </div>
    </button>
  {/snippet}

  {#snippet compact(m: MessageSummary)}
    <button
      class="mb-0.25 flex w-full items-center gap-2.75 rounded-xl px-3 py-2 text-left transition-colors
             {selectedId === m.id ? 'bg-accent-soft' : 'hover:bg-rail'}"
      data-mid={m.id}
      onclick={() => onopen(m)}
      oncontextmenu={(e) => openContext(e, m)}
      draggable="true"
      ondragstart={(e) => e.dataTransfer?.setData("text/lotus-id", String(m.id))}
    >
      <span
        class="grid size-6.5 shrink-0 place-items-center rounded-[9px] bg-rail text-[10.5px]
               font-bold text-ink-soft"
      >
        {initials(m.fromName, m.fromAddr)}
      </span>
      <!-- Nadawca i data w pierwszym wierszu, temat w drugim. W jednej linii
           temat konkurował o miejsce z nadawcą i datą, więc obcinał się już
           przy kilku słowach - a to jego zwykle się szuka. -->
      <span class="min-w-0 flex-1">
        <span class="flex items-baseline gap-2">
          <span class="min-w-0 flex-1 truncate text-[12.5px] font-semibold text-ink-soft">
            {m.fromName || m.fromAddr}
          </span>
          <span class="shrink-0 text-[11.5px] tabular-nums text-muted">{dateOf(m.date)}</span>
        </span>
        <span class="flex items-center gap-1.5">
          <span class="min-w-0 flex-1 truncate text-[13px] text-ink" title={m.subject}>
            {m.subject || "(bez tematu)"}
          </span>
          {#if m.threadCount > 1}
            <span
              class="flex shrink-0 items-center gap-0.75 rounded-full bg-rail px-1.5 text-[10.5px]
                     font-bold tabular-nums text-ink-soft"
              title="{m.threadCount} wiadomości w konwersacji"
            >
              <Icon name="reply" size={9} />
              {m.threadCount}
            </span>
          {/if}
          {#if m.isFlagged}<Icon name="flag" size={11} class="shrink-0 text-flag" />{/if}
          {#if m.snoozedUntil}<Icon name="moon" size={11} class="shrink-0 text-accent" />{/if}
        </span>
      </span>
    </button>
  {/snippet}

  {#if ctx}
    <div
      class="fixed z-40 w-56 rounded-xl bg-surface p-1.5 shadow-xl ring-1 ring-line"
      style="left:{ctx.x}px; top:{ctx.y}px; transform-origin: top left"
      transition:scale={{ start: 0.95, duration: 120 }}
    >
      {#snippet ctxItem(icon: string, label: string, onclick: () => void, danger = false)}
        <button
          class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]
                 transition-colors {danger
            ? 'text-danger hover:bg-danger/10'
            : 'hover:bg-accent-soft'}"
          {onclick}
        >
          <Icon name={icon} size={14} class={danger ? "" : "text-muted"} />
          {label}
        </button>
      {/snippet}

      {@render ctxItem("mail", "Otwórz", () => ctxAction("open"))}
      {@render ctxItem(
        "inbox",
        isUnread(ctx.m) ? "Oznacz jako przeczytane" : "Oznacz jako nieprzeczytane",
        () => ctxAction(isUnread(ctx!.m) ? "read" : "unread"),
      )}
      {@render ctxItem("flag", ctx.m.isFlagged ? "Zdejmij flagę" : "Oflaguj", () => ctxAction("flag"))}
      <div class="my-1 h-px bg-line"></div>
      {@render ctxItem("folder", "Zawsze przenoś od tego nadawcy…", () => ctxAction("rule"))}
      <div class="my-1 h-px bg-line"></div>
      {@render ctxItem("moon", "Drzemka na 3 godziny", () =>
        ctxAction("snooze", Math.floor(Date.now() / 1000) + 3 * 3600),
      )}
      {@render ctxItem("moon", "Drzemka do jutra (9:00)", () => ctxAction("snooze", snoozeTomorrow()))}
      <div class="my-1 h-px bg-line"></div>
      {@render ctxItem("trash", "Usuń", () => ctxAction("delete"), true)}
    </div>
  {/if}
</section>

<svelte:window
  onclick={() => {
    ctx = null;
    sortOpen = false;
  }}
  onkeydown={(e) => {
    if (e.key !== "Escape") return;
    ctx = null;
    sortOpen = false;
  }}
/>