import { readFileSync } from "node:fs";

// The config and its regression tests inspect exactly the scripts loaded by the page.
export function loadScripts() {
  const root = new URL("../", import.meta.url);
  const html = readFileSync(new URL("web/index.html", root), "utf8");
  return [...html.matchAll(/<script\b[^>]*\bsrc=["']\/([^"']+\.js)["'][^>]*>/g)].map(([, path]) => ({
    file: `web/${path}`,
    source: readFileSync(new URL(`web/${path}`, root), "utf8"),
  }));
}
