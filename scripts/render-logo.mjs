// Renderuje znak LotusMail (static/logo.svg) do PNG 1024×1024 — źródła dla
// generatora ikon aplikacji: `npx tauri icon static/logo-1024.png`.
import { readFileSync, writeFileSync } from "node:fs";
import { Resvg } from "@resvg/resvg-js";

const src = new URL("../static/logo.svg", import.meta.url);
const out = new URL("../static/logo-1024.png", import.meta.url);

const resvg = new Resvg(readFileSync(src, "utf8"), {
  fitTo: { mode: "width", value: 1024 },
});
const png = resvg.render().asPng();
writeFileSync(out, png);
console.log(`zapisano logo-1024.png (${png.length} bajtów)`);
