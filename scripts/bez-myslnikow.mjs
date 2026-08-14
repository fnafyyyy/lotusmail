// Podmienia długie myślniki (— –) na zwykły dywiz w kodzie źródłowym.
import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join } from "node:path";

const roots = ["src", "src-tauri/src"];
const exts = [".svelte", ".ts", ".js", ".css", ".rs", ".html", ".md", ".sql"];
let zmienione = 0;

function walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      walk(path);
      continue;
    }
    if (!exts.some((e) => path.endsWith(e))) continue;
    const before = readFileSync(path, "utf8");
    const after = before.replace(/[—–]/g, "-");
    if (after !== before) {
      writeFileSync(path, after);
      zmienione++;
      console.log("poprawiono:", path);
    }
  }
}

for (const root of roots) walk(root);
console.log(`gotowe, plików zmienionych: ${zmienione}`);
