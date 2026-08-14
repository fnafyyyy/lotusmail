<script module lang="ts">
  export type View =
    | { kind: "unified" }
    | { kind: "folder"; folderId: number }
    | { kind: "snoozed" };
</script>

<script lang="ts">
  import { scale, slide } from "svelte/transition";
  import type { Account, Folder } from "$lib/types";
  import Icon from "./Icon.svelte";

  let {
    accounts,
    folders,
    view,
    lastSyncLabel = "",
    syncStatus = "",
    onselect,
    onaddaccount,
    onnewfolder,
    ondeletefolder,
    onmarkread,
    oncleanup,
    onemptytrash,
    onreorder,
  }: {
    accounts: Account[];
    folders: Folder[];
    view: View;
    lastSyncLabel?: string;
    syncStatus?: string;
    onselect: (view: View) => void;
    onaddaccount: () => void;
    onnewfolder: (accountId: number) => void;
    ondeletefolder: (folder: Folder) => void;
    onmarkread: (folder: Folder) => void;
    /** Otwiera okno sprzątania skrzynki. */
    oncleanup: () => void;
    /** Kasuje bezpowrotnie całą zawartość Kosza. */
    onemptytrash: (folder: Folder) => void;
    /** Nowa kolejność folderów konta, po przeciągnięciu. */
    onreorder: (folderIds: number[]) => void;
  } = $props();

  // Przeciąganie folderów: id ciągniętego i id tego, nad którym wisi kursor.
  // Trzymamy jedno i drugie, żeby narysować kreskę w miejscu upuszczenia.
  let dragFolder = $state<number | null>(null);
  let dropFolder = $state<number | null>(null);

  /// Układa foldery konta w nowej kolejności i oddaje listę do zapisania.
  function dropOn(accountFolders: Folder[], targetId: number) {
    const from = accountFolders.findIndex((f) => f.id === dragFolder);
    const to = accountFolders.findIndex((f) => f.id === targetId);
    dragFolder = null;
    dropFolder = null;
    if (from < 0 || to < 0 || from === to) return;
    const next = [...accountFolders];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    onreorder(next.map((f) => f.id));
  }

  // Menu kontekstowe folderu (PPM w panelu bocznym).
  let ctx = $state<{ x: number; y: number; folder: Folder | null; accountId: number } | null>(null);

  function openContext(e: MouseEvent, accountId: number, folder: Folder | null) {
    e.preventDefault();
    e.stopPropagation();
    ctx = {
      x: Math.min(e.clientX, window.innerWidth - 230),
      y: Math.min(e.clientY, window.innerHeight - 140),
      folder,
      accountId,
    };
  }

  const folderIcon: Record<string, string> = {
    inbox: "inbox",
    sent: "send",
    drafts: "edit",
    archive: "archive",
    trash: "trash",
    spam: "flag",
  };

  // Kolor kropki konta: pierwsze konto akcentem, kolejne różem marki.
  const accountDot = ["bg-accent", "bg-flag"];

  let unifiedUnread = $derived(
    folders.filter((f) => f.kind === "inbox").reduce((s, f) => s + f.unreadCount, 0),
  );
  let byAccount = $derived(
    accounts.map((a) => ({
      account: a,
      folders: folders.filter((f) => f.accountId === a.id),
      unread: folders
        .filter((f) => f.accountId === a.id && f.kind === "inbox")
        .reduce((s, f) => s + f.unreadCount, 0),
    })),
  );
  // Domyślnie wszystkie konta rozwinięte; klik w nagłówek konta zwija.
  let collapsed = $state<Set<number>>(new Set());

  function toggleAccount(id: number) {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  }

  function isFolder(f: Folder): boolean {
    return view.kind === "folder" && view.folderId === f.id;
  }
</script>

