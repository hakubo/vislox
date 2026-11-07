// Browser test for Roll back (#69): a lox.toml project after an Apply, whose push left its downloads in
// backups/ (pre-push and post-push), and a stand-in lox. The History offers "Roll back to before this" on the
// deployed Apply; the server shows what it would deploy as a revision compared with what runs now; the same
// confirmation as Apply's asks; then Apply's steps, a commit and git push. It refuses while the module has
// changes nobody committed.
//
//   VISLOX_BIN=target/release/vislox node tests/ui/rollback.mjs
//
// Nothing here talks to a Miniserver: lox is a script, and the lox configuration points at a port that drops
// every connection but the structure file; a stand-in FTP server only greets, so the preflight passes.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { createServer as createHttpServer } from "node:http";
import { createServer as createTcpServer } from "node:net";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync, existsSync, unlinkSync, realpathSync } from "node:fs";
import { crc32 } from "node:zlib";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const bin = process.env.VISLOX_BIN || join(here, "../../target/release/vislox");
const fixture = join(here, "../fixtures/house.Loxone");

/** A config ZIP as the Miniserver writes one: sps0.LoxCC (an LZ4 block of literals, with Loxone's header), stored. */
function configZip(xml) {
  const lits = Buffer.from(xml);
  const tok = [Math.min(lits.length, 15) << 4];
  if (lits.length >= 15) { let r = lits.length - 15; while (r >= 255) { tok.push(255); r -= 255; } tok.push(r); }
  const payload = Buffer.concat([Buffer.from(tok), lits]);
  const head = Buffer.alloc(16);
  head.writeUInt32LE(0xaabbccee, 0); head.writeUInt32LE(payload.length, 4);
  head.writeUInt32LE(lits.length, 8); head.writeUInt32LE(crc32(lits) >>> 0, 12);
  const data = Buffer.concat([head, payload]), name = Buffer.from("sps0.LoxCC"), crc = crc32(data) >>> 0;
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0); local.writeUInt16LE(20, 4); local.writeUInt16LE(0x21, 12);
  local.writeUInt32LE(crc, 14); local.writeUInt32LE(data.length, 18); local.writeUInt32LE(data.length, 22); local.writeUInt16LE(name.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0); central.writeUInt16LE(20, 4); central.writeUInt16LE(20, 6); central.writeUInt16LE(0x21, 14);
  central.writeUInt32LE(crc, 16); central.writeUInt32LE(data.length, 20); central.writeUInt32LE(data.length, 24); central.writeUInt16LE(name.length, 28);
  const cdAt = local.length + name.length + data.length, cd = Buffer.concat([central, name]);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(1, 8); end.writeUInt16LE(1, 10); end.writeUInt32LE(cd.length, 12); end.writeUInt32LE(cdAt, 16);
  return Buffer.concat([local, name, data, cd, end]);
}

