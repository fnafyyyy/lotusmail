<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { api } from "$lib/api";
  import { onMount } from "svelte";
  import type { Account, OutlookSignature, Rule } from "$lib/types";
  import Icon from "./Icon.svelte";

  let {
    signature,
    signatures,
    accounts,
    onclose,
    onsaved,
  }: {
    /** Stopka domyślna - używana przez konta bez własnej. */
    signature: string;
    /** Stopki przypisane do konkretnych kont. */
    signatures: Record<number, string>;
    accounts: Account[];
    onclose: () => void;
    onsaved: (signature: string, signatures: Record<number, string>) => void;
  } = $props();

  // Którą stopkę edytujemy: `null` to domyślna, liczba to konto.
  let sigTarget = $state<number | null>(null);
  // Kopie robocze wszystkich stopek - przełączanie kont nie może gubić zmian.
  // svelte-ignore state_referenced_locally
  let sigDrafts = $state<Record<string, string>>({
    default: signature,
    ...Object.fromEntries(accounts.map((a) => [String(a.id), signatures[a.id] ?? ""])),
  });

  const sigKey = (t: number | null) => (t === null ? "default" : String(t));

  /// Przełączenie edytowanej stopki: bieżącą treść odkładamy do kopii roboczych,
  /// a do edytora wstawiamy tę wybraną.
  function switchSignature(target: number | null) {
    sigDrafts[sigKey(sigTarget)] = html;
    sigTarget = target;
    html = sigDrafts[sigKey(target)] ?? "";
  }

  // Nazwy nadawcy i etykiety kont (kopie robocze do edycji).
  // svelte-ignore state_referenced_locally
  let senderNames = $state<Record<number, string>>(
    Object.fromEntries(accounts.map((a) => [a.id, a.senderName])),
  );
  // svelte-ignore state_referenced_locally
  let labels = $state<Record<number, string>>(
    Object.fromEntries(accounts.map((a) => [a.id, a.displayName])),
  );

  // Kopia robocza do edycji - celowo tylko wartość początkowa propa.
  // svelte-ignore state_referenced_locally
  let html = $state(signature);
  let saving = $state(false);
  let importing = $state(false);
  let importInfo = $state("");
  let outlookChoices = $state<OutlookSignature[] | null>(null);
  let rules = $state<Rule[]>([]);
  let notify = $state(true);
  let notifyPrimaryOnly = $state(false);
  // Identyfikator aplikacji Microsoft dla logowania OAuth2. Każda instalacja
  // rejestruje własną - program na komputerze nie ma gdzie schować sekretu,
  // więc nie da się rozdać jednego wspólnego.
  let oauthClientId = $state("");
  let oauthSaved = $state(false);

  async function saveClientId() {
    await api.setSetting("oauth_client_id", oauthClientId.trim());
    oauthSaved = true;
    setTimeout(() => (oauthSaved = false), 2000);
  }

  onMount(() => {
    api.listRules().then((r) => (rules = r));
    api.getSetting("notify").then((v) => (notify = v !== "0"));
    api.getSetting("notify_primary_only").then((v) => (notifyPrimaryOnly = v === "1"));
    api.getSetting("oauth_client_id").then((v) => (oauthClientId = v ?? ""));
  });

  /// Przełączniki zapisują się od razu - bez czekania na „Zapisz".
  async function toggleNotify(value: boolean) {
    notify = value;
    await api.setSetting("notify", value ? "1" : "0");
  }

  async function togglePrimaryOnly(value: boolean) {
    notifyPrimaryOnly = value;
    await api.setSetting("notify_primary_only", value ? "1" : "0");
  }

  async function removeRule(id: number) {
    await api.deleteRule(id);
    rules = rules.filter((r) => r.id !== id);
  }

  async function importFromOutlook() {
    importing = true;
    importInfo = "";
    outlookChoices = null;
    try {
      const sigs = await api.listOutlookSignatures();
      if (sigs.length === 0) {
        importInfo =
          "Nie znaleziono podpisów w %APPDATA%\\Microsoft\\Signatures. Klasyczny Outlook trzyma je tam; „nowy Outlook\" przechowuje podpisy w chmurze - wtedy skopiuj stopkę w Outlooku i wklej ją poniżej (formatowanie zostanie zachowane).";
      } else if (sigs.length === 1) {
        html = sigs[0].html;
        importInfo = `Zaimportowano podpis „${sigs[0].name}".`;
      } else {
        outlookChoices = sigs;
      }
    } catch (e) {
      importInfo = String(e);
    } finally {
      importing = false;
    }
  }

  // Stopki z Outlooka/Worda niosą wymuszone białe tło i czarny tekst -
  // w ciemnym motywie robi się z tego biała tabliczka. Usuwamy tła i czerń
  // (tekst dziedziczy kolor motywu, u odbiorcy renderuje się normalnie),
  // kolory celowe (linki, branding) zostają.
  function cleanForcedStyles(input: string): string {
    return input
      .replace(/background(-color)?\s*:\s*[^;"']+;?/gi, "")
      .replace(
        /(^|;)\s*color\s*:\s*(rgb\(\s*0\s*,\s*0\s*,\s*0\s*\)|#000000|#000|black|windowtext)\s*;?/gi,
        "$1",
      );
  }

  async function save() {
    saving = true;
    try {
      // Bieżąca treść edytora należy do aktualnie wybranej stopki.
      sigDrafts[sigKey(sigTarget)] = html;
      const cleanedDefault = cleanForcedStyles(sigDrafts.default ?? "");
      await api.setSetting("signature", cleanedDefault);

      const perAccount: Record<number, string> = {};
      for (const a of accounts) {
        const own = cleanForcedStyles(sigDrafts[String(a.id)] ?? "");
        await api.setSetting(`signature:${a.id}`, own);
        if (own.trim()) perAccount[a.id] = own;
      }

      for (const a of accounts) {
        if ((senderNames[a.id] ?? "") !== a.senderName) {
          await api.setSenderName(a.id, (senderNames[a.id] ?? "").trim());
        }
        if ((labels[a.id] ?? "") !== a.displayName) {
          await api.setAccountLabel(a.id, (labels[a.id] ?? "").trim());
        }
      }
      onsaved(cleanedDefault, perAccount);
    } finally {
      saving = false;
    }
  }

  // --- Synchronizacja kont między urządzeniami ---
  let syncHasPass = $state(false);
  let syncPass = $state("");
  // svelte-ignore state_referenced_locally
  let syncAccountId = $state<number>(accounts[0]?.id ?? 0);
  let syncBusy = $state(false);
  let syncMsg = $state("");
  let syncErr = $state(false);

  onMount(() => {
    api.syncHasPassphrase().then((v) => (syncHasPass = v));
  });

  function say(text: string, error = false) {
    syncMsg = text;
    syncErr = error;
  }

  async function saveSyncPass() {
    try {
      await api.syncSetPassphrase(syncPass);
      syncHasPass = true;
      syncPass = "";
      say("Hasło zapisane w pęku kluczy.");
    } catch (e) {
      say(String(e), true);
    }
  }

  async function pushConfig() {
    syncBusy = true;
    try {
      const n = await api.syncPush(syncAccountId);
      say(`Zapisano ${n} kont w folderze LotusMail.`);
    } catch (e) {
      say(String(e), true);
    } finally {
      syncBusy = false;
    }
  }

  async function pullConfig() {
    syncBusy = true;
    try {
      const r = await api.syncPull(syncAccountId);
      say(`Z urządzenia „${r.device}": dodano ${r.added}, zaktualizowano ${r.updated}.`);
    } catch (e) {
      say(String(e), true);
    } finally {
      syncBusy = false;
    }
  }

  async function copyCode() {
    syncBusy = true;
    try {
      await navigator.clipboard.writeText(await api.syncExport());
      say("Kod w schowku - wklej go na drugim urządzeniu.");
    } catch (e) {
      say(String(e), true);
    } finally {
      syncBusy = false;
    }
  }

  async function pasteCode() {
    syncBusy = true;
    try {
      const r = await api.syncImport(await navigator.clipboard.readText());
      say(`Z urządzenia „${r.device}": dodano ${r.added}, zaktualizowano ${r.updated}.`);
    } catch (e) {
      say(String(e), true);
    } finally {
      syncBusy = false;
    }
  }
</script>

<div
  class="fixed inset-0 z-20 grid place-items-center bg-ink/30 p-3"
  role="presentation"
  transition:fade={{ duration: 140 }}
  onclick={(e) => e.target === e.currentTarget && onclose()}
>
  <div
    class="flex max-h-[85vh] w-full max-w-[620px] flex-col rounded-2xl bg-surface shadow-2xl"
    transition:scale={{ start: 0.96, duration: 170 }}
  >
    <header class="flex items-center justify-between border-b border-line px-5 py-3">
      <h2 class="flex items-center gap-2 font-display text-[15px] font-semibold">
        <Icon name="settings" size={16} class="text-muted" />
        Ustawienia
      </h2>
      <button class="rounded-md p-1.5 text-muted hover:bg-line/60" onclick={onclose} aria-label="Zamknij">
        <Icon name="x" size={16} />
      </button>
    </header>

    <div class="flex-1 space-y-3 overflow-y-auto px-5 py-4">
      {#if accounts.length > 0}
        <div>
          <p class="text-xs font-semibold text-ink-soft">Konta</p>
          <p class="mb-2 text-[11px] leading-relaxed text-muted">
            Etykieta to nazwa w panelu bocznym (pusta = adres e-mail). Nadawca to imię
            i nazwisko widoczne u odbiorców w polu „Od".
          </p>
          <div class="space-y-2">
            {#each accounts as a (a.id)}
              <div class="rounded-xl bg-rail/60 p-2.5">
                <p class="mb-1.5 truncate text-[11px] font-semibold text-muted">{a.email}</p>
                <div class="grid grid-cols-2 gap-2">
                  <input
                    bind:value={labels[a.id]}
                    placeholder={a.email}
                    class="min-w-0 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm
                           outline-none placeholder:text-muted focus:border-accent"
                    aria-label="Etykieta konta {a.email}"
                  />
                  <input
                    bind:value={senderNames[a.id]}
                    placeholder="Nadawca, np. Jan Kowalski"
                    class="min-w-0 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm
                           outline-none placeholder:text-muted focus:border-accent"
                    aria-label="Nazwa nadawcy dla {a.email}"
                  />
                </div>
              </div>
            {/each}
          </div>
        </div>
        <div class="h-px bg-line"></div>
      {/if}
      <div>
        <p class="text-xs font-semibold text-ink-soft">Logowanie Microsoft (OAuth2)</p>
        <p class="mb-2 text-[11px] leading-relaxed text-muted">
          Konta Outlook.com i Microsoft 365 nie przyjmują już hasła. Zarejestruj aplikację
          w Microsoft Entra (typ „Public client/native", adres powrotu
          <span class="font-mono">http://localhost</span>) i wklej tutaj jej Application (client) ID.
        </p>
        <div class="flex items-center gap-2">
          <input
            bind:value={oauthClientId}
            placeholder="00000000-0000-0000-0000-000000000000"
            class="min-w-0 flex-1 rounded-lg border border-line bg-paper px-3 py-2 font-mono
                   text-[12px] outline-none placeholder:text-muted focus:border-accent"
          />
          <button
            class="shrink-0 rounded-lg bg-accent px-3 py-2 text-xs font-semibold text-on-accent
                   disabled:opacity-40"
            onclick={saveClientId}
          >
            {oauthSaved ? "Zapisano" : "Zapisz"}
          </button>
        </div>
      </div>
      <div class="h-px bg-line"></div>
      <div>
        <p class="text-xs font-semibold text-ink-soft">Powiadomienia</p>
        <p class="mb-2 text-[11px] leading-relaxed text-muted">
          Systemowy komunikat o nowej poczcie - działa też przy zminimalizowanym oknie.
        </p>
        <div class="space-y-1">
          <label class="flex cursor-pointer items-center gap-2.5 rounded-lg bg-rail/60 px-3 py-2">
            <input
              type="checkbox"
              checked={notify}
              onchange={(e) => toggleNotify(e.currentTarget.checked)}
              class="size-4 accent-[var(--accent)]"
            />
            <span class="flex-1 text-[13px]">Powiadamiaj o nowych wiadomościach</span>
          </label>
          <label
            class="flex items-center gap-2.5 rounded-lg bg-rail/60 px-3 py-2
                   {notify ? 'cursor-pointer' : 'opacity-40'}"
          >
            <input
              type="checkbox"
              checked={notifyPrimaryOnly}
              disabled={!notify}
              onchange={(e) => togglePrimaryOnly(e.currentTarget.checked)}
              class="size-4 accent-[var(--accent)]"
            />
            <span class="flex-1 text-[13px]">
              Tylko z zakładki Główne
              <span class="block text-[11px] text-muted">
                Bez newsletterów i powiadomień z systemów.
              </span>
            </span>
          </label>
        </div>
      </div>
      <div class="h-px bg-line"></div>

      <div>
        <p class="text-xs font-semibold text-ink-soft">Reguły przenoszenia ({rules.length})</p>
        <p class="mb-2 text-[11px] leading-relaxed text-muted">
          Wiadomości od tych nadawców trafiają automatycznie do wskazanych folderów - także na
          serwerze, więc porządek widać w każdym programie pocztowym.
        </p>
        {#if rules.length === 0}
          <p class="rounded-lg bg-rail/60 px-3 py-2 text-[12px] leading-relaxed text-muted">
            Nie masz jeszcze reguł. Kliknij wiadomość prawym przyciskiem i wybierz
            <span class="font-semibold text-ink-soft">„Zawsze przenoś od tego nadawcy…"</span>.
          </p>
        {:else}
          <div class="space-y-1">
            {#each rules as rule (rule.id)}
              <div class="flex items-center gap-2 rounded-lg bg-rail/60 px-3 py-1.5 text-[13px]">
                <Icon name="folder" size={13} class="shrink-0 text-muted" />
                <span class="min-w-0 flex-1 truncate">
                  {rule.fromAddr} <span class="text-muted">do</span>
                  <span class="font-semibold">{rule.folderName}</span>
                </span>
                <button
                  class="grid size-6 shrink-0 place-items-center rounded-md text-muted
                         hover:bg-danger/10 hover:text-danger"
                  onclick={() => removeRule(rule.id)}
                  aria-label="Usuń regułę"
                  title="Usuń regułę"
                >
                  <Icon name="trash" size={13} />
                </button>
              </div>
            {/each}
          </div>
        {/if}
      </div>
      <div class="h-px bg-line"></div>

      <div class="flex items-center justify-between">
        <div>
          <p class="text-xs font-semibold text-ink-soft">Stopka (podpis)</p>
          <p class="text-[11px] leading-relaxed text-muted">
            Dołączana na końcu nowych wiadomości i odpowiedzi. Możesz też wkleić
            stopkę wprost do pola - formatowanie zostanie zachowane.
          </p>
        </div>
        <button
          class="flex shrink-0 items-center gap-1.5 rounded-lg border border-line px-3 py-1.5
                 text-xs font-semibold text-ink-soft hover:bg-paper disabled:opacity-40"
          disabled={importing}
          onclick={importFromOutlook}
        >
          <Icon name="mail" size={13} />
          {importing ? "Importuję…" : "Importuj z Outlooka"}
        </button>
      </div>

      {#if outlookChoices}
        <div class="space-y-1 rounded-xl border border-line bg-paper p-2">
          <p class="px-1 text-[11px] font-semibold tracking-wide text-muted uppercase">
            Wybierz podpis
          </p>
          {#each outlookChoices as sig (sig.name)}
            <button
              class="block w-full rounded-lg px-2.5 py-1.5 text-left text-[13px] hover:bg-accent-soft"
              onclick={() => {
                html = sig.html;
                outlookChoices = null;
                importInfo = `Zaimportowano podpis „${sig.name}".`;
              }}
            >
              {sig.name}
            </button>
          {/each}
        </div>
      {/if}

      {#if importInfo}
        <p class="rounded-lg bg-accent-soft px-3 py-2 text-[11px] leading-relaxed text-ink-soft">
          {importInfo}
        </p>
      {/if}

      <!-- Której stopki dotyczy edytor. Konto bez własnej treści użyje domyślnej. -->
      <div class="flex flex-wrap gap-1.5">
        <button
          class="rounded-full px-3 py-1 text-[12px] font-semibold transition-colors
                 {sigTarget === null ? 'bg-accent text-on-accent' : 'bg-rail text-ink-soft hover:text-ink'}"
          onclick={() => switchSignature(null)}
        >
          Domyślna
        </button>
        {#each accounts as a (a.id)}
          {@const own = (sigDrafts[String(a.id)] ?? "").trim().length > 0}
          <button
            class="rounded-full px-3 py-1 text-[12px] font-semibold transition-colors
                   {sigTarget === a.id ? 'bg-accent text-on-accent' : 'bg-rail text-ink-soft hover:text-ink'}"
            onclick={() => switchSignature(a.id)}
            title={own ? "Ma własną stopkę" : "Używa domyślnej"}
          >
            {a.displayName || a.email}{own ? "" : " ·"}
          </button>
        {/each}
      </div>
      {#if sigTarget !== null}
        <p class="text-[11px] text-muted">
          Puste pole = to konto użyje stopki domyślnej.
        </p>
      {/if}

      <div
        contenteditable="true"
        bind:innerHTML={html}
        role="textbox"
        aria-multiline="true"
        aria-label="Treść stopki"
        tabindex="0"
        class="sig-editor min-h-40 w-full overflow-y-auto rounded-lg border border-line bg-paper
               px-3 py-2 text-sm leading-relaxed outline-none focus:border-accent"
      ></div>
      <div class="h-px bg-line"></div>
      <div>
        <p class="text-xs font-semibold text-ink-soft">Synchronizacja kont</p>
        <p class="mb-2 text-[11px] leading-relaxed text-muted">
          Przenosi konta na inne urządzenia bez żadnego serwera pośredniego. Paczka jest
          szyfrowana hasłem, które znasz tylko Ty, i leży jako wiadomość w folderze
          <span class="font-semibold text-ink-soft">LotusMail</span> na wybranym koncie -
          Twój serwer poczty widzi wyłącznie nieczytelny blob.
        </p>

        <div class="space-y-2 rounded-xl bg-rail/60 p-2.5">
          <div>
            <p class="mb-1 text-[11px] font-semibold text-muted">
              Hasło synchronizacji {syncHasPass ? "(ustawione)" : "(wymagane)"}
            </p>
            <div class="flex gap-2">
              <input
                type="password"
                bind:value={syncPass}
                placeholder={syncHasPass ? "••••••••  zmień, jeśli chcesz" : "to samo na każdym urządzeniu"}
                class="min-w-0 flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm
                       outline-none placeholder:text-muted focus:border-accent"
                aria-label="Hasło synchronizacji"
              />
              <button
                class="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent
                       disabled:opacity-40"
                onclick={saveSyncPass}
                disabled={syncPass.trim().length === 0}
              >
                Zapisz
              </button>
            </div>
            <p class="mt-1 text-[11px] text-muted">
              Zapomnianego hasła nie da się odzyskać - paczkę trzeba wtedy wysłać na nowo.
            </p>
          </div>

          {#if syncHasPass && accounts.length > 0}
            <div class="h-px bg-line"></div>
            <div>
              <p class="mb-1 text-[11px] font-semibold text-muted">Konto-nośnik</p>
              <select
                bind:value={syncAccountId}
                class="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none"
                aria-label="Konto przechowujące paczkę"
              >
                {#each accounts as a (a.id)}
                  <option value={a.id}>{a.displayName || a.email}</option>
                {/each}
              </select>
              <div class="mt-2 flex flex-wrap gap-2">
                <button
                  class="rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent
                         disabled:opacity-40"
                  onclick={pushConfig}
                  disabled={syncBusy}
                >
                  Wyślij stąd
                </button>
                <button
                  class="rounded-lg border border-line px-3 py-1.5 text-sm font-semibold
                         hover:bg-rail disabled:opacity-40"
                  onclick={pullConfig}
                  disabled={syncBusy}
                >
                  Pobierz tutaj
                </button>
                <span class="flex-1"></span>
                <button
                  class="rounded-lg px-2.5 py-1.5 text-sm text-muted hover:bg-rail disabled:opacity-40"
                  onclick={copyCode}
                  disabled={syncBusy}
                  title="Kod do ręcznego przeniesienia, gdy nośnik w skrzynce nie wchodzi w grę"
                >
                  Kopiuj kod
                </button>
                <button
                  class="rounded-lg px-2.5 py-1.5 text-sm text-muted hover:bg-rail disabled:opacity-40"
                  onclick={pasteCode}
                  disabled={syncBusy}
                >
                  Wklej kod
                </button>
              </div>
              {#if syncMsg}
                <p class="mt-2 text-[11.5px] {syncErr ? 'text-danger' : 'text-accent'}">{syncMsg}</p>
              {/if}
            </div>
          {/if}
        </div>
      </div>
    </div>

    <footer class="flex items-center gap-2 border-t border-line px-5 py-3">
      <button
        class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-on-accent
               transition-opacity hover:opacity-90 disabled:opacity-40"
        disabled={saving}
        onclick={save}
      >
        Zapisz
      </button>
      <button
        class="rounded-lg px-3 py-2 text-sm font-semibold text-ink-soft hover:bg-paper"
        onclick={onclose}
      >
        Anuluj
      </button>
    </footer>
  </div>
</div>

<style>
  .sig-editor :global(img) {
    max-width: 100%;
    height: auto;
  }
  .sig-editor :global(table) {
    border-collapse: collapse;
  }
  .sig-editor :global(a) {
    color: var(--accent);
  }
</style>
