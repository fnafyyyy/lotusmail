<script lang="ts">
  import { fade } from "svelte/transition";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
  import { save } from "@tauri-apps/plugin-dialog";
  import { api } from "$lib/api";
  import type { Attachment } from "$lib/types";
  import Icon from "./Icon.svelte";

  let {
    messageId,
    ontoast,
  }: {
    messageId: number;
    ontoast: (text: string) => void;
  } = $props();

  let items = $state<Attachment[]>([]);
  let loading = $state(true);
  let error = $state("");

  // Załączniki dociągają się z serwera przy pierwszym otwarciu wiadomości.
  $effect(() => {
    const id = messageId;
    items = [];
    error = "";
    loading = true;
    api
      .getAttachments(id)
      .then((list) => {
        if (messageId === id) items = list.filter((a) => !a.isInline);
      })
      .catch((e) => {
        if (messageId === id) error = String(e);
      })
      .finally(() => {
        if (messageId === id) loading = false;
      });
  });

  function fmtSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  /// Ikona dobrana po typie pliku - szybciej się rozpoznaje niż po nazwie.
  function iconFor(a: Attachment): string {
    const m = a.mime.toLowerCase();
    const n = a.name.toLowerCase();
    if (m.startsWith("image/")) return "image";
    if (m.includes("pdf")) return "filePdf";
    if (m.includes("sheet") || m.includes("excel") || /\.(xlsx?|csv)$/.test(n)) return "fileSheet";
    if (m.includes("zip") || /\.(zip|7z|rar|tar|gz)$/.test(n)) return "fileZip";
    return "file";
  }

  let images = $derived(items.filter((a) => a.mime.toLowerCase().startsWith("image/")));

  async function open(a: Attachment) {
    try {
      await openPath(a.path);
    } catch (e) {
      ontoast(`Nie udało się otworzyć: ${e}`);
    }
  }

  async function download(a: Attachment) {
    try {
      const target = await save({ defaultPath: a.name });
      if (!target) return;
      await api.saveAttachment(a.id, target);
      ontoast(`Zapisano ${a.name}`);
    } catch (e) {
      ontoast(`Nie udało się zapisać: ${e}`);
    }
  }

  async function reveal(a: Attachment) {
    try {
      await revealItemInDir(a.path);
    } catch (e) {
      ontoast(`Nie udało się pokazać pliku: ${e}`);
    }
  }
</script>

{#if loading}
  <div class="flex items-center gap-2 px-9.5 pb-3 text-[12px] text-muted">
    <Icon name="refresh" size={12} class="animate-spin" />
    Pobieram załączniki…
  </div>
{:else if error}
  <p class="px-9.5 pb-3 text-[12px] text-danger">Załączniki: {error}</p>
{:else if items.length > 0}
  <div class="px-9.5 pb-4" in:fade={{ duration: 150 }}>
    <p class="mb-2 flex items-center gap-1.5 text-[11px] font-bold tracking-[0.09em] text-muted uppercase">
      <Icon name="paperclip" size={11} />
      Załączniki ({items.length})
    </p>

    {#if images.length > 0}
      <!-- Podgląd obrazków: klik otwiera w domyślnej przeglądarce zdjęć -->
      <div class="mb-2 flex flex-wrap gap-2">
        {#each images as a (a.id)}
          <button
            class="group relative overflow-hidden rounded-lg ring-1 ring-line hover:ring-accent"
            onclick={() => open(a)}
            title="Otwórz {a.name}"
          >
            <img
              src={convertSrc(a.path)}
              alt={a.name}
              class="h-24 w-32 object-cover transition-transform duration-200 group-hover:scale-105"
            />
            <span
              class="absolute inset-x-0 bottom-0 truncate bg-ink/70 px-2 py-1 text-[10.5px] text-paper"
            >
              {a.name}
            </span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="flex flex-wrap gap-2">
      {#each items as a (a.id)}
        <div
          class="group flex items-center gap-2.5 rounded-xl bg-rail px-3 py-2 transition-colors
                 hover:bg-accent-soft"
        >
          <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-surface text-accent">
            <Icon name={iconFor(a)} size={15} />
          </span>
          <button class="min-w-0 text-left" onclick={() => open(a)} title="Otwórz">
            <span class="block max-w-52 truncate text-[13px] font-semibold">{a.name}</span>
            <span class="block text-[11px] text-muted">{fmtSize(a.size)}</span>
          </button>
          <span class="flex shrink-0 items-center gap-0.5 opacity-0 group-hover:opacity-100">
            <button
              class="grid size-7 place-items-center rounded-md text-muted hover:bg-surface hover:text-ink"
              onclick={() => download(a)}
              title="Zapisz jako…"
              aria-label="Zapisz jako"
            >
              <Icon name="download" size={14} />
            </button>
            <button
              class="grid size-7 place-items-center rounded-md text-muted hover:bg-surface hover:text-ink"
              onclick={() => reveal(a)}
              title="Pokaż w folderze"
              aria-label="Pokaż w folderze"
            >
              <Icon name="folder" size={14} />
            </button>
          </span>
        </div>
      {/each}
    </div>
  </div>
{/if}

<script module lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  // Ścieżka z dysku zamieniona na adres, który webview może wyświetlić.
  function convertSrc(path: string): string {
    return convertFileSrc(path);
  }
</script>
