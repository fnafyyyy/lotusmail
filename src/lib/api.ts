// Cienka, typowana warstwa nad komendami Rust (src-tauri/src/commands.rs).

import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  Attachment,
  Category,
  CategoryCounts,
  CleanupGroup,
  ComposeDraft,
  Contact,
  DetectedConfig,
  DraftAttachment,
  Folder,
  MessageBody,
  MessageSummary,
  NewAccount,
  OutlookSignature,
  Rule,
  SortKey,
  StoredDraft,
  SyncApplyResult,
  SpellError,
} from "./types";

export const api = {
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (newAccount: NewAccount) => invoke<Account>("add_account", { newAccount }),
  removeAccount: (id: number) => invoke<void>("remove_account", { id }),
  /** Otwiera okno logowania Microsoftu i zapamiętuje token odświeżania. */
  oauthSignIn: (email: string) => invoke<void>("oauth_sign_in", { email }),
  oauthIsConfigured: () => invoke<boolean>("oauth_is_configured"),
  detectSettings: (email: string) => invoke<DetectedConfig>("detect_settings", { email }),
  testLogin: (host: string, port: number, login: string, password: string) =>
    invoke<void>("test_login", { host, port, login, password }),
  listFolders: (accountId?: number) =>
    invoke<Folder[]>("list_folders", { accountId: accountId ?? null }),
  listMessages: (opts: {
    folderId?: number;
    category?: Category;
    sort?: SortKey;
    offset?: number;
    limit?: number;
  }) =>
    invoke<MessageSummary[]>("list_messages", {
      folderId: opts.folderId ?? null,
      category: opts.category ?? null,
      sort: opts.sort ?? null,
      offset: opts.offset ?? 0,
      limit: opts.limit ?? 100,
    }),
  categoryCounts: (folderId?: number) =>
    invoke<CategoryCounts>("category_counts", { folderId: folderId ?? null }),
  listThread: (threadId: string) => invoke<MessageSummary[]>("list_thread", { threadId }),
  getMessage: (id: number) => invoke<MessageSummary>("get_message", { id }),
  listSnoozed: () => invoke<MessageSummary[]>("list_snoozed"),
  getMessageBody: (id: number) => invoke<MessageBody>("get_message_body", { id }),
  setRead: (id: number, read: boolean) => invoke<void>("set_read", { id, read }),
  setThreadRead: (threadId: string, read: boolean) =>
    invoke<number>("set_thread_read", { threadId, read }),
  markFolderRead: (folderId?: number) =>
    invoke<number>("mark_folder_read", { folderId: folderId ?? null }),
  setFlagged: (id: number, flagged: boolean) =>
    invoke<void>("set_flagged", { id, flagged }),
  snoozeMessage: (id: number, until: number | null) =>
    invoke<void>("snooze_message", { id, until }),
  deleteMessage: (id: number) => invoke<void>("delete_message", { id }),
  searchMessages: (query: string) =>
    invoke<MessageSummary[]>("search_messages", { query }),
  searchServer: (query: string) => invoke<number>("search_server", { query }),
  queueSend: (draft: ComposeDraft) => invoke<number>("queue_send", { draft }),
  /** Zapisuje kopię roboczą (bez `id` zakłada nową) i oddaje jej identyfikator. */
  saveDraft: (draft: {
    id: number | null;
    accountId: number;
    toAddrs: string;
    ccAddrs: string;
    bccAddrs: string;
    inReplyTo: string | null;
    references: string | null;
    subject: string;
    bodyHtml: string;
    isReply: boolean;
    attachments: DraftAttachment[];
  }) => invoke<number>("save_draft", { draft }),
  /** Lista kopii roboczych; załączniki bez treści (sama nazwa i rozmiar). */
  listDrafts: () => invoke<StoredDraft[]>("list_drafts"),
  /** Jeden szkic w całości - do wczytania z powrotem do edytora. */
  getDraft: (id: number) => invoke<StoredDraft>("get_draft", { id }),
  deleteDraft: (id: number) => invoke<void>("delete_draft", { id }),
  cleanupScan: (opts: {
    accountId?: number | null;
    minCount?: number;
    olderThanDays?: number | null;
    onlyUnread?: boolean;
  }) =>
    invoke<CleanupGroup[]>("cleanup_scan", {
      accountId: opts.accountId ?? null,
      minCount: opts.minCount ?? 3,
      olderThanDays: opts.olderThanDays ?? null,
      onlyUnread: opts.onlyUnread ?? false,
    }),
  cleanupDelete: (ids: number[]) => invoke<number>("cleanup_delete", { ids }),
  emptyTrash: (folderId: number) => invoke<number>("empty_trash", { folderId }),
  reorderFolders: (folderIds: number[]) => invoke<void>("reorder_folders", { folderIds }),
  // Synchronizacja konfiguracji między urządzeniami. Hasło zostaje w pęku
  // kluczy rdzenia - interfejs nigdy go nie odczytuje, tylko ustawia.
  syncSetPassphrase: (passphrase: string) =>
    invoke<void>("sync_set_passphrase", { passphrase }),
  syncHasPassphrase: () => invoke<boolean>("sync_has_passphrase"),
  syncExport: () => invoke<string>("sync_export"),
  syncImport: (blob: string) => invoke<SyncApplyResult>("sync_import", { blob }),
  syncPush: (accountId: number) => invoke<number>("sync_push", { accountId }),
  syncPull: (accountId: number) => invoke<SyncApplyResult>("sync_pull", { accountId }),
  getSetting: (key: string) => invoke<string | null>("get_setting", { key }),
  listOutlookSignatures: () => invoke<OutlookSignature[]>("list_outlook_signatures"),
  searchContacts: (query: string) => invoke<Contact[]>("search_contacts", { query }),
  setSenderName: (accountId: number, name: string) =>
    invoke<void>("set_sender_name", { accountId, name }),
  setAccountLabel: (accountId: number, label: string) =>
    invoke<void>("set_account_label", { accountId, label }),
  createFolder: (accountId: number, name: string) =>
    invoke<number>("create_folder", { accountId, name }),
  deleteFolder: (folderId: number) => invoke<void>("delete_folder", { folderId }),
  getAttachments: (messageId: number) =>
    invoke<Attachment[]>("get_attachments", { messageId }),
  saveAttachment: (attachmentId: number, target: string) =>
    invoke<void>("save_attachment", { attachmentId, target }),
  readAttachment: (path: string) => invoke<DraftAttachment>("read_attachment", { path }),
  listRules: () => invoke<Rule[]>("list_rules"),
  addRule: (accountId: number, fromAddr: string, folderId: number) =>
    invoke<void>("add_rule", { accountId, fromAddr, folderId }),
  deleteRule: (id: number) => invoke<void>("delete_rule", { id }),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  syncNow: (accountId: number) => invoke<void>("sync_now", { accountId }),
  checkMail: () => invoke<void>("check_mail"),
  spellAvailable: () => invoke<boolean>("spell_available"),
  spellCheck: (text: string) => invoke<SpellError[]>("spell_check", { text }),
  spellSuggest: (word: string) => invoke<string[]>("spell_suggest", { word }),
  spellAdd: (word: string) => invoke<void>("spell_add", { word }),
  seedDemoData: () => invoke<boolean>("seed_demo_data"),
};

