// Browser smoke test: run vislox on a git repo built from the test fixture,
// with an uncommitted edit, and drive the UI the way a reviewer would.
//
//   VISLOX_BIN=target/release/vislox node tests/ui/smoke.mjs
//
// CHROMIUM_PATH picks a browser other than Playwright's own.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { createServer as createTcpServer } from "node:net";
import { mkdtempSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, appendFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { crc32 } from "node:zlib";
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

// A download as lox keeps it next to config.Loxone (backup.zip): only its
// LoxAPP3.json matters here, for when the config last changed (stored, not deflated).
function backupZip(lastModified) {
  const name = Buffer.from("LoxAPP3.json");
  const data = Buffer.from(`\ufeff{"lastModified":"${lastModified}","msInfo":{}}`);
  const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
  const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n >>> 0); return b; };
  const common = [u16(20), u16(0), u16(0), u16(0), u16(0), u32(crc32(data)), u32(data.length), u32(data.length), u16(name.length), u16(0)];
  const local = Buffer.concat([u32(0x04034b50), ...common, name, data]);
  const central = Buffer.concat([u32(0x02014b50), u16(20), ...common, u16(0), u16(0), u16(0), u32(0), u32(0), name]);
  const end = Buffer.concat([u32(0x06054b50), u16(0), u16(0), u16(1), u16(1), u32(central.length), u32(local.length), u16(0)]);
  return Buffer.concat([local, central, end]);
}
const backup = join(dirname(cfg), "backup.zip");
writeFileSync(backup, backupZip("2026-11-01 10:00:00"));

// A stand-in Miniserver for the status chip and the preflight: jdev/cfg/apiKey
// and jdev/sps/LoxAPPversion3 over HTTP, and an FTP server that only greets
// (vislox reads the greeting).
let msConfigDate = "2026-11-01 10:00:00";
const listen = (srv) => new Promise((ok) => srv.listen(0, "127.0.0.1", () => ok(srv.address().port)));
// `msSerial` is what it reports; a test swaps it for another Miniserver's.
let msSerial = "50:4F:94:00:00:01";
const msPort = await listen(createServer((req, res) => {
  const ll = {
    "/jdev/cfg/apiKey": () => ({ control: "dev/cfg/apiKey", Code: "200",
      value: `{'snr': '${msSerial}', 'version':'17.1.7.27', 'key':'00', 'local':true, 'httpsStatus':0}` }),
    "/jdev/sps/LoxAPPversion3": () => ({ control: "dev/sps/LoxAPPversion3", Code: "200", value: msConfigDate }),
  }[req.url];
  if (!ll) { res.writeHead(404); res.end(); return; }
  res.setHeader("Content-Type", "application/json");
  res.end(JSON.stringify({ LL: ll() }));
}));
const ftpSockets = new Set();
const ftp = createTcpServer((s) => { ftpSockets.add(s); s.on("close", () => ftpSockets.delete(s)); s.on("error", () => {}); s.end("220 fake FTP\r\n"); });
const ftpPort = await listen(ftp);

// "Pull from Miniserver" runs `lox config pull`. A lox configuration that pulls
// into this repository, and a stand-in lox that commits a newer config.
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const loxConfig = join(loxDir, "config.yaml");
writeFileSync(loxConfig, `host: http://127.0.0.1:${msPort}\nuser: u\npass: p\nserial: 504F94000001\nconfig_repo: ${repo}\n`);
const fakeLox = join(loxDir, "lox");
writeFileSync(fakeLox, `#!/bin/sh
[ "$*" = "config pull --quiet" ] || { echo "unexpected: $*" >&2; exit 2; }
sed -i 's/Title="Temperatur"/Title="Temperatur Wohnen"/' "${cfg}"
cp "${backup}.new" "${backup}"
git -C "${repo}" -c user.name=lox -c user.email=lox@example.com commit -qam "[504F94000001] Config backup 2026-11-02 10:00:00 (v2)"
echo committed
`);
chmodSync(fakeLox, 0o755);

