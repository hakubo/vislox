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
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync, existsSync, unlinkSync } from "node:fs";
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
// A user's mail address in a stray config (#71): it must never show, nor reach the remote.
const CAROL = "carol@example.com";
// It has a Touch Tree on a Tree Extension.
const TOUCH = "20000003-0000-0030-ffff504f94000001";
const hardware = `\t\t<C Type="LoxLIVE" V="178" U="20000001-0000-0010-ffff504f94000001" Title="Miniserver" Serial="504F94000001">
\t\t\t<C Type="LoxTree" V="178" U="20000002-0000-0020-ffff504f94000001" Title="Tree Extension" Serial="13000001">
\t\t\t\t<C Type="TreeDevice" V="178" U="${TOUCH}" Title="Touch Küche" Serial="B03B368A" SubType="32771" TreeType="32771" AC="1"/>
\t\t\t</C>
\t\t</C>
\t</C>
</ControlList>`;
// Users and an NFC Code Touch (#68): alice may open the front door always and gets the frost warning, bob on
// workdays; the group Bewohner (bob) has no right. The pending compile claims both: Bewohner opens it on
// workdays, alice does not, bob under another schedule of the same title, and bob gets the warning instead
// of her.
const ALICE = "50000001-0000-0010-ffff504f94000001", BOB = "50000002-0000-0020-ffff504f94000001";
const BEWOHNER = "50000003-0000-0030-ffff504f94000001", ALWAYS = "50000006-0000-0060-ffff504f94000001";
const WORKDAYS = "50000007-0000-0070-ffff504f94000001", DOOR = "50000009-0000-0090-ffff504f94000001";
const FROST = "5000000a-0000-00a0-ffff504f94000001", WORKDAYS2 = "50000008-0000-0080-ffff504f94000001";
const prm = (ref, tim, act) => `<PRM REF="${ref}"${tim ? ` TIM="${tim}"` : ""}${act ? ` ACT="${act}"` : ""}/>`;
const users = (alice, bewohner = "", bob = "") => `\t\t<C Type="LoxCaption" V="178" U="50000000-0000-0001-ffff504f94000001" Title="Users and Permissions" CaptionType="20" SubType="20">
\t\t\t<C Type="User" V="178" U="${ALICE}" Title="alice" UsR="-1" NFCArr="0A1B2C3D4E5F" NFCNArr="Schlüssel" CodeArr="00112233"><PH>${alice}</PH></C>
\t\t\t<C Type="User" V="178" U="${BOB}" Title="bob" UsR="129"><PH>${bob}</PH></C>
\t\t\t<C Type="UserGroup" V="178" U="${BEWOHNER}" Title="Bewohner" Rights="129" User="${BOB}" UserGroups=""><PH>${bewohner}</PH></C>
\t\t\t<C Type="SwitchingTimer" V="178" U="${ALWAYS}" Title="Always" M="3" FIX="1"/>
\t\t\t<C Type="SwitchingTimer" V="178" U="${WORKDAYS}" Title="Workdays" M="3"/>
\t\t\t<C Type="SwitchingTimer" V="178" U="${WORKDAYS2}" Title="Workdays" M="3"/>
\t\t\t<C Type="Notification" V="178" U="${FROST}" Title="Frostwarnung" WF="16384" Entries="10"/>
\t\t</C>
\t\t<C Type="Page" V="178" U="50000008-0000-0080-ffff504f94000001" Title="Zugang" WF="16384">
\t\t\t<C Type="NfcCodeTouch" V="178" U="${DOOR}" Title="Haustür" Px="960" Py="960" Px2="2304" Py2="1656" Nio="0" WF="16384"/>
\t\t</C>
`;
// …and holds a Daytimer, whose Feiertag column the Miniserver runs from 07:00 (edited in the app).
const TIMER = "10000030-0000-0300-ffff504f94000001";
const base = readFileSync(fixture, "utf8").replace('<Co K="Input1" Nc="1"', '<IoData Visu="true"/><Co K="Input1" Nc="1"')
  .replace('<Co K="Q" U="10000009-0000-0093-02ff504f94000001"/>\n\t\t\t</C>', `<Co K="Q" U="10000009-0000-0093-02ff504f94000001"/>\n\t\t\t</C>
\t\t\t<C Type="DayTimer" V="178" U="${TIMER}" Title="Uhr" Px="960" Py="2880" Px2="3648" Py2="3768" Nio="2" WF="16384" Analog="true" Modes="00000000-0000-0001-1500000000000000">
\t\t\t\t<Co K="AQ" U="10000030-0000-0302-01ff504f94000001"/><Entry From="390" To="480" V="21"/>
\t\t\t</C>`)
  .replace("\t</C>\n</ControlList>", hardware)
  .replace('\t\t<C Type="Page" V="178" U="10000002', () => users(prm(DOOR, ALWAYS) + prm(FROST, null, "3"), "", prm(DOOR, WORKDAYS)) + '\t\t<C Type="Page" V="178" U="10000002');
