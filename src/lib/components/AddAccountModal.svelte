<script lang="ts">
  import { fade, scale, slide } from "svelte/transition";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api } from "$lib/api";
  import type { DetectedConfig } from "$lib/types";
  import { PROVIDERS, providerFor, type Provider } from "$lib/providers";
  import Icon from "./Icon.svelte";

  let {
    onclose,
    onadded,
    onimported,
    initialStep = "provider",
  }: {
    onclose: () => void;
    onadded: (accountId: number) => void;
    /** Konta wgrane z kodu przeniesienia. */
    onimported: (added: number, updated: number, device: string) => void;
    /** Ekran, od którego zaczynamy - pozwala wejść wprost w przeniesienie kont. */
    initialStep?: "provider" | "transfer";
  } = $props();

  // Krok 1: wybór dostawcy. Krok 2: dane konta.
  // svelte-ignore state_referenced_locally
  let step = $state<"provider" | "details" | "transfer">(initialStep);
  let provider = $state<Provider | null>(null);
  let manual = $state(false);

  let email = $state("");
  let password = $state("");
  let senderName = $state("");
  let displayName = $state("");
  let login = $state("");
  let imapHost = $state("");
  let imapPort = $state(993);
  let smtpHost = $state("");
  let smtpPort = $state(465);

  let detecting = $state(false);
  let connecting = $state(false);
  let advanced = $state(false);
  let detected = $state<DetectedConfig | null>(null);
  let error = $state("");
  let info = $state("");

  let emailValid = $derived(/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()));
  let ready = $derived(emailValid && password.length > 0 && imapHost.trim().length > 0);

  function pick(p: Provider | null) {
    provider = p;
    manual = p === null;
    error = "";
    info = "";
    if (p) {
      imapHost = p.imap.host;
      imapPort = p.imap.port;
      smtpHost = p.smtp.host;
      smtpPort = p.smtp.port;
      // Etykiety nie wypełniamy nazwą dostawcy - przy dwóch kontach u tego
      // samego dostawcy dawałoby to dwa identyczne wpisy w panelu bocznym.
      advanced = false;
    } else {
      imapHost = "";
      smtpHost = "";
      advanced = true;
    }
    step = "details";
  }

  /// Adres z innej domeny niż wybrany dostawca - poprawiamy wybór w locie.
  function onEmailBlur() {
    if (!emailValid) return;
    login = provider?.loginIsLocalPart ? email.split("@")[0] : email.trim();
    const guessed = providerFor(email);
    if (guessed && guessed.id !== provider?.id) {
      provider = guessed;
      imapHost = guessed.imap.host;
      imapPort = guessed.imap.port;
      smtpHost = guessed.smtp.host;
      smtpPort = guessed.smtp.port;
      info = `Rozpoznano dostawcę: ${guessed.name}.`;
      return;
    }
    if (manual && !imapHost) detect();
  }

  async function detect() {
    detecting = true;
    error = "";
    try {
      const cfg = await api.detectSettings(email.trim());
      detected = cfg;
      login = cfg.login || email.trim();
      if (cfg.imap) {
        imapHost = cfg.imap.host;
        imapPort = cfg.imap.port;
        info = "Ustawienia wykryte automatycznie.";
      }
      if (cfg.smtp) {
        smtpHost = cfg.smtp.host;
        smtpPort = cfg.smtp.port;
      }
      if (!cfg.imap) {
        error =
          "Nie udało się wykryć ustawień. Wpisz serwery ręcznie - znajdziesz je w pomocy swojego dostawcy poczty.";
        advanced = true;
      }
    } catch (e) {
      error = String(e);
    } finally {
      detecting = false;
    }
  }

  async function connect() {
    error = "";
    connecting = true;
    try {
      const user = login.trim() || email.trim();
      await api.testLogin(imapHost.trim(), imapPort, user, password);
      const account = await api.addAccount({
        email: email.trim(),
        displayName: displayName.trim(),
        senderName: senderName.trim(),
        login: user,
        imapHost: imapHost.trim(),
        imapPort,
        smtpHost: smtpHost.trim(),
        smtpPort,
        authKind: "password",
        password,
      });
      onadded(account.id);
    } catch (e) {
      error = String(e);
      connecting = false;
    }
  }

  // Przeniesienie kont z innego urządzenia. Musi być dostępne właśnie tutaj:
  // na świeżej instalacji nie ma jeszcze żadnego konta, więc Ustawienia są
  // pustym ekranem, a to jedyny moment, w którym ten kod jest potrzebny.
  let transferPass = $state("");
  let transferCode = $state("");
  let transferBusy = $state(false);

  async function pasteFromClipboard() {
    try {
      transferCode = await navigator.clipboard.readText();
    } catch (e) {
      error = `Nie udało się odczytać schowka: ${e}`;
    }
  }

  async function runTransfer() {
    transferBusy = true;
    error = "";
    try {
      // Hasło musi trafić do pęku kluczy przed importem - rdzeń bierze je
      // stamtąd, celowo nie przyjmuje go jako argumentu.
      await api.syncSetPassphrase(transferPass);
      const r = await api.syncImport(transferCode);
      onimported(r.added, r.updated, r.device);
    } catch (e) {
      error = String(e);
    } finally {
      transferBusy = false;
    }
  }
