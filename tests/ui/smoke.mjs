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
import { createSocket } from "node:dgram";
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
// The meter is shown in the app, in the kitchen (a room of its own config).
const METER = "10000005-0000-0050-ffff504f94000001", ROOM = "10000020-0000-00a1-ffff504f94000001", CELLAR = "10000021-0000-00a1-ffff504f94000001";
writeFileSync(cfg, readFileSync(cfg, "utf8")
  .replace('<C Type="Page" V="178" U="10000002', `<C Type="Place" V="178" U="${ROOM}" Title="Küche"/>\n\t\t<C Type="Page" V="178" U="10000002`)
  .replace('<Co K="Pf" Nc="1"', `<IoData Visu="true" Pr="${ROOM}"/><Co K="Pf" Nc="1"`));
git("add", ".");
git("commit", "-q", "-m", "[504F94000001] Config backup 2026-11-01 10:00:00 (v1)");
// Uncommitted edits: a rename, a parameter and a calendar entry's date.
const edit = (from, to) => writeFileSync(cfg, readFileSync(cfg, "utf8").replace(from, to));
edit('Title="Licht OG"', 'Title="Licht oben"');
edit('K="Input2" Def="1000"', 'K="Input2" Def="100"');
edit('Month="12" Day="24"', 'Month="12" Day="26"');
// … and one whose date no longer decodes (a kind of date vislox does not know).
edit('CalType="-1" Month="12" Day="31"', 'CalType="-1" Mode="5" Month="12" Day="31"');

