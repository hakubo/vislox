// Browser test for editing values in Properties: a lox.toml project with nothing pending, and a
// stand-in lox whose `logic compile` writes the values vislox's file claims into out.Loxone. An edit
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
\t\t\t</C>`);
writeFileSync(join(dir, "base.Loxone"), base);
writeFileSync(join(dir, "out.Loxone"), base);
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\n');
writeFileSync(join(dir, "pages", "README"), "the module\n");
git("init", "-q");
git("add", "-A");
git("commit", "-q", "-m", "Deployed");

// The stand-in lox. `logic compile` reads vislox's claims (extern …(uuid: "…"), x.title = "…",
// x.Port = n) and writes them into a copy of the base, as lox would.
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const calls = join(loxDir, "calls.log");
const fakeLox = join(loxDir, "lox");
writeFileSync(fakeLox, `#!/usr/bin/env node
const fs = require("fs");
fs.appendFileSync(${JSON.stringify(calls)}, process.argv.slice(2).join(" ") + "\\n");
if (process.argv.slice(2, 4).join(" ") !== "logic compile") { console.error("unexpected: " + process.argv.slice(2).join(" ")); process.exit(2); }
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
  const key = KEYS[m[2]] || m[2];
  const j = out.findIndex((l, n) => n > i && l.includes(\`<Co K="\${key}"\`));
  out[j] = out[j].replace(/ Def="[^"]*"/, "").replace(\`<Co K="\${key}"\`, \`<Co K="\${key}" Def="\${unq(m[3])}"\`);
}
fs.writeFileSync("out.Loxone", out.join("\\n"));
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
  page.on("request", (r) => { if (new URL(r.url()).pathname === "/api/edit") edits.push(JSON.parse(r.postData())); });
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

  await step("nothing is pending, and Properties offers to edit the Divide block", async () => {
    await page.waitForSelector("#deploy-pane:not(.hidden)");
    assert.match(await page.textContent("#deploy-pend"), /Nothing pending\. Edit values in Properties/);
    await pick(DIV);
    await page.click('#props [data-ed="open"]');
    // Its name and its divisor; the dividend is wired.
    assert.deepEqual(await page.$$eval("#props [data-ed-field]", (n) => n.map((x) => x.dataset.edField)), ["title", "p:Input2"]);
    assert.equal(await page.inputValue(field("p:Input2")), "1000");
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
    await page.fill(field("p:Input2"), "100");
    await page.press(field("p:Input2"), "Enter");
    await page.waitForFunction(() => S.rev?.startsWith("pending:") && S.view?.objects["10000006-0000-0060-ffff504f94000001"]?.title === "Teiler", null, { timeout: 10000 });
    assert.equal(edits.length, 1);
    assert.deepEqual(edits[0].ops, [
      { op: "title", uuid: DIV, value: "Teiler" },
      { op: "param", uuid: DIV, port: "Input2", value: "100" },
    ]);
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 2);
    assert.match(await page.textContent("#props"), /Saved and compiled: pending, not deployed/);
    assert.match(await page.textContent("#deploy-pend"), /not deployed\. They include 2 values edited here/);
    await page.waitForSelector("#btn-apply:not([disabled])");
    const changes = await page.$$eval(".changes .chg .t", (n) => n.map((x) => x.textContent).join(" | "));
    assert.match(changes, /Teiler/, changes);
    assert.match(changes, /1000 → 100/, changes);
    const file = readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8");
    assert.match(file, /^extern vx_10000006_0000_0060_ffff504f94000001 = Div\(uuid: "10000006-0000-0060-ffff504f94000001"\)$/m);
    assert.match(file, /\.In2 = 100$/m);
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
    await page.click('#props .pr:has-text("In2") [data-ed="revert"]');
    await page.waitForFunction(() => document.querySelectorAll("#props .ed-tag").length === 1);
    assert.deepEqual(edits[n].ops, [{ op: "param", uuid: DIV, port: "Input2", value: null }]);
    assert.doesNotMatch(readFileSync(join(dir, "pages/vislox/edits.loxl"), "utf8"), /In2/);
  });
  await step("Discard drops the edits", async () => {
    await page.click("#btn-discard");
    await page.waitForFunction(() => /Nothing pending/.test(document.querySelector("#deploy-pend")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(existsSync(join(dir, "pages/vislox")), false);
    await page.waitForFunction(() => !document.querySelector("#props .ed-tag"));
  });
  await step("nothing was deployed: lox only compiled", async () => {
    const ran = readFileSync(calls, "utf8").trim().split("\n");
    assert.ok(ran.length >= 2, ran.join(" | "));
    assert.ok(ran.every((l) => l === "logic compile"), ran.join(" | "));
  });
  await step("no errors in the browser", async () => {
    assert.deepEqual(errors, []);
  });
} catch (e) {
  failed = true;
  console.error("FAILED:", e.message);
  if (errors.length) console.error("browser errors:", errors);
  if (page) console.error("props:", await page.textContent("#props").catch(() => "?"));
  console.error(log);
} finally {
  await browser.close();
  server.kill("SIGINT");
}
process.exit(failed ? 1 : 0);
