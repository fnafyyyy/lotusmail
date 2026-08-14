<script lang="ts">
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getVersion } from "@tauri-apps/api/app";
  import { api, isUnread } from "$lib/api";
  import type {
    Account,
    Category,
    CategoryCounts,
    Folder,
    SortKey,
    LocalDraft,
    MessageBody,
    MessageSummary,
  } from "$lib/types";
  import { theme } from "$lib/theme.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Logo from "$lib/components/Logo.svelte";
  import WindowControls from "$lib/components/WindowControls.svelte";
  import Sidebar, { type View } from "$lib/components/Sidebar.svelte";
  import MessageList from "$lib/components/MessageList.svelte";
  import ReadingPane from "$lib/components/ReadingPane.svelte";
  import ComposeView from "$lib/components/ComposeView.svelte";
  import AddAccountModal from "$lib/components/AddAccountModal.svelte";
  import SettingsModal from "$lib/components/SettingsModal.svelte";
  import RuleModal from "$lib/components/RuleModal.svelte";
  import CleanupModal from "$lib/components/CleanupModal.svelte";
  import Dialog from "$lib/components/Dialog.svelte";
  import { checkForUpdate } from "$lib/update";
  import Splash from "$lib/components/Splash.svelte";

  let accounts = $state<Account[]>([]);
  let folders = $state<Folder[]>([]);
  let messages = $state<MessageSummary[]>([]);
  // Lista dociąga się przy przewijaniu. Wyszukiwanie i drzemki mają własne
  // limity po stronie rdzenia i nie stronicują, stąd `hasMore` gaśnie tam
  // od razu.
  const PAGE = 100;
  let hasMore = $state(false);
  let loadingMore = $state(false);
  let view = $state<View>({ kind: "unified" });
  let category = $state<Category>("primary");
  // Nieprzeczytane w zakładkach Smart Inbox - liczniki na przyciskach listy.
  let counts = $state<CategoryCounts>({ primary: 0, newsletters: 0, notifications: 0 });
  // Kolejność listy zapamiętana między uruchomieniami.
  let sort = $state<SortKey>((localStorage.getItem("sort") as SortKey) || "date_desc");
  let searchQuery = $state("");
  let selected = $state<MessageSummary | null>(null);
  let body = $state<MessageBody | null>(null);
  // Drugi panel czytania (split view) - otwierany przeciągnięciem maila
  // z listy na obszar czytania.
  let selected2 = $state<MessageSummary | null>(null);
  let body2 = $state<MessageBody | null>(null);
  // Konwersacje otwartych wiadomości (wszystkie maile z wątku).
  let thread1 = $state<MessageSummary[]>([]);
  let thread2 = $state<MessageSummary[]>([]);
  // Odpowiadanie obok wiadomości: szkic zajmuje prawą połowę widoku.
  let sideDraft = $state<LocalDraft | null>(null);
  let dragOverSplit = $state(false);
  let dragDepth = 0;
  // Iframe z treścią maila połyka zdarzenia drag - na czas przeciągania
  // wyłączamy mu pointer-events (klasa .drag-active w app.css).
  let mailDragging = $state(false);
  // Strona, na którą wyląduje przeciągany mail - podąża za kursorem.
  let dropSide = $state<"left" | "right">("right");
  // Wiadomość otwarta z sekcji „Nowe" zostaje w niej, dopóki nie klikniesz
  // następnej - dopiero wtedy „wpada" do Przejrzanych (jak w Sparku).
  let holdInNewId = $state<number | null>(null);
  let addAccountOpen = $state(false);
  let cleanupOpen = $state(false);
  let addAccountStep = $state<"provider" | "transfer">("provider");
  // Na wąskim ekranie trójpanelowy układ się nie mieści: pokazujemy jeden panel
  // naraz i przechodzimy folder → lista → wiadomość, jak w klientach mobilnych.
  // Wersja czytana z rdzenia, nie wpisana na sztywno - inaczej rozjechałaby
  // się przy pierwszym wydaniu, o którym ktoś zapomni.
  let appVersion = $state("");
  // macOS rysuje własne rogi okna i światła drogowe, więc chowamy tam nasze
  // przyciski. Nagłówek jest wtedy odbity lustrzanie: światła zajmują lewy
  // róg, więc marka idzie w prawo, a przyciski akcji na lewo - inaczej logo
  // wchodziłoby pod systemowe kropki. Systemowe rozmycie spod okna widać
  // dopiero przez półprzezroczyste tło, stąd klasa na dokumencie.
  let isMac = $state(false);
  let narrow = $state(false);
  let mobilePane = $state<"folders" | "list" | "message">("list");
  let settingsOpen = $state(false);
  let signature = $state("");
  // Stopka per konto; klucz ustawienia to `signature:<id>`. Konto bez własnej
  // dostaje domyślną - dzięki temu nic nie znika użytkownikom, którzy mieli
  // jedną stopkę na wszystko.
  let signatures = $state<Record<number, string>>({});
  // Karty kompozycji: szkice żyją obok widoku poczty, przełączane paskiem u góry.
  let drafts = $state<LocalDraft[]>([]);
  let activeTab = $state<number | "mail">("mail");
  let draftSeq = 1;
  let toast = $state("");
  let loaded = $state(false);
  let toastTimer: ReturnType<typeof setTimeout>;
  let syncStatus = $state("");
  let lastSync = $state<Date | null>(null);
  let searchHelp = $state(false);
  // Ekran powitalny: znika po wczytaniu skrzynki, ale nie wcześniej niż
  // animacja zdąży się rozwinąć - inaczej mrugałby przy szybkim starcie.
  let splash = $state(true);
  let minSplashDone = $state(false);
  let splashText = $derived(
    syncStatus || (loaded ? "Prawie gotowe…" : "Wczytuję skrzynkę…"),
  );
  $effect(() => {
    if (loaded && minSplashDone) splash = false;
  });

  /// Ekran powitalny odpływa: rozjaśnia się i lekko powiększa.
  function splashOut(_node: Element) {
    return {
      duration: 380,
      easing: cubicOut,
      css: (t: number, u: number) => `opacity:${t};transform:scale(${1 + 0.06 * u})`,
    };
  }
  // Wiadomość, dla której tworzymy regułę „od nadawcy do folderu".
  let ruleFor = $state<MessageSummary | null>(null);
  // Własne okno potwierdzenia / pytania zamiast systemowego alertu przeglądarki.
  let dialog = $state<{
    title: string;
    message?: string;
    placeholder?: string;
    initialValue?: string | null;
    confirmLabel?: string;
    danger?: boolean;
    onconfirm: (value: string) => void;
  } | null>(null);

  let ruleAccountId = $derived.by(() => {
    if (!ruleFor) return 0;
    return folders.find((f) => f.id === ruleFor!.folderId)?.accountId ?? 0;
  });
  let ruleFolders = $derived(folders.filter((f) => f.accountId === ruleAccountId));

  const searchHints = [
    { op: "od:", desc: "nadawca" },
    { op: "do:", desc: "odbiorca" },
    { op: "temat:", desc: "słowo w temacie" },
    { op: "folder:", desc: "nazwa folderu" },
    { op: "jest:nieprzeczytane", desc: "tylko nowe" },
    { op: "jest:oflagowane", desc: "z flagą" },
    { op: "ma:zalacznik", desc: "z załącznikiem" },
    { op: "po:2026-01-01", desc: "od daty" },
    { op: "przed:2026-08-01", desc: "do daty" },
  ];

  // Motyw stosowany globalnie (przełącznik w pasku górnym).
  $effect(() => {
    document.documentElement.classList.toggle("dark", theme.dark);
    localStorage.setItem("motyw", theme.dark ? "ciemny" : "jasny");
  });

  // Regulowane szerokości paneli - zapamiętywane lokalnie.
  let sidebarW = $state(Number(localStorage.getItem("w-sidebar")) || 206);
  let listW = $state(Number(localStorage.getItem("w-list")) || 364);
  let dragging = $state<"sidebar" | "list" | null>(null);

  function startDrag(e: PointerEvent, which: "sidebar" | "list") {
    e.preventDefault();
    dragging = which;
    const startX = e.clientX;
    const startW = which === "sidebar" ? sidebarW : listW;
    const move = (ev: PointerEvent) => {
      const d = ev.clientX - startX;
      if (which === "sidebar") sidebarW = Math.min(340, Math.max(180, startW + d));
      else listW = Math.min(620, Math.max(300, startW + d));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      dragging = null;
      localStorage.setItem("w-sidebar", String(sidebarW));
      localStorage.setItem("w-list", String(listW));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  let currentFolder = $derived.by(() => {
    const v = view;
    return v.kind === "folder" ? (folders.find((f) => f.id === v.folderId) ?? null) : null;
  });
  let showCategories = $derived(
    !searchQuery.trim() && (view.kind === "unified" || currentFolder?.kind === "inbox"),
  );
  let listTitle = $derived(
    view.kind === "snoozed" ? "Drzemka" : (currentFolder?.displayName ?? "Wszystkie skrzynki"),
  );
  let lastSyncLabel = $derived(
    lastSync
      ? `Ostatnia synchronizacja ${lastSync.toLocaleTimeString("pl-PL", { hour: "2-digit", minute: "2-digit" })}.`
      : "",
  );

  // Licznik nieprzeczytanych w tytule okna - widoczny na pasku zadań.
  let totalUnread = $derived(
    folders.filter((f) => f.kind === "inbox").reduce((s, f) => s + f.unreadCount, 0),
  );
  $effect(() => {
    getCurrentWindow().setTitle(totalUnread > 0 ? `lotusMail (${totalUnread})` : "lotusMail");
  });

  function accountLabelOf(m: MessageSummary | null): string {
    if (!m) return "";
    const f = folders.find((x) => x.id === m.folderId);
    const a = accounts.find((x) => x.id === f?.accountId);
    return a?.displayName || a?.email || "";
  }

  function showToast(text: string) {
    toast = text;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ""), 3500);
  }

  async function loadMessages() {
    hasMore = false;
    if (searchQuery.trim()) {
      messages = await api.searchMessages(searchQuery.trim());
    } else if (view.kind === "snoozed") {
      messages = await api.listSnoozed();
    } else {
      const inbox = view.kind === "unified" || currentFolder?.kind === "inbox";
      // Przeładowanie nie może skracać listy. Synchronizacja woła je przy
      // każdej zmianie, a doładowane strony znikałyby razem z pozycją
      // przewijania - wyglądało to jak wyrzucenie na górę w środku czytania.
      const want = Math.max(PAGE, messages.length);
      messages = await api.listMessages({
        folderId: view.kind === "folder" ? view.folderId : undefined,
        category: inbox ? category : undefined,
        sort,
        limit: want,
      });
      hasMore = messages.length >= want;
      if (inbox) await refreshCounts();
    }
  }

  /// Kolejna strona listy - wołane, gdy przewijanie dobija do końca.
  /// Świadomie bez znacznika czasu: `offset` liczony długością listy wystarcza,
  /// bo nowe wiadomości dochodzą na górze, a te doczytujemy od dołu.
  async function loadMore() {
    if (!hasMore || loadingMore) return;
    loadingMore = true;
    try {
      const inbox = view.kind === "unified" || currentFolder?.kind === "inbox";
      const next = await api.listMessages({
        folderId: view.kind === "folder" ? view.folderId : undefined,
        category: inbox ? category : undefined,
        sort,
        offset: messages.length,
        limit: PAGE,
      });
      // Zabezpieczenie przed duplikatami, gdyby w międzyczasie coś doszło.
      const known = new Set(messages.map((m) => m.id));
      messages = [...messages, ...next.filter((m) => !known.has(m.id))];
      hasMore = next.length >= PAGE;
    } finally {
      loadingMore = false;
    }
  }

  /// Same liczniki - po oznaczeniu wiadomości nie ma po co przeładowywać listy.
  async function refreshCounts() {
    if (!showCategories) return;
    counts = await api.categoryCounts(view.kind === "folder" ? view.folderId : undefined);
  }

  async function refresh() {
    [accounts, folders] = await Promise.all([api.listAccounts(), api.listFolders()]);
    await loadMessages();
    // Stopka mogła zostać zmieniona (ustawienia, import) - tania aktualizacja.
    signature = (await api.getSetting("signature")) ?? "";
    const perAccount = await Promise.all(
      accounts.map(async (a) => [a.id, (await api.getSetting(`signature:${a.id}`)) ?? ""] as const),
    );
    signatures = Object.fromEntries(perAccount.filter(([, v]) => v.trim().length > 0));
  }

  // Synchronizacja zgłasza zmiany po każdym folderze - bez tego przy dużej
  // skrzynce lista przeliczałaby się kilkanaście razy pod rząd.
  let refreshTimer: ReturnType<typeof setTimeout>;
  function refreshSoon() {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, 600);
  }

  onMount(() => {
    isMac = navigator.userAgent.includes("Macintosh");
    if (isMac) document.documentElement.classList.add("mac");
    getVersion().then((v) => (appVersion = v));
    const mq = window.matchMedia("(max-width: 760px)");
    const applyWidth = () => (narrow = mq.matches);
    applyWidth();
    mq.addEventListener("change", applyWidth);
    return () => mq.removeEventListener("change", applyWidth);
  });

  onMount(() => {
    // Ostatni płatek kończy się ok. 1,2 s, status wchodzi w 1,42 s - dajemy
    // animacji dojść do końca i chwilę wybrzmieć, zanim ekran odpłynie.
    const splashTimer = setTimeout(() => (minSplashDone = true), 2600);
    refresh().then(() => (loaded = true));
    api.getSetting("signature").then((s) => (signature = s ?? ""));
    const unlisten = listen("messages-updated", () => refreshSoon());
    const unlistenErr = listen<string>("sync-error", (e) =>
      showToast(`Błąd synchronizacji: ${e.payload}`),
    );
    // Kliknięcie w powiadomienie systemowe otwiera wskazaną wiadomość.
    const unlistenOpen = listen<number>("open-message", async (e) => {
      try {
        const m = await api.getMessage(e.payload);
        // Wiadomość może być w innym folderze niż aktualny widok.
        if (!messages.some((x) => x.id === m.id)) {
          await selectView({ kind: "folder", folderId: m.folderId });
        }
        await openInto(1, m);
      } catch (err) {
        showToast(`Nie udało się otworzyć wiadomości: ${err}`);
      }
    });
    const unlistenSent = listen<string>("outbox-sent", (e) =>
      showToast(`Wysłano: ${e.payload || "(bez tematu)"}`),
    );
    // Aktualizacje sprawdzamy z opóźnieniem: start programu i pierwsza
    // synchronizacja są ważniejsze niż pytanie o nową wersję.
    const updateTimer = setTimeout(async () => {
      const update = await checkForUpdate();
      if (!update) return;
      dialog = {
        title: `Dostępna wersja ${update.version}`,
        message: update.notes || "Zainstalować teraz? Program uruchomi się ponownie.",
        confirmLabel: "Zainstaluj",
        onconfirm: async () => {
          dialog = null;
          try {
            await update.install((percent) => (syncStatus = `Pobieram aktualizację: ${percent}%`));
          } catch (e) {
            syncStatus = "";
            showToast(`Aktualizacja się nie powiodła: ${e}`);
          }
        },
      };
    }, 8000);
    const unlistenStatus = listen<string>("sync-status", (e) => {
      if (e.payload === "") {
        if (syncStatus) lastSync = new Date();
        syncStatus = "";
      } else {
        syncStatus = e.payload;
      }
    });
    return () => {
      clearTimeout(splashTimer);
      clearTimeout(updateTimer);
      unlisten.then((fn) => fn());
      unlistenErr.then((fn) => fn());
      unlistenSent.then((fn) => fn());
      unlistenOpen.then((fn) => fn());
      unlistenStatus.then((fn) => fn());
    };
  });

  async function accountAdded(accountId: number) {
    addAccountOpen = false;
    await refresh();
    showToast("Konto dodane - pobieram wiadomości…");
    api
      .syncNow(accountId)
      .then(() => showToast("Synchronizacja zakończona"))
      .catch((e) => showToast(`Błąd synchronizacji: ${e}`));
  }

  async function selectView(v: View) {
    if (narrow) mobilePane = "list";
    // Nowy folder to nowa lista - dociągnięte strony poprzedniego nie mają
    // tu czego szukać.
    messages = [];
    view = v;
    searchQuery = "";
    selected = null;
    body = null;
    selected2 = null;
    body2 = null;
    holdInNewId = null;
    await loadMessages();
  }

  async function selectCategory(c: Category) {
    messages = [];
    category = c;
    holdInNewId = null;
    await loadMessages();
  }

  async function selectSort(s: SortKey) {
    sort = s;
    localStorage.setItem("sort", s);
    holdInNewId = null;
    await loadMessages();
  }

  /// Tworzenie i usuwanie folderów z panelu bocznego. Operacje idą na serwer
  /// IMAP, więc widać je też w Outlooku.
  function newFolder(accountId: number) {
    dialog = {
      title: "Nowy folder",
      message: "Folder powstanie na serwerze, więc pojawi się też w innych programach pocztowych.",
      placeholder: "np. Kadry",
      initialValue: "",
      confirmLabel: "Utwórz",
      onconfirm: async (name) => {
        dialog = null;
        try {
          await api.createFolder(accountId, name);
          await refresh();
          showToast(`Utworzono folder ${name}`);
        } catch (e) {
          showToast(`Nie udało się utworzyć folderu: ${e}`);
        }
      },
    };
  }

  function deleteFolder(folder: Folder) {
    const count = messages.filter((m) => m.folderId === folder.id).length;
    dialog = {
      title: `Usunąć folder „${folder.displayName}"?`,
      message:
        count > 0
          ? `Folder zniknie z serwera razem z wiadomościami (${count}). Tej operacji nie da się cofnąć.`
          : "Folder zostanie usunięty z serwera. Tej operacji nie da się cofnąć.",
      confirmLabel: "Usuń folder",
      danger: true,
      onconfirm: async () => {
        dialog = null;
        try {
          await api.deleteFolder(folder.id);
          if (view.kind === "folder" && view.folderId === folder.id) {
            await selectView({ kind: "unified" });
          }
          await refresh();
          showToast(`Usunięto folder ${folder.displayName}`);
        } catch (e) {
          showToast(`Nie udało się usunąć folderu: ${e}`);
        }
      },
    };
  }

  function markFolderRead(folder: Folder) {
    dialog = {
      title: `Oznaczyć wszystko w „${folder.displayName}" jako przeczytane?`,
      message: `Dotyczy ${folder.unreadCount} nieprzeczytanych wiadomości. Zmiana trafi też na serwer.`,
      confirmLabel: "Oznacz jako przeczytane",
      danger: false,
      onconfirm: async () => {
        dialog = null;
        const n = await api.markFolderRead(folder.id);
        await refresh();
        showToast(`Oznaczono ${n} wiadomości jako przeczytane`);
      },
    };
  }

  /// Nawigacja po liście strzałkami: kolejność jak na ekranie, czyli
  /// najpierw sekcja „Nowe" (z przytrzymaną wiadomością), potem „Przejrzane".
  let listOrder = $derived.by(() => {
    if (!showCategories) return messages;
    const unseen = messages.filter((m) => isUnread(m) || m.id === holdInNewId);
    const seen = messages.filter((m) => !isUnread(m) && m.id !== holdInNewId);
    return [...unseen, ...seen];
  });

  function moveSelection(step: 1 | -1) {
    const list = listOrder;
    if (list.length === 0) return;
    const at = selected ? list.findIndex((m) => m.id === selected!.id) : -1;
    const next = at < 0 ? (step === 1 ? 0 : list.length - 1) : at + step;
    if (next < 0 || next >= list.length) return;
    openInto(1, list[next]);
  }

  /// Skróty klawiszowe działają, gdy nie piszemy w polu tekstowym.
  function onWindowKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const typing =
      !!target &&
      (target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.isContentEditable);
    if (typing || e.ctrlKey || e.altKey || e.metaKey) return;
    if (activeTab !== "mail") return;

    if (e.key === "ArrowDown" || e.key === "j") {
      e.preventDefault();
      moveSelection(1);
    } else if (e.key === "ArrowUp" || e.key === "k") {
      e.preventDefault();
      moveSelection(-1);
    } else if (e.key === "Escape") {
      if (sideDraft) sideDraft = null;
      else if (selected2) closePane(2);
      else clearSlot(1);
    }
  }

  /// Ręczne sprawdzenie poczty - zagląda tylko do skrzynek odbiorczych,
  /// więc trwa chwilę, a nie minuty jak pełny przebieg.
  let checking = $state(false);
  async function checkMail() {
    if (checking) return;
    checking = true;
    try {
      await api.checkMail();
      await refresh();
    } catch (e) {
      showToast(`Nie udało się sprawdzić poczty: ${e}`);
    } finally {
      checking = false;
    }
  }

  /// Wyszukiwanie sięgające całej skrzynki na serwerze - dla maili, których
  /// nie ma jeszcze w lokalnej bazie.
  let serverSearching = $state(false);
  async function searchOnServer() {
    const q = searchQuery.trim();
    if (!q || serverSearching) return;
    serverSearching = true;
    try {
      const found = await api.searchServer(q);
      await loadMessages();
      showToast(
        found > 0
          ? `Dociągnięto z serwera: ${found}`
          : "Na serwerze nie ma nic ponad to, co już masz",
      );
    } catch (e) {
      showToast(`Wyszukiwanie na serwerze nie powiodło się: ${e}`);
    } finally {
      serverSearching = false;
    }
  }

  /// Szybkie filtry z listy dopisują (lub usuwają) operator w zapytaniu.
  function appendSearchOperator(operator: string) {
    const parts = searchQuery.split(/\s+/).filter(Boolean);
    const at = parts.indexOf(operator);
    if (at >= 0) parts.splice(at, 1);
    else parts.push(operator);
    searchQuery = parts.join(" ");
    loadMessages();
  }

  let searchTimer: ReturnType<typeof setTimeout>;
  function onSearch(q: string) {
    searchQuery = q;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(loadMessages, 250);
  }

  function paneMsg(slot: 1 | 2): MessageSummary | null {
    return slot === 1 ? selected : selected2;
  }

  function clearSlot(slot: 1 | 2) {
    if (slot === 1) {
      selected = null;
      body = null;
      thread1 = [];
      sideDraft = null;
    } else {
      selected2 = null;
      body2 = null;
      thread2 = [];
    }
  }

  async function openInto(slot: 1 | 2, m: MessageSummary) {
    // Poprzednio przytrzymana wiadomość spada teraz do Przejrzanych.
    if (holdInNewId !== m.id) holdInNewId = null;
    if (isUnread(m)) holdInNewId = m.id;
    if (slot === 1) {
      selected = m;
      body = null;
    } else {
      selected2 = m;
      body2 = null;
    }
    // Treść (często ciężki HTML z obrazkami) montujemy po pierwszej klatce,
    // żeby animacja przenoszenia wiadomości na liście zdążyła ruszyć płynnie.
    await new Promise((r) => requestAnimationFrame(() => r(null)));
    // Wątek pobieramy zawsze, gdy wiadomość go ma - licznik z listy bywa
    // nieaktualny (np. przy otwieraniu z powiadomienia), a rozmowa powinna
    // pokazać się wszędzie tam, gdzie istnieje.
    const [b, t] = await Promise.all([
      api.getMessageBody(m.id),
      m.threadId ? api.listThread(m.threadId) : Promise.resolve([]),
    ]);
    if (paneMsg(slot)?.id === m.id) {
      if (slot === 1) {
        body = b;
        thread1 = t;
      } else {
        body2 = b;
        thread2 = t;
      }
    }
    // Panel czytania pokazuje całą konwersację, więc oznaczamy ją w całości.
    // Bez tego starsze wiadomości wątku zostają nieprzeczytane, choć wiersz
    // listy wygląda na przeczytany - i licznik przy zakładce się nie zgadza.
    const threadChanged = m.threadId ? await api.setThreadRead(m.threadId, true) : 0;
    if (!m.isRead || threadChanged > 0) {
      if (!m.isRead) await api.setRead(m.id, true);
      m.isRead = true;
      m.threadUnread = 0;
      folders = await api.listFolders();
      await refreshCounts();
    }
  }

  async function openMessage(m: MessageSummary) {
    if (narrow) mobilePane = "message";
    await openInto(1, m);
  }

  /// Zamknięcie panelu w widoku podzielonym; zamknięcie lewego awansuje prawy.
  function closePane(slot: 1 | 2) {
    if (slot === 2) {
      clearSlot(2);
    } else {
      selected = selected2;
      body = body2;
      clearSlot(2);
    }
  }

  function onSplitDragEnter() {
    dragDepth++;
    dragOverSplit = true;
  }

  function onSplitDragLeave() {
    if (--dragDepth <= 0) {
      dragDepth = 0;
      dragOverSplit = false;
    }
  }

  function onSplitDragOver(e: DragEvent) {
    e.preventDefault();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dropSide = e.clientX < rect.left + rect.width / 2 ? "left" : "right";
  }

  function onSplitDrop(e: DragEvent) {
    e.preventDefault();
    dragDepth = 0;
    dragOverSplit = false;
    mailDragging = false;
    const raw = e.dataTransfer?.getData("text/lotus-id");
    if (!raw) return;
    const m = messages.find((x) => x.id === Number(raw));
    if (!m) return;
    if (!selected && !selected2) {
      openInto(1, m);
    } else if (dropSide === "left") {
      if (selected?.id === m.id) return;
      // Dotychczasowy lewy mail przesuwa się do prawego panelu.
      if (selected) {
        selected2 = selected;
        body2 = body;
      }
      openInto(1, m);
    } else {
      if (!selected) openInto(1, m);
      else if (selected.id !== m.id && selected2?.id !== m.id) openInto(2, m);
    }
  }

  async function flagFor(m: MessageSummary | null) {
    if (!m) return;
    const next = !m.isFlagged;
    await api.setFlagged(m.id, next);
    m.isFlagged = next;
  }

  async function snoozeFor(slot: 1 | 2, until: number) {
    const m = paneMsg(slot);
    if (!m) return;
    await api.snoozeMessage(m.id, until);
    const when = new Date(until * 1000).toLocaleString("pl-PL", {
      weekday: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
    showToast(`Odłożono do: ${when}`);
    clearSlot(slot);
    await refresh();
  }

  async function deleteFor(slot: 1 | 2) {
    const m = paneMsg(slot);
    if (!m) return;
    clearSlot(slot);
    try {
      await api.deleteMessage(m.id);
      showToast("Przeniesiono do Kosza");
    } catch (e) {
      showToast(`Nie udało się usunąć: ${e}`);
    }
    await refresh();
  }

  function accountForSelected(): Account | null {
    const folder = folders.find((f) => f.id === selected?.folderId);
    return accounts.find((a) => a.id === folder?.accountId) ?? null;
  }

  /// Konto, z którego domyślnie piszemy. Kolejność nie jest przypadkowa:
  /// otwarty folder mówi o zamiarze najwięcej („jestem w skrzynce firmowej,
  /// więc piszę służbowo"), potem zaznaczona wiadomość, a dopiero na końcu
  /// pierwsze konto z listy.
  function composeAccountId(): number {
    const v = view;
    if (v.kind === "folder") {
      const folder = folders.find((f) => f.id === v.folderId);
      if (folder) return folder.accountId;
    }
    return accountForSelected()?.id ?? accounts[0]?.id ?? 0;
  }

  /// Operatory rozpoznane w zapytaniu - pokazujemy je pod wyszukiwarką, żeby
  /// było widać, co program zrozumiał jako filtr, a co potraktował jak zwykły
  /// tekst. Nazwy muszą zgadzać się z `SEARCH_KEYS` w commands.rs.
  const FILTER_LABELS: Record<string, string> = {
    od: "Od",
    from: "Od",
    do: "Do",
    to: "Do",
    temat: "Temat",
    subject: "Temat",
    tytul: "Temat",
    folder: "Folder",
    in: "Folder",
    jest: "Jest",
    is: "Jest",
    ma: "Ma",
    has: "Ma",
    po: "Po",
    after: "Po",
    przed: "Przed",
    before: "Przed",
  };

  let activeFilters = $derived.by(() => {
    const tokens = searchQuery.split(/\s+/).filter(Boolean);
    const out: { label: string; value: string; tokens: string[] }[] = [];
    for (let i = 0; i < tokens.length; i++) {
      const parts = tokens[i].split(":");
      const label = FILTER_LABELS[parts[0].toLowerCase()];
      if (!label || !tokens[i].includes(":")) continue;
      const inline = parts.slice(1).join(":");
      if (inline) {
        out.push({ label, value: inline, tokens: [tokens[i]] });
      } else if (tokens[i + 1]) {
        // Wariant z odstępem: "od: nazwisko" - rdzeń rozumie oba zapisy.
        out.push({ label, value: tokens[i + 1], tokens: [tokens[i], tokens[i + 1]] });
        i++;
      }
    }
    return out;
  });

  /// Zapytanie rozbite na kawałki do pokolorowania. Zwykłe pole tekstowe nie
  /// pozwala stylować fragmentów treści, więc pod przezroczystym tekstem wpisu
  /// leży jego kolorowa kopia - stąd potrzeba zachowania odstępów co do znaku.
  let searchParts = $derived.by(() => {
    const parts: { text: string; op: boolean }[] = [];
    for (const chunk of searchQuery.split(/(\s+)/)) {
      const m = /^([a-zA-Z_]+):/.exec(chunk);
      if (m && FILTER_LABELS[m[1].toLowerCase()]) {
        parts.push({ text: m[0], op: true });
        const rest = chunk.slice(m[0].length);
        if (rest) parts.push({ text: rest, op: false });
      } else if (chunk) {
        parts.push({ text: chunk, op: false });
      }
    }
    return parts;
  });

  // Nakładka musi przewijać się razem z polem, inaczej przy dłuższym zapytaniu
  // kolory rozjadą się z tekstem.
  let searchMirror = $state<HTMLElement | undefined>();

  /// Usuwa filtr z zapytania razem z jego wartością.
  function dropFilter(tokens: string[]) {
    const rest = searchQuery
      .split(/\s+/)
      .filter(Boolean)
      .filter((t) => !tokens.includes(t));
    onSearch(rest.join(" "));
  }

  function escapeHtml(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  function signatureHtml(accountId?: number): string {
    const own = accountId != null ? (signatures[accountId] ?? "") : "";
    const sig = (own.trim() || signature).trim();
    if (!sig) return "";
    // Stopka z Outlooka/edytora to HTML; ręcznie wpisany zwykły tekst
    // konwertujemy z zachowaniem podziałów wierszy.
    const html = sig.includes("<") ? sig : escapeHtml(sig).replace(/\n/g, "<br>");
    return `<br><br>${html}`;
  }

  function openDraft(init: Partial<LocalDraft>) {
    const draft: LocalDraft = {
      localId: draftSeq++,
      accountId: init.accountId ?? composeAccountId(),
      toAddrs: init.toAddrs ?? "",
      ccAddrs: init.ccAddrs ?? "",
      bccAddrs: init.bccAddrs ?? "",
      inReplyTo: init.inReplyTo ?? null,
      references: init.references ?? null,
      subject: init.subject ?? "",
      bodyHtml: init.bodyHtml ?? "",
      attachments: [],
    };
    drafts.push(draft);
    activeTab = draft.localId;
  }

  function closeDraft(localId: number) {
    drafts = drafts.filter((d) => d.localId !== localId);
    if (activeTab === localId) activeTab = "mail";
  }

  function reSubjectOf(m: MessageSummary): string {
    return m.subject.startsWith("Re:") ? m.subject : `Re: ${m.subject}`;
  }

  function quotedBodyOf(m: MessageSummary, b: MessageBody | null): string {
    const when = new Date(m.date * 1000).toLocaleString("pl-PL");
    const source = escapeHtml(b?.text ?? "").replace(/\n/g, "<br>");
    const header = escapeHtml(
      `W dniu ${when} ${m.fromName || m.fromAddr} <${m.fromAddr}> napisał(a):`,
    );
    return `<br><br><div>${header}</div><blockquote>${source}</blockquote>`;
  }

  function replyFor(slot: 1 | 2, all: boolean) {
    const m = paneMsg(slot);
    if (!m) return;
    const b = slot === 1 ? body : body2;
    const toAddrs = m.fromAddr;
    // „Odpowiedz wszystkim": nadawca zostaje w polu Do, pozostali adresaci
    // schodzą do DW - tak robią Outlook i Spark, więc odbiorcy widzą, kto
    // jest właściwym adresatem, a kto tylko do wiadomości.
    let ccAddrs = "";
    if (all) {
      const folder = folders.find((f) => f.id === m.folderId);
      const own = accounts.find((a) => a.id === folder?.accountId)?.email.toLowerCase();
      const others = (b?.toAddrs ?? "")
        .split(",")
        .map((a) => a.trim())
        .filter(
          (a) => a && a.toLowerCase() !== own && a.toLowerCase() !== m.fromAddr.toLowerCase(),
        );
      ccAddrs = [...new Set(others)].join(", ");
    }
    const accountId = accountForSelected()?.id ?? accounts[0]?.id ?? 0;
    const draft: LocalDraft = {
      localId: draftSeq++,
      accountId,
      toAddrs,
      ccAddrs,
      bccAddrs: "",
      // Wątkowanie po stronie odbiorcy: In-Reply-To wskazuje wiadomość,
      // na którą odpowiadamy, a References buduje łańcuch całej rozmowy.
      inReplyTo: b?.messageId ?? null,
      references: [b?.inReplyTo, b?.messageId].filter(Boolean).join(" ") || null,
      subject: reSubjectOf(m),
      bodyHtml: signatureHtml(accountId) + quotedBodyOf(m, b),
      isReply: true,
      attachments: [],
    };
    // Odpowiedź pisze się obok wiadomości: mail po lewej, edytor po prawej.
    // Na wąskim ekranie nie ma „obok", więc szkic dostaje własną kartę
    // na pełnym ekranie.
    if (slot === 1) {
      placeSideDraft(draft);
    } else {
      // Odpowiedź z prawego panelu trafia do karty (miejsca obok już nie ma).
      drafts.push(draft);
      activeTab = draft.localId;
    }
  }

  /// Nowa wiadomość pisze się obok poczty (prawa połowa), tak samo jak
  /// odpowiedź. Gdy podzielony widok jest zajęty odpowiedzią, dostaje kartę.
  /// Czy szkic jest nietknięty - poza stopką nic w nim nie ma.
  function isDraftEmpty(d: LocalDraft): boolean {
    const text = (h: string) => h.replace(/<[^>]*>/g, "").replace(/&nbsp;|\s/g, "");
    return (
      !d.toAddrs.trim() &&
      !d.ccAddrs.trim() &&
      !d.bccAddrs.trim() &&
      !d.subject.trim() &&
      text(d.bodyHtml) === text(signatureHtml(d.accountId))
    );
  }

  /// Edytor mieszka z boku wiadomości i jest tam jeden. Nowy szkic przejmuje
  /// to miejsce, ale zaczęta praca nie może zniknąć bez śladu: niepusty szkic
  /// schodzi wtedy do karty. Pusty po prostu ustępuje - dzięki temu drugie
  /// kliknięcie „Napisz" nie mnoży kart.
  function placeSideDraft(draft: LocalDraft) {
    if (narrow) {
      drafts.push(draft);
      activeTab = draft.localId;
      return;
    }
    if (sideDraft && !isDraftEmpty(sideDraft)) drafts.push(sideDraft);
    selected2 = null;
    body2 = null;
    thread2 = [];
    sideDraft = draft;
    activeTab = "mail";
  }

  function composeNew() {
    // Szkic z boku jest pusty - nie ma po co robić drugiego takiego samego.
    if (!narrow && sideDraft && isDraftEmpty(sideDraft)) {
      activeTab = "mail";
      return;
    }
    const accountId = composeAccountId();
    const draft: LocalDraft = {
      localId: draftSeq++,
      accountId,
      toAddrs: "",
      ccAddrs: "",
      bccAddrs: "",
      inReplyTo: null,
      references: null,
      subject: "",
      bodyHtml: signatureHtml(accountId),
      attachments: [],
    };
    placeSideDraft(draft);
  }

  function forwardFor(slot: 1 | 2) {
    const m = paneMsg(slot);
    if (!m) return;
    const b = slot === 1 ? body : body2;
    openDraft({
      toAddrs: "",
      subject: m.subject.startsWith("Fwd:") ? m.subject : `Fwd: ${m.subject}`,
      accountId: composeAccountId(),
      bodyHtml: signatureHtml(composeAccountId()) + quotedBodyOf(m, b),
    });
  }

  // Akcje z menu kontekstowego listy (PPM na wiadomości).
  async function messageAction(m: MessageSummary, action: string, arg?: number) {
    switch (action) {
      case "rule":
        ruleFor = m;
        break;
      case "open":
        await openMessage(m);
        break;
      case "read":
      case "unread": {
        // Wiersz listy to cała konwersacja, więc oznaczamy ją w całości -
        // tak samo jak przy otwarciu.
        const read = action === "read";
        if (m.threadId) await api.setThreadRead(m.threadId, read);
        else await api.setRead(m.id, read);
        m.isRead = read;
        m.threadUnread = read ? 0 : Math.max(1, m.threadUnread);
        if (!read && holdInNewId === m.id) holdInNewId = null;
        folders = await api.listFolders();
        await refreshCounts();
        break;
      }
      case "flag": {
        const next = !m.isFlagged;
        await api.setFlagged(m.id, next);
        m.isFlagged = next;
        break;
      }
      case "snooze": {
        if (arg == null) break;
        await api.snoozeMessage(m.id, arg);
        if (selected?.id === m.id) {
          selected = null;
          body = null;
        }
        showToast("Odłożono na później");
        await refresh();
        break;
      }
      case "delete": {
        if (selected?.id === m.id) {
          selected = null;
          body = null;
        }
        try {
          await api.deleteMessage(m.id);
          showToast("Przeniesiono do Kosza");
        } catch (e) {
          showToast(`Nie udało się usunąć: ${e}`);
        }
        await refresh();
        break;
      }
    }
  }

  function detachSideDraft() {
    if (!sideDraft) return;
    drafts.push(sideDraft);
    activeTab = sideDraft.localId;
    sideDraft = null;
  }

  /// Opróżnienie Kosza jest nieodwracalne - w odróżnieniu od sprzątania nie ma
  /// dokąd przenieść, więc pytamy wprost i na czerwono.
  function emptyTrash(folder: Folder) {
    const count = folder.unreadCount;
    dialog = {
      title: "Opróżnić Kosz?",
      message:
        `Cała zawartość folderu „${folder.displayName}" zniknie z serwera bezpowrotnie` +
        (count > 0 ? ` (w tym ${count} nieprzeczytanych).` : ".") +
        " Tego nie da się cofnąć.",
      confirmLabel: "Opróżnij",
      danger: true,
      onconfirm: async () => {
        dialog = null;
        try {
          const removed = await api.emptyTrash(folder.id);
          showToast(`Kosz opróżniony: ${removed} wiadomości`);
          await refresh();
        } catch (e) {
          showToast(`Nie udało się opróżnić Kosza: ${e}`);
        }
      },
    };
  }

  async function sendDraft(draft: LocalDraft, sendAt: number | null) {
    const tmp = document.createElement("div");
    tmp.innerHTML = draft.bodyHtml;
    await api.queueSend({
      accountId: draft.accountId,
      toAddrs: draft.toAddrs,
      ccAddrs: draft.ccAddrs,
      bccAddrs: draft.bccAddrs,
      inReplyTo: draft.inReplyTo ?? null,
      references: draft.references ?? null,
      subject: draft.subject,
      bodyText: tmp.innerText,
      bodyHtml: draft.bodyHtml || null,
      sendAt,
      attachments: draft.attachments,
    });
    if (sideDraft?.localId === draft.localId) sideDraft = null;
    closeDraft(draft.localId);
    showToast(sendAt ? "Zaplanowano wysyłkę" : "Wysyłam wiadomość…");
  }

</script>

{#if splash}
  <div out:splashOut>
    <Splash statusText={splashText} />
  </div>
{/if}

<div class="app-surface flex h-screen flex-col bg-paper {splash ? '' : 'app-in'}">
  <!-- Pasek górny: marka, wyszukiwarka, akcje globalne. Pełni też rolę paska
       tytułu okna (decorations: false) - pusty obszar przeciąga okno. -->
  <header
    class="relative flex h-13 shrink-0 items-center
           {narrow
      ? 'gap-2 pl-2 pr-2'
      : isMac
        ? 'flex-row-reverse gap-3.5 pr-4 pl-28'
        : 'gap-3.5 pl-4'}"
    data-tauri-drag-region
    role="toolbar"
    tabindex="-1"
    aria-label="Pasek narzędzi aplikacji"
    ondblclick={(e) => {
      if (e.target === e.currentTarget) getCurrentWindow().toggleMaximize();
    }}
  >
    <!-- Ten sam pasek, co przy synchronizacji: cokolwiek chodzi w tle, widać
         to w jednym miejscu, zamiast szukać wskaźnika przy każdej czynności. -->
    {#if syncStatus || loadingMore}<div class="busy-bar"></div>{/if}

    <div
      class="flex shrink-0 items-center gap-2.25"
      style={narrow ? "" : `width:${sidebarW}px`}
      data-tauri-drag-region
    >
      <Logo size={30} />
      {#if !narrow}
        <span class="tight font-display text-[16.5px] font-semibold">lotusMail</span>
        {#if appVersion}
          <span class="text-[11px] font-semibold tabular-nums text-muted">{appVersion}</span>
        {/if}
      {/if}
    </div>

    <!-- Na wąskim ekranie jeden przycisk cofa o poziom: wiadomość → lista → foldery. -->
    {#if narrow && mobilePane !== "folders"}
      <button
        class="grid size-8.5 shrink-0 place-items-center rounded-full bg-surface text-ink-soft
               shadow-[var(--chip-shadow)]"
        onclick={() => (mobilePane = mobilePane === "message" ? "list" : "folders")}
        aria-label="Wstecz"
      >
        <Icon name="chevronDown" size={15} class="rotate-90" />
      </button>
    {/if}

    <!-- Rozpychacze po obu stronach trzymają wyszukiwarkę pośrodku niezależnie
         od tego, ile miejsca zajmują marka i przyciski - a te różnią się
         szerokością między systemami. -->
    <!-- Rozpychacz tylko na macOS: tam marka siedzi po prawej, więc bez niego
         wyszukiwarka przykleiłaby się do przycisków. Na Windowsie marka ma
         szerokość panelu bocznego, dzięki czemu pole zaczyna się dokładnie
         nad listą wiadomości - i tak ma zostać. -->
    {#if isMac && !narrow}
      <div class="flex-1" data-tauri-drag-region></div>
    {/if}

    <div
      class="relative flex h-8.5 max-w-115 min-w-0 flex-1 items-center gap-2.25 rounded-full
             bg-surface px-3.5 shadow-[var(--chip-shadow)]"
    >
      <Icon name="search" size={14} class="text-muted" />
      <span class="relative min-w-0 flex-1">
        <span
          bind:this={searchMirror}
          aria-hidden="true"
          class="pointer-events-none absolute inset-0 flex items-center overflow-hidden
                 whitespace-pre text-[13px]"
        >{#each searchParts as part, i (i)}<span
              class={part.op ? 'font-semibold text-accent' : 'text-ink'}>{part.text}</span
            >{/each}</span
        >
      <input
        type="search"
        placeholder="Szukaj wiadomości"
        value={searchQuery}
        onscroll={(e) => {
          if (searchMirror) searchMirror.scrollLeft = e.currentTarget.scrollLeft;
        }}
        oninput={(e) => {
          // Ściągawka z operatorami tylko przy pustym polu - gdy już piszesz,
          // zasłaniałaby wyniki.
          searchHelp = e.currentTarget.value.trim().length === 0;
          onSearch(e.currentTarget.value);
        }}
        onfocus={(e) => (searchHelp = e.currentTarget.value.trim().length === 0)}
        onblur={() => setTimeout(() => (searchHelp = false), 200)}
        style="caret-color: var(--ink)"
        class="relative w-full bg-transparent text-[13px] text-transparent outline-none
               placeholder:text-muted"
      />
      </span>
      {#if searchQuery}
        <button
          class="grid size-5 place-items-center rounded-full text-muted hover:text-ink"
          onclick={() => onSearch("")}
          aria-label="Wyczyść wyszukiwanie"
        >
          <Icon name="x" size={12} />
        </button>
      {/if}

      {#if searchHelp}
        <div
          class="panel absolute top-11 left-0 z-20 w-115 p-3 ring-1 ring-line"
          transition:fade={{ duration: 120 }}
        >
          <p class="mb-2 text-[11px] font-bold tracking-[0.09em] text-muted uppercase">
            Operatory wyszukiwania
          </p>
          <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-[12px]">
            {#each searchHints as hint (hint.op)}
              <button
                class="flex items-baseline gap-2 rounded-md px-1.5 py-0.75 text-left hover:bg-rail"
                onmousedown={(e) => e.preventDefault()}
                onclick={() => appendSearchOperator(hint.op)}
              >
                <code class="font-mono text-accent">{hint.op}</code>
                <span class="text-muted">{hint.desc}</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>

    <!-- Stan synchronizacji pokazuje wyłącznie panel boczny na dole; tutaj zostaje
         tylko cienki pasek postępu pod krawędzią nagłówka. -->
    {#if !narrow}
      <div class="flex-1 self-stretch" data-tauri-drag-region></div>
    {/if}

    <button
      class="flex h-8.5 items-center gap-1.75 rounded-full bg-accent px-4 text-[13px]
             font-semibold text-on-accent hover:opacity-90"
      onclick={composeNew}
    >
      <Icon name="edit" size={14} />
      {#if !narrow}Napisz{/if}
    </button>
    <button
      class="grid size-8.5 place-items-center rounded-full bg-surface text-ink-soft
             shadow-[var(--chip-shadow)]"
      onclick={checkMail}
      disabled={checking}
      aria-label="Sprawdź pocztę"
      hidden={narrow}
      title="Sprawdź pocztę teraz"
    >
      <Icon name="refresh" size={15} class={checking ? "animate-spin text-accent" : ""} />
    </button>
    <button
      class="grid size-8.5 place-items-center rounded-full bg-surface text-ink-soft
             shadow-[var(--chip-shadow)] {narrow ? 'hidden' : ''}"
      onclick={() => (theme.dark = !theme.dark)}
      aria-label={theme.dark ? "Jasny motyw" : "Ciemny motyw"}
      title={theme.dark ? "Jasny motyw" : "Ciemny motyw"}
    >
      <Icon name={theme.dark ? "sun" : "moon"} size={15} />
    </button>
    <button
      class="grid size-8.5 place-items-center rounded-full bg-surface text-ink-soft
             shadow-[var(--chip-shadow)]"
      onclick={() => (settingsOpen = true)}
      aria-label="Ustawienia"
      title="Ustawienia"
    >
      <Icon name="settings" size={15} />
    </button>

    {#if !narrow && !isMac}<div class="ml-2.5 self-start"><WindowControls /></div>{/if}
  </header>

  <!-- Co program zrozumiał jako filtr. Bez tego „od: ktoś" wyglądało tak samo
       jak zwykły tekst, a wynik potrafił być zupełnie inny. -->
  {#if activeFilters.length > 0}
    <div class="flex flex-wrap items-center gap-1.5 px-4 pb-1.5" transition:fly={{ y: -6, duration: 140 }}>
      {#each activeFilters as f (f.label + f.value)}
        <span
          class="flex h-6.5 items-center gap-1.5 rounded-full bg-accent-soft px-2.5 text-[12px]
                 font-semibold text-accent"
        >
          {f.label}: <span class="font-normal text-ink">{f.value}</span>
          <button
            class="grid size-4 place-items-center rounded-full hover:bg-accent/20"
            onclick={() => dropFilter(f.tokens)}
            aria-label="Usuń filtr {f.label}"
          >
            <Icon name="x" size={10} />
          </button>
        </span>
      {/each}
    </div>
  {/if}

  {#if drafts.length > 0}
    <nav
      class="flex h-10 shrink-0 items-center gap-1.5 px-3 pb-1"
      transition:fly={{ y: -8, duration: 160 }}
    >
      <button
        class="flex h-7.5 items-center gap-1.5 rounded-full px-3.5 text-xs font-semibold
               transition-colors
               {activeTab === 'mail'
          ? 'bg-surface text-accent shadow-[var(--chip-shadow)]'
          : 'text-muted hover:bg-surface/60'}"
        onclick={() => (activeTab = "mail")}
      >
        <Icon name="inbox" size={13} />
        Poczta
      </button>
      {#each drafts as d (d.localId)}
        <div
          class="flex h-7.5 max-w-52 items-center gap-1 rounded-full px-3 text-xs font-semibold
                 transition-colors
                 {activeTab === d.localId
            ? 'bg-surface text-ink shadow-[var(--chip-shadow)]'
            : 'text-muted hover:bg-surface/60'}"
        >
          <button
            class="flex min-w-0 items-center gap-1.5"
            onclick={() => (activeTab = d.localId)}
          >
            <Icon name="edit" size={12} class="shrink-0 {activeTab === d.localId ? 'text-accent' : ''}" />
            <span class="truncate">{d.subject || "Nowa wiadomość"}</span>
          </button>
          <button
            class="rounded p-0.5 text-muted hover:bg-line/60 hover:text-danger"
            onclick={() => closeDraft(d.localId)}
            aria-label="Zamknij szkic"
          >
            <Icon name="x" size={11} />
          </button>
        </div>
      {/each}
    </nav>
  {/if}
  <main
    class="flex min-h-0 flex-1 pb-3 {narrow ? 'px-2' : 'px-3'}
           {activeTab === 'mail' ? '' : 'hidden'}"
  >
  <div
    class="h-full {narrow ? (mobilePane === 'folders' ? 'w-full' : 'hidden') : 'shrink-0'}"
    style={narrow ? "" : `width:${sidebarW}px`}
  >
    <Sidebar
      {accounts}
      {folders}
      {view}
      {lastSyncLabel}
      {syncStatus}
      onselect={selectView}
      onaddaccount={() => (addAccountOpen = true)}
      onnewfolder={newFolder}
      ondeletefolder={deleteFolder}
      onmarkread={markFolderRead}
      oncleanup={() => (cleanupOpen = true)}
      onemptytrash={emptyTrash}
      onreorder={async (ids) => {
        await api.reorderFolders(ids);
        folders = await api.listFolders();
      }}
    />
  </div>
  {#if !narrow}
    <div
      class="resizer {dragging === 'sidebar' ? 'dragging' : ''}"
      role="separator"
      aria-orientation="vertical"
      aria-label="Zmień szerokość panelu bocznego"
      onpointerdown={(e) => startDrag(e, "sidebar")}
    ></div>
  {/if}
  <div
    class="h-full {narrow ? (mobilePane === 'list' ? 'w-full' : 'hidden') : 'shrink-0'}"
    style={narrow ? "" : `width:${listW}px`}
  >
    <MessageList
      {messages}
      selectedId={selected?.id ?? null}
      holdId={holdInNewId}
      {showCategories}
      {category}
      {counts}
      {sort}
      {searchQuery}
      title={listTitle}
      onopen={openMessage}
      oncategory={selectCategory}
      onsort={selectSort}
      onaction={messageAction}
      onsearchappend={appendSearchOperator}
      onsearchserver={searchOnServer}
      {serverSearching}
      {loadingMore}
      {hasMore}
      onloadmore={loadMore}
    />
  </div>
  {#if !narrow}
    <div
      class="resizer {dragging === 'list' ? 'dragging' : ''}"
      role="separator"
      aria-orientation="vertical"
      aria-label="Zmień szerokość listy wiadomości"
      onpointerdown={(e) => startDrag(e, "list")}
    ></div>
  {/if}
  <div
    class="relative flex min-w-0 flex-1 gap-3 {mailDragging ? 'drag-active' : ''}
           {narrow && mobilePane !== 'message' ? 'hidden' : ''}"
    role="region"
    aria-label="Panel czytania"
    ondragenter={onSplitDragEnter}
    ondragleave={onSplitDragLeave}
    ondragover={onSplitDragOver}
    ondrop={onSplitDrop}
  >
    <ReadingPane
      message={selected}
      {body}
      thread={thread1}
      accountLabel={accountLabelOf(selected)}
      onreply={() => replyFor(1, false)}
      onreplyall={() => replyFor(1, true)}
      onforward={() => forwardFor(1)}
      onsnooze={(u) => snoozeFor(1, u)}
      onflag={() => flagFor(selected)}
      ondelete={() => deleteFor(1)}
      ontoast={showToast}
      onclosepane={narrow ? () => (mobilePane = "list") : selected2 ? () => closePane(1) : null}
    />
    {#if sideDraft}
      <div class="flex min-w-0 flex-1 flex-col" in:fly={{ x: 48, duration: 220 }}>
        <ComposeView
          draft={sideDraft}
          {accounts}
          embedded
          onsend={(sendAt) => sendDraft(sideDraft!, sendAt)}
          onclose={() => (sideDraft = null)}
          ondetach={detachSideDraft}
        />
      </div>
    {:else if selected2}
      <div class="flex min-w-0 flex-1" in:fly={{ x: 48, duration: 220 }}>
        <ReadingPane
          message={selected2}
          body={body2}
          thread={thread2}
          accountLabel={accountLabelOf(selected2)}
          onreply={() => replyFor(2, false)}
          onreplyall={() => replyFor(2, true)}
          onforward={() => forwardFor(2)}
          onsnooze={(u) => snoozeFor(2, u)}
          onflag={() => flagFor(selected2)}
          ondelete={() => deleteFor(2)}
          ontoast={showToast}
          onclosepane={() => closePane(2)}
        />
      </div>
    {/if}
    {#if dragOverSplit}
      <div class="pointer-events-none absolute inset-0 z-10 flex" transition:fade={{ duration: 120 }}>
        {#if !selected && !selected2}
          <div class="flex-1 p-2">
            <div
              class="grid h-full place-items-center rounded-2xl border-2 border-dashed
                     border-accent bg-accent-soft/50"
            >
              <p
                class="flex items-center gap-2 rounded-full bg-surface px-4 py-2 text-sm
                       font-semibold text-accent shadow-lg"
              >
                <Icon name="mail" size={15} />
                Upuść, aby otworzyć
              </p>
            </div>
          </div>
        {:else}
          <!-- Wybór strony kursorem: podświetlona połowa = miejsce lądowania -->
          {#snippet dropHalf(side: "left" | "right", label: string)}
            {@const active = dropSide === side}
            <div class="flex-1 p-2 transition-all duration-150">
              <div
                class="grid h-full place-items-center rounded-2xl border-2 transition-all duration-150
                       {active
                  ? 'border-dashed border-accent bg-accent-soft/60'
                  : 'border-transparent bg-ink/15'}"
              >
                <p
                  class="flex items-center gap-2 rounded-full bg-surface px-4 py-2 text-sm
                         font-semibold text-accent shadow-lg transition-all duration-150
                         {active ? 'scale-100 opacity-100' : 'scale-90 opacity-0'}"
                >
                  <Icon name="mail" size={15} />
                  {label}
                </p>
              </div>
            </div>
          {/snippet}
          {@render dropHalf(
            "left",
            selected ? "Tu - obecny mail pojedzie w prawo" : "Otworzy się po lewej",
          )}
          {@render dropHalf(
            "right",
            selected2 ? "Zastąpi prawy panel" : "Otworzy się po prawej",
          )}
        {/if}
      </div>
    {/if}
  </div>
</main>

{#each drafts as d (d.localId)}
  {#if activeTab === d.localId}
    <ComposeView
      draft={d}
      {accounts}
      onsend={(sendAt) => sendDraft(d, sendAt)}
      onclose={() => closeDraft(d.localId)}
    />
  {/if}
{/each}

</div>

{#if dragging}
  <div class="fixed inset-0 z-50 cursor-col-resize"></div>
{/if}

<svelte:window
  ondragstart={() => (mailDragging = true)}
  ondragend={() => (mailDragging = false)}
  onkeydown={onWindowKeydown}
  oncontextmenu={(e) => e.preventDefault()}
/>

{#if loaded && accounts.length === 0}
  <div class="fixed inset-0 z-30 grid place-items-center bg-paper p-4" transition:fade={{ duration: 200 }}>
    <div class="panel flex w-full max-w-105 flex-col items-center gap-4 p-6 text-center sm:p-10">
      <Logo size={56} />
      <h1 class="tight font-display text-xl font-bold">lotusMail</h1>
      <p class="text-sm leading-relaxed text-muted text-pretty">
        Twoja poczta, offline-first: Rust, SQLite i lekki interfejs.
        Podłącz konto IMAP - ustawienia serwerów wykryją się automatycznie.
      </p>
      <button
        class="rounded-full bg-accent px-5 py-2.5 text-sm font-semibold text-on-accent transition-opacity hover:opacity-90"
        onclick={() => {
          addAccountStep = "provider";
          addAccountOpen = true;
        }}
      >
        Dodaj konto
      </button>
      <!-- Druga droga na pierwszym ekranie: kto ma już lotusMaila gdzie indziej,
           nie powinien przepisywać kont ręcznie. -->
      <button
        class="text-[13px] font-semibold text-accent hover:underline"
        onclick={() => {
          addAccountStep = "transfer";
          addAccountOpen = true;
        }}
      >
        Mam już lotusMaila na innym urządzeniu
      </button>
    </div>
  </div>
{/if}

{#if addAccountOpen}
  <AddAccountModal
    initialStep={addAccountStep}
    onclose={() => (addAccountOpen = false)}
    onadded={accountAdded}
    onimported={async (added, updated, device) => {
      addAccountOpen = false;
      showToast(`Z urządzenia „${device}": dodano ${added}, zaktualizowano ${updated}`);
      await refresh();
    }}
  />
{/if}

{#if cleanupOpen}
  <CleanupModal
    {accounts}
    onclose={() => (cleanupOpen = false)}
    ondone={(deleted) => {
      showToast(`Przeniesiono do Kosza: ${deleted}`);
      refresh();
    }}
  />
{/if}

{#if dialog}
  <Dialog
    title={dialog.title}
    message={dialog.message ?? ""}
    placeholder={dialog.placeholder ?? ""}
    initialValue={dialog.initialValue ?? null}
    confirmLabel={dialog.confirmLabel ?? "OK"}
    danger={dialog.danger ?? false}
    onconfirm={dialog.onconfirm}
    oncancel={() => (dialog = null)}
  />
{/if}

{#if ruleFor && ruleAccountId}
  <RuleModal
    message={ruleFor}
    accountId={ruleAccountId}
    folders={ruleFolders}
    onclose={() => (ruleFor = null)}
    onsaved={async (folderName) => {
      ruleFor = null;
      showToast(`Reguła zapisana - przenoszę do folderu ${folderName}…`);
      await refresh();
    }}
  />
{/if}

{#if settingsOpen}
  <SettingsModal
    {signature}
    {signatures}
    {accounts}
    onclose={() => (settingsOpen = false)}
    onsaved={async (s, perAccount) => {
      signature = s;
      signatures = perAccount;
      settingsOpen = false;
      accounts = await api.listAccounts();
      showToast("Zapisano ustawienia");
    }}
  />
{/if}

{#if toast}
  <div class="pointer-events-none fixed bottom-5 left-1/2 z-40 -translate-x-1/2">
    <div
      class="rounded-full bg-ink px-4 py-2 text-[13px] font-medium text-paper shadow-lg"
      transition:fly={{ y: 16, duration: 220 }}
    >
      {toast}
    </div>
  </div>
{/if}

