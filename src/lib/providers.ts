// Znani dostawcy poczty: gotowe serwery i to, czego wymagają do logowania.
// Dzięki temu dodanie konta sprowadza się do adresu i hasła.

export interface Provider {
  id: string;
  name: string;
  /** Domeny adresów, po których rozpoznajemy dostawcę. */
  domains: string[];
  imap: { host: string; port: number };
  smtp: { host: string; port: number };
  /** Login serwera: pełny adres czy sama część przed małpą. */
  loginIsLocalPart?: boolean;
  /** Dostawca wymaga osobnego „hasła aplikacji" zamiast zwykłego. */
  appPassword?: { text: string; url: string };
  /** Dostawca zablokował logowanie hasłem - potrzebne OAuth2. */
  oauthOnly?: boolean;
  /** Kolor kafelka (identyfikacja wzrokowa). */
  hue: string;
  short: string;
}

export const PROVIDERS: Provider[] = [
  {
    id: "gmail",
    name: "Gmail",
    short: "G",
    hue: "#ea4335",
    domains: ["gmail.com", "googlemail.com"],
    imap: { host: "imap.gmail.com", port: 993 },
    smtp: { host: "smtp.gmail.com", port: 465 },
    appPassword: {
      text: "Google wymaga hasła aplikacji (konto musi mieć weryfikację dwuetapową). Wygeneruj je i wklej poniżej zamiast zwykłego hasła.",
      url: "https://myaccount.google.com/apppasswords",
    },
  },
  {
    id: "icloud",
    name: "iCloud Mail",
    short: "",
    hue: "#4c8bf5",
    domains: ["icloud.com", "me.com", "mac.com"],
    imap: { host: "imap.mail.me.com", port: 993 },
    smtp: { host: "smtp.mail.me.com", port: 587 },
    loginIsLocalPart: true,
    appPassword: {
      text: "Apple wymaga hasła dla aplikacji. Wygeneruj je w ustawieniach Apple ID (sekcja Bezpieczeństwo) i wklej poniżej.",
      url: "https://account.apple.com/account/manage",
    },
  },
  {
    id: "outlook",
    name: "Outlook / Microsoft",
    short: "O",
    hue: "#0078d4",
    domains: ["outlook.com", "hotmail.com", "live.com", "msn.com", "onmicrosoft.com"],
    imap: { host: "outlook.office365.com", port: 993 },
    smtp: { host: "smtp-mail.outlook.com", port: 587 },
    oauthOnly: true,
  },
  {
    id: "yahoo",
    name: "Yahoo Mail",
    short: "Y",
    hue: "#6001d2",
    domains: ["yahoo.com", "yahoo.pl", "ymail.com"],
    imap: { host: "imap.mail.yahoo.com", port: 993 },
    smtp: { host: "smtp.mail.yahoo.com", port: 465 },
    appPassword: {
      text: "Yahoo wymaga hasła aplikacji. Wygeneruj je w ustawieniach bezpieczeństwa konta i wklej poniżej.",
      url: "https://login.yahoo.com/account/security",
    },
  },
  {
    id: "wp",
    name: "WP Poczta",
    short: "WP",
    hue: "#e12228",
    domains: ["wp.pl"],
    imap: { host: "imap.wp.pl", port: 993 },
    smtp: { host: "smtp.wp.pl", port: 465 },
    loginIsLocalPart: true,
  },
  {
    id: "o2",
    name: "o2 / tlen",
    short: "o2",
    hue: "#0aa1dd",
    domains: ["o2.pl", "tlen.pl", "go2.pl"],
    imap: { host: "poczta.o2.pl", port: 993 },
    smtp: { host: "poczta.o2.pl", port: 465 },
    loginIsLocalPart: true,
  },
  {
    id: "interia",
    name: "Interia",
    short: "IN",
    hue: "#00a650",
    domains: ["interia.pl", "interia.eu", "poczta.fm"],
    imap: { host: "poczta.interia.pl", port: 993 },
    smtp: { host: "poczta.interia.pl", port: 465 },
  },
  {
    id: "onet",
    name: "Onet Poczta",
    short: "ON",
    hue: "#f36f21",
    domains: ["onet.pl", "op.pl", "vp.pl", "onet.eu", "poczta.onet.pl"],
    imap: { host: "imap.poczta.onet.pl", port: 993 },
    smtp: { host: "smtp.poczta.onet.pl", port: 465 },
  },
];

/** Dostawca rozpoznany po domenie adresu. */
export function providerFor(email: string): Provider | null {
  const domain = email.split("@")[1]?.toLowerCase().trim();
  if (!domain) return null;
  return (
    PROVIDERS.find((p) => p.domains.some((d) => domain === d || domain.endsWith(`.${d}`))) ?? null
  );
}