writeFileSync(join(dir, "base.Loxone"), base);
// The pending compile: a rename, the renamed block shown in the app, the Divide block removed, and the
// Touch replaced by another one (another serial), with its audible acknowledgement off.
const divBlock = base.match(/\t*<C Type="Div"[\s\S]*?<\/C>\n/)[0];
writeFileSync(join(dir, "out.Loxone"), base.replace('Title="Licht OG"', 'Title="Licht oben"')
  .replace('<Co K="I1" Nc="1" U="10000009', '<IoData Visu="true"/><Co K="I1" Nc="1" U="10000009')
  .replace(divBlock, "")
  .replace('Serial="B03B368A" SubType="32771" TreeType="32771" AC="1"', 'Serial="B03B9999" SubType="32771" TreeType="32771" AC="0"')
  // The claims (#68): the rights as above, and the Daytimer's Feiertag from 06:00.
  .replace(users(prm(DOOR, ALWAYS) + prm(FROST, null, "3"), "", prm(DOOR, WORKDAYS)), users("", prm(DOOR, WORKDAYS), prm(FROST, null, "3") + prm(DOOR, WORKDAYS2)))
  .replace('<Entry From="390" To="480" V="21"/>', '<Entry From="360" To="480" V="21"/>'));
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\n');
// The lockfile manages no block at first; later it manages the Divide block, which the module no longer has (#70).
const lockFile = join(dir, "logic.lock.json");
writeFileSync(lockFile, '{"objects":{}}');
writeFileSync(join(dir, "pages", "main.lox"), "# module\n");
// The module's claims on the Code Touch, the notification and the Daytimer (#68): lines 12, 13 and 14.
writeFileSync(join(dir, "pages", "access.loxl"), `extern haustuer = NfcCodeTouch(uuid: "${DOOR}")
extern frost = Notification(uuid: "${FROST}")
extern uhr = DayTimer(uuid: "${TIMER}")
access bewohner
\tgroup "Bewohner": "Workdays"
end
recipients familie
\tuser "bob"
end
schedule feiertag
end
haustuer.access = bewohner
frost.recipients = familie
uhr.schedule = feiertag
`);
// The module's test fails at first (#67).
const failTest = () => join(loxDir, "failtest");
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
const refusal = "compile error: managed block \\`teiler\\` is in the lockfile but not in the source. To delete it, add \\`removed teiler\\` (or pass --allow-removals); to rename it, add \\`moved teiler -> <new_slug>\\`.";
writeFileSync(fakeLox, `#!/bin/sh
case "$1 $2" in
  "logic diff") echo "Rename Licht OG to Licht oben" ;;
  "logic scan")
    # Before the git push (#71), as lox answers: a user's mail address in a config with Ema="…", where and how
    # long, never the value; exit 1 when it found any.
    shift 3; code=0; printf '{'; sep=
    for f in "$@"; do printf '%s"%s": [' "$sep" "$f"; sep=,
      if grep -q 'Ema="' "$f"; then printf '{"rule":"mail","object":"User #1","attr":"Ema","len":17}'; code=1; fi
      printf ']'; done
    echo '}'; exit $code ;;
  "logic test")
    # The module's test (#67): it fails while ${loxDir}/failtest exists.
    if [ -f "${loxDir}/failtest" ]; then
      echo 'test "Licht oben" ... FAILED'; echo '    step 1: expect licht.Q == 1 — actual 0'
      echo '1 tests: 0 passed, 1 failed'; exit 1
    fi
    echo 'test "Licht oben" ... ok'; echo '1 tests: 1 passed, 0 failed' ;;
  "logic compile")
    # The module lost the Divide block (#70) while the lockfile manages it: refused, unless removals are allowed;
    # then the lockfile keeps its tombstone. The program is the pending one, which has no Divide block.
    lock=logic.lock.json; out=out.Loxone; allow=; shift 2
    while [ $# -gt 0 ]; do
      case "$1" in --lock) lock=$2; shift ;; --out) out=$2; shift ;; --time) shift ;; --allow-removals) allow=1 ;; esac
      shift
    done
    if grep -q '"teiler":{' "$lock"; then
      [ -n "$allow" ] || { echo "${refusal}" >&2; exit 1; }
      printf '%s' '{"objects":{},"removed":{"${DIV}":{"slug":"teiler","type":"Div"}}}' > "$lock"
    fi
    [ "$out" = out.Loxone ] || cp out.Loxone "$out" ;;
  "config push")
    echo "$*" >> "${loxDir}/pushes"
    if grep -q '"teiler":{' logic.lock.json; then printf 'Error: Compiling the module failed\n\nCaused by:\n    %s\n' "${refusal}" >&2; exit 1; fi
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
  "extensions -o")
    # Before the push the old Touch is on the bus; after it, the new one, still offline.
    if [ -f "${loxDir}/pushed" ]; then
      echo '[{"name":"Tree Extension","online":true,"serial":"13000001","type":"Tree"},{"name":"Touch Küche","online":false,"parent":"Tree Extension","serial":"b03b9999","type":"Tree Device"}]'
    else
      echo '[{"name":"Tree Extension","online":true,"serial":"13000001","type":"Tree"},{"name":"Touch Küche","online":true,"parent":"Tree Extension","serial":"B03B368A","type":"Tree Device"}]'
    fi ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
`);
chmodSync(fakeLox, 0o755);
writeFileSync(failTest(), "");
const token = join(loxDir, "token");
writeFileSync(token, "secret\n");
const TEMP = "10000003-0000-0032-01ff504f94000001";
const liveFile = join(loxDir, "stream.ndjson");
writeFileSync(liveFile, JSON.stringify({ control: "Temperatur", state: "value", value: "21.5", uuid: TEMP }) + "\n"
  + JSON.stringify({ control: "Uhr", state: "entriesAndDefaultValue", uuid: TIMER,
    value: { schedule: true, default: 0, entries: [{ mode: 0, from: 420, to: 480, activate: false, value: 21 }] } }) + "\n");

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
  const asked = [];
  let dismissNext = false;
  // The answers to the next dialogs, in order (true: accept); then dismissNext, else accept.
  const answers = [];
  page.on("dialog", (d) => {
    dialogs++; asked.push(d.message());
    let yes = true;
    if (answers.length) yes = answers.shift();
    else if (dismissNext) { dismissNext = false; yes = false; }
    if (yes) d.accept(); else d.dismiss();
  });
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
  await step("the pending Touch replacement is a hardware change", async () => {
    // A click shows the compile; compare it with the deployed config from the Changes pane.
    await page.click('.hrev.k-pending');
    await page.click("#cmp-prev");
    await page.waitForSelector(".hrev.k-pending.sel");
    await page.waitForSelector(".chg-filter .pill.hw");
    assert.equal(await page.textContent(".chg-filter .pill.hw"), "hardware 2");
    await page.click(".chg-filter .pill.hw");
    const shown = await page.$$eval(".changes .chg .t", (n) => n.map((x) => x.textContent));
    assert.deepEqual(shown.sort(), ["Touch Küche · AC: 1 → 0", "Touch Küche · device replaced: 0x8003 Touch Tree, serial B03B368A → B03B9999"]);
    await page.click(".chg-filter .pill.hw");
    await page.click('.hrev >> nth=0');
  });
  await step("the review lists the rights and switching times the compile claims, and who loses a right (#68)", async () => {
    await page.click('.hrev.k-pending');
    await page.click("#cmp-prev");
    await page.waitForSelector(".hrev.k-pending.sel");
    // The module's claims come with GET /api/edits.
    await page.waitForFunction(() => /λ pages\/access\.loxl:12/.test(document.querySelector("#rights-box")?.textContent || ""));
    const text = await page.textContent("#rights-box");
    assert.match(text, /Rights and schedules 3 · 1 user loses a right/, text);
    assert.match(text, /Frostwarnung recipients λ pages\/access\.loxl:13⚠ alice stops getting it−alice\+bobbob now gets it/, text);
    // bob's schedule is another one of the same title: rescheduled, by its UUID.
    assert.match(text, /Haustür access λ pages\/access\.loxl:12⚠ alice loses access−alice \(Always\)\+group Bewohner \(Workdays\)~bob \(Workdays\) was WorkdaysUhr/, text);
    // The λ tooltip is plain text.
    assert.equal(await page.getAttribute('#rights-box .rt-src >> nth=1', "title"),
      "Managed by pages/access.loxl:12 (access bewohner, pages/access.loxl:4). Whoever the set leaves out loses access on the next push.");
    assert.match(text, /Uhr switching times λ pages\/access\.loxl:14/, text);
    // The Daytimer's changed mode: the week before next to the week after.
    assert.equal(await page.$$eval("#rights-box .rt-sch-r", (n) => n.length), 1);
    assert.equal(await page.getAttribute("#rights-box .rt-sch-r > div:nth-child(2)", "title"), "06:30–08:00 21");
    assert.equal(await page.getAttribute("#rights-box .rt-sch-r > div:nth-child(3)", "title"), "06:00–08:00 21");
    // A group's members show on hover.
    assert.equal(await page.getAttribute("#rights-box .rt-add[title]", "title"), "Members: bob");
    // The change list has them too, under their own filter.
    const rights = await page.$$eval(".changes .chg", (n) => n.filter((x) => x.querySelector(".k.rights")).map((x) => x.querySelector(".t").textContent));
    assert.deepEqual(rights, ["Frostwarnung · recipients: alice removed", "Frostwarnung · recipients: bob added",
      "Haustür · access: alice removed (Always)", "Haustür · access: group Bewohner added (Workdays)", "Haustür · access: bob, Workdays → Workdays"]);
    // Coloured by what they do: removed red, added green, changed amber.
    const how = await page.$$eval(".changes .chg .k.rights", (n) => n.map((x) => x.className.replace("k rights", "").trim() || "changed"));
    assert.deepEqual(how, ["removed", "added", "removed", "added", "changed"]);
    // The Code Touch opens in Properties, with its claim and who it opens for.
    await page.click('#rights-box .lnk:has-text("Haustür")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Haustür");
    const props = await page.textContent("#props");
    assert.match(props, /Access2/, props);
    assert.match(props, /Managed by pages\/access\.loxl:12 \(access bewohner, pages\/access\.loxl:4\)\. Whoever the set leaves out loses access on the next push\./, props);
    // Marked as the server compares: bob's schedule by UUID, though its title is the same.
    assert.match(props, /~bob \(Workdays\) was Workdays\+group Bewohner \(Workdays\)−alice \(Always\)/, props);
    assert.match(props, /Opens forbob/, props);
    assert.match(props, /⚠ alice loses access here\./, props);
    if (process.env.SHOT) await page.screenshot({ path: process.env.SHOT + "-rights.png" });
    // The Daytimer's Schedule section names its claim.
    await page.click('#rights-box .lnk:has-text("Uhr")');
    await page.waitForFunction(() => document.querySelector(".ph-t")?.textContent === "Uhr");
    assert.match(await page.textContent("#props"), /Managed by pages\/access\.loxl:14 \(schedule feiertag, pages\/access\.loxl:10\)\. The push replaces these switching times/);
    // Names stay on the page: not in the address or the title, and never in vislox's log.
    assert.ok(!/alice|bob|Bewohner/.test(page.url() + (await page.title())), page.url());
    await page.click('.hrev >> nth=0');
  });
  await step("when the access changes can't be read, Apply's confirmation says so (#68)", async () => {
    await page.route("**/api/apply/changes*", (r) => r.abort());
    dismissNext = true;
    await page.click("#btn-apply");
    for (const end = Date.now() + 20000; !asked.length && Date.now() < end;) await new Promise((ok) => setTimeout(ok, 50));
    assert.match(asked[0] || "", /⚠ Couldn't check access changes: .+\. Applying may take away someone's access/, asked[0]);
    await page.unroute("**/api/apply/changes*");
    // Dismissed: nothing was sent.
    assert.equal(applies, 0);
    await page.waitForSelector("#btn-apply:not([disabled])");
    // The aborted request is the only error the browser logs.
    for (let i; (i = errors.findIndex((e) => /ERR_FAILED|Failed to load resource/.test(e))) >= 0;) errors.splice(i, 1);
    dialogs = 0; asked.length = 0;
  });
  await step("a failing test stops Apply at its Tests step, before anything is sent (#67)", async () => {
    // This stand-in has no lox logic check, so the browser knows of no test: the server's preflight refuses.
    await page.click("#btn-apply");
    await page.waitForFunction(() => /1 of 1 test fails/.test(document.querySelector("#deploy-msg")?.textContent || ""), null, { timeout: 20000 });
    assert.match(await page.textContent("#deploy-msg"), /1 of 1 test fails \(lox logic test\), so nothing was deployed[\s\S]*test "Licht oben" failed[\s\S]*expect licht\.Q == 1 — actual 0/);
    assert.equal(await state("Module compiles"), "skip");
    assert.equal(await state("Tests pass"), "fail");
    assert.equal(await state("Download and drift check"), "wait");
    assert.equal(existsSync(join(loxDir, "pushed")), false);
    assert.equal(applies, 1);
    // The refusal (409) is the only error the browser logs.
    // The browser may report it after the message shows: wait for it.
    let i = -1;
    for (const end = Date.now() + 10000; (i = errors.findIndex((e) => /status of 409/.test(e))) < 0 && Date.now() < end;) {
      await new Promise((ok) => setTimeout(ok, 50));
    }
    assert.ok(i >= 0, "the 409 was logged: " + errors.join(" | "));
    errors.splice(i, 1);
    unlinkSync(failTest());
    dialogs = 0; applies = 0; asked.length = 0;
  });
  await step("a block the module lost: Apply asks to remove exactly it, and nothing is removed unasked (#70)", async () => {
    // The lockfile manages the Divide block, which the module no longer has: lox refuses it without consent.
    writeFileSync(lockFile, `{"objects":{"teiler":{"uuid":"${DIV}","type":"Div"}}}`);
    answers.push(true, false);
    await page.click("#btn-apply");
    for (const end = Date.now() + 20000; asked.length < 2 && Date.now() < end;) await new Promise((ok) => setTimeout(ok, 50));
    assert.equal(asked.length, 2, asked.join("\n---\n"));
    // Exactly which block, where it is and what is wired to it.
    assert.match(asked[1], /^Remove 1 block from the Miniserver\?/, asked[1]);
    assert.match(asked[1], /• O1002 \(Div, teiler\) on page "Erdgeschoss": wired from Temperatur\.\S+ → \S+/, asked[1]);
    assert.match(asked[1], /The revision "Removals allowed" shows what would run then\. Apply deploys exactly this/, asked[1]);
    // Dismissed: the plan is forgotten, here and on the server (#70).
    await page.waitForFunction(() => !S.removals && !document.querySelector(".hrev.k-removals"));
    await page.waitForFunction(async () => (await (await fetch("/api/history")).json()).actions?.removals == null);
    // The server refused the one request, at its Removals step, and nothing was removed or pushed.
    await page.waitForFunction(() => /lox deletes them only when you allow it/.test(document.querySelector("#deploy-msg")?.textContent || ""), null, { timeout: 20000 });
    assert.equal(applies, 1);
    assert.equal(await state("Removals you confirmed"), "fail");
    assert.equal(await state("Tests pass"), "wait");
    assert.match(readFileSync(lockFile, "utf8"), /"teiler":\{/);
    assert.equal(existsSync(join(loxDir, "pushes")), false);
    let i = -1;
    for (const end = Date.now() + 10000; (i = errors.findIndex((e) => /status of 409/.test(e))) < 0 && Date.now() < end;) {
      await new Promise((ok) => setTimeout(ok, 50));
    }
    assert.ok(i >= 0, "the 409 was logged: " + errors.join(" | "));
    errors.splice(i, 1);
    await page.click('.hrev >> nth=0');
    dialogs = 0; applies = 0; asked.length = 0;
  });
  await step("Apply shows its steps as they happen", async () => {
    // A downloaded config no .gitignore keeps out: Apply's commit takes it along (#71).
    writeFileSync(join(dir, "live.Loxone"), `<C Type="Document">\n\t<C Type="User" Title="carol" Ema="${CAROL}"/>\n</C>\n`);
    // With live values on, the confirmation names the schedule the Miniserver runs differently.
    await page.click("#sw-live .track");
    await page.waitForFunction(() => document.querySelector("#chk-live").checked);
    // The pending revision shows no values, but they come in.
    await page.waitForFunction(() => S.live.values.has("10000030-0000-0300-ffff504f94000001"));
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
    // The Apply's confirmation, then the removal's (#70): the server refused the first request for it. Then Apply's
    // confirmation again, now about the plan's program, which the retry deploys: its devices and rights too.
    assert.equal(dialogs, 3, asked.join("\n---\n"));
    assert.match(asked[1], /^Remove 1 block from the Miniserver\?/, asked[1]);
    assert.match(asked[2], /Deploy "Removals allowed" to the Miniserver, with the removals you confirmed\?/, asked[2]);
    assert.match(asked[2], /This reconfigures 1 device/, asked[2]);
    assert.match(asked[2], /⚠ alice loses access to Haustür/, asked[2]);
    assert.doesNotMatch(asked[2], /pending revision is from before/, asked[2]);
    assert.match(asked[0], /The Miniserver runs other switching times than this config for:\n  Uhr: Feiertag \(claimed by pages\/access\.loxl:14\)\n/, asked[0]);
    // …and who loses a right, with the claim that takes it away (#68).
    // bob's right moves to another schedule, but the group still lets him in under the old one: no loss (#76).
    assert.match(asked[0], /⚠ alice loses access to Haustür \(pages\/access\.loxl:12\)\n⚠ alice stops getting Frostwarnung \(pages\/access\.loxl:13\)\n\nbob starts getting Frostwarnung\n\n/, asked[0]);
    // The confirmation says which devices this reconfigures, and that the new serial is not on the bus.
    assert.match(asked[0], /⚠ Not on the bus \(lox extensions does not list it\): Touch Küche \(Touch Tree\), serial B03B9999/);
    assert.match(asked[0], /This reconfigures 1 device: it stores the new configuration/);
    assert.match(asked[0], /Touch Küche \(Touch Tree\): replaced, serial B03B368A → B03B9999/);
    assert.equal(applies, 2, "the Apply, then once more with the removal confirmed");
    assert.equal(await state("Removals you confirmed"), "ok");
    assert.match(await page.textContent("#deploy-steps"), /1 block removed in the compile you confirmed \(1 from the Miniserver\)/);
    // Compiled in with the consent; the push itself never gets --allow-removals.
    assert.doesNotMatch(readFileSync(join(loxDir, "pushes"), "utf8"), /allow-removals/);
    assert.match(readFileSync(lockFile, "utf8"), /"removed":\{"10000006/);
    assert.equal(await state("Tests pass"), "ok");
    assert.match(await page.textContent("#deploy-steps"), /Tests pass.*1 test passed/);
    // The first request's refusal (409) is the only error the browser logs for it.
    const i = errors.findIndex((e) => /status of 409/.test(e));
    assert.ok(i >= 0, "the 409 was logged: " + errors.join(" | "));
    errors.splice(i, 1);
    for (const l of ["Download and drift check", "Upload", "Program restarting", "Back online", "New config loaded", "Verified", "Committed"]) {
      assert.equal(await state(l), "ok", l);
    }
    // Deployed and committed, but lox logic scan found the stray config's personal data: not pushed (#71).
    assert.equal(await state("No personal data in git"), "fail");
    assert.equal(await state("Pushed to git"), "skip");
    assert.match(await page.textContent("#deploy-msg"), /^Applied, but (not verified and )?not pushed to git\./);
  });
  await step("the blocked push lists where the personal data is, never the value, and Push to git retries (#71)", async () => {
    await page.waitForSelector("#deploy-push:not(.hidden)");
    const text = await page.textContent("#deploy-push");
    assert.match(text, /^Not pushed to git: personal data/, text);
    assert.match(text, /The Miniserver runs the deployed config and the commit records it: only the git remote is behind\./, text);
    assert.match(text, /live\.Loxone:2mail — User #1 Ema \(17 chars\)/, text);
    assert.match(text, /git rm --cached -- live\.Loxone/, text);
    assert.match(await page.textContent("#deploy-msg"), /live\.Loxone:2: mail — User #1 Ema \(17 chars\)/);
    assert.ok(!(await page.content()).includes(CAROL));
    const remote = () => execFileSync("git", ["-C", join(root, "remote.git"), "log", "-1", "--format=%s"]).toString().trim();
    assert.equal(remote(), "Deployed");
    // Push to git scans again: still there, so still not pushed (409).
    await page.click("#btn-push");
    await page.waitForFunction(() => /Not pushed: lox logic scan found personal data/.test(document.querySelector("#deploy-msg")?.textContent || ""), null, { timeout: 20000 });
    let i = -1;
    for (const end = Date.now() + 10000; (i = errors.findIndex((e) => /status of 409/.test(e))) < 0 && Date.now() < end;) {
      await new Promise((ok) => setTimeout(ok, 50));
    }
    assert.ok(i >= 0, "the 409 was logged: " + errors.join(" | "));
    errors.splice(i, 1);
    assert.equal(remote(), "Deployed");
    // Kept out of git as the box says: Push to git pushes.
    git("rm", "-q", "--cached", "--", "live.Loxone");
    writeFileSync(join(dir, ".gitignore"), "/live.Loxone\n");
    git("add", ".gitignore");
    git("commit", "-q", "--amend", "--no-edit");
    await page.waitForSelector("#btn-push:not([disabled])");
    await page.click("#btn-push");
    await page.waitForSelector("#deploy-push.hidden", { state: "attached", timeout: 20000 });
    await page.waitForFunction(() => /Pushed to git\./.test(document.querySelector("#deploy-msg")?.textContent || ""));
    assert.match(remote(), /Rename Licht OG/);
    assert.doesNotMatch(execFileSync("git", ["-C", join(root, "remote.git"), "ls-tree", "--name-only", "HEAD"]).toString(), /live\.Loxone/);
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
  await step("after Apply, the reconfigured Touch shows that it is offline", async () => {
    await page.waitForSelector("#deploy-new .hw-dev.off");
    assert.match(await page.textContent("#deploy-new .hw-dev"), /replacedTouch Küche\s*offline/);
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
  await step("no user's name in vislox's log (#68)", async () => {
    assert.ok(!/alice|Bewohner|carol/.test(log), log);
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