// Live values as `lox stream -o json --initial` writes them: the fixture's
// temperature (an analog output) and its AND gate (a digital one).
const TEMP = "10000003-0000-0032-01ff504f94000001", GATE = "10000004-0000-0043-02ff504f94000001";
const liveFile = join(mkdtempSync(join(tmpdir(), "vislox-live-")), "stream.ndjson");
const liveLine = (control, state, value, uuid) => JSON.stringify({ control, state, value, uuid }) + "\n";
// The holiday mode (Feiertag) is on: its Q.
const HOLIDAY_Q = "00000000-0000-0001-1500000000000002";
writeFileSync(liveFile, liveLine("Temperatur", "value", "21.5", TEMP) + liveLine("AND", "active", "1", GATE) + liveLine("Feiertag", "active", "1", HOLIDAY_Q));

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
// Debug-Monitor commands the stand-in Miniserver got (dev/sps/log/<ip>, dev/sps/log).
const msMonitor = [];
// Its structure file puts the meter in the cellar, not the kitchen the config names.
const structure = {
  lastModified: "2026-11-01 10:00:00", msInfo: { serialNr: "504F94000001", msName: "Testhaus" },
  rooms: { [ROOM]: { name: "Küche", image: "IconsFilled/kitchen.svg" }, [CELLAR]: { name: "Keller", image: "IconsFilled/kitchen.svg" } },
  cats: {},
  controls: { [METER]: { name: "Zähler", type: "Meter", room: CELLAR, defaultIcon: null, states: {} } },
};
const ICON = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#8b6cc4"/></svg>';
let structureReads = 0;
// Its calendar entries (jdev/sps/calendargetentries), as the deployed config (the last commit) has
// them, not the uncommitted edits on disk (Christmas Eve on 26 Dec, Silvester as Mode 5). A test
// edits them the way the Loxone app would.
const CAL = (n) => `1000001${n}-0000-01${n}0-ffff504f94000001`;
let msCalendar = [
  { uuid: CAL(3), name: "Heiligabend", operatingMode: 0, calMode: 0, startMonth: 12, startDay: 24 },
  { uuid: CAL(4), name: "Ostermontag", operatingMode: 0, calMode: 1, easterOffset: 1 },
  { uuid: CAL(5), name: "Sommerferien", operatingMode: 1, calMode: 3, startYear: 2026, startMonth: 7, startDay: 4, endYear: 2026, endMonth: 9, endDay: 5 },
  { uuid: CAL(6), name: "Heizperiode", operatingMode: 1, calMode: 4, startMonth: 10, startDay: 1, endMonth: 4, endDay: 30 },
  { uuid: CAL(7), name: "Silvester", operatingMode: -1, calMode: 0, startMonth: 12, startDay: 31 },
];
let calendarReads = 0;
// While set, the stand-in answers calendargetentries with this HTTP status.
let calendarDown = 0;
const msPort = await listen(createServer((req, res) => {
  if (req.url.startsWith("/dev/sps/log")) { msMonitor.push(req.url); res.end("ok"); return; }
  if (req.url.toLowerCase() === "/data/loxapp3.json") {
    if (!req.headers.authorization) { res.writeHead(401); res.end(); return; }
    structureReads++;
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify(structure));
    return;
  }
  if (req.url === "/jdev/sps/calendargetentries" && calendarDown) { calendarReads++; res.writeHead(calendarDown); res.end(); return; }
  if (req.url === "/IconsFilled/kitchen.svg") { res.setHeader("Content-Type", "image/svg+xml"); res.end(ICON); return; }
  const ll = {
    "/jdev/cfg/apiKey": () => ({ control: "dev/cfg/apiKey", Code: "200",
      value: `{'snr': '${msSerial}', 'version':'17.1.7.27', 'key':'00', 'local':true, 'httpsStatus':0}` }),
    "/jdev/sps/LoxAPPversion3": () => ({ control: "dev/sps/LoxAPPversion3", Code: "200", value: msConfigDate }),
    "/jdev/sps/calendargetentries": () => (calendarReads++, { control: "dev/sps/calendargetentries", Code: "200", value: JSON.stringify(msCalendar) }),
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
// The Debug-Monitor's UDP port: a free one.
const monPort = await new Promise((ok) => { const u = createSocket("udp4"); u.bind(0, "127.0.0.1", () => { const p = u.address().port; u.close(() => ok(p)); }); });
// A Debug-Monitor frame as the Miniserver sends it (src/monframe.rs): header, one string parameter.
function monFrame(text, counter) {
  const body = Buffer.concat([Buffer.from([1]), Buffer.from(text), Buffer.from([0])]);
  const f = Buffer.alloc(28 + body.length);
  f.writeUInt16LE(0xFA1F, 0); f.writeUInt16LE(f.length, 2); f[5] = 1;
  f.writeUInt16LE(100, 6); f.writeUInt16LE(counter, 8); f.writeUInt16LE(250, 10);
  f.writeUInt32LE(563194643, 12); f.set([127, 0, 0, 1], 16);
  body.copy(f, 28);
  f[4] = f.subarray(5).reduce((x, b) => x ^ b, 0);
  return f;
}
const server = spawn(bin, [repo, "--listen", `127.0.0.1:${port}`, "--live-from", liveFile, "--lox", fakeLox], {
  stdio: ["ignore", "ignore", "pipe"],
  env: { ...process.env, LOX_CONFIG: loxConfig, VISLOX_FTP_PORT: String(ftpPort), VISLOX_CACHE: mkdtempSync(join(tmpdir(), "vislox-cache-")), VISLOX_MONITOR: "1", VISLOX_MONITOR_PORT: String(monPort),
    // As Docker Compose passes it when unset: empty means "not set".
    VISLOX_MONITOR_IP: "" },
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
  await step("a calendar entry reads as a date and an operating mode", async () => {
    const changes = await page.$$eval(".changes .chg .t", (n) => n.map((x) => x.textContent));
    assert.ok(changes.includes("Heiligabend · when: 24 Dec → 26 Dec"), changes.join(" | "));
    await page.click('.changes .chg:has-text("Heiligabend")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Heiligabend");
    const props = await page.textContent("#props");
    assert.match(props, /Calendar entry/);
    assert.match(props, /WhenEvery year on 24 DecEvery year on 26 Dec/);
    assert.match(props, /Switches onFeiertag/);
    assert.equal(await page.getAttribute('.tn-row[data-uuid="10000013-0000-0130-ffff504f94000001"]', "title"), "Every year on 26 Dec · switches on Feiertag");
    // The mode is a link to the Mode object.
    await page.click('#props .lnk:has-text("Feiertag")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Feiertag");
  });
  await step("In the app: the config's settings, then what the Miniserver's app shows", async () => {
    await page.keyboard.press("Control+k");
    await page.keyboard.type("zähler");
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Zähler");
    const sec = () => page.$$eval("#props section", (s) => s.find((x) => x.querySelector(".ps-h")?.dataset.sec === "In the app")?.textContent || "");
    assert.match(await sec(), /ShownYes/);
    assert.match(await sec(), /RoomKüche/);
    // A blue phone on the block, and an entry in the legend.
    assert.ok(await page.$(`#viewport [data-uuid="${METER}"] .abadge, #viewport .abadge`));
    assert.match(await page.textContent("#legend"), /in the app/);
    // vislox reads the structure file only when asked (live values come from a file here).
    assert.equal(structureReads, 0);
    await page.click("#props .app-load");
    await page.waitForFunction(() => /ControlMeter/.test(document.querySelector("#props")?.textContent || ""));
    assert.equal(structureReads, 1);
    // A structure file read also asks for the calendar entries: the same as the config's.
    for (let t = 0; calendarReads < 1 && t < 50; t++) await page.waitForTimeout(100);
    assert.equal(calendarReads, 1);
    await page.waitForFunction(() => S.cal?.entries === 5);
    assert.equal(await page.$eval("#cal-drift", (n) => n.classList.contains("hidden")), true);
    // The current revision is compared with what the app shows.
    assert.match(await sec(), /in room Keller in the app, Küche in the config/);
    // The room's icon, from the Miniserver, in Properties and in the tree.
    await page.waitForFunction(() => [...document.querySelectorAll("#props img.app-ic")].some((i) => i.complete && i.naturalWidth > 0));
    await page.waitForFunction(() => [...document.querySelectorAll("#tree .tn-ic.app img")].some((i) => i.complete && i.naturalWidth > 0));
    // Icons are only fetched for names the structure file lists.
    const other = await page.evaluate(() => fetch("/api/app/icon?name=jdev/cfg/apiKey").then((r) => r.status));
    assert.equal(other, 404);
    const i = errors.findIndex((e) => e.includes("status of 404"));
    if (i >= 0) errors.splice(i, 1);
  });
  await step("Operating modes: a year calendar of the calendar entries", async () => {
    // Ctrl+K lists the view after pages and blocks, and only for a word it starts with.
    await page.keyboard.press("Control+k");
    await page.keyboard.type("o");
    assert.notEqual(await page.textContent('#pal-list [aria-selected="true"] .pk'), "view");
    await page.keyboard.type("bergeschoss");
    assert.ok(!(await page.textContent("#pal-list")).includes("Operating modes"));
    await page.fill("#pal-input", "");
    await page.keyboard.type("operating");
    await page.keyboard.press("Enter");
    await page.waitForSelector("#modes:not(.hidden)");
    assert.match(page.url(), /view=modes/);
    // Easter Sunday, Gregorian computus.
    assert.deepEqual(await page.evaluate(() => [2024, 2025, 2026, 2038, 2285].map(easterSunday)), [[3, 31], [4, 20], [4, 5], [4, 25], [3, 22]]);
    // Dates are whole days, also in years before 100 (Date.UTC would make 50 into 1950).
    assert.deepEqual(await page.evaluate(() => mdYmd(mdDay(50, 3, 1))), [50, 3, 1]);
    // Today is fixed for the test: Easter Monday 2026, a Feiertag.
    await page.evaluate(() => { window.mdToday = () => mdDay(2026, 4, 6); MD.year = 2026; renderModes(); });
    const cls = (m, d, y = 2026) => page.evaluate(([y, m, d]) => document.querySelector(`#md-grid [data-day="${mdDay(y, m, d)}"]`)?.className ?? "", [y, m, d]);
    const bg = (m, d) => page.evaluate(([m, d]) => document.querySelector(`#md-grid [data-day="${mdDay(2026, m, d)}"]`)?.getAttribute("style") ?? "", [m, d]);
    assert.match(await cls(4, 6), /md-on/, "Easter Monday 2026 is 6 Apr");
    // 6 Apr has two modes (Easter Monday's Feiertag, Heizperiode's Urlaub): both colours.
    assert.match(await bg(4, 6), /linear-gradient/);
    assert.doesNotMatch(await bg(4, 5), /linear-gradient/);
    // Heizperiode runs across New Year (1 Oct – 30 Apr).
    assert.match(await cls(1, 15), /md-on/);
    assert.match(await cls(4, 30), /md-on/);
    assert.doesNotMatch(await cls(5, 1), /md-on/);
    assert.match(await cls(10, 1), /md-on/);
    // Sommerferien (4 Jul – 5 Sep 2026) and Heizperiode switch on the same mode, Urlaub; Silvester Party.
    assert.match(await cls(7, 4), /md-on/);
    assert.match(await cls(9, 5), /md-on/);
    assert.doesNotMatch(await cls(9, 6), /md-on/);
    const sw = (t) => page.$eval(`.md-chip[title^="${t}:"] .md-sw`, (n) => n.style.background);
    assert.notEqual(await sw("Party"), await sw("Urlaub"), "each mode has its own colour");
    assert.equal(await bg(1, 15), await bg(7, 4));
    // The uncommitted edit moved Heiligabend from 24 to 26 Dec.
    assert.match(await cls(12, 26), /md-add/);
    assert.match(await cls(12, 24), /md-rem/);
    // Hovering a day lists its entries.
    await page.hover(`#md-grid [data-day="${await page.evaluate(() => mdDay(2026, 12, 24))}"]`);
    assert.match(await page.textContent("#tooltip"), /24 Dec 2026.*Heiligabend · Feiertag removed/);
    // Silvester no longer decodes: its old day (31 Dec, Party) shows as removed.
    assert.match(await cls(12, 31), /md-rem/);
    await page.hover(`#md-grid [data-day="${await page.evaluate(() => mdDay(2026, 12, 31))}"]`);
    assert.match(await page.textContent("#tooltip"), /Silvester · Party removed/);
    // A hidden mode is left out of the day's tooltip too.
    await page.click('.md-chip[title^="Party:"]');
    await page.hover(`#md-grid [data-day="${await page.evaluate(() => mdDay(2026, 12, 31))}"]`);
    assert.doesNotMatch(await page.textContent("#tooltip"), /Silvester/);
    assert.doesNotMatch(await cls(12, 31), /md-rem/);
    await page.click('.md-chip[title^="Party:"]');
    // The list has every entry with its date and mode; clicking one selects it.
    const list = await page.$$eval("#md-list .md-e", (n) => n.map((x) => x.textContent));
    assert.equal(list.length, 5, list.join(" | "));
    assert.ok(list.some((t) => /Ostermontag.*Easter Monday \(Easter \+1\).*Feiertag · 6 Apr 2026/.test(t)), list.join(" | "));
    await page.click('#md-list .md-e:has-text("Ostermontag")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Ostermontag");
    assert.match(await cls(4, 6), /md-hl/);
    assert.ok(await page.isVisible("#modes"), "the view stays open");
    // Another year: the summer holidays of 2026 are not in 2025. The arrows step the year even
    // when focus left the view (a click on a day re-renders it).
    await page.click(`#md-grid [data-day="${await page.evaluate(() => mdDay(2026, 4, 6))}"]`);
    await page.keyboard.press("ArrowLeft");
    assert.equal(await page.textContent("#md-year"), "2025");
    assert.doesNotMatch(await cls(7, 4, 2025), /md-on/);
    assert.ok((await page.textContent("#md-list")).includes("Feiertag · 21 Apr 2025"), "Easter Monday 2025 is 21 Apr");
    // Live values: the modes that are on now.
    assert.equal(await page.$$eval(".md-chip.live", (n) => n.length), 0);
    await page.click("#sw-live .track");
    await page.waitForSelector('.md-chip.live:has-text("Feiertag")');
    assert.match(await page.textContent(".md-chip.live"), /on now/);
    // In another year than today's, the chip still says what the calendar has today.
    assert.match(await page.getAttribute(".md-chip.live", "title"), /the calendar has it today/);
    await page.click("#sw-live .track");
    await page.waitForFunction(() => !document.querySelector(".md-chip.live"));
    // No year before 2009, Loxone's day 0.
    await page.evaluate(() => { MD.year = 2010; renderModes(); });
    await page.click("#md-prev");
    assert.equal(await page.textContent("#md-year"), "2009");
    assert.ok(await page.$eval("#md-prev", (b) => b.disabled));
    await page.keyboard.press("ArrowLeft");
    assert.equal(await page.textContent("#md-year"), "2009");
    // Esc goes back to the page; the Calendar folder's Year button opens the view again, and the
    // folder row itself still folds.
    await page.focus("#md-title");
    await page.keyboard.press("Escape");
    await page.waitForSelector("#modes.hidden", { state: "attached" });
    assert.doesNotMatch(page.url(), /view=modes/);
    const folder = '.tn-row:has-text("Betriebszeiten")';
    const expanded = () => page.getAttribute(folder, "aria-expanded");
    const was = await expanded();
    await page.click(`${folder} .tn-tw`);
    assert.notEqual(await expanded(), was);
    assert.ok(await page.isHidden("#modes"));
    await page.click(`${folder} [data-modes]`);
    await page.waitForSelector("#modes:not(.hidden)");
    await page.click("#md-close");
    await page.waitForSelector("#modes.hidden", { state: "attached" });
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
    // Block queries need a Miniserver connection: a recording offers none.
    assert.ok(await page.$eval("#lg-query", (n) => n.classList.contains("hidden")), "no block queries from a recording");
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
  await step("calendar entries edited in the app are not polled for", async () => {
    // In the app: Easter Monday moves to Good Friday, Silvester goes, a bridge day comes.
    msCalendar = msCalendar.filter((e) => e.name !== "Silvester").map((e) => e.name === "Ostermontag" ? { ...e, easterOffset: -2 } : e);
    msCalendar.push({ uuid: "20000001-0000-0001-ffff504f94000001", name: "Brückentag", operatingMode: -1, calMode: 2, startYear: 2026, startMonth: 12, startDay: 24 });
    await page.waitForTimeout(500);
    assert.equal(calendarReads, 1);
    assert.equal(await page.$eval("#cal-drift", (n) => n.classList.contains("hidden")), true);
  });
  await step("before Apply, calendar entries edited in the app are read and listed", async () => {
    // Apply asks for them first (vislox has no admin token here, so ask the way Apply does).
    await page.evaluate(() => loadCal(true));
    await page.waitForSelector("#cal-drift:not(.hidden)", { timeout: 10000 });
    assert.equal(calendarReads, 2);
    assert.match(await page.textContent("#cal-drift-text"), /^3 calendar entries were changed \(1 added, 1 changed, 1 deleted\) in the Loxone app since the deployed config\./);
    const items = await page.$$eval("#cal-drift-list li", (n) => n.map((x) => x.textContent));
    assert.deepEqual(items, [
      "Ostermontag: when: Easter Monday (Easter +1) → Good Friday (Easter −2)",
      "Brückentag: added, 24 Dec 2026 · switches on Party",
      "Silvester: deleted (was Every year on 31 Dec · switches on Party)",
    ]);
    assert.equal(await page.isVisible("#cal-drift-pull"), true);
    // The entry's Properties say so too, on the deployed revision.
    await page.click('#cal-drift-list .lnk:has-text("Ostermontag")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Ostermontag");
    assert.match(await page.textContent("#props"), /In the appChanged in the app since the deployed config: when: Easter Monday \(Easter \+1\) → Good Friday \(Easter −2\)\. Pull or Sync to take it over\./);
    await page.click("#ms-chip");
    assert.match(await page.textContent("#ms-body"), /Calendar entries3 changed in the app/);
    await page.keyboard.press("Escape");
  });
  await step("before Apply, a calendar check that failed says so, and is tried again", async () => {
    calendarDown = 503;
    // A good read from the last 2 s would be reused.
    await page.waitForTimeout(2100);
    const before = calendarReads;
    const warn = await page.evaluate(async () => calApplyWarning(await loadCal(true)));
    assert.equal(warn, "⚠ Couldn't check calendar entries changed in the app: 503, restarting. Applying overwrites any there are.\n\n");
    assert.equal(calendarReads, before + 1);
    // The failure is not reused: the next check asks again, and finds the entries.
    calendarDown = 0;
    const again = await page.evaluate(async () => calApplyWarning(await loadCal(true)));
    assert.equal(calendarReads, before + 2);
    assert.match(again, /^⚠ 3 calendar entries were changed .* Applying overwrites them:\n  Ostermontag: when: /);
    // vislox itself not answering is a failed check too, not the last answer.
    const failed = await page.evaluate(async () => {
      const real = window.fetch;
      window.fetch = async () => new Response(JSON.stringify({ error: "boom" }), { status: 500 });
      try { return calApplyWarning(await loadCal(true)); } finally { window.fetch = real; }
    });
    assert.equal(failed, "⚠ Couldn't check calendar entries changed in the app: boom. Applying overwrites any there are.\n\n");
    await page.evaluate(() => loadCal());
  });
  await step("an older calendar answer does not replace a newer one", async () => {
    const kept = await page.evaluate(async () => {
      const real = window.fetch, waiting = [];
      window.fetch = () => new Promise((ok) => {
        const n = waiting.length + 1;
        waiting.push(() => ok(new Response(JSON.stringify({ available: true, read_at: n, entries: n, drift: [] }))));
      });
      try {
        const first = loadCal(), second = loadCal();
        waiting[1](); await second;
        waiting[0](); await first;
        return S.cal.entries;
      } finally { window.fetch = real; await loadCal(); }
    });
    assert.equal(kept, 2);
    await page.waitForSelector("#cal-drift:not(.hidden)");
  });
  await step("pull from the Miniserver commits a new backup and shows it", async () => {
    const readsBeforePull = calendarReads;
    // From the notice: the same as the History's Pull button.
    await page.click("#stale-pull");
    await page.waitForFunction(() => /^Pulled/.test(document.querySelector("#pull-msg")?.textContent || ""), null, { timeout: 15000 });
    await page.waitForFunction(() => [...document.querySelectorAll(".hrev .lbl")].some((n) => n.textContent === "Backup v2"), null, { timeout: 10000 });
    // The pulled download carries the Miniserver's date: the base is current again.
    await page.waitForSelector("#stale.hidden", { state: "attached", timeout: 10000 });
    await page.waitForSelector("#ms-chip.ok");
    // Pull reads the calendar entries again.
    for (let t = 0; calendarReads < readsBeforePull + 1 && t < 50; t++) await page.waitForTimeout(100);
    assert.equal(calendarReads, readsBeforePull + 1);
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
  });
  await step("the Debug-Monitor shows the Miniserver's frames live", async () => {
    await page.waitForFunction((p) => document.querySelector("#mon-state")?.textContent.includes(`listening on UDP ${p}`), monPort);
    const udp = createSocket("udp4");
    await new Promise((ok) => udp.send(monFrame("PRG Monitor line from the house", 1), monPort, "127.0.0.1", ok));
    udp.close();
    await page.waitForFunction(() => [...document.querySelectorAll("#log li.monitor .tx")].some((x) => x.textContent === "PRG Monitor line from the house"));
    assert.match(await page.textContent("#log li.monitor .tm"), /2026-11-06 10:57:23\.250\s*monitor/);
  });
  await step("a Monitor Server request gets the XML naming vislox", async () => {
    const r = await fetch(`http://127.0.0.1:${port}/?504F94000001`);
    assert.match(r.headers.get("content-type"), /xml/);
    assert.match(await r.text(), new RegExp(`<LogDest>/dev/udp/127\\.0\\.0\\.1/${monPort}</LogDest>`));
    // Not a serial: the page.
    assert.match(await (await fetch(`http://127.0.0.1:${port}/?x=1`)).text(), /<html/);
  });
  await step("Start and Stop ask the Miniserver to send and to stop", async () => {
    page.once("dialog", (d) => d.accept());
    await page.click("#btn-mon");
    await page.waitForFunction(() => /^Debug-Monitor on until/.test(document.querySelector("#mon-state")?.textContent || ""));
    assert.deepEqual(msMonitor, ["/dev/sps/log/127.0.0.1"]);
    await page.click("#btn-mon");
    await page.waitForFunction(() => /^Debug-Monitor: listening/.test(document.querySelector("#mon-state")?.textContent || ""));
    assert.deepEqual(msMonitor, ["/dev/sps/log/127.0.0.1", "/dev/sps/log"]);
    await page.click("#log-toggle");
    await page.waitForSelector("#log-body.hidden", { state: "attached" });
  });
  await step("operating two inputs: each waits for its own state update", async () => {
    // Operating is off here (--live-from): drive the page's waiting directly, with fake value
    // events. Nothing is sent anywhere.
    const res = await page.evaluate(async () => {
      opExpect("in-a", "Code 200 (ok)", new Set(["a-q"]), 400);
      opExpect("in-b", "Code 200 (ok)", new Set(["b-q"]), 400);
      opSeen({ "b-q": [1, null, ""] });
      const afterB = { a: opRes.get("in-a").cls, b: opRes.get("in-b").cls, waiting: opWait.size };
      opSeen({ "a-q": [21.5, null, ""] });
      const afterA = { a: opRes.get("in-a").text, waiting: opWait.size };
      opExpect("in-c", "Code 200 (ok)", new Set(["c-q"]), 50);
      await new Promise((r) => setTimeout(r, 150));
      const c = opRes.get("in-c");
      return { afterB, afterA, c };
    });
    if (res.afterB.b !== "ok" || res.afterB.a !== "busy" || res.afterB.waiting !== 1) throw new Error(JSON.stringify(res));
    if (!/state update: 21.5$/.test(res.afterA.a) || res.afterA.waiting !== 0) throw new Error(JSON.stringify(res));
    if (res.c.cls !== "warn" || !/no state update came back/.test(res.c.text)) throw new Error(JSON.stringify(res));
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
