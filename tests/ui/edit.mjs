// Browser test for editing in vislox (values, new blocks, moves, wires): a lox.toml project with nothing
// pending, and a stand-in lox whose `logic compile` writes what vislox's files say into out.Loxone. An edit
// must show as the pending revision, marked as not deployed, with Revert and Discard, and nothing
// may ever be deployed: the stand-in logs every call and fails on anything but `logic compile`.
//
//   VISLOX_BIN=target/release/vislox node tests/ui/edit.mjs
//
// Nothing here talks to a Miniserver: vislox runs with --no-live.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const bin = process.env.VISLOX_BIN || join(here, "../../target/release/vislox");
const fixture = join(here, "../fixtures/house.Loxone");

const DIV = "10000006-0000-0060-ffff504f94000001";
const MONO = "1000000a-0000-00a0-ffff504f94000001";
const OG = "10000008-0000-0080-ffff504f94000001";
const EG = "10000002-0000-0020-ffff504f94000001";
const AND = "10000004-0000-0040-ffff504f94000001", METER = "10000005-0000-0050-ffff504f94000001";
// A block the project's module manages (in the lock as `flur`), drawn on the upper floor.
const FLUR = "1000000b-0000-00b0-ffff504f94000001";
// A page of more managed blocks than one request takes ops (200).
const LAGER = "20000000-0000-0000-ffff504f94000001", LAGER_N = 210;
const lagerUuid = (i) => `2${String(i).padStart(7, "0")}-0000-0001-ffff504f94000001`;
const lagerAt = (i) => { const px = 960 + 1920 * (i % 15), py = 960 + 960 * Math.floor(i / 15); return { px, py, px2: px + 1344, py2: py + 696 }; };
// An NFC Code Touch on a page of its own (Zugang), for the users lox logic lint says it opens for none of (#66).
const ZUGANG = "40000000-0000-0000-ffff504f94000001", TOUCH = "40000001-0000-0000-ffff504f94000001";
const KITCHEN = "30000a01-0000-0000-ffff504f94000001", BATH = "30000a02-0000-0000-ffff504f94000001";
const root = mkdtempSync(join(tmpdir(), "vislox-edit-"));
const dir = join(root, "project");
mkdirSync(join(dir, "pages"), { recursive: true });
const git = (...a) => execFileSync("git", ["-C", dir, "-c", "user.name=t", "-c", "user.email=t@example.com", ...a]);
// The deployed config, with a Monoflop on the upper floor (its Time has a documented range: 0 or more).
const base = readFileSync(fixture, "utf8").replace('<Co K="Q" U="10000009-0000-0093-02ff504f94000001"/>\n\t\t\t</C>', `<Co K="Q" U="10000009-0000-0093-02ff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="Monoflop" V="178" U="${MONO}" Title="Treppe" Px="960" Py="2880" Px2="2304" Py2="3576" Cl="141,255,112" Nio="4" WF="16384">
\t\t\t\t<Co K="InputTrigger" U="1000000a-0000-00a1-00ff504f94000001"/>
\t\t\t\t<Co K="Time" Def="2" U="1000000a-0000-00a2-01ff504f94000001"/>
\t\t\t\t<Co K="Q" U="1000000a-0000-00a3-02ff504f94000001"/>
\t\t\t</C>
\t\t\t<C Type="And" V="178" U="${FLUR}" Title="Flur" Px="7104" Py="2880" Px2="8448" Py2="3576" Cl="141,255,112" Nio="3" WF="16384">
\t\t\t\t<Co K="I1" U="1000000b-0000-00b1-00ff504f94000001"/>
\t\t\t\t<Co K="I2" U="1000000b-0000-00b2-01ff504f94000001"/>
\t\t\t\t<Co K="Q" U="1000000b-0000-00b3-02ff504f94000001"/>
\t\t\t</C>`)
  // Two rooms; the Divide block is shown in the app, in the kitchen.
  .replace('\t\t<C Type="Page" V="178" U="10000002', `\t\t<C Type="Place" V="178" U="${KITCHEN}" Title="Küche"/>
\t\t<C Type="Place" V="178" U="${BATH}" Title="Bad"/>
\t\t<C Type="Page" V="178" U="10000002`)
  .replace('<Co K="Input1" Nc="1" U="10000006', `<IoData Visu="true" Pr="${KITCHEN}"/>\n\t\t\t\t<Co K="Input1" Nc="1" U="10000006`)
  .replace("\t\t</C>\n\t</C>\n</ControlList>", `\t\t</C>\n\t\t<C Type="Page" V="178" U="${LAGER}" Title="Lager" WF="16384">\n` +
    Array.from({ length: LAGER_N }, (_, i) => { const L = lagerAt(i); return `\t\t\t<C Type="And" V="178" U="${lagerUuid(i)}" Title="Lager ${i}" Px="${L.px}" Py="${L.py}" Px2="${L.px2}" Py2="${L.py2}" Cl="141,255,112" Nio="3" WF="16384"/>\n`; }).join("") +
    "\t\t</C>\n" +
    `\t\t<C Type="Page" V="178" U="${ZUGANG}" Title="Zugang" WF="16384">\n` +
    `\t\t\t<C Type="NfcCodeTouch" V="178" U="${TOUCH}" Title="Haustür" Px="960" Py="960" Px2="2304" Py2="1656" Cl="141,255,112" Nio="0" WF="16384"/>\n` +
    "\t\t</C>\n\t</C>\n</ControlList>");
assert.ok(base.includes(lagerUuid(LAGER_N - 1)), "the Lager page is in the base");
writeFileSync(join(dir, "base.Loxone"), base);
writeFileSync(join(dir, "out.Loxone"), base);
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\n');
writeFileSync(join(dir, "pages", "README"), "the module\n");
// The module's own source (the stand-in lox reads only vislox's files): `flur` is declared on line 3.
writeFileSync(join(dir, "pages", "flur.loxl"), '# The module\'s own source.\npage "Obergeschoss"\nflur = And("Flur")\n');
const FLUR_AT = { px: 7104, py: 2880, px2: 8448, py2: 3576 };
const lockObjects = { flur: { uuid: FLUR, type: "And", page_uuid: OG, layout: FLUR_AT } };
for (let i = 0; i < LAGER_N; i++) lockObjects[`lager_${i}`] = { uuid: lagerUuid(i), type: "And", page_uuid: LAGER, layout: lagerAt(i) };
writeFileSync(join(dir, "logic.lock.json"), JSON.stringify({ objects: lockObjects }));
// Compiled after the module's files: nothing is uncompiled.
writeFileSync(join(dir, "out.Loxone"), base);
git("init", "-q");
git("add", "-A");
git("commit", "-q", "-m", "Deployed");

