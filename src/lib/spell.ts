// Sprawdzanie pisowni w edytorze wiadomości.
//
// Podkreślenia rysuje CSS Custom Highlight API, więc treść maila zostaje
// nietknięta - żadnych dodatkowych <span> w HTML-u, który poleci na serwer,
// i żadnego psucia cofania zmian. Słownik jest systemowy (patrz spell.rs).

import { api } from "./api";

const HIGHLIGHT = "spell";

/** Fragment tekstu edytora wraz z pozycją w sklejonym łańcuchu. */
interface Piece {
  node: Text;
  start: number;
  end: number;
}

export interface SpellHit {
  word: string;
  start: number;
  length: number;
  range: Range;
}

const supported =
  typeof CSS !== "undefined" &&
  "highlights" in CSS &&
  typeof (window as unknown as { Highlight?: unknown }).Highlight === "function";

export class SpellChecker {
  private hits: SpellHit[] = [];
  private ignored = new Set<string>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private generation = 0;

  constructor(private el: HTMLElement) {}

  /** Sprawdzenie po przerwie w pisaniu - w trakcie stukania nie ma sensu. */
  schedule(delay = 500) {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.run(), delay);
  }

  async run() {
    if (!supported) return;
    const { text, pieces } = this.collect();
    if (!text.trim()) {
      this.clear();
      return;
    }
    const mine = ++this.generation;
    let errors;
    try {
      errors = await api.spellCheck(text);
    } catch {
      return;
    }
    // W międzyczasie mogło pójść nowsze sprawdzenie - to jest już nieaktualne.
    if (mine !== this.generation) return;

    this.hits = [];
    for (const e of errors) {
      const word = text.slice(e.start, e.start + e.length);
      if (this.ignored.has(word.toLowerCase())) continue;
      const range = this.toRange(pieces, e.start, e.start + e.length);
      if (range) this.hits.push({ word, start: e.start, length: e.length, range });
    }
    this.paint();
  }

  /** Błąd pod wskazanym punktem ekranu - do menu podpowiedzi. */
  hitAt(x: number, y: number): SpellHit | null {
    for (const hit of this.hits) {
      for (const rect of hit.range.getClientRects()) {
        if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) {
          return hit;
        }
      }
    }
    return null;
  }

  /** Podmienia słowo tak, żeby zadziałało cofanie (Ctrl+Z). */
  replace(hit: SpellHit, word: string) {
    const sel = window.getSelection();
    if (!sel) return;
    sel.removeAllRanges();
    sel.addRange(hit.range);
    document.execCommand("insertText", false, word);
  }

  /** Słowo pomijane do końca pisania tej wiadomości (bez zapisu w słowniku). */
  ignore(word: string) {
    this.ignored.add(word.toLowerCase());
    this.hits = this.hits.filter((h) => h.word.toLowerCase() !== word.toLowerCase());
    this.paint();
  }

  clear() {
    this.hits = [];
    this.paint();
  }

  destroy() {
    clearTimeout(this.timer);
    this.generation++;
    this.clear();
  }

  private paint() {
    if (!supported) return;
    const highlights = (CSS as unknown as { highlights: Map<string, unknown> }).highlights;
    if (this.hits.length === 0) {
      highlights.delete(HIGHLIGHT);
      return;
    }
    const Ctor = (window as unknown as { Highlight: new (...r: Range[]) => unknown }).Highlight;
    highlights.set(HIGHLIGHT, new Ctor(...this.hits.map((h) => h.range)));
  }

  /** Skleja tekst edytora, pamiętając, z którego węzła pochodzi każdy fragment. */
  private collect(): { text: string; pieces: Piece[] } {
    const walker = document.createTreeWalker(this.el, NodeFilter.SHOW_TEXT);
    const pieces: Piece[] = [];
    let text = "";
    let previousBlock: Element | null = null;
    let node = walker.nextNode() as Text | null;
    while (node) {
      // Cytowanej historii nie sprawdzamy - to nie jest tekst autora.
      if (node.parentElement?.closest("blockquote")) {
        node = walker.nextNode() as Text | null;
        continue;
      }
      const block = node.parentElement?.closest("div,p,li,blockquote,h1,h2,h3,td") ?? null;
      if (previousBlock && block !== previousBlock) text += "\n";
      previousBlock = block;
      const start = text.length;
      text += node.data;
      pieces.push({ node, start, end: text.length });
      node = walker.nextNode() as Text | null;
    }
    return { text, pieces };
  }

  private toRange(pieces: Piece[], start: number, end: number): Range | null {
    const from = pieces.find((p) => start >= p.start && start < p.end);
    const to = pieces.find((p) => end > p.start && end <= p.end);
    if (!from || !to) return null;
    const range = document.createRange();
    range.setStart(from.node, start - from.start);
    range.setEnd(to.node, end - to.start);
    return range;
  }
}

/** Czy da się w ogóle sprawdzać: przeglądarka rysuje podkreślenia, a system ma słownik. */
export async function spellReady(): Promise<boolean> {
  if (!supported) return false;
  try {
    return await api.spellAvailable();
  } catch {
    return false;
  }
}
