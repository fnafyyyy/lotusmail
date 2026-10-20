import { api } from "$lib/api";

/** Wzór maila - gotowy temat, adresat i treść do wstawienia w nowy szkic. */
export interface MailTemplate {
  id: string;
  name: string;
  toAddrs: string;
  subject: string;
  bodyHtml: string;
}

// Wzory leżą jako JSON w tabeli ustawień, tak jak stopki - jest ich kilka,
// więc osobna tabela i komendy w Rust nie byłyby warte zachodu.
const KEY = "templates";

export async function loadTemplates(): Promise<MailTemplate[]> {
  const raw = await api.getSetting(KEY);
  // Brak klucza = wzorów jeszcze nikt nie edytował; pusta lista zapisana
  // przez użytkownika to już świadomy wybór i startowych nie wskrzeszamy.
  if (raw == null) return defaultTemplates();
  try {
    const list = JSON.parse(raw);
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

export async function saveTemplates(list: MailTemplate[]): Promise<void> {
  await api.setSetting(KEY, JSON.stringify(list));
}

export function newTemplateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}

/** Wzór z danymi logowania do podanego systemu. */
export function loginTemplate(system = ""): MailTemplate {
  const subject = system ? `Konto ${system}` : "Dane do logowania";
  return {
    id: newTemplateId(),
    name: subject,
    toAddrs: "",
    subject,
    bodyHtml:
      "<div>Cześć!</div><div><br></div><div>Twoje dane logowania to:</div><div><br></div>" +
      "<div>Login: </div><div>Hasło: </div><div><br></div><div>Pozdrawiam,</div>",
  };
}

/** Wzory na start, zanim użytkownik zapisze własną listę. */
export function defaultTemplates(): MailTemplate[] {
  return ["INTRANET", "HAOS", "FIS"].map(loginTemplate);
}
