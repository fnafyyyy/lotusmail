<script lang="ts">
  import { onDestroy } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, initials } from "$lib/api";
  import { SpellChecker, spellReady, type SpellHit } from "$lib/spell";
  import type { Account, Contact, LocalDraft } from "$lib/types";
  import Icon from "./Icon.svelte";

  let {
    draft,
    accounts,
    embedded = false,
    onsend,
    onclose,
    ondetach = null,
  }: {
    draft: LocalDraft;
    accounts: Account[];
    /** true = szkic obok wiadomości (prawa połowa), nie osobna karta. */
    embedded?: boolean;
    onsend: (sendAt: number | null) => void;
    onclose: () => void;
    ondetach?: (() => void) | null;
  } = $props();

  let laterOpen = $state(false);
  // Wiersze kopii pokazujemy dopiero na żądanie - przy zwykłym mailu tylko
  // zaśmiecałyby nagłówek. „Odpowiedz wszystkim" wypełnia DW, więc wtedy
  // otwierają się same.
  // svelte-ignore state_referenced_locally
  let extraOpen = $state(draft.ccAddrs.trim().length > 0 || draft.bccAddrs.trim().length > 0);
  let editor: HTMLDivElement;
  let toInput: HTMLInputElement;
  let ready = $derived(draft.toAddrs.trim().length > 0);
  let isReply = $derived(draft.isReply === true);

  // Autouzupełnianie adresatów - podpowiedzi z historii poczty dla tokenu
  // po ostatnim przecinku.
  let suggestions = $state<Contact[]>([]);
  let sugIndex = $state(0);
  let sugTimer: ReturnType<typeof setTimeout>;

  function currentToken(): string {
    const parts = draft.toAddrs.split(",");
    return parts[parts.length - 1].trim();
  }

  function onToInput() {
    clearTimeout(sugTimer);
    const token = currentToken();
    if (token.length < 2) {
      suggestions = [];
      return;
    }
    sugTimer = setTimeout(async () => {
      const already = draft.toAddrs.toLowerCase();
      const hits = await api.searchContacts(token);
      suggestions = hits.filter((c) => !already.includes(c.addr.toLowerCase()));
      sugIndex = 0;
    }, 150);
  }

  function acceptSuggestion(c: Contact) {
    const parts = draft.toAddrs.split(",");
    parts[parts.length - 1] = ` ${c.addr}`;
    draft.toAddrs = parts.join(",").replace(/^\s+/, "") + ", ";
    suggestions = [];
    toInput.focus();
  }

  function onToKeydown(e: KeyboardEvent) {
    if (suggestions.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      sugIndex = (sugIndex + 1) % suggestions.length;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      sugIndex = (sugIndex - 1 + suggestions.length) % suggestions.length;
    } else if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      acceptSuggestion(suggestions[sugIndex]);
    } else if (e.key === "Escape") {
      suggestions = [];
    }
  }

  // Formatowanie przez execCommand - przestarzałe wg specyfikacji, ale w
  // WebView2 działa niezawodnie i nie wymaga ciężkiego edytora.
  function fmt(command: string, value?: string) {
    editor.focus();
    document.execCommand(command, false, value);
    draft.bodyHtml = editor.innerHTML;
  }

  // Sprawdzanie pisowni: słownik systemowy Windows, podkreślenia rysowane
  // bez ingerencji w treść (CSS Custom Highlight API).
  let spell: SpellChecker | null = null;
  /** Czy system ma słownik, a WebView potrafi rysować podkreślenia. */
  let spellSupported = $state(false);
  /** Przełącznik użytkownika (ikonka „abc" na pasku narzędzi). */
  let spellOn = $state(true);
  let spellMenu = $state<{ x: number; y: number; hit: SpellHit; tips: string[] } | null>(null);

  $effect(() => {
    if (!editor || spell) return;
    const checker = new SpellChecker(editor);
    spellReady().then((ok) => {
      spellSupported = ok;
      if (!ok) return;
      spell = checker;
      if (spellOn) checker.schedule(300);
    });
  });

  onDestroy(() => spell?.destroy());

  function onBodyInput() {
    draft.bodyHtml = editor.innerHTML;
    spellMenu = null;
    if (spellOn) spell?.schedule();
  }

  async function onBodyContextMenu(e: MouseEvent) {
    e.preventDefault();
    spellMenu = null;
    if (!spellOn || !spell) return;
    const hit = spell.hitAt(e.clientX, e.clientY);
    if (!hit) return;
    const tips = await api.spellSuggest(hit.word);
    spellMenu = { x: e.clientX, y: e.clientY, hit, tips };
  }

  function useSuggestion(word: string) {
    if (!spellMenu || !spell) return;
    spell.replace(spellMenu.hit, word);
    draft.bodyHtml = editor.innerHTML;
    spellMenu = null;
    spell.schedule(150);
  }

  async function addToDictionary() {
    if (!spellMenu || !spell) return;
    const word = spellMenu.hit.word;
    spellMenu = null;
    try {
      await api.spellAdd(word);
    } catch {
      // Brak dostępu do słownika użytkownika - pomijamy słowo choć do końca pisania.
      spell.ignore(word);
      return;
    }
    spell.schedule(0);
  }

  function toggleSpell() {
    spellOn = !spellOn;
    if (spellOn) spell?.schedule(0);
    else spell?.clear();
    spellMenu = null;
  }

  // Załączniki. Z okna wyboru pliku wracają ścieżki (czyta je Rust), a pliki
  // upuszczone na edytor daje przeglądarka - tam czytamy zawartość na miejscu.
  let dropActive = $state(false);
  let attachError = $state("");
  const MAX_BYTES = 25 * 1024 * 1024;

  let totalSize = $derived(draft.attachments.reduce((s, a) => s + a.size, 0));

  function fmtSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} kB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  async function pickFiles() {
    const picked = await open({ multiple: true, title: "Wybierz załączniki" });
    if (!picked) return;
    for (const path of Array.isArray(picked) ? picked : [picked]) {
      try {
        draft.attachments.push(await api.readAttachment(path));
      } catch (e) {
        attachError = String(e);
      }
    }
  }

  async function addDroppedFiles(files: FileList) {
    for (const file of Array.from(files)) {
      if (file.size > MAX_BYTES) {
        attachError = `„${file.name}" ma ${fmtSize(file.size)} - limit to 25 MB`;
        continue;
      }
      const buffer = await file.arrayBuffer();
      draft.attachments.push({
        filename: file.name,
        mime: file.type || "application/octet-stream",
        size: file.size,
        dataB64: toBase64(new Uint8Array(buffer)),
      });
    }
  }

  /** btoa na dużych tablicach przepełnia stos - stąd porcjowanie. */
  function toBase64(bytes: Uint8Array): string {
    let binary = "";
    for (let i = 0; i < bytes.length; i += 8192) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
    }
    return btoa(binary);
  }

  function onDrop(e: DragEvent) {
    // Przeciąganie maili wewnątrz aplikacji ma własną obsługę - tu tylko pliki.
    if (!e.dataTransfer?.files?.length) return;
    e.preventDefault();
    dropActive = false;
    void addDroppedFiles(e.dataTransfer.files);
  }

  // Spark: „wyślij później" - kolejka w outbox z ustawionym send_at.
  function laterPresets(): { label: string; at: number }[] {
    const now = new Date();
    const at = (d: Date, h: number) => {
      const c = new Date(d);
      c.setHours(h, 0, 0, 0);
      return Math.floor(c.getTime() / 1000);
    };
    const tomorrow = new Date(now);
    tomorrow.setDate(tomorrow.getDate() + 1);
    return [
      { label: "Za godzinę", at: Math.floor(now.getTime() / 1000) + 3600 },
      { label: "Dziś o 18:00", at: at(now, 18) },
      { label: "Jutro o 9:00", at: at(tomorrow, 9) },
    ];
  }
