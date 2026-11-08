import js from "@eslint/js";
import globals from "globals";
import { readyListeners } from "./tools/ready-listeners.mjs";
import { loadScripts } from "./tools/web-scripts.mjs";
import { scriptGlobals } from "./tools/script-globals.mjs";

// Check the scripts actually loaded together, even when linting just one file.
const scripts = loadScripts();
const browser = { ...globals.browser };
for (const name of ["name", "status", "event", "top", "parent", "open", "close", "origin", "length", "find", "stop", "print"]) browser[name] = "off";

export default [{
  files: ["web/*.js"],
  ...js.configs.recommended,
  languageOptions: { ecmaVersion: 2022, sourceType: "script", globals: browser },
  plugins: { startup: { rules: { "ready-listeners": readyListeners } } },
  rules: {
    ...js.configs.recommended.rules,
    "startup/ready-listeners": "error",
    "no-unused-vars": ["error", { args: "all", caughtErrors: "none" }],
    eqeqeq: ["error", "always", { null: "ignore" }],
  },
}, ...scriptGlobals(scripts)];
