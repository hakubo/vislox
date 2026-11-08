// One-off review evidence: compare this extraction with an explicit main commit.
// Run: node tools/verify-frontend-moves.mjs <base-sha> > /tmp/frontend-module-moves.md
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { Linter } from "eslint";

const base = process.argv[2];
assert(base && /^[0-9a-f]{7,40}$/.test(base), "Pass the main commit SHA to compare against");
const original = (file) => execFileSync("git", ["show", `${base}:${file}`], { encoding: "utf8" });
function items(source) {
  let result;
  const messages = new Linter().verify(source, {
    languageOptions: { ecmaVersion: 2022, sourceType: "script" },
    plugins: { proof: { rules: { collect: { create() {
      return { Program(node) {
        result = node.body.filter((n) => n.directive !== "use strict").map((n) => ({
          code: source.slice(...n.range), line: n.loc.start.line,
          label: n.type === "FunctionDeclaration" ? n.id.name : source.slice(...n.range).split("\n")[0].slice(0, 100),
        }));
      } };
    } } } } },
    rules: { "proof/collect": "error" },
  });
  assert.deepEqual(messages, []);
  return result;
}
const before = items(original("web/app.js"));
const remaining = new Map();
for (const [index, item] of before.entries()) {
  if (!remaining.has(item.code)) remaining.set(item.code, []);
  remaining.get(item.code).push({ ...item, index });
}
const moved = [], counts = {};
for (const file of ["web/app.js", "web/deploy.js", "web/editing.js", "web/start.js"]) {
  let previous = -1;
  const source = readFileSync(file, "utf8");
  assert(source.includes('"use strict";'), `${file}: strict mode missing`);
  for (const item of items(source)) {
    const match = remaining.get(item.code)?.shift();
    assert(match, `${file}:${item.line}: added or changed item: ${item.label}`);
    assert(match.index > previous, `${file}:${item.line}: item order changed`);
    previous = match.index;
    counts[file] = (counts[file] || 0) + 1;
    if (file !== "web/app.js") moved.push({ ...item, file, from: match.line });
  }
}
assert([...remaining.values()].every((v) => !v.length), "Original items are missing");
for (const file of ["web/navigation.js", "web/modes.js"]) {
  const stripHeader = (s) => s.replace(/^\/\* exported[^]*?\*\/\n/, "");
  assert.equal(stripHeader(readFileSync(file, "utf8")), stripHeader(original(file)), `${file}: changed beyond exported header`);
}
console.log(`# Frontend module move verification\n\nBase: \`${base}\` (main).\n`);
console.log(`All ${before.length} original top-level items match byte for byte; none added or removed.\nOrder within each destination is unchanged. Strict mode is retained in every script.\nNavigation and modes also match byte for byte apart from exported headers.\n`);
console.log("The comparison includes entire function bodies, state initializers, event handlers and startup\nstatements. Only file boundaries, exported headers, module contract comments and strict-mode\ndirectives are new. HTML script order and embedded routes are checked separately by lint and the UI suites.\n");
console.log("Reproduce from the repo root after `npm ci`:\n\n```sh\nnode tools/verify-frontend-moves.mjs " + base + "\n```\n");
for (const [file, count] of Object.entries(counts)) console.log(`- \`${file}\`: ${count} items (${file === "web/app.js" ? "retained" : "moved"}).`);
console.log("\n| Item | Main app.js line | Destination | SHA-256 prefix |\n|---|---:|---|---|");
for (const item of moved) {
  const label = item.label.replaceAll("|", "\\|").replaceAll("`", "'");
  console.log(`| ${label} | ${item.from} | ${item.file}:${item.line} | ${createHash("sha256").update(item.code).digest("hex").slice(0, 16)} |`);
}
