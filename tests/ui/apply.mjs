// Browser test for Apply: a lox.toml project with a pending compile, and a
// stand-in lox that says what `lox config push` says. The steps must show as
// they happen, and afterwards the view must switch to the deployed revision,
// compared with the one it replaced, with live values on.
//
//   VISLOX_BIN=target/release/vislox node tests/ui/apply.mjs
//
// Nothing here talks to a Miniserver: lox is a script, and the lox
// configuration points at a port that drops every connection, so vislox does
// not follow the restart itself (the unit tests in src/restart.rs and
// src/actions.rs do). The port takes connections, and a stand-in FTP server
// greets, so that the preflight before Apply passes.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { createServer as createHttpServer } from "node:http";
import { createServer as createTcpServer } from "node:net";
import { createSocket } from "node:dgram";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const bin = process.env.VISLOX_BIN || join(here, "../../target/release/vislox");
const fixture = join(here, "../fixtures/house.Loxone");

const root = mkdtempSync(join(tmpdir(), "vislox-apply-"));
const dir = join(root, "project");
mkdirSync(join(dir, "pages"), { recursive: true });
const git = (...a) => execFileSync("git", ["-C", dir, "-c", "user.name=t", "-c", "user.email=t@example.com", ...a]);
// The deployed config shows the Divide block in the app.
const DIV = "10000006-0000-0060-ffff504f94000001";
const base = readFileSync(fixture, "utf8").replace('<Co K="Input1" Nc="1"', '<IoData Visu="true"/><Co K="Input1" Nc="1"');
writeFileSync(join(dir, "base.Loxone"), base);
// The pending compile: a rename, the renamed block shown in the app, and the Divide block removed.
const divBlock = base.match(/\t*<C Type="Div"[\s\S]*?<\/C>\n/)[0];
writeFileSync(join(dir, "out.Loxone"), base.replace('Title="Licht OG"', 'Title="Licht oben"')
  .replace('<Co K="I1" Nc="1" U="10000009', '<IoData Visu="true"/><Co K="I1" Nc="1" U="10000009')
  .replace(divBlock, ""));
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\n');
writeFileSync(join(dir, "pages", "main.lox"), "# module\n");
execFileSync("git", ["init", "-q", "--bare", join(root, "remote.git")]);
git("init", "-q");
git("add", "-A");
git("commit", "-q", "-m", "Deployed");
git("remote", "add", "origin", join(root, "remote.git"));
git("push", "-q", "-u", "origin", "HEAD");

// A lox configuration at a port that drops every request but the structure file,
// so no real Miniserver is reached, and an FTP server that only greets. The
// structure file still lists the removed Divide block, and not the renamed one.
const listen = (srv) => new Promise((ok) => srv.listen(0, "127.0.0.1", () => ok(srv.address().port)));
let structureReads = 0;
const webPort = await listen(createHttpServer((req, res) => {
  if (req.url.toLowerCase() !== "/data/loxapp3.json") { req.socket.destroy(); return; }
  structureReads++;
  res.setHeader("Content-Type", "application/json");
  res.end(JSON.stringify({ lastModified: "2026-11-06 12:00:00", msInfo: { serialNr: "504F94000001" }, rooms: {}, cats: {},
    controls: { [DIV]: { name: "Divide", type: "InfoOnlyAnalog", defaultIcon: null } } }));
}));
const ftpPort = await listen(createTcpServer((s) => { s.on("error", () => {}); s.end("220 fake FTP\r\n"); }));
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const loxConfig = join(loxDir, "config.yaml");
writeFileSync(loxConfig, `host: http://127.0.0.1:${webPort}\nuser: u\npass: p\nserial: 504F94000001\n`);
const fakeLox = join(loxDir, "lox");
writeFileSync(fakeLox, `#!/bin/sh
case "$1 $2" in
  "logic diff") echo "Rename Licht OG to Licht oben" ;;
  "config push")
    echo "Downloading the live config..." >&2; sleep 1
    echo "Uploading as sps_new.zip..." >&2; sleep 1
    echo "Restarting the program..." >&2; sleep 2
    echo "Downloading the config again to verify..." >&2; sleep 1
    cp out.Loxone base.Loxone
    touch "${loxDir}/pushed"
    echo "Pushed and verified." >&2 ;;
  "log -n")
    echo "2026-11-06 09:00:00.000;Program changed by user 'admin'"
    echo "2026-11-06 09:00:02.000;PRG start program"
    echo "2026-11-06 09:05:00.000;Warning 1100, Meter offline, all, MeterAbsUni (Zähler,10000005-0000-0050-ffff504f94000001)"
    echo "2026-11-06 09:10:00.000;Remote Connect RC Registration done"
    if [ -f "${loxDir}/pushed" ]; then
      echo "2026-11-06 10:00:00.000;Program changed by user 'lox'"
      echo "2026-11-06 10:00:00.000;PRG got restart command; user: lox(127.0.0.1)"
      echo "2026-11-06 10:00:05.000;Warning 1167, Meter reading decreased, all, MeterAbsUni (Zähler,10000005-0000-0050-ffff504f94000001)"
    fi ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
`);
chmodSync(fakeLox, 0o755);
const token = join(loxDir, "token");
writeFileSync(token, "secret\n");
const TEMP = "10000003-0000-0032-01ff504f94000001";
const liveFile = join(loxDir, "stream.ndjson");
writeFileSync(liveFile, JSON.stringify({ control: "Temperatur", state: "value", value: "21.5", uuid: TEMP }) + "\n");

