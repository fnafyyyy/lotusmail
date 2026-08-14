<script lang="ts">
  import { fade, fly, scale, slide } from "svelte/transition";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type { MessageBody, MessageSummary } from "$lib/types";
  import { api, fmtDate, fmtDateFull, initials } from "$lib/api";
  import { theme } from "$lib/theme.svelte";
  import Icon from "./Icon.svelte";
  import Attachments from "./Attachments.svelte";

  let {
    message,
    body,
    thread = [],
    accountLabel = "",
    onreply,
    onreplyall,
    onforward,
    onsnooze,
    onflag,
    ondelete,
    ontoast,
    onclosepane = null,
  }: {
    message: MessageSummary | null;
    body: MessageBody | null;
    /** Cała konwersacja (od najstarszej). Pusta = pojedyncza wiadomość. */
    thread?: MessageSummary[];
    accountLabel?: string;
    onreply: () => void;
    onreplyall: () => void;
    onforward: () => void;
    onsnooze: (until: number) => void;
    onflag: () => void;
    ondelete: () => void;
    ontoast: (text: string) => void;
    onclosepane?: (() => void) | null;
  } = $props();

  let snoozeOpen = $state(false);
  // Treści pobrane dla rozwiniętych wiadomości wątku.
  let bodies = $state<Record<number, MessageBody>>({});
  let expanded = $state<Set<number>>(new Set());

  // Konwersacja: gdy wątek ma jedną wiadomość, zachowujemy zwykły widok.
  let items = $derived(thread.length > 1 ? thread : message ? [message] : []);
  let isThread = $derived(thread.length > 1);

  // Nowa wiadomość w panelu → rozwijamy wybraną, resztę zwijamy.
  $effect(() => {
    const id = message?.id;
    if (id == null) return;
    expanded = new Set([id]);
    bodies = body ? { [id]: body } : {};
  });

  async function toggle(m: MessageSummary) {
    const next = new Set(expanded);
    if (next.has(m.id)) {
      next.delete(m.id);
      expanded = next;
      // Zwinięta wiadomość oddaje treść. Mail z obrazkami osadzonymi jako
      // data: to bywa kilka megabajtów łańcucha - trzymanie ich dla wszystkich
      // kiedykolwiek otwartych wiadomości wątku rosło bez końca.
      if (m.id !== message?.id) {
        const rest = { ...bodies };
        delete rest[m.id];
        bodies = rest;
      }
      return;
    }
    next.add(m.id);
    expanded = next;
    if (!bodies[m.id]) {
      const b = await api.getMessageBody(m.id);
      bodies = { ...bodies, [m.id]: b };
    }
  }

  // Presety drzemki w stylu Sparka.
  function snoozePresets(): { label: string; until: number }[] {
    const now = new Date();
    const at = (d: Date, h: number) => {
      const c = new Date(d);
      c.setHours(h, 0, 0, 0);
      return Math.floor(c.getTime() / 1000);
    };
    const addDays = (n: number) => {
      const c = new Date(now);
      c.setDate(c.getDate() + n);
      return c;
    };
    const nextSaturday = addDays(((6 - now.getDay()) % 7) || 7);
    const nextMonday = addDays(((1 - now.getDay() + 7) % 7) || 7);
    return [
      { label: "Za 3 godziny", until: Math.floor(now.getTime() / 1000) + 3 * 3600 },
      { label: "Jutro rano (9:00)", until: at(addDays(1), 9) },
      { label: "W weekend (sob. 9:00)", until: at(nextSaturday, 9) },
      { label: "W przyszłym tygodniu (pon. 9:00)", until: at(nextMonday, 9) },
    ];
  }

  function escapeHtml(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // Mail deklarujący własne kolory dostaje w ciemnym motywie inteligentną
  // inwersję (jak Gmail); maile bez własnych kolorów - paletę aplikacji.
  function declaresOwnColors(html: string): boolean {
    return /bgcolor\s*=|background(?:-color)?\s*:|[^-]color\s*:/i.test(html);
  }

  // Cytowana historia („Od: … Wysłano: … Temat: …", `>` w tekście) powtarza to,
  // co widać wyżej w konwersacji - odcinamy ją i chowamy za przyciskiem.
  const QUOTE_MARKERS = [
    'id="divrplyfwdmsg"',
    'id="appendonsend"',
    'id="mail-editor-reference-message-container"',
    'class="gmail_quote',
    "<blockquote type=\"cite\"",
    "-----original message-----",
    "-----wiadomość oryginalna-----",
  ];

  /// Nagłówek cytatu w treści („Od: … Wysłano: … Temat: …" / „From: … Sent: …"),
  /// rozpoznawany po samym tekście - atrybuty id/class bywają usuwane.
  /// „Od:"/„From:" muszą mieć dwukropek, a „W dniu"/„Dnia" - domknięcie
  /// w „napisał(a):". Bez tego wzorzec łapał zwykłe słowa zaczynające się od
  /// „od" (choćby „odpowiedź"): cięcie wypadało w środku pierwszego zdania,
  /// bezpiecznik od zbyt krótkiej treści rezygnował i mail szedł z cytatem.
  function findQuoteHeader(lower: string): number {
    const re = /(?:^|>)[^<>]{0,40}\b(?:(od|from)\s*:|(w dniu|dnia)\b)/g;
    for (let m = re.exec(lower); m; m = re.exec(lower)) {
      const window = lower.slice(m.index, m.index + 700);
      const wrote = /napisa[łl]\(?a?\)?\s*:/.test(window);
      if (m[2]) {
        // Sama data to jeszcze nie cytat - dopiero „W dniu … napisał(a):".
        if (wrote) return m.index;
        continue;
      }
      const hasSent = /(wysłano|wyslano|sent)\s*:/.test(window);
      const hasSubject = /(temat|subject)\s*:/.test(window);
      if ((hasSent && hasSubject) || wrote) return m.index;
    }
    return -1;
  }

  function splitQuotedHtml(html: string): [string, string | null] {
    const lower = html.toLowerCase();
    let cut = -1;
    for (const marker of QUOTE_MARKERS) {
      const i = lower.indexOf(marker);
      if (i >= 0 && (cut < 0 || i < cut)) cut = i;
    }
    const byText = findQuoteHeader(lower);
    if (byText >= 0 && (cut < 0 || byText < cut)) cut = byText;
    if (cut < 0) return [html, null];
    // Cofnij się do początku znacznika i ewentualnej poprzedzającej linii <hr>.
    let start = html.lastIndexOf("<", cut);
    if (start < 0) start = cut;
    const hr = lower.lastIndexOf("<hr", start);
    if (hr >= 0 && start - hr < 220) start = hr;
    const main = html.slice(0, start).trim();
    // Zbyt agresywne cięcie (nic nie zostało) → pokazujemy całość.
    return main.length < 20 ? [html, null] : [main, html.slice(start)];
  }

  function splitQuotedText(text: string): [string, string | null] {
    const lines = text.split("\n");
    const idx = lines.findIndex((l) =>
      /^\s*(>|od:\s|from:\s|w dniu .*napisa|dnia .*napisa|-{2,}\s*(original message|wiadomość oryginalna))/i.test(
        l,
      ),
    );
    if (idx <= 0) return [text, null];
    return [lines.slice(0, idx).join("\n").trimEnd(), lines.slice(idx).join("\n")];
  }

  /// Otwiera odnośniki z treści maila w systemowej przeglądarce.
  ///
  /// Ramka jest sandboxowana, więc samo kliknięcie nic nie daje: `target="_blank"`
  /// wymagałby `allow-popups`, a odnośnik bez celu wciągnąłby stronę do środka
  /// ramki zamiast otworzyć ją obok. Przechwytujemy więc kliknięcie i oddajemy
  /// adres systemowi.
  ///
  /// Wpuszczamy wyłącznie http(s), mailto i tel - `javascript:` czy `data:`
  /// z cudzego maila nie mają czego tu szukać.
  function externalLinks(el: HTMLIFrameElement) {
    let doc: Document | null = null;

    const onClick = (e: MouseEvent) => {
      const target = e.target as Element | null;
      const link = target?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!link) return;
      const href = link.getAttribute("href")?.trim() ?? "";
      // Kotwica w obrębie maila zostaje ramce.
      if (!href || href.startsWith("#")) return;
      e.preventDefault();
      if (!/^(https?:|mailto:|tel:)/i.test(href)) return;
      void openUrl(href).catch((err: unknown) =>
        ontoast(`Nie udało się otworzyć odnośnika: ${err}`),
      );
    };

    const attach = () => {
      doc?.removeEventListener("click", onClick, true);
      doc = el.contentDocument;
      doc?.addEventListener("click", onClick, true);
    };

    el.addEventListener("load", attach);
    if (el.contentDocument?.readyState === "complete") attach();

    return {
      destroy() {
        doc?.removeEventListener("click", onClick, true);
        el.removeEventListener("load", attach);
      },
    };
  }

  /// Dopasowuje wysokość ramki do treści maila. Bez tego karta w konwersacji
  /// ma sztywną wysokość: dłuższy mail przewija się wewnątrz siebie zamiast
  /// razem z wątkiem, a krótki zostawia pół ekranu pustki.
  ///
  /// Zmierzenie treści wymaga dostępu do dokumentu ramki, stąd
  /// `sandbox="allow-same-origin"`. Skrypty nadal są zablokowane - brak
  /// `allow-scripts` wyłącza je niezależnie od origin - więc mail dalej nie
  /// może nic wykonać ani sięgnąć do aplikacji.
  function autosize(el: HTMLIFrameElement) {
    let observer: ResizeObserver | undefined;
    let last = -1;

    const fit = () => {
      const body = el.contentDocument?.body;
      // Gdyby pomiar był niemożliwy, wracamy do dawnej stałej wysokości.
      if (!body) {
        el.style.height = "35rem";
        return;
      }
      // Mierzymy `body`, nie `documentElement`: ten drugi rozciąga się na całą
      // ramkę, więc po zwinięciu cytowanej historii wysokość by nie zmalała.
      const h = Math.ceil(Math.max(body.scrollHeight, body.offsetHeight));
      if (h !== last) {
        last = h;
        el.style.height = `${h}px`;
      }
    };

    const onload = () => {
      fit();
      const body = el.contentDocument?.body;
      if (!body) return;
      // Obrazki dochodzą już po załadowaniu dokumentu, a cytowana historia
      // rozwija się checkboxem - jedno i drugie zmienia wysokość.
      observer?.disconnect();
      observer = new ResizeObserver(fit);
      observer.observe(body);
    };

    el.addEventListener("load", onload);
    if (el.contentDocument?.readyState === "complete") onload();

    return {
      destroy() {
        observer?.disconnect();
        el.removeEventListener("load", onload);
      },
    };
  }

  // Treść maila renderuje się w sandboxowanym iframe: bez skryptów, bez
  // dostępu do aplikacji. HTML jest już zsanityzowany po stronie Rust.
  function makeSrcdoc(b: MessageBody | undefined): string {
    if (!b) return "";
    let mainHtml: string;
    let quotedHtml: string | null;
    if (b.html) {
      // `blob:` to odnośnik do pamięci klienta nadawcy (Outlook mobile) -
      // nigdy się nie załaduje, więc usuwamy zamiast pokazywać „Image".
      // Obrazki poza widokiem nie mają być dekodowane - to one napełniają
      // pamięć procesu GPU. Przy mailach z galerią zdjęć różnica idzie
      // w setki megabajtów.
      const html = b.html
        .replace(/<img[^>]*src="blob:[^"]*"[^>]*>/gi, "")
        .replace(/<img\s/gi, '<img loading="lazy" decoding="async" ');
      [mainHtml, quotedHtml] = splitQuotedHtml(html);
    } else {
      const [m, q] = splitQuotedText(b.text ?? "");
      const pre = (s: string) =>
        `<pre style="white-space:pre-wrap;font:inherit;margin:0">${escapeHtml(s)}</pre>`;
      mainHtml = pre(m);
      quotedHtml = q ? pre(q) : null;
    }
    // Przełącznik bez JavaScriptu (iframe jest w pełni sandboxowany).
    const inner =
      mainHtml +
      (quotedHtml
        ? `<input type="checkbox" id="lm-q" class="lm-toggle">
           <label for="lm-q" class="lm-label"><span class="lm-dots">•••</span><span class="lm-txt">Pokaż cytowaną historię</span></label>
           <div class="lm-quoted">${quotedHtml}</div>`
        : "");
    const ownColors = b.html ? declaresOwnColors(b.html) : false;
    const invert = theme.dark && ownColors;
    const c =
      theme.dark && !ownColors
        ? { bg: "#141d22", fg: "#c6d3da", link: "#4dc0b2", quote: "#1e2a31", quoteFg: "#8195a1" }
        : { bg: "#ffffff", fg: "#2b3a44", link: "#0d5f6e", quote: "#eef2f4", quoteFg: "#5b6b76" };
    const invertCss = invert
      ? `html{filter:invert(0.92) hue-rotate(180deg);background:#ffffff}
         img,video,svg,[style*="background-image"]{filter:invert(1.087) hue-rotate(180deg)}`
      : "";
    return `<!doctype html><html><head><meta charset="utf-8"><style>
      ${invertCss}
      body{font-family:'Segoe UI Variable Text','Segoe UI',system-ui,sans-serif;font-size:15px;
           background:${c.bg};color:${c.fg};margin:0;padding:4px 38px 30px;line-height:1.7;
           word-wrap:break-word}
      img{max-width:100%;height:auto} a{color:${c.link}}
      table{border-collapse:collapse}
      blockquote{border-left:3px solid ${c.quote};margin-left:0;padding-left:12px;color:${c.quoteFg}}
      .lm-toggle{position:absolute;opacity:0;pointer-events:none}
      .lm-label{display:inline-flex;align-items:center;gap:8px;margin:18px 0 0;padding:3px 12px;
        border-radius:999px;background:${c.quote};color:${c.quoteFg};font-size:12px;
        line-height:1.8;cursor:pointer;user-select:none}
      .lm-dots{letter-spacing:1px}
      .lm-quoted{display:none;margin-top:14px;padding-left:14px;
        border-left:3px solid ${c.quote};color:${c.quoteFg}}
      .lm-toggle:checked ~ .lm-quoted{display:block}
      .lm-toggle:checked ~ .lm-label .lm-txt{visibility:hidden;width:0;display:none}
      .lm-toggle:checked ~ .lm-label::after{content:"Ukryj cytowaną historię"}
    </style></head><body>${inner}</body></html>`;
  }