const root = mkdtempSync(join(tmpdir(), "vislox-rollback-"));
const dir = join(root, "project");
mkdirSync(join(dir, "pages"), { recursive: true });
mkdirSync(join(dir, "backups"), { recursive: true });
const git = (...a) => execFileSync("git", ["-C", dir, "-c", "user.name=t", "-c", "user.email=t@example.com", ...a]).toString();
// Before the Apply: the fixture. The Apply renamed Licht OG and added a file to the module.
const before = readFileSync(fixture, "utf8");
const applied = before.replace('Title="Licht OG"', 'Title="Licht oben"');
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\nlock = "logic.lock.json"\n');
writeFileSync(join(dir, "base.Loxone"), before);
writeFileSync(join(dir, "out.Loxone"), before);
writeFileSync(join(dir, "pages", "main.loxl"), "# v1\n");
writeFileSync(join(dir, "logic.lock.json"), '{"lock": 1}\n');
// lox's backups directory ignores itself: configs carry personal data.
writeFileSync(join(dir, "backups", ".gitignore"), "*\n");
execFileSync("git", ["init", "-q", "--bare", join(root, "remote.git")]);
git("init", "-q");
git("add", "-A");
git("commit", "-q", "-m", "Deployed");
git("remote", "add", "origin", join(root, "remote.git"));
git("push", "-q", "-u", "origin", "HEAD");
writeFileSync(join(dir, "base.Loxone"), applied);
writeFileSync(join(dir, "out.Loxone"), applied);
writeFileSync(join(dir, "pages", "main.loxl"), "# v2\n");
writeFileSync(join(dir, "pages", "licht.loxl"), "# the rename\n");
writeFileSync(join(dir, "logic.lock.json"), '{"lock": 2}\n');
git("add", "-A");
git("commit", "-q", "-m", "Rename Licht OG to Licht oben");
git("push", "-q");
const apply = git("rev-parse", "HEAD").trim();
// What that Apply's lox config push left: the download from before it, and the one it verified.
const STAMP = "20261107-091500";
writeFileSync(join(dir, "backups", `${STAMP}-pre-push.zip`), configZip(before));
writeFileSync(join(dir, "backups", `${STAMP}-pre-push.Loxone`), before);
writeFileSync(join(dir, "backups", `${STAMP}-post-push.Loxone`), applied);

const listen = (srv) => new Promise((ok) => srv.listen(0, "127.0.0.1", () => ok(srv.address().port)));
const webPort = await listen(createHttpServer((req, res) => {
  if (req.url.toLowerCase() !== "/data/loxapp3.json") { req.socket.destroy(); return; }
  res.setHeader("Content-Type", "application/json");
  res.end(JSON.stringify({ lastModified: "2026-11-07 09:15:00", msInfo: { serialNr: "504F94000001" }, rooms: {}, cats: {}, controls: {} }));
}));
const ftpPort = await listen(createTcpServer((s) => { s.on("error", () => {}); s.end("220 fake FTP\r\n"); }));
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const loxConfig = join(loxDir, "config.yaml");
writeFileSync(loxConfig, `host: http://127.0.0.1:${webPort}\nuser: u\npass: p\nserial: 504F94000001\n`);
// The stand-in lox. `logic compile` copies --base to --out (the module from before gives back what ran
// before), and in the project lox.toml's base to out. `config push` downloads what runs now as its pre-push;
// without --force it stages lox.toml's base and says where; with it, it "deploys" lox.toml's base, keeps it as
// its post-push download and points lox.toml's base at that, as lox does. Each logs its arguments.
const fakeLox = join(loxDir, "lox");
writeFileSync(fakeLox, `#!/bin/sh
# VISLOX_TEST_LOX_DELAY (ms): compile and test as slowly as a busy CI runner does.
slow() { sleep "$(awk "BEGIN { print \${VISLOX_TEST_LOX_DELAY:-0} / 1000 }")"; }
base() { sed -n 's/^base *= *"\\(.*\\)"/\\1/p' lox.toml; }
case "$1 $2" in
  # Before the git push (#71): no personal data in any config it is given.
  "logic scan") shift 3; printf '{'; sep=; for f in "$@"; do printf '%s"%s": []' "$sep" "$f"; sep=,; done; echo '}' ;;
  "logic test") slow; echo 'test "Licht" ... ok'; echo '1 tests: 1 passed, 0 failed' ;;
  "logic compile")
    slow
    if [ $# -eq 2 ]; then cp "$(base)" out.Loxone; exit 0; fi
    shift 2
    while [ $# -gt 0 ]; do case "$1" in --base) b=$2; shift;; --out) o=$2; shift;; esac; shift; done
    cp "$b" "$o" ;;
  "config push")
    stamp=$(date +%Y%m%d-%H%M%S)
    echo "Downloading the live config..." >&2; sleep 1
    cp backups/${STAMP}-post-push.Loxone "backups/$stamp-pre-push.Loxone"
    if [ "$3" != "--force" ]; then
      echo "$@" > "${loxDir}/stage-args"
      cp "$(base)" "backups/$stamp-push.Loxone"
      echo "Staged backups/$stamp-push.zip (extracts to exactly backups/$stamp-push.Loxone). Run again with --force to upload it." >&2
      exit 0
    fi
    echo "$@" > "${loxDir}/push-args"
    echo "Uploading as sps_new.zip..." >&2; sleep 1
    echo "Restarting the program..." >&2; sleep 1
    echo "Downloading the config again to verify..." >&2; sleep 1
    b=$(base)
    cp "$b" out.Loxone
    cp "$b" "backups/$stamp-post-push.Loxone"
    sed "s|^base *=.*|base = \\\"backups/$stamp-post-push.Loxone\\\"|" lox.toml > lox.toml.new && mv lox.toml.new lox.toml
    touch "${loxDir}/pushed"
    echo "Pushed and verified." >&2 ;;
  "log -n") echo "2026-11-07 09:15:02.000;PRG start program" ;;
  "extensions -o") echo '[]' ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
`);
chmodSync(fakeLox, 0o755);
const token = join(loxDir, "token");
writeFileSync(token, "secret\n");

