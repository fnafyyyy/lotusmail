// Auto-aktualizacja programu.
//
// Aplikacja pyta GitHub Releases o plik `latest.json`, porównuje wersję
// z własną i - za zgodą użytkownika - pobiera podpisany instalator, po czym
// uruchamia się ponownie. Podpis weryfikuje klucz publiczny wpisany
// w `tauri.conf.json`; instalator bez pasującego podpisu jest odrzucany.

import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface PendingUpdate {
  version: string;
  /** Opis wydania z GitHuba - bywa pusty. */
  notes: string;
  /** Pobiera, instaluje i restartuje program. `onProgress` dostaje procenty. */
  install: (onProgress?: (percent: number) => void) => Promise<void>;
}

/// Zwraca oczekującą aktualizację albo `null`, gdy program jest aktualny.
/// Brak sieci nie jest tu błędem do pokazania - klient poczty ma działać
/// offline, więc nieudane sprawdzenie przechodzi po cichu do logu.
export async function checkForUpdate(): Promise<PendingUpdate | null> {
  let update;
  try {
    update = await check();
  } catch (e) {
    console.warn("[update] nie udało się sprawdzić aktualizacji:", e);
    return null;
  }
  if (!update) return null;

  return {
    version: update.version,
    notes: update.body ?? "",
    install: async (onProgress) => {
      let total = 0;
      let got = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") {
          total = e.data.contentLength ?? 0;
        } else if (e.event === "Progress") {
          got += e.data.chunkLength;
          if (total > 0) onProgress?.(Math.round((got / total) * 100));
        }
      });
      await relaunch();
    },
  };
}