</script>

{#snippet tool(label: string, onclick: () => void, title: string)}
  <button
    class="grid size-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-line/60"
    {title}
    tabindex="-1"
    onmousedown={(e) => e.preventDefault()}
    {onclick}
  >
    {label}
  </button>
{/snippet}

{#snippet toolIcon(icon: string, onclick: () => void, title: string)}
  <button
    class="grid size-7 place-items-center rounded-md text-ink-soft transition-colors hover:bg-line/60"
    {title}
    tabindex="-1"
    onmousedown={(e) => e.preventDefault()}
    {onclick}
  >
    <Icon name={icon} size={14} />
  </button>
{/snippet}

<section class="flex min-h-0 flex-1 flex-col" in:fade={{ duration: 150 }}>
  <div class="flex h-full min-h-0 flex-col {embedded ? '' : 'px-4 py-3'}">
    <div class="panel relative flex min-h-0 flex-1 flex-col">
      {#if embedded}
        <div class="flex h-13 shrink-0 items-center gap-2 border-b border-line px-4.5">
          <Icon name={isReply ? "reply" : "edit"} size={15} class="text-accent" />
          <span class="text-[13px] font-semibold">{isReply ? "Odpowiedź" : "Nowa wiadomość"}</span>
          <span class="flex-1"></span>
          <button
            class="flex h-7.5 items-center gap-1.75 rounded-full bg-rail px-3.25 text-[12.5px]
                   font-semibold text-ink-soft hover:text-ink"
            onclick={() => ondetach?.()}
            title="Otwórz jako osobną kartę"
          >
            <Icon name="edit" size={13} />
            W karcie
          </button>
          <button
            class="grid size-7 place-items-center rounded-full text-muted hover:bg-rail hover:text-ink"
            onclick={onclose}
            aria-label="Zamknij odpowiedź"
            title="Zamknij odpowiedź"
          >
            <Icon name="x" size={14} />
          </button>
        </div>
      {/if}
      <div class="px-5 {embedded ? 'pt-1' : ''}">
        {#if accounts.length > 1}
          <label class="flex items-center gap-3 border-b border-line py-2.5 text-sm">
            <span class="w-14 shrink-0 text-muted">Od</span>
            <select bind:value={draft.accountId} class="flex-1 bg-transparent outline-none">
              {#each accounts as a (a.id)}
                <option value={a.id}>{a.displayName || a.email} &lt;{a.email}&gt;</option>
              {/each}
            </select>
          </label>
        {/if}
        <label class="relative flex items-center gap-3 border-b border-line py-2.5 text-sm">
          <span class="w-14 shrink-0 text-muted">Do</span>
          <input
            bind:this={toInput}
            bind:value={draft.toAddrs}
            placeholder="adres@domena.pl, drugi@domena.pl"
            class="flex-1 bg-transparent outline-none placeholder:text-muted"
            oninput={onToInput}
            onkeydown={onToKeydown}
            onblur={() => setTimeout(() => (suggestions = []), 150)}
          />
          {#if !extraOpen}
            <button
              class="shrink-0 rounded-md px-2 py-0.5 text-[12px] font-semibold text-muted
                     transition-colors hover:text-ink"
              onclick={(e) => {
                e.preventDefault();
                extraOpen = true;
              }}
              title="Kopia i ukryta kopia"
            >
              DW/UDW
            </button>
          {/if}
          {#if suggestions.length > 0}
            <div
              class="absolute top-full left-14 z-10 w-96 rounded-xl border border-line bg-surface
                     p-1.5 shadow-xl"
              transition:scale={{ start: 0.97, duration: 100 }}
              style="transform-origin: top left"
            >
              {#each suggestions as c, i (c.addr)}
                <button
                  class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left
                         {i === sugIndex ? 'bg-accent-soft' : 'hover:bg-paper'}"
                  onmousedown={(e) => e.preventDefault()}
                  onclick={() => acceptSuggestion(c)}
                >
                  <span
                    class="grid size-7 shrink-0 place-items-center rounded-full bg-accent text-[10px]
                           font-semibold text-on-accent"
                  >
                    {initials(c.name, c.addr)}
                  </span>
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-[13px] font-semibold">
                      {c.name || c.addr}
                    </span>
                    {#if c.name}
                      <span class="block truncate text-[11px] text-muted">{c.addr}</span>
                    {/if}
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        </label>
        {#if extraOpen}
          <label class="flex items-center gap-3 border-b border-line py-2.5 text-sm">
            <span class="w-14 shrink-0 text-muted">DW</span>
            <input
              bind:value={draft.ccAddrs}
              placeholder="adres@domena.pl, drugi@domena.pl"
              class="flex-1 bg-transparent outline-none placeholder:text-muted"
            />
          </label>
          <label class="flex items-center gap-3 border-b border-line py-2.5 text-sm">
            <span class="w-14 shrink-0 text-muted">UDW</span>
            <input
              bind:value={draft.bccAddrs}
              placeholder="ukryta kopia - pozostali odbiorcy jej nie zobaczą"
              class="flex-1 bg-transparent outline-none placeholder:text-muted"
            />
          </label>
        {/if}
        <label class="flex items-center gap-3 py-2.5 text-sm">
          <span class="w-14 shrink-0 text-muted">Temat</span>
          <input bind:value={draft.subject} class="flex-1 bg-transparent outline-none" />
        </label>
      </div>

      <div class="flex items-center gap-0.5 border-y border-line bg-paper/50 px-3 py-1.5">
        {@render tool("B", () => fmt("bold"), "Pogrubienie (Ctrl+B)")}
        {@render tool("I", () => fmt("italic"), "Kursywa (Ctrl+I)")}
        {@render tool("U", () => fmt("underline"), "Podkreślenie (Ctrl+U)")}
        {@render tool("S̶", () => fmt("strikeThrough"), "Przekreślenie")}
        <span class="mx-1 h-4 w-px bg-line"></span>
        {@render toolIcon("listUl", () => fmt("insertUnorderedList"), "Lista punktowana")}
        {@render toolIcon("listOl", () => fmt("insertOrderedList"), "Lista numerowana")}
        {@render toolIcon("quote", () => fmt("formatBlock", "blockquote"), "Cytat")}
        <span class="mx-1 h-4 w-px bg-line"></span>
        {@render tool("H", () => fmt("formatBlock", "h3"), "Nagłówek")}
        {@render tool("¶", () => fmt("formatBlock", "div"), "Zwykły tekst")}
        {@render toolIcon("eraser", () => fmt("removeFormat"), "Wyczyść formatowanie")}
        {#if spellSupported}
          <span class="mx-1 h-4 w-px bg-line"></span>
          <button
            class="grid h-7 place-items-center rounded-md px-1.5 text-[12px] font-semibold
                   transition-colors hover:bg-line/60
                   {spellOn ? 'text-accent' : 'text-muted'}"
            title={spellOn ? "Wyłącz sprawdzanie pisowni" : "Włącz sprawdzanie pisowni"}
            tabindex="-1"
            onmousedown={(e) => e.preventDefault()}
            onclick={toggleSpell}
          >
            <span class="underline decoration-wavy decoration-1 underline-offset-2">abc</span>
          </button>
        {/if}
      </div>

      <div
        bind:this={editor}
        contenteditable="true"
        bind:innerHTML={draft.bodyHtml}
        role="textbox"
        aria-multiline="true"
        aria-label="Treść wiadomości"
        tabindex="0"
        spellcheck="false"
        oninput={onBodyInput}
        oncontextmenu={onBodyContextMenu}
        ondragover={(e) => {
          if (!e.dataTransfer?.types.includes("Files")) return;
          e.preventDefault();
          dropActive = true;
        }}
        ondragleave={() => (dropActive = false)}
        ondrop={onDrop}
        class="editor min-h-0 flex-1 overflow-y-auto px-5 py-3 text-sm leading-relaxed outline-none
               {dropActive ? 'bg-accent-soft/40 ring-2 ring-accent ring-inset' : ''}"
      ></div>

      {#if dropActive}
        <p
          class="pointer-events-none absolute inset-x-0 bottom-24 text-center text-[13px]
                 font-semibold text-accent"
          transition:fade={{ duration: 100 }}
        >
          Upuść pliki, żeby je załączyć
        </p>
      {/if}

      {#if draft.attachments.length > 0 || attachError}
        <div class="flex flex-wrap items-center gap-1.5 border-t border-line px-5 py-2">
          {#each draft.attachments as a, i (a.filename + i)}
            <span
              class="flex max-w-64 items-center gap-1.5 rounded-lg bg-rail px-2 py-1 text-[12px]"
              transition:fade={{ duration: 120 }}
            >
              <Icon name="paperclip" size={11} class="shrink-0 text-muted" />
              <span class="min-w-0 flex-1 truncate" title={a.filename}>{a.filename}</span>
              <span class="shrink-0 text-[10.5px] tabular-nums text-muted">{fmtSize(a.size)}</span>
              <button
                class="shrink-0 rounded p-0.5 text-muted hover:text-danger"
                onclick={() => (draft.attachments = draft.attachments.filter((_, j) => j !== i))}
                aria-label="Usuń załącznik {a.filename}"
              >
                <Icon name="x" size={10} />
              </button>
            </span>
          {/each}
          {#if draft.attachments.length > 1}
            <span class="text-[11px] text-muted">razem {fmtSize(totalSize)}</span>
          {/if}
          {#if attachError}
            <span class="text-[11.5px] text-danger">{attachError}</span>
          {/if}
        </div>
      {/if}

      {#if spellMenu}
        <div
          class="panel fixed z-50 min-w-44 p-1 ring-1 ring-line"
          style="left:{spellMenu.x}px; top:{spellMenu.y}px"
          transition:fade={{ duration: 90 }}
        >
          {#if spellMenu.tips.length > 0}
            {#each spellMenu.tips.slice(0, 6) as tip (tip)}
              <button
                class="flex w-full items-center rounded-md px-2.5 py-1.5 text-left text-[13px]
                       font-semibold hover:bg-rail"
                onclick={() => useSuggestion(tip)}
              >
                {tip}
              </button>
            {/each}
            <div class="my-1 h-px bg-line"></div>
          {:else}
            <p class="px-2.5 py-1.5 text-[12.5px] text-muted">Brak podpowiedzi</p>
          {/if}
          <button
            class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px]
                   text-ink-soft hover:bg-rail"
            onclick={addToDictionary}
          >
            <Icon name="plus" size={12} />
            Dodaj do słownika
          </button>
          <button
            class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px]
                   text-ink-soft hover:bg-rail"
            onclick={() => {
              if (spellMenu) spell?.ignore(spellMenu.hit.word);
              spellMenu = null;
            }}
          >
            <Icon name="eraser" size={12} />
            Pomiń w tej wiadomości
          </button>
        </div>
      {/if}

      <footer class="relative flex items-center gap-2 border-t border-line px-5 py-3">
        <button
          class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-on-accent
                 transition-opacity hover:opacity-90 disabled:opacity-40"
          disabled={!ready}
          onclick={() => onsend(null)}
        >
          Wyślij
        </button>
        <button
          class="flex items-center gap-1.5 rounded-lg border border-line px-3 py-2 text-sm
                 font-semibold text-ink-soft hover:bg-paper disabled:opacity-40"
          disabled={!ready}
          onclick={() => (laterOpen = !laterOpen)}
        >
          <Icon name="clock" size={14} />
          Wyślij później
          <Icon name="chevronDown" size={12} />
        </button>
        <button
          class="flex items-center gap-1.5 rounded-lg border border-line px-3 py-2 text-sm
                 font-semibold text-ink-soft hover:bg-paper"
          onclick={pickFiles}
          title="Załącz pliki (możesz je też upuścić na treść wiadomości)"
        >
          <Icon name="paperclip" size={14} />
          Załącz
          {#if draft.attachments.length > 0}
            <span class="text-accent">{draft.attachments.length}</span>
          {/if}
        </button>
        <span class="flex-1"></span>
        <button
          class="flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-semibold text-muted
                 hover:bg-paper hover:text-danger"
          onclick={onclose}
        >
          <Icon name="trash" size={14} />
          Odrzuć
        </button>

        {#if laterOpen}
          <div
            class="absolute bottom-14 left-24 z-10 w-52 rounded-xl border border-line bg-surface
                   p-1.5 shadow-lg"
            style="transform-origin: bottom left"
            transition:scale={{ start: 0.95, duration: 130 }}
          >
            {#each laterPresets() as preset (preset.label)}
              <button
                class="block w-full rounded-lg px-2.5 py-1.5 text-left text-[13px] hover:bg-accent-soft"
                onclick={() => {
                  laterOpen = false;
                  onsend(preset.at);
                }}
              >
                {preset.label}
              </button>
            {/each}
          </div>
        {/if}
      </footer>
    </div>
  </div>
</section>

<svelte:window
  onclick={() => (spellMenu = null)}
  onkeydown={(e) => e.key === "Escape" && (spellMenu = null)}
/>

<style>
  .editor :global(blockquote) {
    border-left: 3px solid var(--line);
    margin: 0.4em 0;
    padding-left: 12px;
    color: var(--ink-soft);
  }
  .editor :global(ul) {
    list-style: disc;
    padding-left: 1.4em;
  }
  .editor :global(ol) {
    list-style: decimal;
    padding-left: 1.4em;
  }
  .editor :global(h3) {
    font-size: 1.15em;
    font-weight: 700;
    margin: 0.5em 0 0.25em;
  }
  .editor :global(a) {
    color: var(--accent);
  }
</style>
