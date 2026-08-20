// Miękkie pojawianie się liter w edytorze.
//
// Znaku w polu `contenteditable` nie da się animować bezpośrednio - trzeba by
// owinąć go elementem, a to zaśmieciłoby HTML wysyłany na serwer i rozbiło
// cofanie zmian. Dlatego rysujemy duszka: świeżo wpisany znak gaśnie na czas
// animacji (CSS Custom Highlight API - `color: transparent`, zero ingerencji
// w treść), a nad nim leci jego kopia w warstwie `position: fixed`, która
// wpływa opacity, przesunięciem i rozmyciem. Po animacji duszek znika,
// a prawdziwy znak zapala się dokładnie w swoim miejscu.

/** Highlight dzielony przez wszystkie edytory - `::highlight(typing)` jest jeden na dokument. */
let shared: Highlight | null = null;

function highlight(): Highlight | null {
  if (!("highlights" in CSS) || typeof Highlight === "undefined") return null;
  if (!shared) {
    shared = new Highlight();
    CSS.highlights.set("typing", shared);
  }
  return shared;
}

/** Ile duszków może lecieć naraz - przy szybkim pisaniu wolimy odpuścić animację niż zamulić okno. */
const MAX_LIVE = 14;
const DURATION = 170;

export class TypeGlow {
  private layer: HTMLDivElement | null = null;
  private live = new Set<Range>();

  constructor(private editor: HTMLElement) {}

  /** Czy w ogóle jest czym animować (WebView bez Highlight API po prostu odpuszcza). */
  static get supported(): boolean {
    return (
      "highlights" in CSS &&
      typeof Highlight !== "undefined" &&
      !window.matchMedia("(prefers-reduced-motion: reduce)").matches
    );
  }

  /// Wołane po `input`. Animujemy tylko zwykłe pisanie: wklejanie, cofanie
  /// i składanie znaków (IME) zostawiamy w spokoju.
  onInput(e: InputEvent) {
    if (!TypeGlow.supported) return;
    if (e.isComposing || e.inputType !== "insertText") return;
    const data = e.data ?? "";
    if (!data || data.length > 2 || data === "\n") return;
    if (this.live.size >= MAX_LIVE) return;

    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || !sel.isCollapsed) return;
    const caret = sel.getRangeAt(0);
    const node = caret.startContainer;
    if (node.nodeType !== Node.TEXT_NODE || caret.startOffset < data.length) return;

    const range = document.createRange();
    range.setStart(node, caret.startOffset - data.length);
    range.setEnd(node, caret.startOffset);
    const rect = range.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) return;

    const hl = highlight();
    if (!hl) return;
    hl.add(range);
    this.live.add(range);

    const ghost = this.spawn(range, rect, data);
    const done = () => {
      ghost.remove();
      hl.delete(range);
      this.live.delete(range);
    };
    ghost
      .animate(
        [
          { opacity: 0, transform: "translateY(2px) scale(0.9)", filter: "blur(1.4px)" },
          { opacity: 1, transform: "none", filter: "blur(0)" },
        ],
        { duration: DURATION, easing: "cubic-bezier(0.2, 0.7, 0.3, 1)" },
      )
      .addEventListener("finish", done, { once: true });
    // Gdyby animacja nie doszła do końca (przerysowanie, zmiana karty), znak
    // nie może zostać przezroczysty na zawsze.
    setTimeout(done, DURATION + 400);
  }

  /// Kopia znaku w warstwie nad tekstem. Współrzędne z `getBoundingClientRect`
  /// są względem okna, więc warstwa jest `fixed` - nie obchodzi jej przewijanie
  /// ani zagnieżdżenie paneli, a duszek żyje i tak ułamek sekundy.
  private spawn(range: Range, rect: DOMRect, text: string): HTMLSpanElement {
    if (!this.layer) {
      this.layer = document.createElement("div");
      this.layer.className = "typeglow-layer";
      document.body.appendChild(this.layer);
    }
    const source = (range.startContainer.parentElement ?? this.editor) as HTMLElement;
    const style = window.getComputedStyle(source);
    const ghost = document.createElement("span");
    ghost.textContent = text;
    // Wysokość i interlinię bierzemy z samego pomiaru, nie ze stylu: duszek ma
    // trafić dokładnie w ten prostokąt, w którym przed chwilą zmierzyliśmy znak.
    ghost.style.cssText =
      `position:fixed;left:${rect.left}px;top:${rect.top}px;height:${rect.height}px;` +
      `font:${style.font};letter-spacing:${style.letterSpacing};color:${style.color};` +
      // Po skrócie `font`, bo on sam w sobie ustawia interlinię i skasowałby to wyżej.
      `line-height:${rect.height}px;white-space:pre;will-change:opacity,transform,filter;`;
    this.layer.appendChild(ghost);
    return ghost;
  }

  destroy() {
    const hl = highlight();
    for (const range of this.live) hl?.delete(range);
    this.live.clear();
    this.layer?.remove();
    this.layer = null;
  }
}