</script>

{#snippet chip(icon: string, label: string, onclick: () => void, active = false)}
  <button
    class="flex h-7.5 items-center gap-1.75 rounded-full px-3.25 text-[12.5px] font-semibold
           {active ? 'bg-accent-soft text-flag' : 'bg-rail text-ink-soft hover:text-ink'}"
    {onclick}
  >
    <Icon name={icon} size={14} />
    {label}
  </button>
{/snippet}

<section class="panel flex h-full min-w-0 flex-1 flex-col">
  {#if !message}
    <div class="flex h-full flex-col items-center justify-center gap-3" in:fade={{ duration: 200 }}>
      <span class="grid size-14 place-items-center rounded-2xl bg-rail">
        <Icon name="mail" size={26} class="text-muted" />
      </span>
      <p class="text-sm text-muted">Wybierz wiadomość, żeby ją przeczytać</p>
    </div>
  {:else}
    {#key message.threadId || message.id}
      <div class="flex h-full min-h-0 flex-col" in:fly={{ y: 10, duration: 200 }}>
        <div class="relative flex h-13 shrink-0 items-center gap-1.5 border-b border-line px-4.5">
          {@render chip("moon", "Drzemka", () => (snoozeOpen = !snoozeOpen))}
          {@render chip(
            "flag",
            message.isFlagged ? "Zdejmij flagę" : "Flaga",
            onflag,
            message.isFlagged,
          )}
          {@render chip("trash", "Usuń", ondelete)}
          <span class="flex-1"></span>
          {#if accountLabel}
            <span class="rounded-full bg-accent-soft px-2.5 py-1 text-[11.5px] font-semibold text-accent">
              {accountLabel}
            </span>
          {/if}
          {#if onclosepane}
            <button
              class="grid size-7 place-items-center rounded-full text-muted hover:bg-rail hover:text-ink"
              onclick={onclosepane}
              title="Zamknij ten panel"
              aria-label="Zamknij ten panel"
            >
              <Icon name="x" size={14} />
            </button>
          {/if}

          {#if snoozeOpen}
            <div
              class="absolute top-11 left-4 z-10 w-64 rounded-xl bg-surface p-1.5 shadow-lg ring-1 ring-line"
              style="transform-origin: top left"
              transition:scale={{ start: 0.95, duration: 130 }}
            >
              <p class="px-2.5 pt-1 pb-1.5 text-[11px] font-bold tracking-[0.09em] text-muted uppercase">
                Przypomnij mi
              </p>
              {#each snoozePresets() as preset (preset.label)}
                <button
                  class="block w-full rounded-lg px-2.5 py-1.5 text-left text-[13px] hover:bg-accent-soft"
                  onclick={() => {
                    snoozeOpen = false;
                    onsnooze(preset.until);
                  }}
                >
                  {preset.label}
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <div class="selectable shrink-0 px-9.5 pt-6 pb-3">
          <h1 class="tight font-display text-[24px] leading-tight font-bold text-pretty">
            {message.subject || "(bez tematu)"}
          </h1>
          {#if isThread}
            <p class="mt-1.5 flex items-center gap-1.5 text-[12.5px] text-muted">
              <Icon name="reply" size={12} />
              Konwersacja · {items.length} wiadomości
            </p>
          {/if}
        </div>

        <div class="min-h-0 flex-1 overflow-y-auto {isThread ? 'px-4 pb-4' : 'flex flex-col'}">
          {#each items as m (m.id)}
            {@const open = expanded.has(m.id)}
            {#if isThread}
              <!-- Konwersacja: każda wiadomość jako składana karta -->
              <div class="mb-2 overflow-hidden rounded-xl bg-rail/60 ring-1 ring-line">
                <button
                  class="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-rail"
                  onclick={() => toggle(m)}
                >
                  <span
                    class="grid size-8 shrink-0 place-items-center rounded-[10px] bg-accent text-[11px]
                           font-bold text-on-accent"
                  >
                    {initials(m.fromName, m.fromAddr)}
                  </span>
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-[13.5px] font-semibold">
                      {m.fromName || m.fromAddr}
                    </span>
                    {#if !open}
                      <span class="block truncate text-[12.5px] text-muted">{m.preview}</span>
                    {:else}
                      <span class="block truncate text-[12px] text-muted">{m.fromAddr}</span>
                    {/if}
                  </span>
                  {#if m.isFlagged}<Icon name="flag" size={12} class="shrink-0 text-flag" />{/if}
                  {#if m.hasAttachments}
                    <Icon name="paperclip" size={12} class="shrink-0 text-muted" />
                  {/if}
                  <span class="shrink-0 text-[11.5px] tabular-nums text-muted">
                    {open ? fmtDateFull(m.date) : fmtDate(m.date)}
                  </span>
                  <Icon
                    name="chevronDown"
                    size={13}
                    class="shrink-0 text-muted transition-transform duration-200 {open ? '' : '-rotate-90'}"
                  />
                </button>
                {#if open}
                  <div transition:slide={{ duration: 160 }}>
                    {#if m.hasAttachments}
                      <div class="pt-3">
                        <Attachments messageId={m.id} {ontoast} />
                      </div>
                    {/if}
                    {#if bodies[m.id]}
                      {#key bodies[m.id]}
                        <iframe
                          title="Treść wiadomości"
                          sandbox="allow-same-origin"
                          srcdoc={makeSrcdoc(bodies[m.id])}
                          class="block w-full border-0 bg-surface"
                          style="height:0"
                          use:autosize
                          use:externalLinks
                        ></iframe>
                      {/key}
                    {:else}
                      <p class="px-9.5 py-4 text-sm text-muted">Wczytuję treść…</p>
                    {/if}
                  </div>
                {/if}
              </div>
            {:else}
              <!-- Pojedyncza wiadomość: nagłówek nadawcy + treść na pełną wysokość -->
              <div class="selectable shrink-0 px-9.5">
                <div class="flex items-center gap-3">
                  <span
                    class="grid size-10 place-items-center rounded-[13px] bg-accent text-[13px]
                           font-bold text-on-accent"
                  >
                    {initials(m.fromName, m.fromAddr)}
                  </span>
                  <div class="min-w-0 flex-1">
                    <p class="truncate text-sm font-semibold">{m.fromName || m.fromAddr}</p>
                    <p class="truncate text-[12.5px] text-muted">
                      {m.fromAddr} · do: {bodies[m.id]?.toAddrs || "…"}
                    </p>
                  </div>
                  <span class="shrink-0 text-[12.5px] text-muted">{fmtDateFull(m.date)}</span>
                </div>
                <div class="mt-5 h-px bg-line"></div>
              </div>
              {#if m.hasAttachments}
                <div class="shrink-0 pt-3">
                  <Attachments messageId={m.id} {ontoast} />
                </div>
              {/if}
              {#if bodies[m.id]}
                {#key bodies[m.id]}
                  <iframe
                    title="Treść wiadomości"
                    sandbox="allow-same-origin"
                    srcdoc={makeSrcdoc(bodies[m.id])}
                    class="min-h-0 w-full flex-1 border-0 bg-transparent"
                    use:externalLinks
                  ></iframe>
                {/key}
              {:else}
                <div class="flex min-h-0 flex-1 items-start px-9.5 pt-4">
                  <span class="text-sm text-muted">Wczytuję treść…</span>
                </div>
              {/if}
            {/if}
          {/each}
        </div>

        <div class="flex h-16 shrink-0 items-center gap-2 border-t border-line px-4.5">
          <button
            class="flex h-9.5 items-center gap-2 rounded-full bg-accent px-4.5 text-[13.5px]
                   font-semibold text-on-accent hover:opacity-90"
            onclick={onreply}
          >
            <Icon name="reply" size={15} />
            Odpowiedz
          </button>
          <button
            class="flex h-9.5 items-center rounded-full bg-rail px-4.5 text-[13.5px] font-semibold
                   text-ink-soft hover:text-ink"
            onclick={onreplyall}
          >
            Odpowiedz wszystkim
          </button>
          <button
            class="flex h-9.5 items-center rounded-full bg-rail px-4.5 text-[13.5px] font-semibold
                   text-ink-soft hover:text-ink"
            onclick={onforward}
          >
            Prześlij dalej
          </button>
        </div>
      </div>
    {/key}
  {/if}
</section>