/** Czy wiersz listy ma wyglądać na nieprzeczytany.
 *
 *  Wiersz reprezentuje całą konwersację, więc liczy się stan wątku, a nie samej
 *  najnowszej wiadomości - inaczej mail nieprzeczytany schowany głębiej w wątku
 *  jest niewidoczny, choć wchodzi do licznika. `threadUnread` bywa zerowe tam,
 *  gdzie nie grupujemy (wyniki wyszukiwania), stąd drugi warunek. */
export function isUnread(m: MessageSummary): boolean {
  return !m.isRead || m.threadUnread > 0;
}

const dniTygodnia = ["nd.", "pon.", "wt.", "śr.", "czw.", "pt.", "sob."];

/** Format daty jak w liście wiadomości: dziś → godzina, ten tydzień → dzień, dalej → data. */
export function fmtDate(epoch: number): string {
  const d = new Date(epoch * 1000);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) {
    return d.toLocaleTimeString("pl-PL", { hour: "2-digit", minute: "2-digit" });
  }
  const daysAgo = (now.getTime() - d.getTime()) / 86_400_000;
  if (daysAgo < 6) return dniTygodnia[d.getDay()];
  // Rok dokładamy poza bieżącym: przy doładowywaniu starszej poczty samo
  // „12 sty" nie mówi, czy chodzi o ten styczeń, czy o któryś sprzed lat.
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString("pl-PL", {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

/** Data z rokiem - używana w wynikach wyszukiwania, gdzie maile bywają stare. */
export function fmtDateYear(epoch: number): string {
  return new Date(epoch * 1000).toLocaleDateString("pl-PL", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/** Pełny format daty do nagłówka wiadomości. */
export function fmtDateFull(epoch: number): string {
  return new Date(epoch * 1000).toLocaleString("pl-PL", {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Inicjały do awatara nadawcy. */
export function initials(name: string, addr: string): string {
  const source = name.trim() || addr;
  const parts = source.split(/[\s.@_-]+/).filter(Boolean);
  return ((parts[0]?.[0] ?? "?") + (parts[1]?.[0] ?? "")).toUpperCase();
}
