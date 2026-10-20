<script lang="ts">
  import { untrack } from "svelte";
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import { loginTemplate, newTemplateId, type MailTemplate } from "$lib/templates";

  let {
    templates,
    startNew = false,
    onclose,
    onsaved,
  }: {
    templates: MailTemplate[];
    /** Otwórz od razu z nowym, pustym wzorem („Nowy wzór…" z menu). */
    startNew?: boolean;
    onclose: () => void;
    onsaved: (list: MailTemplate[]) => void;
  } = $props();

  // Edytujemy kopię - „Anuluj" i X nie mogą zostawić połowicznych zmian.
  let list = $state<MailTemplate[]>(untrack(() => structuredClone($state.snapshot(templates))));
  let selectedId = $state<string | null>(list[0]?.id ?? null);
  let current = $derived(list.find((t) => t.id === selectedId) ?? null);

  function add(t: MailTemplate) {
    list.push(t);
    selectedId = t.id;
  }

  function addEmpty() {
    add({ id: newTemplateId(), name: "Nowy wzór", toAddrs: "", subject: "", bodyHtml: "" });
  }

  if (untrack(() => startNew)) addEmpty();

  function remove(id: string) {
    const i = list.findIndex((t) => t.id === id);
    if (i < 0) return;
    list.splice(i, 1);
    selectedId = list[Math.min(i, list.length - 1)]?.id ?? null;
  }

  function move(id: string, by: -1 | 1) {
    const i = list.findIndex((t) => t.id === id);
    const j = i + by;
    if (i < 0 || j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
  }

  function save() {
    onsaved(
      $state.snapshot(list).map((t) => ({ ...t, name: t.name.trim() || t.subject.trim() || "Bez nazwy" })),
    );
  }

  const field =
    "w-full min-w-0 rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none " +
    "placeholder:text-muted focus:border-accent";
</script>

<div
  class="fixed inset-0 z-20 grid place-items-center bg-ink/30 p-3"
  role="presentation"
  transition:fade={{ duration: 140 }}
  onclick={(e) => e.target === e.currentTarget && onclose()}
>
  <div
    class="flex h-[min(560px,85vh)] w-full max-w-[760px] flex-col rounded-2xl bg-surface shadow-2xl"
    transition:scale={{ start: 0.96, duration: 170 }}
  >
    <header class="flex items-center justify-between border-b border-line px-5 py-3">
      <h2 class="flex items-center gap-2 font-display text-[15px] font-semibold">
        <Icon name="file" size={16} class="text-muted" />
        Wzory wiadomości
      </h2>
      <button class="rounded-md p-1.5 text-muted hover:bg-line/60" onclick={onclose} aria-label="Zamknij">
        <Icon name="x" size={16} />
      </button>
    </header>

    <div class="flex min-h-0 flex-1">
      <aside class="flex w-52 shrink-0 flex-col border-r border-line p-2">
        <div class="min-h-0 flex-1 space-y-0.5 overflow-y-auto">
          {#each list as t (t.id)}
            <button
              class="flex w-full flex-col rounded-lg px-2.5 py-1.5 text-left transition-colors
                     {t.id === selectedId ? 'bg-accent-soft text-ink' : 'text-ink-soft hover:bg-rail'}"
              onclick={() => (selectedId = t.id)}
            >
              <span class="truncate text-[13px] font-semibold">{t.name || "Bez nazwy"}</span>
              <span class="truncate text-[11px] text-muted">{t.subject || "(bez tematu)"}</span>
            </button>
          {:else}
            <p class="px-2.5 py-2 text-[12px] leading-relaxed text-muted">
              Nie masz jeszcze wzorów. Wzór to gotowy temat i treść, które wstawiasz strzałką
              obok „Napisz".
            </p>
            <button
              class="mx-1 mt-1 rounded-lg bg-rail px-2.5 py-1.5 text-left text-[12px] font-semibold
                     text-ink-soft hover:text-ink"
              onclick={() => add(loginTemplate())}
            >
              + Przykład: dane do logowania
            </button>
          {/each}
        </div>
        <button
          class="mt-2 flex items-center justify-center gap-1.5 rounded-lg bg-rail px-3 py-1.5
                 text-[12.5px] font-semibold text-ink-soft hover:text-ink"
          onclick={addEmpty}
        >
          <Icon name="plus" size={12} />
          Nowy wzór
        </button>
      </aside>

      {#if current}
        <section class="flex min-w-0 flex-1 flex-col gap-2.5 overflow-y-auto px-5 py-4">
          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold text-muted">Nazwa w menu</span>
            <input bind:value={current.name} placeholder="np. Dane do logowania" class={field} />
          </label>
          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold text-muted">Do (opcjonalnie)</span>
            <input bind:value={current.toAddrs} placeholder="adresat@firma.pl" class={field} />
          </label>
          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold text-muted">Temat</span>
            <input bind:value={current.subject} placeholder="Temat wiadomości" class={field} />
          </label>
          <div class="flex min-h-0 flex-1 flex-col">
            <span class="mb-1 block text-[11px] font-semibold text-muted">
              Treść - stopka dokleja się sama pod spodem
            </span>
            {#key current.id}
              <div
                contenteditable="true"
                bind:innerHTML={current.bodyHtml}
                role="textbox"
                aria-multiline="true"
                aria-label="Treść wzoru"
                tabindex="0"
                class="min-h-40 w-full flex-1 overflow-y-auto rounded-lg border border-line bg-paper
                       px-3 py-2 text-sm leading-relaxed outline-none focus:border-accent"
              ></div>
            {/key}
          </div>
          <div class="flex items-center gap-1.5">
            <button
              class="rounded-lg px-2.5 py-1.5 text-[12px] font-semibold text-ink-soft hover:bg-rail"
              onclick={() => move(current!.id, -1)}
              title="Wyżej w menu"
            >
              ↑
            </button>
            <button
              class="rounded-lg px-2.5 py-1.5 text-[12px] font-semibold text-ink-soft hover:bg-rail"
              onclick={() => move(current!.id, 1)}
              title="Niżej w menu"
            >
              ↓
            </button>
            <div class="flex-1"></div>
            <button
              class="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-semibold
                     text-danger hover:bg-danger/10"
              onclick={() => remove(current!.id)}
            >
              <Icon name="trash" size={12} />
              Usuń wzór
            </button>
          </div>
        </section>
      {:else}
        <div class="grid flex-1 place-items-center px-6 text-center text-[13px] text-muted">
          Wybierz wzór z listy albo dodaj nowy.
        </div>
      {/if}
    </div>

    <footer class="flex justify-end gap-2 border-t border-line px-5 py-3">
      <button class="rounded-lg px-3 py-2 text-xs font-semibold text-ink-soft hover:bg-rail" onclick={onclose}>
        Anuluj
      </button>
      <button
        class="rounded-lg bg-accent px-4 py-2 text-xs font-semibold text-on-accent hover:opacity-90"
        onclick={save}
      >
        Zapisz wzory
      </button>
    </footer>
  </div>
</div>