const port = 7900 + Math.floor(Math.random() * 90);
const server = spawn(bin, [repo, "--listen", `127.0.0.1:${port}`, "--live-from", liveFile, "--lox", fakeLox], {
  stdio: ["ignore", "ignore", "pipe"],
  env: { ...process.env, LOX_CONFIG: loxConfig, VISLOX_FTP_PORT: String(ftpPort) },
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
    // The Divide block streams nothing: vislox computes 21.5 / 100 (the edited parameter).
    await page.click('[data-uuid="10000006-0000-0060-ffff504f94000001"]');
    await page.waitForFunction(() => [...document.querySelectorAll("#props .lv.computed")].some((n) => /^0\.2\d$/.test(n.textContent)));
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
  await step("the Miniserver chip shows its serial and firmware", async () => {
    await page.waitForSelector("#ms-chip.ok");
    assert.equal(await page.textContent("#ms-lbl"), "504F94000001 · 17.1.7.27");
    await page.click("#ms-chip");
    await page.waitForSelector("#ms-pop:not(.hidden)");
    assert.match(await page.textContent("#ms-body"), /FTP \(port \d+\) answers: 220 fake FTP/);
    assert.match(await page.textContent("#ms-body"), /Config belongs to504F94000001 \(the repository folder\) ✓/);
    await page.keyboard.press("Escape");
    await page.waitForSelector("#ms-pop.hidden", { state: "attached" });
  });
  await step("another Miniserver at the address: Pull warns, and the server refuses it", async () => {
    msSerial = "50:4F:94:99:99:99";
    // "Check again" reuses a check from the last 2 s.
    await page.waitForTimeout(2100);
    await page.click("#ms-chip");
    await page.click("#ms-recheck");
    await page.waitForSelector("#ms-chip.bad");
    assert.equal(await page.textContent("#ms-lbl"), "Wrong Miniserver (504F94999999)");
    assert.match(await page.textContent("#ms-body"), /Pull, Apply, Sync would fail: this is not the Miniserver of this config/);
    assert.match(await page.textContent("#ms-body"), /Config belongs to504F94000001 \(the repository folder\) ✗ not this one/);
    // The button stays on (the cause may clear); its tooltip warns.
    assert.equal(await page.$eval("#btn-pull", (b) => b.disabled), false);
    assert.match(await page.getAttribute("#btn-pull", "title"), /The last check found: this is not the Miniserver of this config/);
    const r = await page.evaluate(() => fetch("/api/pull", { method: "POST", headers: { "X-Vislox": "pull" } }).then(async (r) => [r.status, (await r.json()).error]));
    assert.equal(r[0], 503);
    assert.match(r[1], /^Cannot pull: this is not the Miniserver of this config: 127\.0\.0\.1 reports serial 504F94999999, but the config belongs to 504F94000001/);
    const i = errors.findIndex((e) => e.includes("status of 503"));
    if (i >= 0) errors.splice(i, 1);
    // The right one is back: Pull works again.
    msSerial = "50:4F:94:00:00:01";
    await page.waitForTimeout(2100);
    await page.click("#ms-recheck");
    await page.waitForSelector("#ms-chip.ok");
    await page.keyboard.press("Escape");
  });
  await step("a config saved on the Miniserver after the base is noticed", async () => {
    assert.equal(await page.$eval("#stale", (n) => n.classList.contains("hidden")), true);
    // Someone saves from Loxone Config: the Miniserver's config date moves on.
    msConfigDate = "2026-11-02 09:30:00";
    writeFileSync(backup + ".new", backupZip(msConfigDate));
    // "Check again" reuses a check from the last 2 s.
    await page.waitForTimeout(2500);
    await page.click("#ms-chip");
    await page.click("#ms-recheck");
    await page.waitForSelector("#ms-chip.warn");
    assert.match(await page.textContent("#ms-body"), /Config last changed2026-11-02 09:30 \(after the base\)/);
    assert.match(await page.textContent("#ms-body"), /Base taken at2026-11-01 10:00/);
    await page.keyboard.press("Escape");
    await page.waitForSelector("#stale:not(.hidden)");
    assert.match(await page.textContent("#stale-text"), /^The Miniserver's config changed at 2026-11-02 09:30, after the base vislox shows \(taken at 2026-11-01 10:00\)/);
    assert.equal(await page.isVisible("#stale-pull"), true);
    assert.equal(await page.isVisible("#stale-sync"), false);
  });
  await step("pull from the Miniserver commits a new backup and shows it", async () => {
    // From the notice: the same as the History's Pull button.
    await page.click("#stale-pull");
    await page.waitForFunction(() => /^Pulled/.test(document.querySelector("#pull-msg")?.textContent || ""), null, { timeout: 15000 });
    await page.waitForFunction(() => [...document.querySelectorAll(".hrev .lbl")].some((n) => n.textContent === "Backup v2"), null, { timeout: 10000 });
    // The pulled download carries the Miniserver's date: the base is current again.
    await page.waitForSelector("#stale.hidden", { state: "attached", timeout: 10000 });
    await page.waitForSelector("#ms-chip.ok");
  });
  await step("blocked FTP warns on Pull, and the server refuses it", async () => {
    await new Promise((ok) => { ftp.close(ok); for (const s of ftpSockets) s.destroy(); });
    // "Check again" reuses a check from the last 2 s (the pull just ran one).
    await page.waitForTimeout(2500);
    await page.click("#ms-chip");
    await page.click("#ms-recheck");
    await page.waitForSelector("#ms-chip.bad");
    assert.match(await page.textContent("#ms-body"), /Pull, Apply, Sync would fail: FTP \(port \d+\) is blocked/);
    await page.keyboard.press("Escape");
    // The button stays on (the cause may clear); its tooltip warns.
    assert.equal(await page.$eval("#btn-pull", (b) => b.disabled), false);
    assert.match(await page.getAttribute("#btn-pull", "title"), /The last check found: FTP \(port \d+\) is blocked between this host and the Miniserver/);
    // Clicking it, the server checks and refuses with the reason.
    await page.click("#btn-pull");
    await page.waitForFunction(() => /^Cannot pull: FTP \(port \d+\) is blocked/.test(document.querySelector("#pull-msg")?.textContent || ""), null, { timeout: 15000 });
    // The browser logs that refusal; it is the one expected here.
    const i = errors.findIndex((e) => e.includes("status of 503"));
    if (i >= 0) errors.splice(i, 1);
  });
  await step("the Miniserver log says why it cannot be read", async () => {
    // The stand-in lox here only pulls, so `lox log` fails; the panel says so.
    await page.click("#log-toggle");
    await page.waitForFunction(() => /^Cannot read the log: lox log failed: unexpected: log/.test(document.querySelector("#log-msg")?.textContent || ""), null, { timeout: 15000 });
    await page.click("#log-toggle");
    await page.waitForSelector("#log-body.hidden", { state: "attached" });
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
