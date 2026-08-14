// Typy lustrzane wobec DTO z src-tauri/src/models.rs (serde camelCase).

export interface Account {
  id: number;
  email: string;
  displayName: string;
  senderName: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  authKind: string;
}

export interface Folder {
  id: number;
  accountId: number;
  name: string;
  displayName: string;
  kind: string;
  unreadCount: number;
  /** Wszystkie wiadomości folderu (bez odłożonych na drzemkę). */
  totalCount: number;
}

export type Category = "primary" | "newsletters" | "notifications";

/** Nieprzeczytane w każdej zakładce Smart Inbox (liczniki przy przyciskach). */
export type CategoryCounts = Record<Category, number>;

/** Kolejność listy wiadomości; wartości rozpoznaje `order_clause` w commands.rs. */
export type SortKey = "date_desc" | "date_asc" | "from" | "subject" | "unread" | "attachments";

export interface MessageSummary {
  id: number;
  folderId: number;
  subject: string;
  fromName: string;
  fromAddr: string;
  date: number;
  preview: string;
  isRead: boolean;
  isFlagged: boolean;
  hasAttachments: boolean;
  category: Category;
  snoozedUntil: number | null;
  /** Klucz konwersacji; liczniki wypełnione tylko w pogrupowanej liście. */
  threadId: string;
  threadCount: number;
  threadUnread: number;
}

export interface MessageBody {
  id: number;
  toAddrs: string;
  html: string | null;
  text: string | null;
  /** Nagłówki potrzebne przy odpowiadaniu, żeby wątek się skleił. */
  messageId: string | null;
  inReplyTo: string | null;
}

export interface ServerConfig {
  host: string;
  port: number;
  starttls: boolean;
}

export interface DetectedConfig {
  imap: ServerConfig | null;
  pop3: ServerConfig | null;
  smtp: ServerConfig | null;
  login: string;
  source: "known" | "autoconfig" | "guess" | "none";
}

export interface NewAccount {
  email: string;
  displayName: string;
  senderName: string;
  login: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  authKind: string;
  password: string | null;
}

export interface OutlookSignature {
  name: string;
  html: string;
}

export interface Attachment {
  id: number;
  name: string;
  mime: string;
  size: number;
  path: string;
  isInline: boolean;
}

export interface Rule {
  id: number;
  accountId: number;
  fromAddr: string;
  folderId: number;
  folderName: string;
  enabled: boolean;
}

export interface Contact {
  addr: string;
  name: string;
}

/** Szkic wiadomości otwarty w karcie kompozycji (stan lokalny UI). */
export interface LocalDraft {
  localId: number;
  accountId: number;
  toAddrs: string;
  ccAddrs: string;
  bccAddrs: string;
  /** Nagłówki wątku przepisane z wiadomości, na którą odpowiadamy. */
  inReplyTo?: string | null;
  references?: string | null;
  subject: string;
  bodyHtml: string;
  /** Odpowiedź (nie nowa wiadomość) - wpływa tylko na nagłówek edytora. */
  isReply?: boolean;
  attachments: DraftAttachment[];
}

/** Załącznik dopięty do szkicu; treść w base64, bo tyle przenosi most Tauriego. */
export interface DraftAttachment {
  filename: string;
  mime: string;
  size: number;
  dataB64: string;
}

export interface ComposeDraft {
  accountId: number;
  toAddrs: string;
  ccAddrs: string;
  bccAddrs: string;
  /** Message-ID wiadomości, na którą odpowiadamy (bez nawiasów kątowych). */
  inReplyTo: string | null;
  references: string | null;
  subject: string;
  bodyText: string;
  bodyHtml: string | null;
  sendAt: number | null;
  attachments: DraftAttachment[];
}

/** Błąd pisowni zwrócony przez systemowy słownik. Pozycje w jednostkach UTF-16,
 *  czyli w indeksach łańcucha JavaScriptu - bez przeliczania. */
export interface SpellError {
  start: number;
  length: number;
  /** Poprawka narzucona przez autokorektę (np. „nie ma" zamiast „niema"). */
  replacement: string | null;
}

/** Grupa kandydatów do sprzątania - wszystko od jednego nadawcy. */
export interface CleanupGroup {
  fromAddr: string;
  fromName: string;
  count: number;
  unread: number;
  /** Nigdy nieotwarte - najmocniejsza przesłanka, że można skasować całość. */
  neverRead: number;
  oldest: number;
  newest: number;
  category: string;
  /** Kilka ostatnich tematów na podgląd. */
  samples: string[];
  /** Identyfikatory do skasowania; kasowanie bierze dokładnie tę listę. */
  ids: number[];
}

/** Wynik wgrania paczki synchronizacji. */
export interface SyncApplyResult {
  added: number;
  updated: number;
  /** Urządzenie, które zapisało paczkę. */
  device: string;
}
