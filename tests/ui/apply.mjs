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
import { createServer as createTcpServer } from "node:net";
import { mkdtempSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
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
copyFileSync(fixture, join(dir, "base.Loxone"));
// The pending compile: a rename.
writeFileSync(join(dir, "out.Loxone"), readFileSync(fixture, "utf8").replace('Title="Licht OG"', 'Title="Licht oben"'));
writeFileSync(join(dir, "lox.toml"), 'base = "base.Loxone"\nmodule = "pages"\nout = "out.Loxone"\n');
writeFileSync(join(dir, "pages", "main.lox"), "# module\n");
execFileSync("git", ["init", "-q", "--bare", join(root, "remote.git")]);
git("init", "-q");
git("add", "-A");
git("commit", "-q", "-m", "Deployed");
git("remote", "add", "origin", join(root, "remote.git"));
git("push", "-q", "-u", "origin", "HEAD");

// A lox configuration at a port that drops every connection, so no real
// Miniserver is reached, and an FTP server that only greets.
const listen = (srv) => new Promise((ok) => srv.listen(0, "127.0.0.1", () => ok(srv.address().port)));
const webPort = await listen(createTcpServer((s) => s.destroy()));
const ftpPort = await listen(createTcpServer((s) => { s.on("error", () => {}); s.end("220 fake FTP\r\n"); }));
const loxDir = mkdtempSync(join(tmpdir(), "vislox-lox-"));
const loxConfig = join(loxDir, "config.yaml");
writeFileSync(loxConfig, `host: 127.0.0.1:${webPort}\nuser: u\npass: p\nserial: 504F94000001\n`);
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
    echo "Pushed and verified." >&2 ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
`);
chmodSync(fakeLox, 0o755);
const token = join(loxDir, "token");
writeFileSync(token, "secret\n");
const TEMP = "10000003-0000-0032-01ff504f94000001";
const liveFile = join(loxDir, "stream.ndjson");
writeFileSync(liveFile, JSON.stringify({ control: "Temperatur", state: "value", value: "21.5", uuid: TEMP }) + "\n");

const port = 7990 + Math.floor(Math.random() * 9);
const server = spawn(bin, [dir, "--listen", `127.0.0.1:${port}`, "--lox", fakeLox, "--admin-token-file", token, "--live-from", liveFile], {
  stdio: ["ignore", "ignore", "pipe"],
  // Apply commits: a runner without a git identity needs one.
  env: {
    ...process.env, LOX_CONFIG: loxConfig, VISLOX_FTP_PORT: String(ftpPort),
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
  page.on("dialog", (d) => d.accept());
  await page.addInitScript(() => localStorage.setItem("vislox.token", "secret"));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForSelector(".tab");
  const step = async (name, fn) => { await fn(); console.log("ok -", name); };
  const state = (id) => page.$eval("#deploy-steps", (box, i) => {
    const li = [...box.querySelectorAll("li")].find((l) => l.querySelector(".l").textContent === i);
    return li?.className.replace("st-", "");
  }, id);

  await step("the compile is pending", async () => {
    await page.waitForSelector("#btn-apply:not([disabled])");
  });
  await step("Apply shows its steps as they happen", async () => {
    await page.click("#btn-apply");
    await page.waitForSelector("#deploy-steps:not(.hidden)");
    await page.waitForFunction(() => document.querySelector('#deploy-steps li[aria-current="step"] .l')?.textContent === "Program restarting", null, { timeout: 10000 });
    assert.equal(await state("Upload"), "ok");
    assert.equal(await state("Verified"), "wait");
    if (process.env.SHOT) await page.locator("#right").screenshot({ path: process.env.SHOT + "-running.png" });
  });
  await step("every step ends done", async () => {
    await page.waitForFunction(() => /^Applied/.test(document.querySelector("#deploy-msg")?.textContent || ""), null, { timeout: 20000 });
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