</script>

<div
  class="fixed inset-0 z-30 grid place-items-center bg-ink/30 p-3"
  role="presentation"
  transition:fade={{ duration: 140 }}
  onclick={(e) => e.target === e.currentTarget && !connecting && onclose()}
>
  <div
    class="panel flex max-h-[88vh] w-full max-w-[560px] flex-col"
    transition:scale={{ start: 0.96, duration: 170 }}
  >
    <header class="flex items-center gap-2 border-b border-line px-5 py-3">
      {#if step !== "provider"}
        <button
          class="grid size-7 place-items-center rounded-full text-muted hover:bg-rail hover:text-ink"
          onclick={() => (step = "provider")}
          aria-label="Wróć do wyboru dostawcy"
          title="Wróć"
        >
          <Icon name="reply" size={14} />
        </button>
      {/if}
      <h2 class="tight font-display text-[15px] font-semibold">
        {step === "provider"
          ? "Dodaj konto pocztowe"
          : step === "transfer"
            ? "Przenieś z innego urządzenia"
            : (provider?.name ?? "Konto IMAP")}
      </h2>
      <span class="flex-1"></span>
      <button
        class="grid size-7 place-items-center rounded-full text-muted hover:bg-rail hover:text-ink"
        onclick={onclose}
        aria-label="Zamknij"
      >
        <Icon name="x" size={15} />
      </button>
    </header>

    {#if step === "provider"}
      <div class="flex-1 overflow-y-auto px-5 py-4" in:fade={{ duration: 140 }}>
        <p class="mb-3 text-[13px] text-muted">Wybierz dostawcę - ustawienia serwerów wypełnią się same.</p>
        <div class="grid grid-cols-3 gap-2">
          {#each PROVIDERS as p (p.id)}
            <button
              class="flex flex-col items-center gap-2 rounded-xl bg-rail px-3 py-4 transition-colors
                     hover:bg-accent-soft"
              onclick={() => pick(p)}
            >
              <span
                class="grid size-10 place-items-center rounded-xl text-[15px] font-bold text-white"
                style="background:{p.hue}"
              >
                {p.short}
              </span>
              <span class="text-center text-[12.5px] leading-tight font-semibold">{p.name}</span>
            </button>
          {/each}
          <button
            class="flex flex-col items-center gap-2 rounded-xl bg-rail px-3 py-4 transition-colors
                   hover:bg-accent-soft"
            onclick={() => pick(null)}
          >
            <span class="grid size-10 place-items-center rounded-xl bg-surface text-muted">
              <Icon name="settings" size={18} />
            </span>
            <span class="text-center text-[12.5px] leading-tight font-semibold">Inne konto IMAP</span>
          </button>
        </div>

        <button
          class="mt-3 flex w-full items-center justify-center gap-2 rounded-xl border border-line
                 py-2.5 text-[13px] font-semibold text-ink-soft transition-colors hover:bg-rail"
          onclick={() => {
            error = "";
            step = "transfer";
          }}
        >
          <Icon name="download" size={15} class="text-muted" />
          Mam już lotusMaila na innym urządzeniu
        </button>
      </div>
    {:else if step === "transfer"}
      <div class="flex-1 space-y-3 overflow-y-auto px-5 py-4" in:fade={{ duration: 140 }}>
        <p class="text-[13px] leading-relaxed text-muted">
          Na urządzeniu, które ma już Twoje konta, wejdź w
          <span class="font-semibold text-ink-soft">Ustawienia → Synchronizacja kont</span>
          i kliknij <span class="font-semibold text-ink-soft">Kopiuj kod</span>. Potem wklej go tutaj.
        </p>

        <div>
          <p class="mb-1 text-[11px] font-semibold text-muted">Hasło synchronizacji</p>
          <input
            type="password"
            bind:value={transferPass}
            placeholder="to samo, co na tamtym urządzeniu"
            class="w-full rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none
                   placeholder:text-muted focus:border-accent"
          />
        </div>

        <div>
          <div class="mb-1 flex items-baseline gap-2">
            <p class="text-[11px] font-semibold text-muted">Kod przeniesienia</p>
            <span class="flex-1"></span>
            <button class="text-[11.5px] font-semibold text-accent hover:underline" onclick={pasteFromClipboard}>
              Wklej ze schowka
            </button>
          </div>
          <textarea
            bind:value={transferCode}
            rows="5"
            placeholder="LOTUSMAIL-SYNC-1&#10;…"
            class="w-full resize-none rounded-lg border border-line bg-paper px-3 py-2 font-mono
                   text-[11px] leading-snug outline-none placeholder:text-muted focus:border-accent"
          ></textarea>
        </div>

        {#if error}
          <p class="text-[12.5px] text-danger">{error}</p>
        {/if}

        <button
          class="w-full rounded-lg bg-accent py-2 text-sm font-semibold text-on-accent disabled:opacity-40"
          onclick={runTransfer}
          disabled={transferBusy || !transferPass.trim() || !transferCode.trim()}
        >
          {transferBusy ? "Wgrywam konta…" : "Wgraj konta"}
        </button>
      </div>

    {:else}
      <div class="flex-1 space-y-3 overflow-y-auto px-5 py-4" in:fade={{ duration: 140 }}>
        {#if provider?.oauthOnly}
          <div class="rounded-xl bg-danger/10 px-3.5 py-3 text-[12.5px] leading-relaxed text-danger">
            <p class="font-semibold">Microsoft wyłączył logowanie hasłem do IMAP.</p>
            <p class="mt-1">
              Konta Outlook.com i Microsoft 365 wymagają logowania OAuth2, którego jeszcze nie ma
              w lotusMailu (jest w planach). Jeśli Twoja firma nadal dopuszcza logowanie hasłem,
              możesz spróbować poniżej.
            </p>
          </div>
        {:else if provider?.appPassword}
          <div class="rounded-xl bg-accent-soft px-3.5 py-3 text-[12.5px] leading-relaxed text-ink-soft">
            <p>{provider.appPassword.text}</p>
            <button
              class="mt-2 flex items-center gap-1.5 font-semibold text-accent hover:underline"
              onclick={() => openUrl(provider!.appPassword!.url)}
            >
              <Icon name="send" size={12} />
              Otwórz stronę haseł aplikacji
            </button>
          </div>
        {/if}

        <label class="block">
          <span class="mb-1 block text-xs font-semibold text-muted">Adres e-mail</span>
          <input
            bind:value={email}
            type="email"
            placeholder="jan@przyklad.pl"
            class="w-full rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none focus:border-accent"
            onblur={onEmailBlur}
          />
        </label>

        <label class="block">
          <span class="mb-1 block text-xs font-semibold text-muted">
            {provider?.appPassword ? "Hasło aplikacji" : "Hasło"}
          </span>
          <input
            bind:value={password}
            type="password"
            class="w-full rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none focus:border-accent"
          />
          <span class="mt-1 block text-[11px] text-muted">
            Hasło trafia do Menedżera poświadczeń Windows, nie do bazy aplikacji.
          </span>
        </label>

        <div class="grid grid-cols-2 gap-3">
          <label class="block">
            <span class="mb-1 block text-xs font-semibold text-muted">Imię i nazwisko (nadawca)</span>
            <input
              bind:value={senderName}
              placeholder="np. Jan Kowalski"
              class="w-full rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none focus:border-accent"
            />
          </label>
          <label class="block">
            <span class="mb-1 block text-xs font-semibold text-muted">Etykieta konta</span>
            <input
              bind:value={displayName}
              placeholder={email.trim() || "np. Praca"}
              class="w-full rounded-lg border border-line bg-paper px-3 py-2 text-sm outline-none focus:border-accent"
            />
            <span class="mt-1 block text-[11px] text-muted">Puste = adres e-mail.</span>
          </label>
        </div>

        <div class="flex items-center gap-2">
          <button
            class="flex items-center gap-1.5 text-[12px] font-semibold text-muted hover:text-ink"
            onclick={() => (advanced = !advanced)}
          >
            <Icon
              name="chevronDown"
              size={12}
              class="transition-transform duration-200 {advanced ? '' : '-rotate-90'}"
            />
            Ustawienia serwerów
          </button>
          {#if manual}
            <button
              class="flex items-center gap-1.5 rounded-full bg-rail px-2.5 py-1 text-[11.5px]
                     font-semibold text-ink-soft hover:text-ink disabled:opacity-40"
              disabled={!emailValid || detecting}
              onclick={detect}
            >
              <Icon name="search" size={12} />
              {detecting ? "Wykrywanie…" : "Wykryj automatycznie"}
            </button>
          {/if}
        </div>

        {#if advanced}
          <div class="grid grid-cols-[1fr_5rem] gap-2 rounded-xl bg-rail/60 p-3" transition:slide={{ duration: 160 }}>
            <label class="block">
              <span class="mb-1 block text-[11px] text-muted">Serwer IMAP (odbiór)</span>
              <input
                bind:value={imapHost}
                class="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none focus:border-accent"
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] text-muted">Port</span>
              <input
                bind:value={imapPort}
                type="number"
                class="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none focus:border-accent"
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] text-muted">Serwer SMTP (wysyłka)</span>
              <input
                bind:value={smtpHost}
                class="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none focus:border-accent"
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[11px] text-muted">Port</span>
              <input
                bind:value={smtpPort}
                type="number"
                class="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none focus:border-accent"
              />
            </label>
            <label class="col-span-2 block">
              <span class="mb-1 block text-[11px] text-muted">Login (jeśli inny niż adres)</span>
              <input
                bind:value={login}
                class="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none focus:border-accent"
              />
            </label>
          </div>
        {/if}

        {#if info}
          <p class="rounded-lg bg-accent-soft px-3 py-2 text-[11.5px] text-accent">{info}</p>
        {/if}
        {#if error}
          <p class="rounded-lg bg-danger/10 px-3 py-2 text-xs leading-relaxed text-danger">{error}</p>
        {/if}
      </div>

      <footer class="flex items-center gap-2 border-t border-line px-5 py-3">
        <button
          class="flex items-center gap-2 rounded-full bg-accent px-4 py-2 text-sm font-semibold
                 text-on-accent transition-opacity hover:opacity-90 disabled:opacity-40"
          disabled={!ready || connecting}
          onclick={connect}
        >
          {#if connecting}
            <Icon name="refresh" size={14} class="animate-spin" />
            Łączenie…
          {:else}
            Połącz i dodaj konto
          {/if}
        </button>
        <button
          class="rounded-full px-3 py-2 text-sm font-semibold text-ink-soft hover:bg-rail"
          onclick={onclose}
          disabled={connecting}
        >
          Anuluj
        </button>
      </footer>
    {/if}
  </div>
</div>
