import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { Linter } from "eslint";
import config from "../eslint.config.mjs";
import { scriptGlobals } from "./script-globals.mjs";

const scripts = ["navigation", "app", "modes"].map((name) => ({
  file: `web/${name}.js`,
  source: readFileSync(new URL(`../web/${name}.js`, import.meta.url), "utf8"),
}));

test("export headers generate readonly globals in the other loaded scripts", () => {
  const configs = scriptGlobals([
    { file: "web/a.js", source: "/* exported value */ const value = 1;" },
    { file: "web/b.js", source: "value;" },
  ]);
  assert.deepEqual(configs[0].languageOptions.globals, {});
  assert.deepEqual(configs[1].languageOptions.globals, { value: "readonly" });
});

test("renaming the producer's select without updating its export fails", () => {
  assert.throws(() => scriptGlobals(scripts.map((s) => s.file === "web/app.js"
    ? { ...s, source: s.source.replace("function select(", "function renamedSelect(") } : s)), /exported name select has no top-level binding/);
});

test("removing an export removes the consumer's global", () => {
  const changed = scripts.map((s) => s.file === "web/app.js"
    ? { ...s, source: s.source.replace(", select,", ",") } : s);
  const messages = new Linter().verify("select(null);", [config[0], ...scriptGlobals(changed)], { filename: "web/modes.js" });
  assert(messages.some((m) => m.ruleId === "no-undef" && m.message.includes("select")));
});

test("a second historyRetryTimer declaration fails across scripts", () => {
  assert.throws(() => scriptGlobals(scripts.map((s) => s.file === "web/app.js"
    ? { ...s, source: s.source + "\nlet historyRetryTimer = null;" } : s)), /duplicate top-level binding historyRetryTimer/);
});

test("cross-script clashes include destructuring, classes, functions and nested var", () => {
  for (const declaration of ["const { value } = {};", "class value {}", "function value() {}", "if (true) { var value; }"]) {
    assert.throws(() => scriptGlobals([
      { file: "web/a.js", source: "let value;" },
      { file: "web/b.js", source: declaration },
    ]), /duplicate top-level binding value/);
  }
});

test("confusable browser names fail no-undef but local bindings work", () => {
  const names = ["name", "status", "event", "top", "parent", "open", "close", "origin", "length", "find", "stop", "print"];
  for (const name of names) {
    const lint = (source) => new Linter().verify(source, config, { filename: "web/probe.js" });
    assert(lint(`${name};`).some((m) => m.ruleId === "no-undef"), name);
    assert.deepEqual(lint(`const ${name} = 1; console.log(${name});`), [], name);
  }
});

test("recommended rules catch self assignment", () => {
  const messages = new Linter().verify("let value = 1; value = value;", config, { filename: "web/probe.js" });
  assert(messages.some((m) => m.ruleId === "no-self-assign"));
});

test("popstate before app.js provides S is ignored", async () => {
  let popstate;
  runInNewContext(scripts[0].source, { window: { addEventListener(type, listener) {
    assert.equal(type, "popstate");
    popstate = listener;
  } } });
  await popstate();
});