// The stand-in lox. `logic compile` reads vislox's claims (extern …(uuid: "…"), x.title = "…",
// x.Port = n) and writes them into a copy of the base, as lox would. It draws every block of the lock
// where its `layout` says, and `--relayout <page>` lines that page's blocks up along the top. It takes
// the title of a block the module's own files declare (`flur = And("Flur")`). `logic check --json`
// finds an error on each line of the module's own files that says BROKEN; `logic fmt --check` finds
// nothing to format.
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const calls = join(loxDir, "calls.log");
const fakeLox = join(loxDir, "lox");
writeFileSync(fakeLox, `#!/usr/bin/env node
const fs = require("fs");
fs.appendFileSync(${JSON.stringify(calls)}, process.argv.slice(2).join(" ") + "\\n");
const own = () => fs.readdirSync("pages").filter((f) => f.endsWith(".loxl")).map((f) => ["pages/" + f, fs.readFileSync("pages/" + f, "utf8").split("\\n")]);
if (process.argv.slice(2, 5).join(" ") === "logic check --json") {
  const errors = own().flatMap(([path, lines]) => lines.flatMap((l, i) => l.includes("BROKEN") ? [{ path, line: i + 1, message: "compile error: unknown block type BROKEN" }] : []));
  console.log(JSON.stringify(errors.length ? { ok: false, path: errors[0].path, errors } : { ok: true, path: "pages", counts: {} }));
  process.exit(errors.length ? 1 : 0);
}
if (process.argv.slice(2, 5).join(" ") === "logic fmt --check") process.exit(0);
// \`logic lint .\` (#66): dead outputs on each block of the module's own files declared on a line that ends
// in "# dead", and a user whose tag opens no Code Touch when a file says NOTAG; printed as lox prints them.
if (process.argv.slice(2, 5).join(" ") === "logic lint .") {
  // Lint compiles in memory: as slow as a compile (VISLOX_TEST_LOX_DELAY, below).
  for (const end = Date.now() + (+process.env.VISLOX_TEST_LOX_DELAY || 0); Date.now() < end;);
  const dead = own().flatMap(([path, lines]) => lines.flatMap((l) => { const m = l.match(/^(\\w+) = .*# dead$/); return m ? [\`  \${path}: \\\`\${m[1]}\\\` — no wire draws from any of its ports (compiled config, GUI wires included)\`] : []; }));
  const access = own().some(([, lines]) => lines.some((l) => l.includes("NOTAG")))
    ? ["  \\\`Anna\\\` — user with an NFC tag has no access right on any NFC Code Touch, so the tag or code opens nothing."] : [];
  if (dead.length) console.log(\`dead-outputs (\${dead.length})\\n\${dead.join("\\n")}\`);
  if (access.length) console.log(\`no-access-right (1)\\n\${access[0]}\`);
  const n = dead.length + access.length;
  console.log(n ? \`lint: \${n} finding\${n === 1 ? "" : "s"}\` : "lint: clean");
  process.exit(n ? 1 : 0);
}
// \`logic test .\` (#67): runs each \`test "…" … end\` of the module's own files; an \`expect\` on a line that says
// FAILTEST reads 0, the others hold. Printed as lox prints it.
if (process.argv.slice(2, 5).join(" ") === "logic test .") {
  for (const end = Date.now() + (+process.env.VISLOX_TEST_LOX_DELAY || 0); Date.now() < end;);
  const out = [];
  let failed = 0, n = 0;
  for (const [, lines] of own()) for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^test "(.*)"/);
    if (!m) continue;
    n++;
    const bad = [];
    let step = 0;
    for (let j = i + 1; j < lines.length && lines[j].trim() !== "end"; j++) {
      const l = lines[j].trim();
      if (/^(tick|step) /.test(l)) step++;
      if (l.startsWith("expect ") && l.includes("FAILTEST")) bad.push(\`    step \${step}: \${l.split("#")[0].trim()} — actual 0\`);
    }
    if (bad.length) failed++;
    out.push(\`test "\${m[1]}" ... \${bad.length ? "FAILED" : "ok"}\`, ...bad);
  }
  if (!n) { console.log("no tests in pages"); process.exit(0); }
  console.log(out.join("\\n") + \`\\n\${n} tests: \${n - failed} passed, \${failed} failed\`);
  process.exit(failed ? 1 : 0);
}
if (process.argv.slice(2, 4).join(" ") !== "logic compile") { console.error("unexpected: " + process.argv.slice(2).join(" ")); process.exit(2); }
// VISLOX_TEST_LOX_DELAY (ms): compile as slowly as a busy CI runner does.
for (const end = Date.now() + (+process.env.VISLOX_TEST_LOX_DELAY || 0); Date.now() < end;);
let out = fs.readFileSync("base.Loxone", "utf8").split("\\n");
const file = "pages/vislox/edits.loxl";
const src = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
const slugs = {};
const KEYS = { In1: "Input1", In2: "Input2", Trigger: "InputTrigger" };
const unq = (v) => v.startsWith('"') ? JSON.parse(v) : v;
const xml = (v) => v.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
for (const line of src.split("\\n")) {
  let m;
  if ((m = line.match(/^extern (\\w+) = \\w+\\(uuid: "([^"]+)"\\)/))) { slugs[m[1]] = m[2]; continue; }
  if (!(m = line.match(/^(\\w+)\\.(\\w+) = (.*)$/))) continue;
  const uuid = slugs[m[1]], i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`));
  if (m[2] === "title") { out[i] = out[i].replace(/Title="[^"]*"/, \`Title="\${xml(unq(m[3]))}"\`); continue; }
  if (m[2] === "room") {
    const place = out.find((l) => l.includes('Type="Place"') && l.includes(\`Title="\${unq(m[3])}"\`)).match(/U="([^"]+)"/)[1];
    const j = out.findIndex((l, n) => n > i && l.includes("<IoData"));
    out[j] = out[j].replace(/Pr="[^"]*"/, \`Pr="\${place}"\`);
    continue;
  }
  const key = KEYS[m[2]] || m[2];
  const j = out.findIndex((l, n) => n > i && l.includes(\`<Co K="\${key}"\`));
  out[j] = out[j].replace(/ Def="[^"]*"/, "").replace(\`<Co K="\${key}"\`, \`<Co K="\${key}" Def="\${unq(m[3])}"\`);
}
// vislox's new blocks: one file per page (page "…"), a declaration per block. Each gets a UUID in the
// lock (kept by slug across compiles) and is drawn on its page with the constants it is given.
// A scratch compile (--lock, --out elsewhere) leaves the project's lock and the UUIDs minted alone.
const lockAt = process.argv.indexOf("--lock");
const lockFile = lockAt > 0 ? process.argv[lockAt + 1] : "logic.lock.json";
const lock = fs.existsSync(lockFile) ? JSON.parse(fs.readFileSync(lockFile, "utf8")) : { objects: {} };
const objects = {};
// Like lox, a block added again gets a UUID it never had: the count of UUIDs minted is kept.
const mintFile = ${JSON.stringify(join(loxDir, "minted"))};
let n = fs.existsSync(mintFile) ? +fs.readFileSync(mintFile, "utf8") : 0;
for (const f of fs.existsSync("pages/vislox") ? fs.readdirSync("pages/vislox") : []) {
  if (f === "edits.loxl" || !f.endsWith(".loxl")) continue;
  let pageAt = -1;
  for (const line of fs.readFileSync("pages/vislox/" + f, "utf8").split("\\n")) {
    let m;
    if ((m = line.match(/^page "(.*)"$/))) { pageAt = out.findIndex((l) => l.includes('Type="Page"') && l.includes(\`Title="\${m[1]}"\`)); continue; }
    if (!(m = line.match(/^(vx_\\w+) = (\\w+)\\((.*)\\)$/))) continue;
    const [, slug, ty, rest] = m;
    const uuid = lock.objects[slug]?.uuid || \`bbbbbb\${String(++n).padStart(2, "0")}-0000-0000-ffff504f94000001\`;
    const page_uuid = out[pageAt].match(/U="([^"]+)"/)[1];
    objects[slug] = { uuid, type: ty, page_uuid, layout: lock.objects[slug]?.layout || { px: 96, py: 96, px2: 1440, py2: 792 } };
    const title = rest.match(/^"((?:[^"\\\\]|\\\\.)*)"/);
    const cos = [...rest.matchAll(/(\\w+): (-?[\\d.]+)/g)].map(([, k, v], i) => \`<Co K="\${KEYS[k] || k}" Def="\${v}" U="\${uuid.slice(0, 9)}0000-00\${i}0-ffff504f94000001"/>\`);
    out.splice(pageAt + 1, 0, \`\\t\\t\\t<C Type="\${ty}" V="178" U="\${uuid}" Title="\${xml(title ? JSON.parse('"' + title[1] + '"') : slug)}" Px="96" Py="96" Px2="1440" Py2="792" Nio="2">\${cos.join("")}</C>\`);
  }
}
// The blocks the rest of the module manages stay in the lock, with the titles their files give them.
for (const [s, o] of Object.entries(lock.objects)) if (!s.startsWith("vx_")) objects[s] = o;
// The module's own files declare \`flur\`. When they no longer do, the compile is refused while the lock manages
// it (#70), unless removals are allowed: then the lock forgets it, with a tombstone, and the config loses it.
let removed = lock.removed;
if (objects.flur && !own().some(([, lines]) => lines.some((l) => /^flur = /.test(l)))) {
  if (!process.argv.includes("--allow-removals")) {
    console.error("error: compile error: managed block \`flur\` is in the lockfile but not in the source. To delete it, add \`removed flur\` (or pass --allow-removals); to rename it, add \`moved flur -> <new_slug>\`.");
    process.exit(1);
  }
  removed = { ...removed, [objects.flur.uuid]: { slug: "flur", type: "And" } };
  const i = out.findIndex((l) => l.includes(\`U="\${objects.flur.uuid}"\`));
  out.splice(i, out.findIndex((l, n) => n > i && l.trim() === "</C>") - i + 1);
  delete objects.flur;
}
for (const [, lines] of own()) for (const line of lines) {
  const m = line.match(/^(\\w+) = \\w+\\("(.*)"\\)$/), o = m && objects[m[1]];
  const i = o ? out.findIndex((l) => l.includes(\`U="\${o.uuid}"\`)) : -1;
  if (i >= 0) out[i] = out[i].replace(/Title="[^"]*"/, \`Title="\${xml(m[2])}"\`);
}
const relayout = process.argv.flatMap((a, i) => process.argv[i - 1] === "--relayout" ? [a] : []);
const cols = {};
for (const o of Object.values(objects)) {
  const pg = out.find((l) => l.includes('Type="Page"') && l.includes(\`U="\${o.page_uuid}"\`));
  if (!pg || !relayout.some((t) => pg.includes(\`Title="\${t}"\`))) continue;
  const w = o.layout.px2 - o.layout.px, h = o.layout.py2 - o.layout.py;
  // Ten a row, from (9600, 960).
  const c = (cols[o.page_uuid] = (cols[o.page_uuid] ?? -1) + 1);
  const x = 9600 + 1920 * (c % 10), y = 960 + 960 * Math.floor(c / 10);
  o.superseded_layout = o.layout;
  o.layout = { px: x, py: y, px2: x + w, py2: y + h };
}
for (const o of Object.values(objects)) {
  const i = out.findIndex((l) => l.includes(\`U="\${o.uuid}"\`));
  const L = o.layout;
  if (i >= 0 && L) out[i] = out[i].replace(/Px="-?\\d+" Py="-?\\d+" Px2="-?\\d+" Py2="-?\\d+"/, \`Px="\${L.px}" Py="\${L.py}" Px2="\${L.px2}" Py2="\${L.py2}"\`);
}
// vislox's wires (to.Port <- from.Port) between the objects its externs pin: an <In> in the sink's
// connector, from the source's connector, as lox draws it.
const METER_KEYS = { Power: "Pf", ReadingIn: "Mr", Reset: "R", ReadingOffset: "Mro" };
const coOf = (uuid, name) => {
  const i = out.findIndex((l) => l.includes(\`U="\${uuid}"\`));
  for (const k of [name, KEYS[name], METER_KEYS[name], name.replace(/^In(\\d)$/, "I$1")]) {
    const j = out.findIndex((l, n) => n > i && l.includes(\`<Co K="\${k}"\`));
    if (j > i && !out.slice(i + 1, j).some((l) => /<C Type=/.test(l))) return j;
  }
  return -1;
};
for (const line of src.split("\\n")) {
  const m = line.match(/^(\\w+)\\.(\\w+) <- (\\w+)\\.(\\w+)$/);
  if (!m) continue;
  const to = coOf(slugs[m[1]], m[2]), from = coOf(slugs[m[3]], m[4]);
  const srcU = out[from].match(/U="([^"]+)"/)[1];
  out[to] = out[to].replace(/\\/>$/, \`><In Input="\${srcU}"/></Co>\`);
}
fs.writeFileSync(lockFile, JSON.stringify(removed ? { objects, removed } : { objects }));
if (lockAt < 0) fs.writeFileSync(mintFile, String(n));
const to = process.argv.indexOf("--out");
fs.writeFileSync(to > 0 ? process.argv[to + 1] : "out.Loxone", out.join("\\n"));
`);
chmodSync(fakeLox, 0o755);
const token = join(loxDir, "token");
writeFileSync(token, "secret\n");
const loxConfig = join(loxDir, "config.yaml");
writeFileSync(loxConfig, "host: http://127.0.0.1:9\nuser: u\npass: p\nserial: 504F94000001\n");

const port = 7980 + Math.floor(Math.random() * 9);
const server = spawn(bin, [dir, "--listen", `127.0.0.1:${port}`, "--lox", fakeLox, "--admin-token-file", token, "--no-live"], {
  stdio: ["ignore", "ignore", "pipe"],
  env: { ...process.env, LOX_CONFIG: loxConfig, VISLOX_CACHE: join(root, "cache") },
});
let log = "";
server.stderr.on("data", (d) => (log += d));
await new Promise((ok, fail) => {
  const t = setTimeout(() => fail(new Error("vislox did not start:\n" + log)), 15000);
  server.stderr.on("data", () => { if (log.includes("open http")) { clearTimeout(t); ok(); } });
  server.on("exit", (c) => fail(new Error(`vislox exited ${c}:\n${log}`)));
});

const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH, args: ["--no-sandbox"] } : {});
const errors = [];
let failed = false;
let page;
try {
  page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  page.on("pageerror", (e) => errors.push(e.message));
  // A refused edit answers 422 (the browser logs it as a failed request); anything else is an error.
  page.on("console", (m) => { if (m.type() === "error" && !/status of 422/.test(m.text())) errors.push(m.text()); });
  const edits = [];
  // Each edit request, and the status its response came back with (by request: a response event may
  // reach this script after the next request's).
  const reqs = [], statuses = [];
  page.on("request", (r) => { if (new URL(r.url()).pathname === "/api/edit") { reqs.push(r); edits.push(JSON.parse(r.postData())); } });
  page.on("response", (r) => { const i = reqs.indexOf(r.request()); if (i >= 0) statuses[i] = r.status(); });
  /** Wait in this script until `ok()` holds. */
  const until = async (ok, what, ms = 20000) => {
    for (const end = Date.now() + ms; !ok();) {
      if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
      await new Promise((r) => setTimeout(r, 50));
    }
  };
  page.on("dialog", (d) => d.accept());
  await page.addInitScript(() => localStorage.setItem("vislox.token", "secret"));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForSelector(".tab");
  const step = async (name, fn) => { await fn(); console.log("ok -", name); };
  const pick = async (uuid) => {
    await page.evaluate((u) => { const pg = S.objPage.get(u); if (pg) gotoPage(pg, { focus: u }); select(u); }, uuid);
    await page.waitForFunction((u) => S.sel === u && document.querySelector('[data-body="Edit"]'), uuid);
  };
  const field = (f) => `#props [data-ed-field="${f}"]`;
  // What the page shows once it settles: an edit's reply, the History and /api/edits come one after another.
  const waitText = (sel, re) => page.waitForFunction(([s, src]) => new RegExp(src).test(document.querySelector(s)?.textContent || ""), [sel, re.source], { timeout: 10000 });
  // Undo and redo (#62): the key, then the one request it sent, once the page settled on what it
  // compiled (`done`: what the page shows then).
  // Settled: no undo or redo queued or sent, nothing compiling on the server, the newest config shown,
  // and Properties not waiting for a compile.
  const SETTLED = `!UNDO.busy && !UNDO.inflight && !S.hist?.actions?.running && S.viewRev === S.rev && S.rev === editRev()?.id &&
    !/Compiling…|Undo or redo is compiling/.test(document.querySelector("#props")?.textContent || "")`;
  const settled = () => page.waitForFunction(SETTLED, null, { timeout: 20000 });
  const undoKey = async (redo, done, arg) => {
    const n = edits.length;
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press(redo ? "Control+Shift+z" : "Control+z");
    await until(() => edits.length > n && statuses[n] !== undefined, `the ${redo ? "redo" : "undo"} request`);
    await page.waitForFunction(done, arg, { timeout: 20000 });
    await settled();
    assert.equal(edits.length, n + 1, "one request");
    assert.equal(statuses[n], 200);
    return edits[n].ops;
  };


  await step("nothing is pending, and Properties offers to edit the Divide block", async () => {
    await page.waitForSelector("#deploy-pane:not(.hidden)");
    await waitText("#deploy-pend", /Nothing pending\. Edit values in Properties/);
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    // Its name and its divisor; the dividend is wired.
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["title", "room", "p:Input2"]);
    assert.equal(await page.inputValue(field("room")), "Küche");
    assert.equal(await page.inputValue(field("p:Input2")), "1000");
  });
  await step("the blocks the module manages are marked, with the line that declares them (#64)", async () => {
    await page.waitForFunction((u) => S.edits?.sources?.[u], FLUR);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, FLUR);
    await page.waitForSelector(`.blk.managed[data-uuid="${FLUR}"] .mbadge`);
    // The Monoflop beside it is Loxone Config's.
    assert.equal(await page.$(`.blk[data-uuid="${MONO}"].managed`), null);
    await page.waitForSelector('[data-body="Lox Logic"]');
    assert.match(await page.textContent('[data-body="Lox Logic"]'), /flur[\s\S]*pages\/flur\.loxl:3[\s\S]*Declared on this line/);
    assert.match(await page.textContent("#props .ph-facts"), /λ flur/);
    // Its values are edited in that file, which Properties names.
    await waitText('[data-body="Edit"]', /change its values in pages\/flur\.loxl:3/);
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: `http://127.0.0.1:${port}` });
    // A double click copies twice and still puts the label back.
    await page.dblclick('[data-body="Lox Logic"] button.copy');
    await waitText('[data-body="Lox Logic"] button.copy', /^Copied$/);
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), "pages/flur.loxl:3");
    await waitText('[data-body="Lox Logic"] button.copy', /^Copy path$/);
    // A block the lock pins that no .loxl file declares: its slug, and why there is no line.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, lagerUuid(3));
    await waitText('[data-body="Lox Logic"]', /lager_3[\s\S]*no \.loxl file declares it now/);
  });
  await step("the legend fades the blocks the module does not manage", async () => {
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(null); }, FLUR);
    await page.waitForSelector("#legend.managed #chk-only-managed", { state: "visible" });
    await page.check("#chk-only-managed");
    await page.waitForSelector("#canvas.only-managed");
    const op = (u) => page.$eval(`.blk[data-uuid="${u}"]`, (n) => +getComputedStyle(n).opacity);
    assert.ok((await op(MONO)) < 0.5, "the Monoflop is faded");
    assert.equal(await op(FLUR), 1);
    await page.uncheck("#chk-only-managed");
    await page.waitForFunction(() => !document.querySelector("#canvas.only-managed"));
  });
  await step("Ctrl+K finds a managed block by its slug", async () => {
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("Control+k");
    await page.fill("#pal-input", "lager_12");
    await page.waitForSelector("#pal-list [role=option][aria-selected=true] .pal-slug mark");
    assert.match(await page.textContent("#pal-list [role=option][aria-selected=true]"), /Lager 12[\s\S]*lager_12/);
    await page.press("#pal-input", "Enter");
    await page.waitForFunction((u) => S.sel === u, lagerUuid(12));
  });
  await step("a line moved in the module, not compiled, is followed", async () => {
    writeFileSync(join(dir, "pages", "flur.loxl"), '# The module\'s own source.\npage "Obergeschoss"\n\n# Flur\nflur = And("Flur")\n');
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, FLUR);
    await waitText('[data-body="Lox Logic"]', /pages\/flur\.loxl:5/);
    // Back to the Divide block, whose form is still open, for the steps that follow.
    await pick(DIV);
    await page.waitForSelector(field("p:Input2"));
  });
  await step("a value that is not a number is refused in the browser", async () => {
    await page.fill(field("p:Input2"), "1,5");
    await page.click('#props [data-ed="save"]');
    await page.waitForSelector(`${field("p:Input2")}[aria-invalid="true"]`);
    assert.match(await page.textContent("#props .ed-bad"), /decimal point/);
    assert.equal(edits.length, 0);
  });
  await step("saving compiles the edit and shows it as pending, not deployed", async () => {
    await page.fill(field("title"), "Teiler");
    await page.selectOption(field("room"), "Bad");
    await page.fill(field("p:Input2"), "100");
    await page.press(field("p:Input2"), "Enter");
    await page.waitForFunction(() => S.rev?.startsWith("pending:") && S.view?.objects["10000006-0000-0060-ffff504f94000001"]?.title === "Teiler", null, { timeout: 10000 });
    assert.equal(edits.length, 1);
    assert.deepEqual(edits[0].ops, [
      { op: "title", uuid: DIV, value: "Teiler" },
      { op: "room", uuid: DIV, value: "Bad" },
      { op: "param", uuid: DIV, port: "Input2", value: "100" },
    ]);
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 3);
    assert.equal(await page.evaluate((u) => S.view.objects[u].room, DIV), "Bad");
    await waitText("#props", /Saved and compiled: pending, not deployed/);
    await waitText("#deploy-pend", /not deployed\. They include 3 values edited here/);
    await page.waitForSelector("#btn-apply:not([disabled])");
    await waitText(".changes", /Teiler[\s\S]*1000 → 100|1000 → 100[\s\S]*Teiler/);
    const file = readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8");
    assert.match(file, /^extern vx_10000006_0000_0060_ffff504f94000001 = Div\(uuid: "10000006-0000-0060-ffff504f94000001"\)$/m);
    assert.match(file, /\.In2 = 100$/m);
  });
  await step("undo puts the values back as a new edit, redo makes it again", async () => {
    await page.waitForSelector("#btn-undo:not([disabled])");
    assert.match(await page.getAttribute("#btn-undo", "title"), /^Undo: 3 values of .+ \((Ctrl\+Z|⌘Z)\)/);
    assert.equal(await page.isDisabled("#btn-redo"), true);
    let ops = await undoKey(false, (u) => S.view?.objects[u]?.title !== "Teiler" && S.view.objects[u].room === "Küche", DIV);
    // The reply's `undo`: each op's previous, the last op's first.
    assert.deepEqual(ops, [
      { op: "param", uuid: DIV, port: "Input2", value: null },
      { op: "room", uuid: DIV, value: null },
      { op: "title", uuid: DIV, value: null },
    ]);
    assert.equal(await page.evaluate((u) => S.view.objects[u].ports.find((p) => p.key === "Input2").def, DIV), "1000");
    assert.doesNotMatch(existsSync(join(dir, "pages/vislox/edits.loxl")) ? readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8") : "", /title|In2/);
    await waitText("#deploy-msg", /Undone: 3 values of .*Compiled: pending, not deployed/);
    assert.match(await page.getAttribute("#btn-redo", "title"), /^Redo: 3 values of /);
    ops = await undoKey(true, (u) => S.view?.objects[u]?.title === "Teiler" && S.view.objects[u].room === "Bad", DIV);
    assert.deepEqual(ops, edits[0].ops);
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 3);
    // The list of recent edits names it.
    await page.click("#btn-undo-list");
    await waitText("#undo-pop", /3 values of/);
    await page.click("#btn-undo-list");
  });
  await step("a value out of its documented range is refused by the server, and nothing changes", async () => {
    await pick(MONO);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("p:Time"), "-3");
    assert.equal(await page.textContent("#props .ed-unit"), "s");
    await page.click('#props [data-ed="save"]');
    await page.waitForFunction(() => /outside the documented range/.test(document.querySelector("#props .ed-msg.err")?.textContent || ""));
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /Time/);
    await page.press(field("p:Time"), "Escape");
  });
  await step("Revert hands one value back", async () => {
    await pick(DIV);
    const n = edits.length;
    // The list is replaced while the page shows it: Revert still reverts its own value.
    await page.evaluate(() => S.edits.claims.reverse());
    await page.click('#props .pr:has-text("In2") [data-ed="revert"]');
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 2);
    assert.deepEqual(edits[n].ops, [{ op: "param", uuid: DIV, port: "Input2", value: null }]);
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /In2/);
  });
  const saveTitle = async (t) => {
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("title"), t);
    await page.press(field("title"), "Enter");
  };
  // The Divide block named `t`, and the page settled (both at once: a name read mid-compile is no answer).
  const titleIs = (t) => page.waitForFunction(`S.view?.objects["${DIV}"]?.title === ${JSON.stringify(t)} && ${SETTLED}`, null, { timeout: 20000 });
  await step("a quick Ctrl+Z right after a save undoes that save, and quick steps wait their turn", async () => {
    const n = edits.length;
    await saveTitle("Schnell");
    // At once: the save is still compiling, and the History has not shown it.
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("Control+z");
    await until(() => edits.length >= n + 2 && statuses[n + 1] !== undefined, "the save, then its undo");
    await titleIs("Teiler");
    assert.equal(edits.length, n + 2, "the save, then its undo");
    assert.deepEqual(edits[n + 1].ops, [{ op: "title", uuid: DIV, value: "Teiler" }]);
    // Three keys in a row: redo, undo, redo, each against the config the one before left.
    await page.keyboard.press("Control+Shift+z");
    await page.keyboard.press("Control+z");
    await page.keyboard.press("Control+Shift+z");
    await until(() => edits.length >= n + 5 && statuses[n + 4] !== undefined, "three steps");
    await titleIs("Schnell");
    assert.equal(edits.length, n + 5);
    assert.deepEqual(statuses.slice(n, n + 5), [200, 200, 200, 200, 200], "no stale base, no 409");
    await undoKey(false, (u) => S.view?.objects[u]?.title === "Teiler", DIV);
  });
  await step("an undo that changes nothing is done, and its redo kept", async () => {
    await saveTitle("Gleich");
    await titleIs("Gleich");
    // The undo is sent as a claim of what is claimed already: the server compiles nothing.
    await page.route("**/api/edit", (r) => r.continue({ postData: JSON.stringify({ ...JSON.parse(r.request().postData()), ops: [{ op: "title", uuid: DIV, value: "Gleich" }] }) }), { times: 1 });
    await page.keyboard.press("Control+z");
    await waitText("#deploy-msg", /Undone: Name of .*Nothing changed/);
    await page.waitForFunction(`/Gleich/.test(UNDO.undone.at(-1)?.label || "") && ${SETTLED}`, null, { timeout: 20000 });
    assert.equal(await page.evaluate(() => UNDO.note), "");
    // Its redo is the edit as made (the config already is that way), and the undo after it is real.
    await page.keyboard.press("Control+Shift+z");
    await page.waitForFunction(`/Gleich/.test(UNDO.done.at(-1)?.label || "") && !UNDO.undone.length && ${SETTLED}`, null, { timeout: 20000 });
    await undoKey(false, (u) => S.view?.objects[u]?.title === "Teiler", DIV);
  });
  await step("a redo whose reply has no undo clears the history and says why", async () => {
    await page.route("**/api/edit", async (r) => {
      const res = await r.fetch(), j = await res.json();
      delete j.undo;
      j.undo_error = "op 1 (title): no way back for the test";
      await r.fulfill({ response: res, json: j });
    }, { times: 1 });
    await page.keyboard.press("Control+Shift+z");
    await titleIs("Gleich");
    await page.waitForFunction(`!UNDO.done.length && !UNDO.undone.length && ${SETTLED}`, null, { timeout: 20000 });
    assert.match(await page.evaluate(() => UNDO.note), /cannot be undone: op 1 \(title\): no way back for the test\. The undo history was cleared\./);
    await waitText("#deploy-msg", /cannot be undone/);
    await page.waitForSelector("#btn-undo[disabled]", { state: "attached" });
    // Back to the name the later steps expect: an edit of its own.
    await saveTitle("Teiler");
    await titleIs("Teiler");
  });
  await step("Add block offers the types lox can create, and checks values in the browser", async () => {
    await page.evaluate((u) => gotoPage(u), OG);
    await page.waitForSelector("#btn-add-block:not([disabled])");
    await page.click("#btn-add-block");
    await page.waitForSelector("#ab-q");
    assert.equal(await page.textContent("#ab-page"), "on Obergeschoss");
    // Virtual inputs are not blocks of a page.
    await page.fill("#ab-q", "virtual in");
    assert.doesNotMatch(await page.textContent("#ab-list"), /VirtualIn\b/);
    await page.fill("#ab-q", "monofl");
    assert.match(await page.textContent("#ab-list [aria-selected=true]"), /Monoflop/);
    await page.press("#ab-q", "Enter");
    await page.waitForSelector('#ab-body [data-ab-field="title"]');
    const fields = await page.$$eval("#ab-body [data-ab-field]", (n) => n.map((x) => x.dataset.abField));
    assert.ok(fields.includes("p:Time") && fields.includes("p:InputTrigger"), fields.join(" "));
    await page.fill('#ab-body [data-ab-field="title"]', "Nachlauf");
    await page.fill('#ab-body [data-ab-field="p:Time"]', "1,5");
    await page.click('#ab-body [data-ab="add"]');
    await page.waitForSelector('#ab-body [data-ab-field="p:Time"][aria-invalid="true"]');
    assert.match(await page.textContent("#ab-body .ed-bad"), /decimal point/);
  });
  let added;
  await step("adding compiles it into the pending revision, on the page, not deployed", async () => {
    const n = edits.length;
    await page.fill('#ab-body [data-ab-field="p:Time"]', "30");
    await page.press('#ab-body [data-ab-field="p:Time"]', "Enter");
    await page.waitForFunction(() => S.sel && S.view?.objects[S.sel]?.title === "Nachlauf", null, { timeout: 10000 });
    assert.equal(await page.isVisible("#addblk"), false);
    assert.deepEqual(edits[n].ops, [{ op: "add_block", page: OG, type: "Monoflop", title: "Nachlauf", params: { Time: "30" }, settings: {} }]);
    added = await page.evaluate(() => S.sel);
    assert.equal(await page.evaluate((u) => S.objPage.get(u), added), OG);
    const file = readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8");
    assert.match(file, /^page "Obergeschoss"$/m);
    assert.match(file, /^vx_monoflop_1 = Monoflop\("Nachlauf", Time: 30\)$/m);
    await waitText("#props", /added here/);
    await waitText("#props", /Added and compiled: pending, not deployed/);
    await waitText("#deploy-pend", /1 block added here/);
    await waitText(".changes", /Nachlauf/);
  });
  await step("undo takes the new block out, redo adds it again under a new UUID", async () => {
    let ops = await undoKey(false, (u) => S.view && !S.view.objects[u], added);
    assert.deepEqual(ops, [{ op: "remove_block", uuid: added }]);
    assert.match(await page.getAttribute("#btn-redo", "title"), /^Redo: Add Nachlauf/);
    ops = await undoKey(true, () => Object.values(S.view?.objects || {}).some((o) => o.title === "Nachlauf"));
    assert.equal(ops[0].op, "add_block");
    assert.equal(ops[0].restores, added);
    assert.deepEqual(ops[0].params, { Time: "30" });
    added = await page.evaluate(() => Object.values(S.view.objects).find((o) => o.title === "Nachlauf").uuid);
    await pick(added);
    await waitText("#props", /added here/);
  });
  await step("a block added here is edited like any other", async () => {
    await page.click('#props [data-ed="open"]');
    // Its declaration takes the name and constants; lox has no room or category for it.
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["title", "p:Time"]);
    await page.fill(field("p:Time"), "45");
    await page.press(field("p:Time"), "Enter");
    await page.waitForFunction((u) => S.view?.objects[u]?.ports.find((p) => p.key === "Time")?.def === "45", added, { timeout: 10000 });
    await waitText("#props", /Saved and compiled: pending, not deployed/);
    assert.match(readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8"), /Time: 45\)$/m);
  });
  await step("Remove takes it out again", async () => {
    const n = edits.length;
    await page.click('#props [data-ed="remove"]');
    await page.waitForFunction((u) => S.view && !S.view.objects[u], added, { timeout: 10000 });
    assert.deepEqual(edits[n].ops, [{ op: "remove_block", uuid: added }]);
    const file = readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8");
    assert.match(file, /^removed vx_monoflop_1$/m);
    assert.doesNotMatch(file, /= Monoflop/);
    await page.waitForFunction(() => !/added here/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("undo adds the removed block again, and later undos follow its new UUID", async () => {
    let ops = await undoKey(false, () => Object.values(S.view?.objects || {}).some((o) => o.title === "Nachlauf"));
    assert.equal(ops[0].op, "add_block");
    assert.equal(ops[0].restores, added);
    const again = await page.evaluate(() => Object.values(S.view.objects).find((o) => o.title === "Nachlauf").uuid);
    assert.notEqual(again, added, "lox minted a new UUID");
    assert.equal(await page.evaluate((u) => S.view.objects[u].ports.find((p) => p.key === "Time").def, again), "45");
    // The edit of Time made before the removal now names the new UUID.
    ops = await undoKey(false, (u) => S.view?.objects[u]?.ports.find((p) => p.key === "Time")?.def === "30", again);
    assert.deepEqual(ops, [{ op: "param", uuid: again, port: "Time", value: "30" }]);
    ops = await undoKey(true, (u) => S.view?.objects[u]?.ports.find((p) => p.key === "Time")?.def === "45", again);
    assert.deepEqual(ops, [{ op: "param", uuid: again, port: "Time", value: "45" }]);
    ops = await undoKey(true, (u) => S.view && !S.view.objects[u], again);
    assert.deepEqual(ops, [{ op: "remove_block", uuid: again }]);
    assert.match(readFileSync(join(dir, "pages/vislox/obergeschoss.loxl"), "utf8"), /^removed vx_monoflop_\d+$/m);
  });
  // Moving (#60): lox keeps a managed block's position in the lock; vislox writes it there and compiles.
  const flurNow = () => page.evaluate((u) => curPage()?.objects.find((o) => o.uuid === u && !o.moved_to), FLUR);
  const flurLock = () => JSON.parse(readFileSync(join(dir, "logic.lock.json"), "utf8")).objects.flur;
  // A newer pending revision than `since`, once it is shown and draws the block at `x`, marked moved or not
  // (while a move is sent, the page already draws the block where it was dropped).
  const rev = () => page.evaluate(() => S.rev);
  const flurAt = (since, x, moved) => page.waitForFunction(([u, since, x, moved]) => {
    const o = curPage()?.objects.find((o) => o.uuid === u && !o.moved_to);
    return S.rev !== since && S.viewRev === S.rev && S.rev?.startsWith("pending:") && !MV && !S.mvBusy &&
      o?.rect[0] === x && (o.status === "moved") === moved;
  }, [FLUR, since, x, moved], { timeout: 10000 });
  let moved;
  await step("a block the module manages is dragged on the page, snapped to the grid, and shows as moved", async () => {
    // In the middle of the canvas, where nothing covers it.
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, [OG, FLUR]);
    const g = page.locator(`#canvas g.movable[data-uuid="${FLUR}"]`).first();
    await g.waitFor();
    const n = edits.length, r0 = await rev();
    const box = await g.boundingBox();
    const [cx, cy] = [box.x + box.width / 2, box.y + box.height / 2];
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 80, cy + 50, { steps: 8 });
    await page.mouse.up();
    await page.waitForFunction((k) => document.querySelectorAll("#props .ed-tag").length > 0 && S.sel === k, FLUR, { timeout: 10000 });
    assert.equal(edits.length, n + 1);
    const op = edits[n].ops[0];
    assert.equal(op.op, "move");
    assert.equal(op.uuid, FLUR);
    assert.ok(op.x % 96 === 0 && op.y % 96 === 0 && (op.x !== 7104 || op.y !== 2880), JSON.stringify(op));
    moved = op;
    await flurAt(r0, op.x, true);
    assert.equal((await flurNow()).rect[1], op.y);
    // In the lock, with the position the base still draws superseded; vislox's files claim nothing of it.
    const l = flurLock();
    assert.deepEqual([l.layout.px, l.layout.py, l.layout.px2 - l.layout.px], [op.x, op.y, 8448 - 7104]);
    assert.equal(l.superseded_layout.px, 7104);
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /1000000b/);
    await waitText("#props", /Position\s*moved\s*not deployed/);
    await waitText("#deploy-pend", /1 block moved here/);
    await page.waitForSelector("#canvas .ghost");
  });
  await step("Alt and an arrow key nudge it one grid step", async () => {
    const n = edits.length, r0 = await rev();
    await page.keyboard.press("Alt+ArrowRight");
    await flurAt(r0, moved.x + 96, true);
    assert.equal(edits.length, n + 1);
    assert.deepEqual(edits[n].ops, [{ op: "move", uuid: FLUR, x: moved.x + 96, y: moved.y }]);
  });
  await step("undo and redo a move", async () => {
    const at = ([u, r0, x]) => S.rev !== r0 && !MV && !S.mvBusy && curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect[0] === x;
    let ops = await undoKey(false, at, [FLUR, await rev(), moved.x]);
    assert.deepEqual(ops, [{ op: "move", uuid: FLUR, x: moved.x, y: moved.y }]);
    ops = await undoKey(true, at, [FLUR, await rev(), moved.x + 96]);
    assert.deepEqual(ops, [{ op: "move", uuid: FLUR, x: moved.x + 96, y: moved.y }]);
    await page.evaluate((u) => select(u), FLUR);
  });
  await step("nudged past the top of the page, it stops at the edge", async () => {
    const n = edits.length, r0 = await rev(), x = (await flurNow()).rect[0];
    for (let i = 0; i < 4; i++) await page.keyboard.press("Alt+Shift+ArrowUp");
    await flurAt(r0, x, true);
    assert.equal(edits.length, n + 1);
    assert.deepEqual(edits[n].ops, [{ op: "move", uuid: FLUR, x, y: 0 }]);
    assert.equal((await flurNow()).rect[1], 0);
  });
  await step("a refused move puts the block back; one compiled but not shown stays", async () => {
    // Settled: no move in flight, the edit's message shown.
    const settled = (re) => page.waitForFunction((src) => !MV && !S.mvBusy && new RegExp(src).test(S.edit.err), re.source, { timeout: 10000 });
    const x = (await flurNow()).rect[0];
    await page.route("**/api/edit", (r) => r.fulfill({ status: 422, contentType: "application/json", body: JSON.stringify({ error: "refused for the test" }) }), { times: 1 });
    await page.keyboard.press("Alt+ArrowLeft");
    await settled(/refused for the test/);
    assert.equal((await flurNow()).rect[0], x);
    // Compiled, but showing the new revision fails once: the canvas keeps the move.
    const r0 = await rev();
    await page.evaluate(() => { const show = editShowNewest; window.editShowNewest = async () => { window.editShowNewest = show; throw new Error("no history"); }; });
    await page.keyboard.press("Alt+ArrowLeft");
    await settled(/could not show it: no history/);
    assert.equal((await flurNow()).rect[0], x - 96);
    await page.evaluate(() => editShowNewest());
    await flurAt(r0, x - 96, true);
  });
  await step("Revert puts it back where it is deployed", async () => {
    const n = edits.length, r0 = await rev();
    await page.click('#props [data-ed="unmove"]');
    await flurAt(r0, 7104, false);
    assert.deepEqual(edits[n].ops, [{ op: "move", uuid: FLUR, x: 7104, y: 2880 }]);
    assert.equal(flurLock().superseded_layout, undefined);
    await page.waitForFunction(() => !/moved here/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("Arrange lays out the page's managed blocks afresh", async () => {
    const n = edits.length, r0 = await rev();
    await page.waitForSelector("#btn-relayout:not([disabled])");
    await page.click("#btn-relayout");
    await flurAt(r0, 9600, true);
    assert.deepEqual(edits[n].ops, [{ op: "relayout", page: OG }]);
    assert.match(readFileSync(calls, "utf8"), /^logic compile --relayout Obergeschoss$/m);
  });
  await step("undo moves the arranged block back, redo arranges it again", async () => {
    const at = ([u, r0, x]) => S.rev !== r0 && curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect[0] === x;
    let ops = await undoKey(false, at, [FLUR, await rev(), 7104]);
    assert.deepEqual(ops, [{ op: "place", moves: [{ uuid: FLUR, x: 7104, y: 2880 }] }]);
    assert.equal(flurLock().layout.px, 7104);
    ops = await undoKey(true, at, [FLUR, await rev(), 9600]);
    assert.deepEqual(ops, [{ op: "place", moves: [{ uuid: FLUR, x: 9600, y: 960 }] }]);
    assert.equal(flurLock().layout.px, 9600);
  });
  await step("Arrange on a page of 210 managed blocks is undone in one op", async () => {
    await page.evaluate((pg) => gotoPage(pg), LAGER);
    await page.waitForSelector("#btn-relayout:not([disabled])");
    const n = edits.length, r0 = await rev(), last = lagerUuid(LAGER_N - 1);
    const rectOf = (u) => page.evaluate((u) => curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect.slice(0, 2), u);
    const shows = ([u, r0, x, y]) => S.rev !== r0 && S.viewRev === S.rev && S.rev === editRev()?.id && !S.mvBusy && !UNDO.busy &&
      (([a, b]) => a === x && b === y)(curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect || []);
    const moved = ([u, r0, x]) => S.rev !== r0 && S.viewRev === S.rev && S.rev === editRev()?.id && !S.mvBusy &&
      curPage()?.objects.find((o) => o.uuid === u && !o.moved_to)?.rect[0] !== x;
    await page.click("#btn-relayout");
    await page.waitForFunction(moved, [last, r0, lagerAt(LAGER_N - 1).px], { timeout: 15000 });
    assert.deepEqual(edits[n].ops, [{ op: "relayout", page: LAGER }]);
    const arranged = await rectOf(last);
    let ops = await undoKey(false, shows, [last, await rev(), lagerAt(LAGER_N - 1).px, lagerAt(LAGER_N - 1).py]);
    assert.equal(ops.length, 1, "one op, not one per block");
    assert.equal(ops[0].op, "place");
    assert.equal(ops[0].moves.length, LAGER_N);
    assert.deepEqual(await rectOf(lagerUuid(7)), [lagerAt(7).px, lagerAt(7).py]);
    ops = await undoKey(true, shows, [last, await rev(), ...arranged]);
    assert.equal(ops.length, 1);
    assert.equal(ops[0].moves.length, LAGER_N);
    // Back where it was, for the steps after.
    await undoKey(false, shows, [lagerUuid(0), await rev(), lagerAt(0).px, lagerAt(0).py]);
  });
  await step("a block Loxone Config drew is not moved: dragging it pans the page", async () => {
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, ["10000002-0000-0020-ffff504f94000001", DIV]);
    const g = page.locator(`#canvas g[data-uuid="${DIV}"]`).first();
    await g.waitFor();
    assert.equal(await page.locator(`#canvas g.movable[data-uuid="${DIV}"]`).count(), 0);
    // A page without managed blocks has nothing to arrange.
    assert.equal(await page.isDisabled("#btn-relayout"), true);
    const n = edits.length, vx = await page.evaluate(() => S.vp[S.page].x);
    const box = await g.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 80, box.y + box.height / 2, { steps: 8 });
    await page.mouse.up();
    assert.notEqual(await page.evaluate(() => S.vp[S.page].x), vx);
    assert.equal(edits.length, n);
  });
  // Wires (#61): drawn by dragging from a port to a port, written as `to <- from` into vislox's file.
  const portBox = async (uuid, key) => {
    const b = await page.locator(`#canvas .port-hit[data-port="${uuid}|${key}"]`).boundingBox();
    return [b.x + b.width / 2, b.y + b.height / 2];
  };
  const dragTo = async ([x0, y0], [x1, y1]) => {
    await page.mouse.move(x0, y0);
    await page.mouse.down();
    await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 6 });
    await page.mouse.move(x1, y1, { steps: 6 });
    await page.mouse.up();
  };
  // A wire's ends as the page draws it, once the newest pending revision shows it (or not).
  const wireShown = (from, to, there) => page.waitForFunction(([f, t, there]) => S.viewRev === S.rev && S.rev?.startsWith("pending:") && !S.wrBusy &&
    curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there, [from, to, there], { timeout: 10000 });
  /** Click a wire: its hit path, halfway along it (port targets drawn over a wire's ends may cover the
   *  point a mouse would hit, so the click goes to the path itself, mouse down and up first). */
  const clickWire = async (from, to) => {
    await page.waitForFunction(() => document.querySelector("#canvas .port-hit") && !S.wrBusy, null, { timeout: 10000 });
    await page.evaluate(([f, t]) => {
      const line = [...document.querySelectorAll("#canvas path.wire")].find((l) => l.dataset.f === f[0] && l.dataset.t === t[0] && !l.classList.contains("d-removed"));
      const hit = line.nextElementSibling, len = hit.getTotalLength(), pt = hit.getPointAtLength(len / 2);
      const m = hit.getScreenCTM();
      const at = { bubbles: true, clientX: m.a * pt.x + m.e, clientY: m.d * pt.y + m.f };
      for (const t of ["mousedown", "mouseup", "click"]) hit.dispatchEvent(new MouseEvent(t, at));
    }, [from, to]);
    await page.waitForSelector("#wcard:not(.hidden) .wc-head");
  };
  const wireFile = () => existsSync(join(dir, "pages/vislox/edits.loxl")) ? readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8") : "";
  let target;
  await step("a wire is drawn by dragging from an output to an input, and shows as pending", async () => {
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, [EG, DIV]);
    await page.waitForSelector(`#canvas .port-hit[data-port="${DIV}|AQ"]`);
    const n = edits.length;
    await dragTo(await portBox(DIV, "AQ"), await portBox(AND, "I2"));
    await wireShown([DIV, "AQ"], [AND, "I2"], true);
    assert.deepEqual(edits[n].ops, [{ op: "wire", from: [DIV, "AQ"], to: [AND, "I2"] }]);
    assert.match(wireFile(), /^vx_10000004_0000_0040_ffff504f94000001\.In2 <- vx_10000006_0000_0060_ffff504f94000001\.AQ$/m);
    await waitText("#deploy-pend", /1 wire drawn here/);
    await waitText(".changes", /\+Teiler\.AQ → O1001\.In2/);
    await page.waitForFunction(() => curPage().wires.some((w) => w.status === "added"));
  });
  await step("undo takes the drawn wire out, redo draws it again", async () => {
    const has = ([f, t, there]) => S.viewRev === S.rev && curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there;
    let ops = await undoKey(false, has, [[DIV, "AQ"], [AND, "I2"], false]);
    assert.deepEqual(ops, [{ op: "unwire", from: [DIV, "AQ"], to: [AND, "I2"] }]);
    assert.doesNotMatch(wireFile(), /<-/);
    ops = await undoKey(true, has, [[DIV, "AQ"], [AND, "I2"], true]);
    assert.deepEqual(ops, [{ op: "wire", from: [DIV, "AQ"], to: [AND, "I2"] }]);
    assert.match(wireFile(), /\.In2 <- /);
  });
  await step("a link Loxone Config would not have is refused in the browser, with the reason", async () => {
    const n = edits.length;
    // The AND gate already feeds the meter.
    await dragTo(await portBox(AND, "Q"), await portBox(METER, "Pf"));
    await waitText("#wcard .wc-msg.err", /already wired/);
    // An output onto an output is not even a target.
    assert.equal(await page.locator(`#canvas .port-hit[data-port="${METER}|Pf"]`).count(), 1);
    assert.equal(edits.length, n);
    await page.keyboard.press("Escape");
    await page.waitForSelector("#wcard.hidden", { state: "attached" });
  });
  await step("a click on a port, without a drag, selects its block", async () => {
    const n = edits.length;
    await page.evaluate(() => select(null));
    const [x, y] = await portBox(METER, "Pf");
    await page.mouse.click(x, y);
    await page.waitForFunction((u) => S.sel === u, METER);
    assert.equal(edits.length, n);
    assert.equal(await page.locator("#canvas .wire-draft").count(), 0);
  });
  await step("a drop while another edit compiles says so, and the card does not hang", async () => {
    const n = edits.length;
    const [x0, y0] = await portBox(AND, "Q"), [x1, y1] = await portBox(DIV, "Input2");
    await page.mouse.move(x0, y0);
    await page.mouse.down();
    await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 6 });
    await page.mouse.move(x1, y1, { steps: 6 });
    // A move starts compiling before the mouse is let go.
    await page.evaluate(() => { S.mvBusy = true; });
    await page.mouse.up();
    await waitText("#wcard .wc-msg.err", /Another edit is compiling/);
    assert.equal(edits.length, n);
    assert.equal(await page.locator("#canvas .wire-draft").count(), 0);
    await page.evaluate(() => { S.mvBusy = false; renderPage(); });
    await page.click('#wcard [data-wc="close"]');
    await page.waitForSelector("#wcard.hidden", { state: "attached" });
  });
  await step("a wire Loxone Config drew says so, and offers no Delete", async () => {
    await clickWire([AND, "Q"], [METER, "Pf"]);
    await waitText("#wcard", /Drawn in Loxone Config/);
    assert.equal(await page.locator('#wcard [data-wc="unwire"]').count(), 0);
    await page.click('#wcard [data-wc="close"]');
  });
  await step("an end of a wire drawn here is dragged to another input: reconnected in one edit", async () => {
    await clickWire([DIV, "AQ"], [AND, "I2"]);
    await waitText("#wcard", /drawn here\s*not deployed/);
    // An unwired input of the meter that the page draws.
    target = await page.evaluate((u) => {
      const o = curPage().objects.find((o) => o.uuid === u), wired = L.wired.get(u);
      return o.ports.find((q) => q.side === "L" && !wired?.has(q.key)).key;
    }, METER);
    const n = edits.length;
    const h = await page.locator('#canvas .wire-end[data-end="to"]').boundingBox();
    await dragTo([h.x + h.width / 2, h.y + h.height / 2], await portBox(METER, target));
    await wireShown([DIV, "AQ"], [METER, target], true);
    assert.deepEqual(edits[n].ops, [
      { op: "unwire", from: [DIV, "AQ"], to: [AND, "I2"] },
      { op: "wire", from: [DIV, "AQ"], to: [METER, target] },
    ]);
    await wireShown([DIV, "AQ"], [AND, "I2"], false);
    assert.doesNotMatch(wireFile(), /In2 <-/);
  });
  await step("undo of a reconnect is one edit that puts the old wire back", async () => {
    const has = ([f, t, there]) => S.viewRev === S.rev && curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there;
    assert.match(await page.getAttribute("#btn-undo", "title"), /^Undo: Reconnect /);
    let ops = await undoKey(false, has, [[DIV, "AQ"], [AND, "I2"], true]);
    assert.deepEqual(ops, [
      { op: "unwire", from: [DIV, "AQ"], to: [METER, target] },
      { op: "wire", from: [DIV, "AQ"], to: [AND, "I2"] },
    ]);
    await wireShown([DIV, "AQ"], [METER, target], false);
    ops = await undoKey(true, has, [[DIV, "AQ"], [METER, target], true]);
    assert.deepEqual(ops, [
      { op: "unwire", from: [DIV, "AQ"], to: [AND, "I2"] },
      { op: "wire", from: [DIV, "AQ"], to: [METER, target] },
    ]);
    await wireShown([DIV, "AQ"], [AND, "I2"], false);
  });
  await step("Delete takes a wire drawn here out again", async () => {
    await clickWire([DIV, "AQ"], [METER, target]);
    const n = edits.length;
    await page.keyboard.press("Delete");
    await wireShown([DIV, "AQ"], [METER, target], false);
    assert.deepEqual(edits[n].ops, [{ op: "unwire", from: [DIV, "AQ"], to: [METER, target] }]);
    assert.doesNotMatch(wireFile(), /<-/);
    await page.waitForFunction(() => !/wire/.test(document.querySelector("#deploy-pend")?.textContent || ""));
  });
  await step("the undo history survives a reload: Delete is undone after it", async () => {
    const has = ([f, t, there]) => S.viewRev === S.rev && curPage()?.wires.some((w) => w.from.join() === f.join() && w.to.join() === t.join() && w.status !== "removed") === there;
    await page.reload();
    await page.waitForSelector(".tab");
    await page.waitForFunction(() => UNDO.blob && S.view && S.viewRev === S.rev, null, { timeout: 10000 });
    assert.match(await page.evaluate(() => UNDO.done.at(-1)?.label || ""), /^Delete wire /);
    await page.evaluate(([pg, u]) => { gotoPage(pg); focusObj(u, 0.05); }, [EG, DIV]);
    let ops = await undoKey(false, has, [[DIV, "AQ"], [METER, target], true]);
    assert.deepEqual(ops, [{ op: "wire", from: [DIV, "AQ"], to: [METER, target] }]);
    ops = await undoKey(true, has, [[DIV, "AQ"], [METER, target], false]);
    assert.deepEqual(ops, [{ op: "unwire", from: [DIV, "AQ"], to: [METER, target] }]);
  });
  await step("an older revision shows no managed blocks: the lockfile is today's", async () => {
    const pending = await page.evaluate(() => editRev().id);
    assert.match(pending, /^pending:/);
    const deployed = await page.evaluate(() => S.hist.history[0].id);
    await page.selectOption("#sel-rev", deployed);
    await page.waitForFunction((r) => S.viewRev === r, deployed, { timeout: 10000 });
    await page.evaluate((u) => gotoPage(S.objPage.get(u), { focus: u }), FLUR);
    await page.waitForSelector(`.blk[data-uuid="${FLUR}"]`);
    assert.equal(await page.$(".mbadge"), null);
    assert.equal(await page.$("#legend.managed"), null);
    // Back on the newest config, following it again.
    await page.selectOption("#sel-rev", pending);
    await page.evaluate(() => { const f = $("chk-follow"); f.checked = true; f.dispatchEvent(new Event("change")); });
    await page.waitForFunction((r) => S.follow && S.viewRev === r, pending, { timeout: 10000 });
    await page.evaluate((u) => gotoPage(S.objPage.get(u), { focus: u }), FLUR);
    await page.waitForSelector(`.blk.managed[data-uuid="${FLUR}"] .mbadge`);
    await page.waitForSelector("#legend.managed");
  });
  await step("Discard drops the edits", async () => {
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(existsSync(join(dir, "pages/vislox")), false);
    await page.waitForFunction(() => !document.querySelector("#props .ed-tag"));
  });
  await step("Discard clears the undo history and says why", async () => {
    await page.waitForFunction(() => /Discard dropped the pending config/.test(UNDO.note) && !UNDO.done.length, null, { timeout: 10000 });
    await page.waitForSelector("#btn-undo[disabled]");
    assert.match(await page.getAttribute("#btn-undo", "title"), /Nothing to undo\nUndo history cleared: Discard dropped/);
    await page.click("#btn-undo-list");
    await waitText("#undo-pop .u-note", /Discard dropped the pending config/);
    await page.click("#btn-undo-list");
    // The key sends nothing now.
    const n = edits.length;
    await page.keyboard.press("Control+z");
    await page.waitForTimeout(500);
    assert.equal(edits.length, n);
  });
  await step("a compile made outside this tab clears it too", async () => {
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    await page.fill(field("title"), "Außen");
    await page.press(field("title"), "Enter");
    await page.waitForFunction(() => UNDO.done.length === 1 && S.rev?.startsWith("pending:") && S.viewRev === S.rev, null, { timeout: 10000 });
    // Someone compiles the module elsewhere: the pending config is another one.
    writeFileSync(join(dir, "out.Loxone"), readFileSync(join(dir, "out.Loxone"), "utf8") + "\n<!-- compiled elsewhere -->\n");
    await page.waitForFunction(() => /changed outside this tab/.test(UNDO.note) && !UNDO.done.length, null, { timeout: 15000 });
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(existsSync(join(dir, "pages/vislox")), false);
  });
  await step("a module that does not compile is listed, error by error, and Apply is off (#65)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    writeFileSync(flur, was + "flur.In1 <- BROKEN.Q\n");
    await waitText("#deploy-check", /The module doesn't compile: 1 error/);
    assert.match(await page.textContent("#deploy-check li"), /pages\/flur\.loxl:4unknown block type BROKEN/);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), true);
    // The error's line names `flur`: its button shows the block, and Properties says why.
    await page.click('#deploy-check .go:has-text("Flur")');
    await page.waitForFunction((u) => S.sel === u, FLUR);
    await waitText("#props", /Doesn't compile \(pages\/flur\.loxl:4\): unknown block type BROKEN/);
    // Fixed, but not compiled: the module changed since out, and Compile compiles it.
    writeFileSync(flur, was.replace('And("Flur")', 'And("Flur oben")'));
    await waitText("#deploy-check", /The module changed \(pages\/flur\.loxl\) and is not compiled yet/);
    await page.click("#btn-compile");
    await waitText("#deploy-pend", /Compiled changes are pending/);
    await page.waitForSelector("#deploy-check.hidden", { state: "attached" });
    assert.match(readFileSync(join(dir, "out.Loxone"), "utf8"), /Title="Flur oben"/);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), false);
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(readFileSync(flur, "utf8"), was);
  });
  await step("lint findings are marked on their blocks, listed, and on the Code Touch (#66)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    writeFileSync(flur, was.replace('flur = And("Flur")', 'flur = And("Flur") # dead') + "# NOTAG\n");
    await waitText("#deploy-lint", /Lint: 2 findings/);
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); }, FLUR);
    await page.waitForSelector(`.blk.linted[data-uuid="${FLUR}"] .lbadge`);
    assert.match(await page.textContent(`.blk[data-uuid="${FLUR}"] .lbadge title`), /Lint \(dead-outputs\): no wire draws/);
    assert.equal(await page.$(`.blk[data-uuid="${MONO}"] .lbadge`), null);
    // The list, by kind: each finding with its file and line, and a button that shows its block.
    await page.click("#deploy-lint summary");
    assert.match(await page.textContent("#deploy-lint"), /dead-outputs \(1\)[\s\S]*pages\/flur\.loxl:3flur: no wire draws/);
    assert.match(await page.textContent("#deploy-lint"), /no-access-right \(1\)[\s\S]*Anna: user with an NFC tag/);
    await page.click('#deploy-lint .go:has-text("Flur")');
    await page.waitForFunction((u) => S.sel === u, FLUR);
    await waitText('[data-body="Lox Logic"]', /Lint \(dead-outputs, pages\/flur\.loxl:3\): no wire draws/);
    // A hint only: Apply is not turned off for it.
    assert.doesNotMatch(await page.getAttribute("#btn-apply", "title"), /lint/i);
    // The Code Touch says whose tag opens nothing.
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); select(u); }, TOUCH);
    await waitText('[data-body="Lox Logic"]', /Lint \(no-access-right\): Anna, user with an NFC tag has no access right/);
    // Fixed: the badge and the list go.
    writeFileSync(flur, was);
    await page.waitForSelector("#deploy-lint.hidden", { state: "attached" });
    await page.evaluate((u) => { gotoPage(S.objPage.get(u), { focus: u }); }, FLUR);
    await page.waitForSelector(`.blk[data-uuid="${FLUR}"]`);
    assert.equal(await page.$(`.blk[data-uuid="${FLUR}"] .lbadge`), null);
    await page.waitForSelector("#deploy-check.hidden", { state: "attached" });
  });
  await step("a failing test is listed with expected and actual, marked on its block, and Apply is off (#67)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    // A test of `flur`, and a rename, so that Compile makes something pending.
    const withTest = (mark) => was.replace('And("Flur")', 'And("Flur getestet")') +
      `\ntest "Flur schaltet"\n\tflur.In1 = 1\n\tflur.In2 = 1\n\ttick 1s\n\texpect flur.Q == 1 # ${mark}\nend\n`;
    writeFileSync(flur, withTest("FAILTEST"));
    await waitText("#deploy-check", /The module changed \(pages\/flur\.loxl\) and is not compiled yet/);
    await page.click("#btn-compile");
    await waitText("#deploy-pend", /Compiled changes are pending/);
    // The final state: the tests of these files ran, and failed.
    await page.waitForFunction(() => S.check?.current && S.check.test?.available && !S.check.test.running && S.check.test.failed === 1, null, { timeout: 15000 });
    await waitText("#deploy-tests", /Tests: 0 of 1 passed, 1 failed/);
    // Open while it fails: the test with its file and line, the expect with expected and actual, and its block.
    assert.match(await page.textContent("#deploy-tests"), /pages\/flur\.loxl:5Flur schaltet[\s\S]*pages\/flur\.loxl:9step 1: expect flur\.Q == 1 — expected 1, actual 0/);
    assert.match(await page.textContent("#deploy-tests"), /A pass covers only what the simulator models/);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), true);
    assert.match(await page.getAttribute("#btn-apply", "title"), /Cannot apply: 1 test of lox logic test fails/);
    await page.click('#deploy-tests .go:has-text("Flur getestet")');
    await page.waitForFunction((u) => S.sel === u, FLUR);
    await page.waitForSelector(`.blk.test-failed[data-uuid="${FLUR}"] .tbadge`);
    assert.match(await page.textContent(`.blk[data-uuid="${FLUR}"] .tbadge title`), /Test "Flur schaltet" \(pages\/flur\.loxl:5\) fails: step 1: expect flur\.Q == 1/);
    await waitText('[data-body="Lox Logic"]', /✕ Test "Flur schaltet" \(pages\/flur\.loxl:5\) fails/);
    // Fixed: it passes, the badge goes, and Apply is on again (the module changed since the compile, which
    // Apply only warns about).
    writeFileSync(flur, withTest("fixed"));
    await page.waitForFunction(() => S.check?.current && S.check.test?.available && !S.check.test.running && S.check.test.passed === 1, null, { timeout: 15000 });
    await waitText("#deploy-tests", /Tests: 1 of 1 passed/);
    await page.waitForFunction((u) => !document.querySelector(`.blk[data-uuid="${u}"] .tbadge`), FLUR);
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), false);
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(readFileSync(flur, "utf8"), was);
    // No tests: the box goes.
    await page.waitForSelector("#deploy-tests.hidden", { state: "attached", timeout: 15000 });
  });
  await step("a block the module lost: the Deploy pane says so, and its removal is reviewed before Apply (#70)", async () => {
    const flur = join(dir, "pages", "flur.loxl"), was = readFileSync(flur, "utf8");
    const lockWas = readFileSync(join(dir, "logic.lock.json"), "utf8"), outWas = readFileSync(join(dir, "out.Loxone"), "utf8");
    writeFileSync(flur, was.replace('flur = And("Flur")\n', ""));
    await waitText("#deploy-check", /The module no longer has managed blocks: lox deletes them only when you allow it/);
    assert.match(await page.textContent("#deploy-check"), /flur is in the lockfile but not in the source/);
    // Apply is on: it asks for exactly these removals first.
    assert.equal(await page.evaluate(() => document.querySelector("#btn-apply").disabled), false);
    await page.click("#btn-removals");
    // What Apply would deploy, compared with what runs, and the block: where it is and what is wired to it.
    await page.waitForSelector(".hrev.k-removals.sel");
    await waitText("#deploy-check", /Flur \(And, flur\) on page "Obergeschoss": nothing wired/);
    await page.waitForFunction(() => S.viewRev === S.rev && S.view?.compared);
    assert.match(await page.textContent(".changes"), /Flur/);
    // Its button shows the block, marked as removed in the comparison.
    await page.click('#deploy-check .rm-list .go:has-text("Flur")');
    await page.waitForFunction((u) => S.sel === u && S.view.objects[u]?.status === "removed", FLUR);
    // Cancelled from another tab: the server forgets the plan, and this tab's preview goes with it.
    const other = await browser.newPage();
    await other.goto(page.url());
    const rev = await page.evaluate(() => S.removals.rev);
    await other.evaluate((r) => fetch("/api/removals/cancel", { method: "POST", headers: { "Content-Type": "application/json", "X-Vislox": "removals" }, body: JSON.stringify({ rev: r }) }), rev);
    await other.close();
    await page.waitForFunction(() => !S.removals && !document.querySelector(".hrev.k-removals"), null, { timeout: 10000 });
    await waitText("#deploy-check", /Review the removals/);
    // The review changed nothing in the project.
    assert.equal(readFileSync(join(dir, "logic.lock.json"), "utf8"), lockWas);
    assert.equal(readFileSync(join(dir, "out.Loxone"), "utf8"), outWas);
    // Declared again: nothing to remove.
    writeFileSync(flur, was);
    await page.waitForSelector("#deploy-check.hidden", { state: "attached", timeout: 15000 });
    await page.click(".hrev.k-git >> nth=0");
  });
  await step("nothing was deployed: lox only compiled, checked, linted and tested", async () => {
    const ran = readFileSync(calls, "utf8").trim().split("\n");
    assert.ok(ran.length >= 2, ran.join(" | "));
    assert.ok(ran.every((l) => /^logic (compile|check --json|fmt --check|lint \.|test \.)( |$)/.test(l)), ran.join(" | "));
  });
  await step("no errors in the browser", async () => {
    assert.deepEqual(errors, []);
  });
} catch (e) {
  failed = true;
  console.error("FAILED:", e.message);
  if (errors.length) console.error("browser errors:", errors);
  if (page) console.error("props:", await page.textContent("#props").catch(() => "?"));
  if (page) console.error("check:", await page.evaluate(() => JSON.stringify(S.check)).catch(() => "?"));
  if (page) console.error("server check:", await page.evaluate(async () => JSON.stringify([S.checkKey, S.hist?.actions?.edit, await (await fetch("/api/check")).json()])).catch(() => "?"));
  if (page) console.error("add block:", await page.textContent("#ab-body").catch(() => "?"));
  if (page) console.error("wire card:", await page.textContent("#wcard").catch(() => "?"), "| deploy:", await page.textContent("#deploy-msg").catch(() => "?"));
  console.error(log);
} finally {
  await browser.close();
  server.kill("SIGINT");
}
process.exit(failed ? 1 : 0);