{#snippet row(active: boolean, icon: string, label: string, badge: number, onclick: () => void)}
  <button
    class="flex h-9 w-full items-center gap-2.5 rounded-[10px] px-3.5 text-left text-[13.5px]
           {active
      ? 'bg-surface font-semibold text-accent shadow-[var(--chip-shadow)]'
      : 'text-ink-soft hover:bg-surface/60'}"
    {onclick}
  >
    <Icon name={icon} size={16} class={active ? "text-accent" : "text-muted"} />
    <span class="min-w-0 flex-1 truncate">{label}</span>
    {#if badge > 0}
      <span class="text-[11.5px] font-bold tabular-nums {active ? 'text-accent' : 'text-muted'}">
        {badge}
      </span>
    {/if}
  </button>
{/snippet}

<aside class="flex h-full w-full flex-col gap-0.5 py-1">
  {@render row(view.kind === "unified", "inbox", "Skrzynka", unifiedUnread, () =>
    onselect({ kind: "unified" }),
  )}
  {@render row(view.kind === "snoozed", "moon", "Drzemka", 0, () => onselect({ kind: "snoozed" }))}
  {@render row(false, "eraser", "Sprzątanie", 0, oncleanup)}

  <p class="px-3.5 pt-5 pb-1.5 text-[11px] font-bold tracking-[0.09em] text-muted uppercase">
    Konta
  </p>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#each byAccount as { account, folders: accountFolders, unread }, i (account.id)}
      <div
        class="group flex h-8.5 w-full items-center gap-2.5 px-3.5 text-[13.5px] text-ink-soft"
        role="presentation"
        oncontextmenu={(e) => openContext(e, account.id, null)}
      >
        <button
          class="flex min-w-0 flex-1 items-center gap-2.5 text-left"
          onclick={() => toggleAccount(account.id)}
        >
          <span class="size-1.75 shrink-0 rounded-full {accountDot[i % accountDot.length]}"></span>
          <span class="min-w-0 flex-1 truncate" title={account.email}>
            {account.displayName || account.email}
          </span>
        </button>
        <!-- Licznik konta tylko po zwinięciu - przy rozwiniętych folderach
             powtarzałby liczbę ze Skrzynki i zabierał miejsce nazwie. -->
        {#if unread > 0 && collapsed.has(account.id)}
          <span class="shrink-0 text-[11.5px] tabular-nums text-muted">{unread}</span>
        {/if}
        <button
          class="grid size-5 shrink-0 place-items-center rounded text-muted opacity-0
                 transition-opacity group-hover:opacity-100 hover:text-accent"
          onclick={() => onnewfolder(account.id)}
          title="Nowy folder"
          aria-label="Nowy folder"
        >
          <Icon name="plus" size={13} />
        </button>
        <button
          class="grid size-5 shrink-0 place-items-center text-muted"
          onclick={() => toggleAccount(account.id)}
          aria-label="Zwiń konto"
        >
          <Icon
            name="chevronDown"
            size={11}
            class="transition-transform duration-200 {collapsed.has(account.id) ? '-rotate-90' : ''}"
          />
        </button>
      </div>
      {#if !collapsed.has(account.id)}
        <div class="pb-1" transition:slide={{ duration: 160 }}>
          {#each accountFolders as f (f.id)}
            <button
              class="flex h-8 w-full items-center gap-2.5 rounded-[10px] pr-3.5 pl-8 text-left text-[13px]
                     {isFolder(f)
                ? 'bg-surface font-semibold text-accent shadow-[var(--chip-shadow)]'
                : 'text-muted hover:bg-surface/60'}
                     {dropFolder === f.id && dragFolder !== f.id
                ? 'ring-1 ring-accent'
                : ''}
                     {dragFolder === f.id ? 'opacity-40' : ''}"
              onclick={() => onselect({ kind: "folder", folderId: f.id })}
              oncontextmenu={(e) => openContext(e, account.id, f)}
              draggable="true"
              ondragstart={(e) => {
                dragFolder = f.id;
                e.dataTransfer?.setData("text/lotus-folder", String(f.id));
                if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
              }}
              ondragend={() => {
                dragFolder = null;
                dropFolder = null;
              }}
              ondragover={(e) => {
                // Kolejność zmieniamy tylko w obrębie jednego konta - foldery
                // należą do skrzynek, nie da się ich przenieść między nimi.
                if (dragFolder === null || !accountFolders.some((x) => x.id === dragFolder)) return;
                e.preventDefault();
                dropFolder = f.id;
              }}
              ondragleave={() => {
                if (dropFolder === f.id) dropFolder = null;
              }}
              ondrop={(e) => {
                e.preventDefault();
                dropOn(accountFolders, f.id);
              }}
            >
              <Icon name={folderIcon[f.kind] ?? "folder"} size={14} />
              <span class="min-w-0 flex-1 truncate">{f.displayName}</span>
              <!-- Nieprzeczytane wybijamy akcentem, obok blada liczba wszystkich
                   wiadomości - żeby jedno nie myliło się z drugim. -->
              {#if f.unreadCount > 0}
                <span class="shrink-0 text-[11.5px] font-bold tabular-nums text-accent">
                  {f.unreadCount}
                </span>
              {/if}
              {#if f.totalCount > 0}
                <span class="shrink-0 text-[10.5px] tabular-nums text-muted/70">
                  {f.totalCount}
                </span>
              {/if}
            </button>
          {/each}
        </div>
      {/if}
    {/each}

    <button
      class="flex h-8.5 w-full items-center gap-2.5 px-3.5 text-left text-[13.5px] text-muted
             hover:text-ink-soft"
      onclick={onaddaccount}
    >
      <Icon name="plus" size={14} />
      Dodaj konto
    </button>
  </div>

  {#if ctx}
    <div
      class="fixed z-40 w-56 rounded-xl bg-surface p-1.5 shadow-xl ring-1 ring-line"
      style="left:{ctx.x}px; top:{ctx.y}px; transform-origin: top left"
      transition:scale={{ start: 0.95, duration: 120 }}
    >
      <button
        class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]
               hover:bg-accent-soft"
        onclick={() => {
          onnewfolder(ctx!.accountId);
          ctx = null;
        }}
      >
        <Icon name="plus" size={14} class="text-muted" />
        Nowy folder
      </button>
      {#if ctx.folder && ctx.folder.unreadCount > 0}
        <button
          class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]
                 hover:bg-accent-soft"
          onclick={() => {
            onmarkread(ctx!.folder!);
            ctx = null;
          }}
        >
          <Icon name="check" size={14} class="text-muted" />
          Oznacz wszystkie jako przeczytane
        </button>
      {/if}
      {#if ctx.folder && ctx.folder.kind === "trash"}
        <div class="my-1 h-px bg-line"></div>
        <button
          class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]
                 text-danger hover:bg-danger/10"
          onclick={() => {
            onemptytrash(ctx!.folder!);
            ctx = null;
          }}
        >
          <Icon name="eraser" size={14} />
          Opróżnij Kosz
        </button>
      {/if}
      {#if ctx.folder && ctx.folder.kind !== "inbox" && ctx.folder.kind !== "trash"}
        <div class="my-1 h-px bg-line"></div>
        <button
          class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]
                 text-danger hover:bg-danger/10"
          onclick={() => {
            ondeletefolder(ctx!.folder!);
            ctx = null;
          }}
        >
          <Icon name="trash" size={14} />
          Usuń folder „{ctx.folder.displayName}"
        </button>
      {/if}
    </div>
  {/if}

  <div class="mt-2 rounded-xl bg-surface/70 px-3.5 py-3">
    <p class="flex items-center gap-1.5 text-xs font-semibold text-ink">
      {#if syncStatus}
        <Icon name="refresh" size={11} class="animate-spin text-accent" />
      {/if}
      Offline-first
    </p>
    <p class="mt-0.5 text-[11.5px] leading-snug text-muted">
      {#if syncStatus}
        {syncStatus}
      {:else}
        Wszystko czytane z lokalnej bazy.{lastSyncLabel ? ` ${lastSyncLabel}` : ""}
      {/if}
    </p>
  </div>
</aside>

<svelte:window onclick={() => (ctx = null)} onkeydown={(e) => e.key === "Escape" && (ctx = null)} />