const port = 7990 + Math.floor(Math.random() * 9);
const server = spawn(bin, [dir, "--listen", `127.0.0.1:${port}`, "--lox", fakeLox, "--admin-token-file", token], {
  stdio: ["ignore", "ignore", "pipe"],
  env: {
    ...process.env, LOX_CONFIG: loxConfig, VISLOX_FTP_PORT: String(ftpPort), VISLOX_CACHE: join(root, "cache"),
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
const errors = [], asked = [];
let failed = false;
let page;
try {
  page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  let dismissNext = false, holdNext = false, held = null, rollbacks = 0;
  page.on("dialog", (d) => {
    asked.push(d.message());
    if (holdNext) { holdNext = false; held = d; } else if (dismissNext) { dismissNext = false; d.dismiss(); } else d.accept();
  });
  page.on("request", (r) => { if (r.method() === "POST" && new URL(r.url()).pathname === "/api/rollback") rollbacks++; });
  await page.addInitScript(() => localStorage.setItem("vislox.token", "secret"));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForSelector(".tab");
  const step = async (name, fn) => { await fn(); console.log("ok -", name); };
  const state = (label) => page.$eval("#deploy-steps", (box, l) => {
    const li = [...box.querySelectorAll("li")].find((x) => x.querySelector(".l").textContent === l);
    return li?.className.replace("st-", "");
  }, label);
  /** Wait until the statement holds, for the deploy message's text. */
  const msgMatches = (re, timeout = 20000) => page.waitForFunction((src) => new RegExp(src).test(document.querySelector("#deploy-msg")?.textContent || ""), re.source, { timeout });
  const drop409 = async () => {
    let i = -1;
    for (const end = Date.now() + 10000; (i = errors.findIndex((e) => /status of 409/.test(e))) < 0 && Date.now() < end;) await new Promise((ok) => setTimeout(ok, 50));
    assert.ok(i >= 0, "the 409 was logged: " + errors.join(" | "));
    errors.splice(i, 1);
  };

  await step("the deployed Apply offers a Roll back; the first commit, not a push, none", async () => {
    await page.waitForSelector(".rb-row .rb-btn:not([disabled])");
    assert.equal(await page.$$eval(".rb-row", (n) => n.length), 1);
    // The row sits under the Apply's entry, which is on the Miniserver.
    assert.match(await page.$eval(".rb-row", (r) => r.previousElementSibling.textContent), /Rename Licht OG to Licht oben[\s\S]*on Miniserver/);
    assert.match(await page.getAttribute(".rb-btn", "title"), new RegExp(`backups/${STAMP}-pre-push\\.zip`));
    if (process.env.SHOT) await page.locator("#right").screenshot({ path: process.env.SHOT + "-offer.png" });
  });
  await step("with changes nobody committed in the module, it is refused and nothing changes", async () => {
    writeFileSync(join(dir, "pages", "draft.loxl"), "# not committed\n");
    await page.click(".rb-btn");
    await msgMatches(/Cannot roll back: The project has changes that are not committed/);
    assert.match(await page.textContent("#deploy-msg"), /pages\/draft\.loxl/);
    assert.equal(asked.length, 0, "no confirmation");
    assert.equal(rollbacks, 0);
    await drop409();
    unlinkSync(join(dir, "pages", "draft.loxl"));
  });
  await step("the confirmation shows what it deploys, compared with what runs now; dismissed, nothing is sent", async () => {
    holdNext = true;
    await page.click(".rb-btn");
    for (const end = Date.now() + 20000; !asked.length && Date.now() < end;) await new Promise((ok) => setTimeout(ok, 50));
    assert.match(asked[0] || "", /Roll back "Rename Licht OG to Licht oben"\?/, asked[0]);
    assert.match(asked[0], new RegExp(`runs what it ran before it \\(backups/${STAMP}-pre-push\\.zip\\): 1 change from what it runs now`), asked[0]);
    assert.match(asked[0], /The module and lockfile go back to before it/);
    assert.match(asked[0], /tests run \(lox logic test\), lox config push checks that the Miniserver still runs this config/);
    // Behind it (the count above comes from it), the view: the revision a Roll back deploys, compared with
    // the Apply. While the dialog is open this page runs no script: ask the server from another browser,
    // which never shows the plan (it is the asking tab's).
    assert.equal(rollbacks, 0);
    assert.equal(existsSync(join(loxDir, "push-args")), false);
    const other = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    await other.addInitScript(() => localStorage.setItem("vislox.token", "secret"));
    await other.goto(`http://127.0.0.1:${port}/`);
    await other.waitForSelector(".rb-row");
    assert.equal(await other.$$eval(".hrev.k-rollback", (n) => n.length), 0);
    const view = await other.evaluate(async (q) => (await fetch(`/api/view?${q}`)).json(), `rev=rollback:${apply}&base=git:${apply}`);
    assert.ok(view.changes.some((c) => /Licht oben → Licht OG/.test(JSON.stringify(c))), JSON.stringify(view.changes));
    await other.close();
    // Cancelled: the plan is gone, here and on the server, and the view is back on what runs.
    await held.dismiss();
    await page.waitForFunction(() => !document.querySelector(".hrev.k-rollback"));
    if (process.env.SHOT) await page.screenshot({ path: process.env.SHOT + "-review.png" });
    await page.waitForFunction(() => /Rename Licht OG to Licht oben/.test(document.querySelector(".hrev.sel")?.textContent || ""));
    assert.equal(await page.evaluate(async (id) => (await fetch(`/api/view?rev=${id}`)).status, "rollback:" + apply), 422);
    // That 422 is the only error the browser logs here.
    for (const end = Date.now() + 5000; !errors.some((e) => /status of 422/.test(e)) && Date.now() < end;) await new Promise((ok) => setTimeout(ok, 50));
    errors.splice(errors.findIndex((e) => /status of 422/.test(e)), 1);
    asked.length = 0;
  });
  await step("accepted, Apply's steps run, and every one ends done", async () => {
    await page.waitForSelector(".rb-row .rb-btn:not([disabled])");
    await page.click(".rb-btn");
    await page.waitForSelector("#deploy-steps:not(.hidden)");
    // vislox's own check by load gives up on the stand-in's port after a few tries.
    await msgMatches(/^Rolled back/, 60000);
    assert.equal(asked.length, 1, "one confirmation");
    assert.equal(rollbacks, 1);
    for (const l of ["Backup checked", "Module before the Apply compiles to it", "Tests pass", "Upload is what you reviewed", "Download and drift check", "Upload",
      "Program restarting", "Back online", "Config loaded", "Verified", "Committed", "No personal data in git", "Pushed to git"]) {
      assert.equal(await state(l), "ok", l);
    }
    assert.match(await page.textContent("#deploy-steps"), /Backup checked.*backups\/20261107-091500-pre-push\.zip/);
    // lox pushed onto the download the Apply verified: it refuses unless the Miniserver still runs it.
    const post = join(realpathSync(dir), "backups", `${STAMP}-post-push.Loxone`);
    // Staged first, without --force: what it would upload must be what was reviewed.
    assert.equal(readFileSync(join(loxDir, "stage-args"), "utf8").trim(), `config push --patched-from ${post}`);
    assert.equal(readFileSync(join(loxDir, "push-args"), "utf8").trim(), `config push --force --patched-from ${post}`);
  });
  await step("the project is back to before the Apply, committed and pushed", async () => {
    assert.equal(readFileSync(join(dir, "base.Loxone"), "utf8"), before);
    // lox pointed the base at its download; vislox keeps the deployed config in git, as after Apply.
    assert.match(readFileSync(join(dir, "lox.toml"), "utf8"), /^base += "base\.Loxone"$/m);
    assert.equal(readFileSync(join(dir, "pages", "main.loxl"), "utf8"), "# v1\n");
    assert.equal(existsSync(join(dir, "pages", "licht.loxl")), false);
    assert.equal(readFileSync(join(dir, "logic.lock.json"), "utf8"), '{"lock": 1}\n');
    assert.equal(git("status", "--porcelain"), "");
    assert.equal(execFileSync("git", ["-C", join(root, "remote.git"), "log", "-1", "--format=%s"]).toString().trim(), "Roll back: Rename Licht OG to Licht oben");
    assert.match(git("log", "-1", "--format=%b"), new RegExp(`Restored backups/${STAMP}-pre-push\\.zip: what the Miniserver ran before ${apply.slice(0, 10)}`));
  });
  await step("the view shows what runs now, compared with the Apply it replaced", async () => {
    await page.waitForFunction(() => /Roll back: Rename Licht OG/.test(document.querySelector(".hrev.sel")?.textContent || ""));
    assert.match(await page.textContent(".hrev.base"), /Rename Licht OG to Licht oben/);
    const changes = await page.$$eval(".changes .chg .t", (n) => n.map((x) => x.textContent));
    assert.ok(changes.some((t) => t.includes("Licht oben → Licht OG")), changes.join(" | "));
    await page.waitForSelector("#deploy-new:not(.hidden)");
    assert.match(await page.textContent("#deploy-new .h"), /^Changed by this Roll back/);
    // The preview is gone: it was about the Apply, which no longer runs.
    assert.equal(await page.$$eval(".hrev.k-rollback", (n) => n.length), 0);
    if (process.env.SHOT) await page.screenshot({ path: process.env.SHOT + "-done.png" });
  });
  await step("the Roll back can itself be rolled back; the Apply now can't", async () => {
    await page.waitForFunction(() => document.querySelectorAll(".rb-row").length === 2);
    const rows = await page.$$eval(".rb-row", (n) => n.map((r) => ({ prev: r.previousElementSibling.textContent, off: r.querySelector("button").disabled, tip: r.querySelector("button").title })));
    assert.match(rows[0].prev, /Roll back: Rename Licht OG/);
    assert.equal(rows[0].off, false);
    assert.match(rows[1].prev, /Rename Licht OG to Licht oben/);
    assert.equal(rows[1].off, true);
    assert.match(rows[1].tip, /Only the newest deploy can be rolled back/);
  });
  await step("no errors in the browser", async () => {
    assert.deepEqual(errors, []);
  });
} catch (e) {
  failed = true;
  console.error("FAILED:", e.message);
  if (errors.length) console.error("browser errors:", errors);
  if (page) console.error("deploy message:", await page.textContent("#deploy-msg").catch(() => "?"),
    "\nsteps:", await page.textContent("#deploy-steps").catch(() => "?"), "\nasked:", asked);
  console.error(log);
} finally {
  await browser.close();
  server.kill("SIGINT");
}
process.exit(failed ? 1 : 0);
