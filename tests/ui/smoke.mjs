// Browser smoke test: run vislox on a git repo built from the test fixture,
// with an uncommitted edit, and drive the UI the way a reviewer would.
//
//   VISLOX_BIN=target/release/vislox node tests/ui/smoke.mjs
//
// CHROMIUM_PATH picks a browser other than Playwright's own.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, appendFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const bin = process.env.VISLOX_BIN || join(here, "../../target/release/vislox");
const fixture = join(here, "../fixtures/house.Loxone");

// A `lox config init` style repository: <serial>/config.Loxone, one commit.
const repo = mkdtempSync(join(tmpdir(), "vislox-smoke-"));
const git = (...a) => execFileSync("git", ["-C", repo, "-c", "user.name=t", "-c", "user.email=t@example.com", ...a]);
git("init", "-q");
mkdirSync(join(repo, "504F94000001"));
const cfg = join(repo, "504F94000001", "config.Loxone");
copyFileSync(fixture, cfg);
git("add", ".");
git("commit", "-q", "-m", "[504F94000001] Config backup 2026-11-01 10:00:00 (v1)");
// Uncommitted edits: a rename and a parameter.
const edit = (from, to) => writeFileSync(cfg, readFileSync(cfg, "utf8").replace(from, to));
edit('Title="Licht OG"', 'Title="Licht oben"');
edit('K="Input2" Def="1000"', 'K="Input2" Def="100"');

// Live values as `lox stream -o json --initial` writes them: the fixture's
// temperature (an analog output) and its AND gate (a digital one).
const TEMP = "10000003-0000-0032-01ff504f94000001", GATE = "10000004-0000-0043-02ff504f94000001";
const liveFile = join(mkdtempSync(join(tmpdir(), "vislox-live-")), "stream.ndjson");
const liveLine = (control, state, value, uuid) => JSON.stringify({ control, state, value, uuid }) + "\n";
writeFileSync(liveFile, liveLine("Temperatur", "value", "21.5", TEMP) + liveLine("AND", "active", "1", GATE));

const port = 7900 + Math.floor(Math.random() * 90);
const server = spawn(bin, [repo, "--listen", `127.0.0.1:${port}`, "--live-from", liveFile], { stdio: ["ignore", "ignore", "pipe"] });
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
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  const url = `http://127.0.0.1:${port}/`;
  await page.goto(url);
  await page.waitForSelector(".tab");

  const step = async (name, fn) => { await fn(); console.log("ok -", name); };

  await step("both pages are tabs", async () => {
    assert.deepEqual(await page.$$eval(".tab", (t) => t.map((x) => x.textContent.replace(/\d+$/, "").trim())), ["Erdgeschoss", "Obergeschoss"]);
  });
  await step("the uncommitted edit is compared with the last commit", async () => {
    assert.match(await page.textContent(".hrev.sel"), /Changed on disk, not committed/);
    await page.waitForSelector(".changes .chg");
    const changes = await page.$$eval(".changes .chg .t", (n) => n.map((x) => x.textContent));
    assert.ok(changes.some((t) => t.includes("Licht OG → Licht oben")), changes.join(" | "));
    assert.ok(changes.some((t) => t.includes("1000 → 100")), changes.join(" | "));
  });
  await step("blocks are drawn", async () => {
    assert.ok((await page.$$("#viewport .blk")).length >= 3);
  });
  await step("next change opens its block in Properties", async () => {
    await page.focus("#canvas");
    await page.keyboard.press("]");
    await page.waitForSelector(".ph-t");
    assert.match(await page.textContent("#chg-pos"), /^1 \/ \d+$/);
  });
  await step("Ctrl+K finds a block and goes to its page", async () => {
    await page.keyboard.press("Control+k");
    await page.keyboard.type("licht oben");
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Licht oben");
    assert.equal(await page.textContent('.tab[aria-selected="true"]').then((t) => t.replace(/\d+$/, "").trim()), "Obergeschoss");
  });
  await step("a link reopens the same page and block", async () => {
    const link = page.url();
    assert.match(link, /#page=.*&sel=/);
    const again = await browser.newPage();
    again.on("pageerror", (e) => errors.push(e.message));
    await again.goto(link);
    await again.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Licht oben");
    await again.close();
  });
  await step("live values go on the wires and follow the stream", async () => {
    await page.click('.tab:has-text("Erdgeschoss")');
    await page.click("#sw-live .track");
    const pill = (t) => page.waitForFunction((t) => [...document.querySelectorAll(".live-pill")].some((n) => n.style.display !== "none" && n.textContent === t), t);
    await pill("21.5");
    assert.ok((await page.$$("#viewport .wire.lv-on")).length >= 1, "the AND gate's wire is on");
    appendFileSync(liveFile, liveLine("Temperatur", "value", "22.25", TEMP));
    await pill("22.25");
    await page.click("#sw-live .track");
    await page.waitForFunction(() => !document.querySelector("#canvas").classList.contains("live"));
  });
  await step("an edit on disk shows up live", async () => {
    const before = (await page.$$(".hrev")).length;
    edit('Title="Zähler"', 'Title="Stromzähler"');
    await page.waitForFunction((n) => document.querySelectorAll(".hrev").length > n, before, { timeout: 10000 });
  });
  await step("keyboard help opens and closes", async () => {
    await page.focus("#canvas");
    await page.keyboard.press("?");
    await page.waitForSelector("#help:not(.hidden)");
    await page.keyboard.press("Escape");
    await page.waitForSelector("#help.hidden", { state: "attached" });
  });
  await step("no errors in the browser", async () => {
    assert.deepEqual(errors, []);
  });
} catch (e) {
  failed = true;
  console.error("FAILED:", e.message);
  if (errors.length) console.error("browser errors:", errors);
} finally {
  await browser.close();
  server.kill("SIGINT");
}
process.exit(failed ? 1 : 0);
