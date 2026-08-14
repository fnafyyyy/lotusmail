<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { api } from "$lib/api";
  import type { Folder, MessageSummary } from "$lib/types";
  import Icon from "./Icon.svelte";

  let {
    message,
    accountId,
    folders,
    onclose,
    onsaved,
  }: {
    message: MessageSummary;
    accountId: number;
    /** Foldery konta, do którego należy wiadomość. */
    folders: Folder[];
    onclose: () => void;
    onsaved: (folderName: string) => void;
  } = $props();

  let selectedFolder = $state<number | null>(null);
  let newFolderName = $state("");
  let creating = $state(false);
  let saving = $state(false);
  let error = $state("");

  // Foldery, do których przenoszenie ma sens (bez Kosza i Wysłanych).
  let targets = $derived(
    folders.filter((f) => !["trash", "sent", "drafts"].includes(f.kind)),
  );

  async function createAndSelect() {
    const name = newFolderName.trim();
    if (!name) return;
    creating = true;
    error = "";
    try {
      selectedFolder = await api.createFolder(accountId, name);
      newFolderName = "";
    } catch (e) {
      error = String(e);
    } finally {
      creating = false;
    }
  }

  async function save() {
    if (selectedFolder == null) return;
    saving = true;
    error = "";
    try {
      await api.addRule(accountId, message.fromAddr, selectedFolder);
      const name =
        targets.find((f) => f.id === selectedFolder)?.displayName ?? "wybranego folderu";
      onsaved(name);
    } catch (e) {
      error = String(e);
      saving = false;
    }
  }
</script>

<div
  class="fixed inset-0 z-30 grid place-items-center bg-ink/30 p-3"
  role="presentation"
  transition:fade={{ duration: 140 }}
  onclick={(e) => e.target === e.currentTarget && onclose()}
>
  <div
    class="panel flex max-h-[80vh] w-full max-w-[480px] flex-col"
    transition:scale={{ start: 0.96, duration: 170 }}
  >
    <header class="flex items-center justify-between border-b border-line px-5 py-3">
      <h2 class="flex items-center gap-2 font-display text-[15px] font-semibold">
        <Icon name="folder" size={16} class="text-accent" />
        Zawsze przenoś od tego nadawcy
      </h2>
      <button class="rounded-md p-1.5 text-muted hover:bg-rail" onclick={onclose} aria-label="Zamknij">
        <Icon name="x" size={16} />
      </button>
    </header>

    <div class="flex-1 space-y-3 overflow-y-auto px-5 py-4">
      <p class="text-[13px] leading-relaxed text-muted">
        Wiadomości od
        <span class="font-semibold text-ink">{message.fromName || message.fromAddr}</span>
        <span class="text-ink-soft">&lt;{message.fromAddr}&gt;</span>
        będą trafiać do wybranego folderu - również te, które już są w skrzynce.
      </p>

      <div>
        <p class="mb-1.5 text-[11px] font-bold tracking-[0.09em] text-muted uppercase">
          Folder docelowy
        </p>
        <div class="max-h-56 space-y-0.5 overflow-y-auto rounded-xl bg-rail/60 p-1.5">
          {#each targets as f (f.id)}
            <button
              class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]
                     {selectedFolder === f.id
                ? 'bg-accent text-on-accent font-semibold'
                : 'hover:bg-surface'}"
              onclick={() => (selectedFolder = f.id)}
            >
              <Icon name="folder" size={14} />
              {f.displayName}
            </button>
          {/each}
        </div>
      </div>

      <div>
        <p class="mb-1.5 text-[11px] font-bold tracking-[0.09em] text-muted uppercase">
          Albo utwórz nowy
        </p>
        <div class="flex gap-2">
          <input
            bind:value={newFolderName}
            placeholder="np. Kadry"
            class="min-w-0 flex-1 rounded-lg border border-line bg-paper px-3 py-1.5 text-sm
                   outline-none placeholder:text-muted focus:border-accent"
            onkeydown={(e) => e.key === "Enter" && createAndSelect()}
          />
          <button
            class="flex items-center gap-1.5 rounded-lg bg-rail px-3 py-1.5 text-[13px] font-semibold
                   text-ink-soft hover:text-ink disabled:opacity-40"
            disabled={creating || !newFolderName.trim()}
            onclick={createAndSelect}
          >
            <Icon name="plus" size={13} />
            {creating ? "Tworzę…" : "Utwórz"}
          </button>
        </div>
      </div>

      {#if error}
        <p class="rounded-lg bg-danger/10 px-3 py-2 text-xs leading-relaxed text-danger">{error}</p>
      {/if}
    </div>

    <footer class="flex items-center gap-2 border-t border-line px-5 py-3">
      <button
        class="rounded-full bg-accent px-4 py-2 text-sm font-semibold text-on-accent
               hover:opacity-90 disabled:opacity-40"
        disabled={selectedFolder == null || saving}
        onclick={save}
      >
        {saving ? "Zapisuję…" : "Zapisz regułę"}
      </button>
      <button
        class="rounded-full px-3 py-2 text-sm font-semibold text-ink-soft hover:bg-rail"
        onclick={onclose}
      >
        Anuluj
      </button>
    </footer>
  </div>
</div>
