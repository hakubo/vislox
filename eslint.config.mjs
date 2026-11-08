import { readFileSync } from "node:fs";
import js from "@eslint/js";
import globals from "globals";
import { scriptGlobals } from "./tools/script-globals.mjs";

// Check the scripts actually loaded together, even when linting just one file.
const html = readFileSync(new URL("./web/index.html", import.meta.url), "utf8");
const scripts = [...html.matchAll(/<script\b[^>]*\bsrc=["']\/([^"']+\.js)["'][^>]*>/g)].map(([, path]) => ({
  file: `web/${path}`,
  source: readFileSync(new URL(`./web/${path}`, import.meta.url), "utf8"),
}));
const browser = { ...globals.browser };
for (const name of ["name", "status", "event", "top", "parent", "open", "close", "origin", "length", "find", "stop", "print"]) browser[name] = "off";

export default [{
  files: ["web/*.js"],
  ...js.configs.recommended,
  languageOptions: { ecmaVersion: 2022, sourceType: "script", globals: browser },
  rules: {
    ...js.configs.recommended.rules,
    "no-unused-vars": ["error", { args: "all", caughtErrors: "none" }],
    eqeqeq: ["error", "always", { null: "ignore" }],
  },
}, ...scriptGlobals(scripts)];