// The Debug-Monitor's UDP port: a free one; and a frame as the Miniserver sends it (src/monframe.rs).
const monPort = await new Promise((ok) => { const u = createSocket("udp4"); u.bind(0, "127.0.0.1", () => { const p = u.address().port; u.close(() => ok(p)); }); });
function monFrame(text, counter) {
  const body = Buffer.concat([Buffer.from([1]), Buffer.from(text), Buffer.from([0])]);
  const f = Buffer.alloc(28 + body.length);
  f.writeUInt16LE(0xFA1F, 0); f.writeUInt16LE(f.length, 2); f[5] = 1;
  f.writeUInt16LE(100, 6); f.writeUInt16LE(counter, 8);
  f.writeUInt32LE(563194643, 12); f.set([127, 0, 0, 1], 16);
  body.copy(f, 28);
  f[4] = f.subarray(5).reduce((x, b) => x ^ b, 0);
  return f;
}
const port = 7990 + Math.floor(Math.random() * 9);
const server = spawn(bin, [dir, "--listen", `127.0.0.1:${port}`, "--lox", fakeLox, "--admin-token-file", token, "--live-from", liveFile], {
  stdio: ["ignore", "ignore", "pipe"],
  // Apply commits: a runner without a git identity needs one.
  env: {
    ...process.env, LOX_CONFIG: loxConfig, VISLOX_FTP_PORT: String(ftpPort), VISLOX_MONITOR: "1", VISLOX_MONITOR_PORT: String(monPort), VISLOX_CACHE: join(root, "cache"),
    GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@example.com", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@example.com",
  },
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
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  let dialogs = 0, applies = 0;
  page.on("dialog", (d) => { dialogs++; d.accept(); });
  page.on("request", (r) => { if (r.method() === "POST" && new URL(r.url()).pathname === "/api/apply") applies++; });
  await page.addInitScript(() => localStorage.setItem("vislox.token", "secret"));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForSelector(".tab");
  const step = async (name, fn) => { await fn(); console.log("ok -", name); };
  const state = (id) => page.$eval("#deploy-steps", (box, i) => {
    const li = [...box.querySelectorAll("li")].find((l) => l.querySelector(".l").textContent === i);
    return li?.className.replace("st-", "");
  }, id);

  await step("the Miniserver log opens and marks what came after the last program change", async () => {
    await page.waitForSelector("#log-pane:not(.hidden)");
    await page.click("#log-toggle");
    await page.waitForSelector("#log li.since");
    const texts = await page.$$eval("#log li .tx", (n) => n.map((x) => x.textContent));
    assert.equal(texts[0], "Remote Connect RC Registration done", texts.join(" | "));
    assert.equal(await page.$$eval("#log li.since", (n) => n.length), 4);
    assert.match(await page.textContent("#log li.mark"), /The program changed here/);
    // Closed during the Apply: it is read again when it opens.
    await page.click("#log-toggle");
    await page.waitForSelector("#log-body.hidden", { state: "attached" });
  });
  await step("the compile is pending", async () => {
    await page.waitForSelector("#btn-apply:not([disabled])");
  });
  await step("Apply shows its steps as they happen", async () => {
    // A double click asks once, and applies once (Apply checks the Miniserver first, then asks).
    await page.dblclick("#btn-apply");
    await page.waitForSelector("#deploy-steps:not(.hidden)");
    await page.waitForFunction(() => document.querySelector('#deploy-steps li[aria-current="step"] .l')?.textContent === "Program restarting", null, { timeout: 10000 });
    assert.equal(await state("Upload"), "ok");
    assert.equal(await state("Verified"), "wait");
    if (process.env.SHOT) await page.locator("#right").screenshot({ path: process.env.SHOT + "-running.png" });
  });
  await step("every step ends done", async () => {
    await page.waitForFunction(() => /^Applied/.test(document.querySelector("#deploy-msg")?.textContent || ""), null, { timeout: 20000 });
    assert.equal(dialogs, 1, "one confirmation");
    assert.equal(applies, 1, "one /api/apply request");
    for (const l of ["Download and drift check", "Upload", "Program restarting", "Back online", "New config loaded", "Verified", "Committed", "Pushed to git"]) {
      assert.equal(await state(l), "ok", l);
    }
  });
  await step("the deployed revision shows, compared with the one it replaced, with live values", async () => {
    await page.waitForSelector("#deploy-new:not(.hidden)");
    assert.match(await page.textContent(".hrev.sel"), /Rename Licht OG to Licht oben/);
    const changes = await page.$$eval(".changes .chg .t", (n) => n.map((x) => x.textContent));
    assert.ok(changes.some((t) => t.includes("Licht OG → Licht oben")), changes.join(" | "));
    assert.match(await page.textContent("#deploy-new"), /Licht oben/);
    await page.waitForFunction(() => document.querySelector("#canvas").classList.contains("live"));
    if (process.env.SHOT) await page.screenshot({ path: process.env.SHOT + "-done.png" });
  });
  await step("after Apply, the app is checked against the new config, also for a removed block", async () => {
    // Live values come from a file here, so the new structure file is read only when asked.
    await page.waitForSelector("#deploy-new .app-load");
    assert.match(await page.textContent("#deploy-new"), /Read what the app shows now/);
    assert.equal(structureReads, 0);
    await page.click("#deploy-new .app-load");
    await page.waitForFunction(() => /In the app/.test(document.querySelector("#deploy-new")?.textContent || ""));
    assert.equal(structureReads, 1);
    const text = await page.textContent("#deploy-new");
    assert.match(text, /Licht oben\s*not in the app/);
    assert.match(text, /Divi?d?e?\s*removed, but still in the app/);
    // The box keeps this Apply's changes when the view changes.
    await page.click(".hrev >> nth=1");
    await page.waitForFunction(() => document.querySelector(".hrev.sel") === document.querySelectorAll(".hrev")[1]);
    assert.match(await page.textContent("#deploy-new"), /Licht oben\s*not in the app/);
    await page.click(".hrev >> nth=0");
    await page.waitForFunction(() => document.querySelector(".hrev.sel") === document.querySelectorAll(".hrev")[0]);
  });
  await step("after Apply the old log marks no block, and opening the panel reads it again", async () => {
    // The warning read before the Apply is about the old program.
    assert.equal(await page.evaluate(() => logMarks().size), 0);
    await page.click("#log-toggle");
  });
  await step("after Apply the log shows what the new program logged, linked to its block", async () => {
    await page.waitForFunction(() => /Warning 1167/.test(document.querySelector("#log li .tx")?.textContent || ""), null, { timeout: 10000 });
    assert.equal(await page.$$eval("#log li.since", (n) => n.length), 3);
    assert.equal(await page.textContent("#log-count"), "3 new");
    assert.match(await page.getAttribute("#log li.warning", "class"), /since/);
    await page.click('#log li.warning .go:has-text("Zähler")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Zähler");
    // The block carries a mark on the canvas, for the new warning.
    await page.waitForSelector('#viewport .lbadge.warning');
    assert.deepEqual(await page.evaluate(() => [...logMarks().values()].map((e) => e.text.slice(0, 12))), ["Warning 1167"]);
  });
  await step("a Debug-Monitor line marks its block, and the mark opens that line", async () => {
    const udp = createSocket("udp4");
    await new Promise((ok) => udp.send(monFrame("Warning 1, all, MeterAbsUni (Zähler,10000005-0000-0050-ffff504f94000001)", 1), monPort, "127.0.0.1", ok));
    udp.close();
    await page.waitForFunction(() => S.mon.lines.length === 1);
    // As after a read of the system log that failed in the browser: no entries at all.
    await page.evaluate(() => { S.log.data = { error: "offline" }; });
    // The marks are redrawn at most twice a second: wait until the block's mark is the monitor line's.
    await page.waitForFunction(() => S.log.drawn.includes("Warning 1, all"));
    await page.waitForSelector("#viewport .lbadge.warning");
    await page.click("#log-toggle");
    await page.waitForSelector("#log-body.hidden", { state: "attached" });
    await page.click("#viewport .lbadge.warning", { force: true });
    await page.waitForSelector("#log li.monitor.flash");
  });
  await step("a changed block opens from the list", async () => {
    await page.click('#deploy-new .go:has-text("Licht oben")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Licht oben");
  });
  await step("committed and pushed", async () => {
    assert.match(execFileSync("git", ["-C", join(root, "remote.git"), "log", "-1", "--format=%s"]).toString(), /Rename Licht OG/);
  });
  await step("no errors in the browser", async () => {
    assert.deepEqual(errors, []);
  });
} catch (e) {
  failed = true;
  console.error("FAILED:", e.message);
  if (errors.length) console.error("browser errors:", errors);
  if (page) console.error("deploy message:", await page.textContent("#deploy-msg").catch(() => "?"));
  console.error(log);
} finally {
  await browser.close();
  server.kill("SIGINT");
}
process.exit(failed ? 1 : 0);
