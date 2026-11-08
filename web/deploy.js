/* exported act, checkAt, checkErrors, deployMsg, deployedRev, dfltList, lintAll, lintFindings, loadCheck, pendingChange, removalsPreview, renderActions, renderDeployed, renderLint, renderTests, rollbackPreview, rollbackRow, testFindings, testLine */
"use strict";
// Deployment pane, checks and Apply/Sync/Discard/Roll back (classic script).
// Reads: S.hist/view/viewRev/rev/base/sel/edits/check/live/app/deployed/applyReads,
// S.removals/removalsAt/rollbackRev/rollbackAt, UNDO and MD.open;
// editor pending summaries and the Miniserver/hardware/calendar reports.
// Writes: S.rev/base/follow/deployed/applyReads/removals/removalsAt/rollbackRev/rollbackAt,
// S.check/checkKey/checkSeq, local confirmation flags and render clock/last-outcome state.
// Refreshes: renderHistory, renderSelects, loadView, writeHash, loadHistory, setLive,
// renderDeployed, renderActions, renderCheck, renderTests, renderLint, renderStale,
// renderAddHud, renderPage, renderProps, applyFocus, mdRenderEdit, loadRunning, logStale;
// loadMs/loadCal/loadAppFromMs and loadBus/watchBus read status before/after deployment.
// Editing calls go through editAct/editShowNewest; confirmations and stale-response guards
// are unchanged. Defaults naming is shared with the per-kind editor in app.js.

// ── apply, sync, discard ───────────────────────────────────────────────
// Whoever edits the lox.toml project's module and compiles it (a person, or an agent working in the
// repository), the compile shows as the pending revision. Apply deploys it (lox config push) and
// pushes to git; Sync makes the Miniserver's running config the base; Discard drops the compile.
function adminToken(ask) {
  let t = "";
  try { t = localStorage.getItem("vislox.token") || ""; } catch { /* private mode */ }
  if (!t && ask) {
    t = (window.prompt("Admin token for changes (the contents of vislox's --admin-token-file):") || "").trim();
    if (t) try { localStorage.setItem("vislox.token", t); } catch { /* private mode */ }
  }
  return t;
}
async function act(path, body) {
  const t = adminToken(true);
  if (!t) throw new Error("Changes need the admin token.");
  const r = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Vislox-Token": t, "X-Vislox": "act" },
    body: JSON.stringify(body || {}),
  });
  const j = await r.json().catch(() => ({}));
  if (r.status === 401) try { localStorage.removeItem("vislox.token"); } catch { /* private mode */ }
  if (j.preflight || /apply|sync/.test(path)) loadMs(true);
  if (!r.ok) { const e = new Error(j.error || r.statusText); e.data = j; throw e; }
  return j;
}
function deployMsg(text, error, warn) {
  const m = $("deploy-msg");
  m.textContent = text || "";
  m.className = "deploy-msg" + (error ? " error" : warn ? " warn" : "") + (text ? "" : " hidden");
}
/** “ They include 3 values edited here and 1 block added here.”, or nothing. */
function editsHere() {
  const parts = [];
  if (editsPending().length) parts.push(`${plural(editsPending().length, "value")} edited here`);
  if (editsAdded().length) parts.push(`${plural(editsAdded().length, "block")} added here`);
  if (pagesCreated().length) parts.push(`${plural(pagesCreated().length, "page")} created here`);
  if (editsMoved().length) parts.push(`${plural(editsMoved().length, "block")} moved here`);
  if (schedsPending().length) parts.push(`switching times of ${plural(schedsPending().length, "timer")} edited here`);
  if (moodsPending().length) parts.push(`moods of ${plural(moodsPending().length, "lighting controller")} edited here`);
  if (membersPending().length) parts.push(`members of ${plural(membersPending().length, "central block")} edited here`);
  if (rulesPending().length) parts.push(`${plural(rulesPending().length, "Automatic Designer rule")} edited here`);
  if (accessPending().length) parts.push(`access to ${accessPending().length} NFC Code Touch${accessPending().length === 1 ? "" : "es"} edited here`);
  if (accessPending("recipients").length) parts.push(`recipients of ${plural(accessPending("recipients").length, "push notification")} edited here`);
  if (tilesPending().length) parts.push(`tiles of ${tilesPending().length === 1 ? "1 Touch Pure Flex" : `${tilesPending().length} Touch Pure Flex devices`} edited here`);
  // The module's constants and defaults (#81): a default names the blocks it changes.
  for (const p of constPending()) {
    parts.push(p.kind === "let" ? `the constant ${p.name} (${plural(p.uses, "use")})`
      : `the default ${p.name} ${p.port}, which changes ${plural(p.blocks.length, "block")} (${dfltList(p.blocks)})`);
  }
  const drawn = editsWires().filter((w) => w.now).length, out = editsWires().length - drawn;
  if (drawn) parts.push(`${plural(drawn, "wire")} drawn here`);
  if (out) parts.push(`${plural(out, "wire")} taken out here`);
  return parts.length ? ` They include ${parts.join(" and ")}.` : "";
}
/** The newest compile, if it differs from what is deployed. */
function pendingChange() {
  const p = S.hist?.pending?.[0];
  const base = p && revById(p.base);
  return p && base && p.blob !== base.blob ? p : null;
}
// One Apply at a time: from the click until the confirmation is answered (and the request sent),
// a second click does nothing.
let applyAsking = false;
function renderActions() {
  const a = S.hist?.actions;
  $("deploy-pane").classList.toggle("hidden", !a?.shown);
  // Add mode and Add entry wait while Apply or another action runs (#86).
  if (MD.open) mdRenderEdit();
  renderStale();
  renderAddHud();
  if (!a?.shown) return;
  const busy = !!a.running;
  const pend = pendingChange();
  renderCheck();
  $("deploy-pend").classList.toggle("idle", !pend && !a.running);
  $("deploy-pend").textContent = a.running === "Apply" ? "Deploying to the Miniserver… (download, check, upload, restart, verify)"
    : a.running === "Roll back" ? "Rolling back on the Miniserver… (backup, tests, download, check, upload, restart, verify)"
    : a.running === "Edit" ? "Compiling the edit… (lox logic compile; nothing is deployed)"
    : a.running === "Compile" ? "Compiling the module… (lox logic compile; nothing is deployed)"
    : a.running ? `${a.running}…` : pend ? `Compiled changes are pending (${fmtWhen(pend.time)}), not deployed.` +
      editsHere() + " Review them, then apply."
    : !a.apply.available ? `Apply is off: ${a.apply.reason}.`
    : adoptedUncommitted().length ? `Nothing pending: adopting changes nothing the Miniserver runs. ${plural(adoptedUncommitted().length, "block")} adopted here ${adoptedUncommitted().length === 1 ? "goes" : "go"} into git with the next Apply; Undo hands ${adoptedUncommitted().length === 1 ? "it" : "them"} back.`
    : `Nothing pending. ${a.edit?.available ? "Edit values in Properties, add blocks to a page, or edit" : "Edit"} the project's module and compile it (lox logic …); the result shows here for review.`;
  $("deploy-row").classList.toggle("hidden", !pend && !a.running && !removing());
  $("btn-apply").disabled = !a.apply.available || busy || (!pend && !removing()) || applyAsking || checkBroken() || !!testsFailing();
  const sync = $("btn-sync");
  sync.disabled = !a.apply.available || busy || !!pend;
  sync.classList.toggle("busy", a.running === "Sync");
  sync.querySelector(".lbl").textContent = a.running === "Sync" ? "Syncing…" : "Sync from Miniserver";
  sync.title = !a.apply.available ? `Cannot sync: ${a.apply.reason}`
    : pend ? "Apply or discard the pending changes first"
    : "Make the Miniserver's running config the project's base (lox config merge), then commit and push" + msWarn("sync");
  $("btn-discard").disabled = !a.apply.available || busy || !pend;
  $("btn-apply").title = !a.apply.available ? `Cannot apply: ${a.apply.reason}`
    : checkBroken() ? "Cannot apply: the module doesn't compile (see above)"
    : testsFailing() ? `Cannot apply: ${testsFailing()} (see Tests above)`
    : "Deploy to the Miniserver with lox config push, then commit and push to git" + msWarn("apply");
  renderSteps(a);
  renderPush(a);
  renderStaleLines(a);
  const last = a.last;
  if (last && last.at !== renderActions.shownAt) {
    renderActions.shownAt = last.at;
    const done = (verb) => verb + (last.warn && last.unpushed ? ", but not verified and not pushed to git" : last.warn ? ", but not verified"
      : last.unpushed ? ", but not pushed to git" : "") + ".\n";
    const head = { apply: [done("Applied"), "Apply failed.\n"],
      rollback: [done("Rolled back"), "Roll back failed.\n"], sync: ["", "Sync failed.\n"], discard: ["", "Discard failed.\n"], compile: ["", "Compile failed.\n"],
      push: ["", "Not pushed to git.\n"] }[last.kind] || ["", ""];
    deployMsg(head[last.ok ? 0 : 1] + last.message, !last.ok, last.ok && (last.warn || last.unpushed));
    // What the new program logged as it started.
    if (last.kind === "apply" || last.kind === "rollback") logStale();
    if (last.kind !== "discard" && S.hist?.running) loadRunning(true);
  }
}
// ── personal data before a git push (#71) ─────────────────────────────
// Every git push vislox makes (Apply, Roll back, Sync) first asks lox logic scan about what it would send: each
// config file in the commits the remote does not have. lox names where personal data is (element and attribute,
// the value's length), never the value. On a finding, a file lox cannot read, or no answer, nothing is pushed: the
// commit stays local (an Apply has deployed it), and this box lists the findings and what to do. Push to git scans
// again, then pushes.
/** The commits the git remote does not have, and why. */
function renderPush(a) {
  const box = $("deploy-push"), p = a.push;
  box.classList.toggle("hidden", !p);
  if (!p) { box.innerHTML = ""; return; }
  box.classList.toggle("error", p.state !== "behind");
  box.classList.toggle("hint", p.state === "behind");
  const head = { found: "Not pushed to git: personal data", unscanned: "Not pushed to git: couldn't check for personal data",
    failed: "Not pushed to git: git push failed", behind: "Not pushed to git yet" }[p.state] || "Not pushed to git";
  const ran = a.last?.ok && ["apply", "rollback"].includes(a.last.kind) && a.last.at >= p.at - 1
    ? "The Miniserver runs the deployed config and the commit records it: only the git remote is behind. " : "";
  const items = (p.findings || []).map((f) => `<li><span class="ck-at">${esc(f.file + (f.entry ? ` › ${f.entry}` : "") + (f.line ? `:${f.line}` : ""))}</span>` +
    `${esc(f.kind)} — ${esc(f.what)}${f.len != null ? ` (${plural(f.len, "char")})` : ""}${f.now ? "" : ` · only in ${f.commit ? `commit ${esc(f.commit)}` : "an earlier commit"}, deleted since`}</li>`);
  if (p.total > items.length) items.push(`<li>… and ${plural(p.total - items.length, "more finding")}</li>`);
  const why = !a.apply?.available ? `Cannot push: ${a.apply?.reason || ""}` : a.running ? `${a.running} is running` : "";
  box.innerHTML = `<div class="ck-h">${esc(head)}</div><div class="ck-sub">${esc(ran + p.message)}.</div>` +
    (items.length ? `<ul class="ps-list">${items.join("")}</ul>` : "") + (p.advice ? `<pre>${esc(p.advice)}</pre>` : "") +
    `<div class="ck-row"><button type="button" id="btn-push" class="ghost small"${why ? " disabled" : ""}
      title="${esc(why || "Check the commits not pushed yet for personal data again (lox logic scan), then git push")}">${a.running === "Push" ? "Pushing…" : "Push to git"}</button></div>`;
}
// ── obsolete statements (#80) ─────────────────────────────────────
// A `removed` or `moved` line of vislox's files is done once the compile applied it (lox logic lint calls it
// stale-lifecycle). The next edit removes it; this box offers to remove them now, as an edit of its own.
function renderStaleLines(a) {
  const box = $("deploy-lines"), lines = S.edits?.stale_lines || [];
  const r = editRev(), shown = !!lines.length && !!r && S.rev === r.id && !!a.edit?.available;
  box.classList.toggle("hidden", !shown);
  if (!shown) { box.innerHTML = ""; return; }
  const why = a.running ? `${a.running} is running` : UNDO.busy || UNDO.inflight ? "An edit is compiling" : "";
  box.innerHTML = `<div class="ck-h">${plural(lines.length, "obsolete statement")} in vislox-managed files</div>` +
    `<div class="ck-sub">The compile applied ${lines.length === 1 ? "it" : "them"} (lint: stale-lifecycle); the next edit removes ${lines.length === 1 ? "it" : "them"} too.</div>` +
    `<ul class="ps-list">${lines.map((l) => `<li class="mono">${esc(l)}</li>`).join("")}</ul>` +
    `<div class="ck-row"><button type="button" id="btn-tidy" class="ghost small"${why ? ` disabled title="${esc(why)}"` : ' title="Take the obsolete statements out of vislox-managed files and compile: the config does not change"'}>Remove obsolete statements</button></div>`;
}
$("deploy-lines").addEventListener("click", whenReady(async (e) => {
  if (!e.target.closest("#btn-tidy")) return;
  const r = editRev();
  if (!r) return;
  deployMsg("");
  try {
    const res = await editAct(r.blob, [{ op: "tidy" }], "Remove obsolete statements");
    await editShowNewest(res.blob);
  } catch (err) { deployMsg(err.message, true); }
}));
$("deploy-push").addEventListener("click", whenReady(async (e) => {
  if (!e.target.closest("#btn-push")) return;
  deployMsg("");
  try { const j = await act("/api/push"); deployMsg(j.message); } catch (err) { deployMsg(err.message, true); }
}));
/** Apply's steps as they happen: lox's progress, and what vislox sees of the program restart. */
function renderSteps(a) {
  const box = $("deploy-steps"), p = a.progress;
  box.classList.toggle("hidden", !p);
  clearTimeout(renderSteps.tick);
  if (!p) return;
  const icon = { wait: "·", run: "●", ok: "✓", skip: "–", fail: "✕" };
  const nowS = Date.now() / 1000;
  const secs = (s) => s.at == null ? "" : s.state === "run" ? `${Math.max(0, Math.round(nowS - s.at))} s` : s.end != null && s.end - s.at >= 1 ? `${s.end - s.at} s` : "";
  box.innerHTML = p.steps.map((s) => `<li class="st-${esc(s.state)}"${s.state === "run" ? ' aria-current="step"' : ""}>` +
    `<span class="ic" aria-label="${esc(s.state)}">${icon[s.state] || "·"}</span><span class="l">${esc(s.label)}</span><span class="t">${esc(secs(s))}</span>` +
    (s.detail ? `<span class="d">${esc(s.detail)}</span>` : "") + "</li>").join("");
  // The running step's clock.
  if (a.running === "Apply" || a.running === "Roll back") renderSteps.tick = setTimeout(() => renderSteps(S.hist?.actions || a), 1000);
}
/** After an Apply: view the deployed revision compared with the one it replaced, with live values,
 *  and list what changed with each block's first value. */
async function showDeployed() {
  await loadHistory();
  const d = S.hist?.actions?.last?.deployed;
  if (!d) return;
  const hist = S.hist.history;
  const rev = hist.find((r) => r.blob === d.after);
  if (!rev) return;
  const prev = d.before && d.before !== d.after ? hist.find((r) => r.blob === d.before) : null;
  S.rev = rev.id;
  S.base = prev?.id || rev.base;
  if (rev !== hist[0]) { S.follow = false; $("chk-follow").checked = false; }
  renderHistory(); renderSelects();
  await loadView();
  writeHash(true);
  if (S.hist.live?.available && !S.live.on) setLive(true, false); // not a retry after a stop
  // What this Apply changed, kept: the view may change before the box does.
  const v = S.view, seen = new Set(), rows = [], objs = {};
  for (const c of v?.compared ? v.changes : []) {
    if (!c.obj || seen.has(c.obj) || !v.objects[c.obj]) continue;
    seen.add(c.obj);
    rows.push(c);
    objs[c.obj] = v.objects[c.obj];
  }
  // Removed devices are gone from the bus on purpose.
  S.deployed = { rows, objs, hw: (v?.compared ? v.hardware || [] : []).filter((x) => x.how !== "removed"), kind: S.hist.actions?.last?.kind };
  renderDeployed();
  watchBus(S.deployed);
}
// ── hardware ───────────────────────────────────────────────────────────
// A change to an extension or a device object reconfigures hardware: the Miniserver sends the device a
// new configuration, which the device stores. `view.hardware` lists those devices. The devices on the bus
// come from `lox extensions` (GET /api/hardware, read-only): before Apply, a new serial that is not on the
// bus is a warning; after Apply, the reconfigured devices show whether they are online.
function hwInfo() { return S.hist?.hardware || { available: false }; }
/** The devices on the bus; with `ms`, gives up after that long (a hung lox must not block Apply's dialog). */
async function loadBus(ms) {
  const ctl = new AbortController(), t = ms && setTimeout(() => ctl.abort(), ms);
  let r;
  try { r = await fetch("/api/hardware", { headers: { "X-Vislox": "hardware" }, signal: ctl.signal }); } catch (e) {
    throw new Error(e.name === "AbortError" ? `no answer within ${ms / 1000} s` : e.message);
  } finally { clearTimeout(t); }
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  if (j.available === false) throw new Error(j.reason || "the bus cannot be read");
  if (j.error) throw new Error(j.error);
  return j.devices || [];
}
/** A device of `view.hardware` on the bus. The server writes serials as the bus list does; a device
 *  whose serial cannot be looked for (`check` false: not paired yet, or an Air device, whose `IP=` is no
 *  bus serial) is matched by name, and only to show its state, never to warn that it is missing. */
function onBus(d, bus) {
  return d.check ? bus.find((b) => b.serial === d.serial) : bus.find((b) => b.name === d.title);
}
/** What is on the Miniserver now: the newest revision that is not a compile. Both Apply's confirmation and
 *  the after-Apply list compare with it, so they count the same devices. */
function deployedRev() { return S.hist?.history.find(isDeployed) || null; }
/** How long Apply's confirmation waits for the device list and the bus, together, before it asks anyway
 *  (and says the bus check did not finish). */
const HW_WAIT = 6000;
/** The devices the pending change reconfigures, and, for new serials, whether they are on the bus; and what
 *  it does to access rights and notification recipients (#68): one request (`/api/apply/changes`). */
async function pendingHardware(pend, against) {
  const base = against || deployedRev()?.id || pend.base || "";
  const until = Date.now() + HW_WAIT;
  let devs, rights;
  try {
    const ctl = new AbortController(), t = setTimeout(() => ctl.abort(), HW_WAIT);
    try {
      const r = await fetch(`/api/apply/changes?rev=${encodeURIComponent(pend.id)}&base=${encodeURIComponent(base)}`, { signal: ctl.signal });
      const j = await r.json();
      if (!r.ok) throw new Error(j.error || r.statusText);
      devs = j.devices || [];
      rights = j.rights || [];
    } finally { clearTimeout(t); }
  } catch (e) { return { error: e.name === "AbortError" ? `no answer within ${HW_WAIT / 1000} s` : e.message, devs: [], rights: null }; }
  const fresh = devs.filter((d) => (d.how === "added" || d.how === "replaced" || d.how === "paired") && d.check);
  if (!fresh.length || !hwInfo().available) return { devs, rights, fresh: [] };
  try {
    const bus = await loadBus(Math.max(1000, until - Date.now()));
    return { devs, rights, fresh, missing: fresh.filter((d) => !onBus(d, bus)) };
  } catch (e) { return { devs, rights, fresh, busError: e.message }; }
}
function hwLine(d) {
  const what = d.how === "replaced" || d.how === "unpaired" ? `${d.how}, serial ${d.old_serial || "—"} → ${d.serial || "—"}`
    : d.how === "paired" ? `paired, serial ${d.serial || "—"}` : d.how === "changed" ? "settings"
    // A Tree device with no serial (created by the module, #85): lox config push says so too.
    : d.how === "added" && d.unpaired ? "added, not paired: pair it with a Tree search in Loxone Config" : d.how;
  return `  ${d.title} (${d.kind}): ${what}`;
}
/** Apply's confirmation: "This reconfigures 3 devices", and new serials that are not on the bus. */
function hwApplyText(hw) {
  if (!hw) return "";
  if (hw.error) return `Could not check which devices this reconfigures: ${hw.error}\n\n`;
  if (!hw.devs.length) return "";
  const n = hw.devs.length;
  let t = `This reconfigures ${plural(n, "device")}: ${n === 1 ? "it stores" : "they store"} the new configuration, and a device can drop offline while it does.\n` +
    hw.devs.slice(0, 8).map(hwLine).join("\n") + (n > 8 ? `\n  and ${n - 8} more` : "") + "\n\n";
  if (hw.missing?.length) {
    t = `⚠ Not on the bus (lox extensions does not list ${hw.missing.length === 1 ? "it" : "them"}): ` +
      hw.missing.map((d) => `${d.title} (${d.kind}), serial ${d.serial}`).join("; ") +
      `. Until that device is connected, its inputs and outputs do not work.\n\n` + t;
  } else if (hw.busError) {
    t = `The new serials were not checked against the bus (lox extensions: ${hw.busError}).\n\n` + t;
  }
  return t;
}
/** After Apply: ask the bus for the reconfigured devices' online state now and for a few minutes. */
const BUS_EVERY = 15000, BUS_FOR = 180000;
async function watchBus(d) {
  if (!d.hw.length || !hwInfo().available) return;
  d.bus = { at: 0, devices: null, error: "", offline: new Map(), until: Date.now() + BUS_FOR };
  while (S.deployed === d) {
    try {
      const bus = await loadBus();
      if (S.deployed !== d) return;
      d.bus.devices = bus; d.bus.error = ""; d.bus.at = Date.now();
      for (const x of d.hw) { const b = onBus(x, bus); if (b && !b.online && !d.bus.offline.has(x.uuid)) d.bus.offline.set(x.uuid, d.bus.at); }
    } catch (e) { if (S.deployed !== d) return; d.bus.error = e.message; }
    renderDeployed();
    if (Date.now() >= d.bus.until) return;
    await new Promise((ok) => setTimeout(ok, BUS_EVERY));
  }
}
function deployedHardware(d) {
  if (!d.hw?.length) return "";
  const b = d.bus;
  const rows = d.hw.slice(0, 12).map((x) => {
    const on = b?.devices ? onBus(x, b.devices) : null;
    const was = b?.offline.get(x.uuid);
    const state = !b ? "" : !b.devices ? (b.error ? "" : "checking…") : !on ? (x.check ? "not on the bus" : "—") : on.online ? (was ? `online (was offline at ${new Date(was).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })})` : "online") : "offline";
    const bad = b?.devices && ((!on && x.check) || (on && !on.online) || was);
    return `<div class="pr hw-dev${bad ? " off" : ""}"><span class="k">${esc(x.how === "changed" ? "settings" : x.how)}</span>` +
      `<button type="button" class="go" data-go="${esc(x.uuid)}" title="${esc(x.kind)}">${esc(x.title)}</button><span class="why">${esc(state)}</span></div>`;
  }).join("");
  const note = !hwInfo().available ? `The bus cannot be read: ${hwInfo().reason || "no lox"}.`
    : b?.error ? `Cannot read the bus: ${b.error}`
    : b && Date.now() < b.until ? "Checked every 15 s for 3 minutes (lox extensions)." : b ? "Checked with lox extensions." : "";
  return `<div class="h">Reconfigured devices</div>${rows}` + (d.hw.length > 12 ? `<div class="note">and ${d.hw.length - 12} more</div>` : "") + (note ? `<div class="note">${esc(note)}</div>` : "");
}
/** The changed blocks of the deployed revision, each with a link and its first live value. */
function renderDeployed() {
  const box = $("deploy-new"), d = S.deployed;
  const rows = (d?.rows || []).filter((c) => c.kind !== "removed");
  const check = d ? deployedAppCheck(d) : "";
  const hw = d ? deployedHardware(d) : "";
  box.classList.toggle("hidden", !rows.length && !check && !hw);
  if (!rows.length && !check && !hw) { box.innerHTML = ""; return; }
  const shown = rows.slice(0, 12).map((c) => {
    const o = d.objs[c.obj];
    const q = o.ports.find((p) => p.conns.some((x) => x.dir === "out")) || o.ports[0];
    const name = autoTitle(o.title) ? prettyType(o.ty) : o.title;
    return `<div class="pr"><span class="k ${esc(c.kind)}">${esc(KIND[c.kind]?.[1] || c.kind)}</span>` +
      `<button type="button" class="go" data-go="${esc(c.obj)}" title="${esc(o.path || "")}">${esc(name)}</button>${q ? lvSpan(q.uuid) : ""}</div>`;
  }).join("");
  box.innerHTML = `<div class="h">Changed by this ${S.deployed.kind === "rollback" ? "Roll back" : "Apply"}<button type="button" class="x" aria-label="Close">×</button></div>${shown}` +
    (rows.length > 12 ? `<div class="note">and ${rows.length - 12} more in Changes</div>` : "") +
    (rows.length ? '<div class="note">Values fill in as the Miniserver sends them; the canvas marks every change.</div>' : "") + hw + check;
}
/** After Apply: blocks the new config shows in the app that the Miniserver's new structure file lacks, or has
 *  elsewhere, and removed blocks it still shows. */
function deployedAppCheck(d) {
  const a = S.app;
  const objs = Object.values(d.objs).filter((o) => o.status === "removed" ? o.app?.shown || a?.controls?.[o.uuid] : o.app && (o.app.shown || a?.controls?.[o.uuid]));
  if (!objs.length) return "";
  // The server counts the structure files it read: one read after Apply started is the new config's.
  if (!awaitedAppRead()) {
    // A live session with the Miniserver reads the new structure file itself.
    const viaLive = S.live.on && /^https?:/.test(S.hist?.live?.source || "");
    const why = a?.available && viaLive ? "Waiting for the Miniserver's new structure file…" : "";
    const btn = a?.available && !viaLive ? `<button type="button" class="app-load lnk">Read what the app shows now</button>` : "";
    return why || btn ? `<div class="note">${esc(why)}${btn}</div>` : "";
  }
  const bad = objs.map((o) => [o, appIssues(o)]).filter(([, i]) => i.length);
  if (!bad.length) return `<div class="note">The app shows these blocks as the config says.</div>`;
  return `<div class="h warn-h">In the app</div>` + bad.map(([o, issues]) =>
    `<div class="pr app-warn"><span class="k">⚠</span><button type="button" class="go" data-go="${esc(o.uuid)}">${esc(autoTitle(o.title) ? prettyType(o.ty) : o.title)}</button><span class="why">${esc(issues.join("; "))}</span></div>`).join("");
}

$("deploy-new").addEventListener("click", whenReady((e) => {
  if (e.target.closest(".x")) { $("deploy-new").classList.add("hidden"); S.deployed = null; return; }
  if (e.target.closest(".app-load")) { loadAppFromMs(); return; }
  const g = e.target.closest("[data-go]");
  if (!g) return;
  const u = g.dataset.go, pg = S.view?.objects[u]?.page;
  if (pg) gotoPage(pg, { focus: u });
  select(u);
}));
$("btn-apply").onclick = whenReady(async () => {
  if (applyAsking) return;
  applyAsking = true;
  $("btn-apply").disabled = true;
  try { await apply(); } finally { applyAsking = false; renderActions(); }
});
/** For Apply's confirmation: the schedules on the Miniserver the pending config would change. */
async function pendingScheduleText(pend) {
  let blocks;
  try { blocks = await api(`/api/schedules?rev=${encodeURIComponent(pend.id)}`); } catch (e) {
    return `⚠ The switching times could not be compared with the Miniserver's (${e.message}): edits made in the app may be replaced.\n\n`;
  }
  if (!blocks.length) return "";
  if (!liveStreaming()) return "Live values are off, so the switching times were not compared with the Miniserver's (edits in the app).\n\n";
  const c = scheduleConflicts(blocks);
  return c.length ? `⚠ The Miniserver runs other switching times than this config for:\n  ${c.join("\n  ")}\nApplying replaces them with the config's (an edit in the app among them).\n\n` : "";
}
/** The push would remove blocks (#70): lox refuses the module for a block it lost, or the lockfile deletes blocks
 *  the Miniserver runs. Apply asks for exactly those first. */
function removing() { return S.check?.available && S.check.ok && S.check.removals ? S.check.removals : null; }
async function apply() {
  // Blocks the push would remove (#70): first exactly which, with consent; then Apply's confirmation is about the
  // program deployed with them removed (the plan), not the compile from before.
  if (removing()) {
    const consent = await askRemovals("applying");
    if (!consent) return;
    if (!(await confirmApply(S.removals.revision, true))) { await dropRemovals(); return; }
    await deploy("/api/apply", { allow_removals: consent }, "applying");
    return;
  }
  const pend = pendingChange();
  if (!pend || !(await confirmApply(pend, false))) return;
  await deploy("/api/apply", {}, "applying");
}
/** Apply's confirmation for the program `pend` would deploy: what changed on the Miniserver since, who loses access
 *  (#68), switching times, calendar entries, devices. `plan`: it is a removal plan (#70), deployed exactly. */
async function confirmApply(pend, plan) {
  // Ask the Miniserver once more whether its config changed after the base, and for the
  // calendar entries edited in the app.
  const [, calNow, hw] = await Promise.all([loadMs(true), loadCal(true), pendingHardware(pend)]);
  const stale = staleText();
  const sched = await pendingScheduleText(pend);
  const rights = rightsApplyText(hw);
  const src = !plan && S.check?.available && S.check.ok && S.check.stale;
  const srcText = src ? `⚠ The pending revision is from before the latest edit to ${src.files?.length ? src.files.join(", ") : "the module"}. ` +
    "lox config push compiles the module as it is, so Apply deploys changes this review does not show. Compile first to review them.\n\n" : "";
  return window.confirm((stale ? `⚠ ${stale} Applying overwrites what changed there since.\n\n` : "") + srcText + rights + sched + constApplyText() + (adoptedUncommitted().length ? `Adopt into vislox: ${adoptedUncommitted().map((b) => b.slug).join(", ")}. Ownership is committed with Apply.\n\n` : "") +
    calApplyWarning(calNow) + hwApplyText(hw) +
    (plan ? `Deploy "${pend.label}" to the Miniserver, with the removals you confirmed?\n\nvislox compiles exactly this program into the project (it refuses if anything changed since), ` +
      "then the tests must pass (lox logic test). "
      : "Deploy the pending changes to the Miniserver?\n\nFirst the module must compile and its tests pass (lox logic test). ") +
    "Then lox config push uploads the config and restarts the Miniserver's program (a few seconds), " +
    "checks that it runs exactly the compiled config, then vislox commits and pushes to git.");
}
// ── removed managed blocks (#70) ───────────────────────────────────────
// A push deletes every block its lockfile tombstones: those the module lost (lox compiles it only with the removal
// allowed, --allow-removals), and those an earlier compile tombstoned. The server works out what Apply would deploy
// with them removed, changing nothing (POST /api/removals/plan): the blocks it deletes, where they are, what is
// wired to them, who loses access or a notification with them (#68), and the program, shown here as the revision
// "Removals allowed" compared with what runs. Apply sends the program's blob id and exactly those blocks back; the
// server compiles exactly that, and puts the project back if anything fails before the push. A consent is for one
// request: a retry asks again.
/** The removal plan this tab asked for, while the server keeps it (one at a time: another tab's replaces it), it
 *  is about what runs, and younger than the server keeps it. */
function removalsPreview() {
  const p = S.removals;
  if (!p) return null;
  const deployed = deployedRev();
  const kept = S.hist?.actions ? S.hist.actions.removals : p.rev;
  if (kept !== p.rev || (deployed?.id ?? null) !== (p.base ?? null) || Date.now() - S.removalsAt > PREVIEW_TTL) { S.removals = null; return null; }
  return p;
}
/** Forget this tab's plan, here and on the server; show the newest revision again if the plan was shown. */
async function dropRemovals() {
  const p = S.removals;
  if (!p) return;
  S.removals = null;
  try { await fetch("/api/removals/cancel", { method: "POST", headers: { "Content-Type": "application/json", "X-Vislox": "removals" }, body: JSON.stringify({ rev: p.rev }) }); } catch { /* the server forgets it anyway */ }
  // The view load draws the Deploy pane again; without one, it is drawn here.
  if (S.rev === p.rev) { S.rev = editRev()?.id || p.base; S.base = null; renderHistory(); renderSelects(); await loadView(); writeHash(true); }
  else { renderHistory(); renderCheck(); }
}
/** `Licht (Or) on page "EG": from Taster.Q, to Lampe.I1`: where a block is and what is wired to it. */
function removalLine(b) {
  if (!b.deployed) return `${b.slug} (${b.type || "block"}): not on the Miniserver, the lockfile forgets it`;
  const at = b.page_title ? ` on page "${b.page_title}"` : "";
  const end = (w) => `${w.other.title}${w.other.port ? "." + w.other.port : ""}${w.other.page_title && w.other.page_title !== b.page_title ? ` (${w.other.page_title})` : ""}`;
  const from = (b.wires || []).filter((w) => w.dir === "in").map((w) => `${end(w)} → ${w.port}`);
  const to = (b.wires || []).filter((w) => w.dir === "out").map((w) => `${w.port} → ${end(w)}`);
  const wired = [...(from.length ? [`wired from ${from.join(", ")}`] : []), ...(to.length ? [`wired to ${to.join(", ")}`] : [])];
  if (b.refs) wired.push(plural(b.refs, "reference") + " on other pages");
  return `${b.title} (${b.type || "block"}, ${b.slug})${at}${wired.length ? ": " + wired.join("; ") : ": nothing wired"}`;
}
/** Ask the server for the plan and show it, compared with what runs. */
async function removalsPlan() {
  const plan = await act("/api/removals/plan");
  S.removals = plan; S.removalsAt = Date.now();
  // The server keeps this plan now (the next history read says so too).
  if (S.hist?.actions) S.hist.actions.removals = plan.rev;
  S.rev = plan.rev; S.base = plan.base; S.follow = false; $("chk-follow").checked = false;
  renderHistory(); renderSelects(); await loadView(); writeHash(true);
  return plan;
}
/** The confirmation: exactly the blocks the plan deletes. The consent, or null (the plan is then forgotten). */
async function askRemovals(doing) {
  let plan;
  try { plan = await removalsPlan(); } catch (err) { deployMsg(`Cannot work out the removals: ${err.message}`, true); return null; }
  const on = plan.blocks.filter((b) => b.deployed), off = plan.blocks.filter((b) => !b.deployed);
  const lose = rightsApplyText({ rights: plan.rights });
  const text = (on.length ? `Remove ${plural(on.length, "block")} from the Miniserver?\n\n` +
    "The module no longer has them, or its lockfile deletes them, so lox removes them only when you allow it:\n" +
    on.map((b) => `  • ${removalLine(b)}`).join("\n") + (off.length ? "\n\n" : "")
    : `Let the lockfile forget ${plural(off.length, "block")} the module no longer has?\n\n`) +
    (off.length ? `Never deployed, only the lockfile forgets ${off.length === 1 ? "it" : "them"}:\n` + off.map((b) => `  • ${removalLine(b)}`).join("\n") : "") +
    (lose ? `\n\n${lose.trim()}` : "") +
    `\n\nThe revision "${plan.revision.label}" shows what would run then. Apply deploys exactly this and refuses if the module, ` +
    `its lockfile or the base change before the push. Remove ${plan.blocks.length === 1 ? "it" : "them"} and go on ${doing}?`;
  if (!window.confirm(text)) { await dropRemovals(); return null; }
  return { reviewed: plan.blob, blocks: plan.blocks.map((b) => b.uuid) };
}
/** Apply's and Roll back's request (#69): lox refuses some pushes until the user allows them; ask, then retry
 *  with that allowed. Afterwards, the deployed revision compared with the one it replaced. */
async function deploy(path, body, doing) {
  deployMsg("");
  $("deploy-new").classList.add("hidden");
  S.deployed = null;
  // The structure file the server reads after this is the new config's.
  try { S.applyReads = (await api("/api/app")).reads ?? 0; } catch { S.applyReads = S.app?.reads ?? 0; }
  const opts = { ...body };
  for (let tries = 0; tries < 3; tries++) {
    // Deployed: the plan is history, and the view switches to what runs now.
    try { await act(path, opts); if (opts.allow_removals) S.removals = null; await showDeployed(); return; } catch (err) {
      const h = err.data?.hints || {};
      // A consent is for one request: refused, the server put the project back. A retry asks again.
      const consented = !!opts.allow_removals;
      delete opts.allow_removals;
      if (h.accept_version && !opts.accept_version && window.confirm(
        `This config was saved by a newer Loxone Config (version ${h.accept_version}) than the project last compiled with.\n\n` +
        `Accept that version and go on ${doing}? Nothing else changes; the project's lockfile records the new version.`)) {
        opts.accept_version = h.accept_version; continue;
      }
      if (h.custom_changes && !opts.discard_custom_changes && window.confirm(
        `The Miniserver holds ${h.custom_changes.length} change(s) that are not in its program file ` +
        "(made in the app, the web UI or by Loxone Config without saving):\n\n  " + h.custom_changes.join("\n  ") +
        `\n\nThe upload DELETES them. To keep them instead, load the project from the Miniserver in Loxone Config and save it, then try again.` +
        `\n\nDelete them and go on ${doing}?`)) {
        opts.discard_custom_changes = true; continue;
      }
      // A block the module lost (#70): asked each time, never carried over by itself. A Roll back takes none.
      // The full confirmation again, now about the plan's program: that is what the retry deploys.
      if (h.removals && !consented && path === "/api/apply") {
        const consent = await askRemovals(doing);
        if (consent && await confirmApply(S.removals.revision, true)) { opts.allow_removals = consent; continue; }
        if (consent) await dropRemovals();
      }
      deployMsg(err.message, true);
      return;
    }
  }
}
// ── roll back (#69) ────────────────────────────────────────────────────
// lox config push keeps what the Miniserver ran before each push (backups/<stamp>-pre-push.zip). A Roll back
// deploys that again the way Apply deploys: the server works out what will run (the module from before the Apply,
// compiled onto that download) and shows it as a revision compared with what runs now; the same confirmation as
// Apply's; then the same steps (tests, lox config push with its drift check, restart, verify), a commit and git push.
let rollbackAsking = false;
/** Under an Apply in the History: "Roll back to before this", when its download from before is kept. */
function rollbackRow(r, rb) {
  const d = document.createElement("div");
  d.className = "rb-row";
  const a = S.hist?.actions;
  const why = !a?.apply?.available ? `Cannot roll back: ${a?.apply?.reason || ""}`
    : !rb.available ? rb.reason
    : a.running ? `${a.running} is running`
    : pendingChange() ? "Apply or discard the pending changes first: a Roll back puts the module back to before this Apply"
    : "";
  const tip = why || `Deploy again what the Miniserver ran before this (${rb.backup}), the way Apply deploys, and put the module back to before it`;
  d.innerHTML = `<button type="button" class="rb-btn"${why || rollbackAsking || applyAsking ? " disabled" : ""} title="${esc(tip)}">↶ Roll back to before this</button>`;
  d.querySelector("button").onclick = () => rollBack(r);
  return d;
}
async function rollBack(r) {
  if (rollbackAsking || applyAsking) return;
  rollbackAsking = true; renderHistory();
  try { await rollBackAsk(r); } finally { rollbackAsking = false; renderHistory(); renderActions(); }
}
/** The Roll back's plan this tab asked for (#69), shown only here, while it is about the deployed commit and
 *  younger than the server keeps it. */
function rollbackPreview() {
  const r = S.rollbackRev;
  if (!r) return null;
  const deployed = S.hist?.history.find((x) => x.kind === "git");
  if (deployed?.id !== r.base || Date.now() - S.rollbackAt > PREVIEW_TTL) { S.rollbackRev = null; return null; }
  return r;
}
/** Forget the plan, here and on the server; show what runs again if the plan was shown. */
async function dropRollback(back) {
  const r = S.rollbackRev;
  if (!r) return;
  S.rollbackRev = null;
  try { await fetch("/api/rollback/cancel", { method: "POST", headers: { "Content-Type": "application/json", "X-Vislox": "rollback" }, body: JSON.stringify({ rev: r.id }) }); } catch { /* the server forgets it anyway */ }
  if (back && S.rev === r.id) { S.rev = r.base; S.base = null; renderHistory(); renderSelects(); await loadView(); writeHash(true); }
  else renderHistory();
}
async function rollBackAsk(r) {
  deployMsg("");
  let plan;
  try { plan = await act("/api/rollback/plan", { commit: r.id }); } catch (err) { deployMsg(`Cannot roll back: ${err.message}`, true); return; }
  // What it deploys, compared with what runs now, behind the confirmation: in this tab only.
  S.rollbackRev = plan.revision; S.rollbackAt = Date.now();
  const target = plan.revision;
  S.rev = plan.rev; S.base = plan.base; S.follow = false; $("chk-follow").checked = false;
  renderHistory(); renderSelects(); await loadView(); writeHash(true);
  const [, calNow, hw] = await Promise.all([loadMs(true), loadCal(true), pendingHardware(target, plan.base)]);
  const stale = staleText();
  const sched = await pendingScheduleText(target);
  const n = S.viewRev === plan.rev && S.view?.compared ? S.view.changes.length : null;
  if (!window.confirm((stale ? `⚠ ${stale} Rolling back overwrites what changed there since.\n\n` : "") + rightsApplyText(hw) + sched +
    calApplyWarning(calNow) + hwApplyText(hw) +
    `Roll back "${plan.subject}"?\n\nThe Miniserver then runs what it ran before it (${plan.backup})` +
    (plan.same_bytes ? "" : ", recompiled from the module version before that deployment") +
    (n == null ? "" : `: ${plural(n, "change")} from what it runs now, as the view shows`) +
    ". The module and lockfile go back to before it, so nothing is pending afterwards.\n\n" +
    "Whatever changed on the Miniserver since is lost: a save from Loxone Config stops the Roll back, and custom changes made in the app are deleted only if you allow it.\n\n" +
    "As Apply: the tests run (lox logic test), lox config push checks that the Miniserver still runs this config and stages exactly what you reviewed, uploads, restarts the program and verifies, then vislox commits and pushes to git.")) {
    await dropRollback(true);
    return;
  }
  // The server works the plan out again, and deploys only what was reviewed.
  await deploy("/api/rollback", { commit: plan.commit, reviewed: plan.blob }, "rolling back");
  await dropRollback(false);
}
$("btn-sync").onclick = whenReady(async () => {
  deployMsg("");
  try { const j = await act("/api/sync"); deployMsg(j.message); } catch (err) { deployMsg(err.message, true); }
});
$("btn-discard").onclick = whenReady(async () => {
  if (!window.confirm("Discard the pending changes? The module (with the values edited here), lock and compiled config go back to the last commit.")) return;
  deployMsg("");
  try { const j = await act("/api/discard"); deployMsg(j.message); } catch (err) { deployMsg(err.message, true); }
});

/** For Apply's confirmation: who loses or gains a right the pending config changes, against the deployed one
 *  (`rights` of `/api/apply/changes`, fetched with the devices). Never silently empty: when that request failed,
 *  it says so. */
function rightsApplyText(hw) {
  if (!hw || !hw.rights) return `⚠ Couldn't check access changes: ${hw?.error || "no answer"}. Applying may take away someone's access to a door or a notification.\n\n`;
  const lose = [], gain = [];
  for (const c of hw.rights) {
    const by = claimsOf(c.uuid).map(claimAt);
    const src = by.length ? ` (${by.join(", ")})` : "";
    const gone = c.removed_block ? ", which this removes" : "";
    for (const u of c.lose || []) lose.push(c.kind === "access" ? `${u.title} loses access to ${c.title}${gone}${src}` : `${u.title} stops getting ${c.title}${gone}${src}`);
    // A user who holds the new right directly gains it under its schedule: "alice gains access to Haustür (Always)".
    const when = (u) => { const p = (c.added || []).find((x) => x.kind === "user" && x.uuid === u.uuid); return p?.schedule ? ` (${p.schedule})` : ""; };
    for (const u of c.gain || []) gain.push(c.kind === "access" ? `${u.title} gains access to ${c.title}${when(u)}` : `${u.title} starts getting ${c.title}`);
    // Who keeps access, but not under every schedule they had (#76): a loss of opening times, warned as one.
    for (const n of c.narrowed || []) lose.push(narrowText(n, c.title) + src);
  }
  return (lose.length ? `⚠ ${lose.join("\n⚠ ")}\n\n` : "") + (gain.length ? `${gain.join("\n")}\n\n` : "");
}

// ── whether the module compiles (#65) ──────────────────────────────────
// The server runs lox logic check on the project's module whenever it changes; GET /api/check reads the
// last check (`actions.edit.check` counts them). While it has errors,
// out.Loxone stays at the last compile that worked, so the review would show an older change than the
// source: the Deploy pane says so, lists each error with its file, line and the blocks its line names,
// and Apply is off. A module that compiles to something else than the pending config (changed outside
// vislox, not compiled yet) offers Compile. lox logic fmt --check is a hint only.
S.check = null;
S.checkKey = null;
S.checkSeq = 0;
async function loadCheck() {
  const seq = ++S.checkSeq;
  if (!S.hist?.actions?.shown) {
    const was = marksSet();
    S.check = null;
    if (S.view && marksSet() !== was) { renderPage(); applyFocus(); }
    renderActions();
    return;
  }
  let c;
  try { c = await api("/api/check"); } catch { c = null; }
  // An older request answering late does not overwrite a newer one.
  if (seq !== S.checkSeq) return;
  const was = marksSet();
  S.check = c;
  // The lint badges follow the findings (#66).
  if (S.view && marksSet() !== was) { renderPage(); applyFocus(); }
  renderActions();
  if (S.sel && S.view) renderProps();
}
/** lox logic lint's findings (#66), when the last check has them; else none. */
function lintAll() { return S.check?.available && S.check.lint?.available ? S.check.lint.findings || [] : []; }
/** The findings about block `uuid`, shown on the newest config only (as the check's errors are). */
function lintFindings(uuid) { return S.rev === editRev()?.id ? lintAll().filter((f) => f.blocks?.includes(uuid)) : []; }
/** The findings and failed tests on blocks, to redraw the page (badges and their tooltips) only when they change. */
function marksSet() {
  if (S.rev !== editRev()?.id) return "";
  return JSON.stringify([lintAll().filter((f) => f.blocks?.length).map((f) => [f.blocks, f.kind, f.message]),
    testsBad().map((t) => [t.name, testBlocks(t), t.failures, t.error])]);
}
/** The module does not compile (lox logic check found errors). */
function checkBroken() { return !!(S.check?.available && S.check.ok === false); }
/** `pages/a.loxl:5`, or the file alone. */
function checkAt(e) { return e.file ? `${e.file}${e.line ? ":" + e.line : ""}` : ""; }
/** The errors whose line names the block `uuid`. */
function checkErrors(uuid) { return checkBroken() ? S.check.errors.filter((e) => e.blocks?.includes(uuid)) : []; }
function renderCheck() {
  const box = $("deploy-check"), c = S.check;
  const stale = c?.available && c.ok && c.stale;
  const rm = removing();
  const fmt = c?.available ? c.unformatted || [] : [];
  box.className = "deploy-check" + (checkBroken() || stale?.error || rm ? " error" : stale ? "" : " hint") + (checkBroken() || stale || rm || fmt.length ? "" : " hidden");
  const parts = [];
  if (checkBroken()) {
    const n = c.errors.length + (c.truncated || 0);
    const items = c.errors.map((e) =>
      `<li>${e.file ? `<span class="ck-at">${esc(checkAt(e))}</span>` : ""}${esc(e.message)} ${goButtons(e.blocks)}</li>`);
    if (c.truncated) items.push(`<li>… and ${plural(c.truncated, "more error")}</li>`);
    parts.push(`<div class="ck-h">The module doesn't compile: ${plural(n, "error")}</div>`,
      `<div class="ck-sub">${pendingChange() ? "The pending revision is from the last compile that worked, not from the module as it is." : "Nothing new is compiled until it does."} Apply is off until it compiles.</div>`,
      `<ul>${items.join("")}</ul>`);
  } else if (rm) {
    // The push would remove blocks (#70): review them; Apply asks.
    const a = S.hist?.actions, busy = !!a?.running;
    const why = !a?.apply?.available ? `Cannot review: ${a?.apply?.reason || ""}` : "";
    const plan = removalsPreview();
    const tombs = (rm.blocks || []).map((b) => `${b.slug} (${b.type})`).join(", ");
    parts.push(rm.refused ? `<div class="ck-h">The module no longer has managed blocks: lox deletes them only when you allow it.</div>`
      : `<div class="ck-h">The push would delete ${plural(rm.blocks.length, "block")} the Miniserver runs: only when you allow it.</div>`,
      `<div class="ck-sub">${rm.refused ? `lox refuses to compile it (<span class="mono">${esc(rm.refused)}</span> is in the lockfile but not in the source). `
        : `The lockfile deletes them: <span class="mono">${esc(tombs)}</span>. `}` +
      "Review what Apply would delete, and what is wired to it; Apply asks you to confirm exactly these blocks.</div>",
      plan ? `<ul class="rm-list">${plan.blocks.map((b) => `<li>${esc(removalLine(b))} ${goButtons([b.uuid], plan.rev)}</li>`).join("")}</ul>` : "",
      `<div class="ck-row"><button type="button" id="btn-removals" class="ghost small"${why || busy ? " disabled" : ""}
        title="${esc(why || "Compile the module with the removals allowed in a scratch copy and show it, compared with what runs. Nothing changes and nothing is deployed.")}">${plan ? "Review the removals again" : "Review the removals"}</button></div>`);
  } else if (stale) {
    const files = stale.files?.length ? stale.files.join(", ") : "the module";
    const a = S.hist?.actions, busy = !!a?.running;
    // A module that does not compile against the base would only fail again.
    const why = !a?.apply?.available ? `Cannot compile: ${a?.apply?.reason || ""}`
      : stale.error ? "Cannot compile: the module doesn't compile against the base (see above)" : "";
    parts.push(`<div class="ck-h">${pendingChange() ? `Pending is from before the latest edit to ${esc(files)}.` : `The module changed (${esc(files)}) and is not compiled yet.`}</div>`,
      stale.error ? `<div class="ck-sub">It doesn't compile against the base:</div><pre>${esc(stale.error)}</pre>`
        : `<div class="ck-sub">Compile it to review what Apply would deploy: lox config push compiles the module as it is.</div>`,
      `<div class="ck-row"><button type="button" id="btn-compile" class="ghost small"${why || busy ? " disabled" : ""}
        title="${esc(why || "Compile the module into the pending revision (lox logic compile). Nothing is deployed.")}">${a?.running === "Compile" ? "Compiling…" : "Compile"}</button></div>`);
  }
  if (fmt.length) parts.push(`<div class="ck-fmt">Not formatted as lox logic fmt writes it: ${esc(fmt.join(", "))}. A hint only; lox logic fmt --write formats it.</div>`);
  box.innerHTML = parts.join("");
  renderTests();
  renderLint();
}
/** The block buttons of an error or finding: each shows its block, on its page. */
/** The loaded view when it is revision `rev`'s and the one shown, else null: names of blocks are read from no other. */
function viewFor(rev) { return rev && S.view && S.rev === rev && S.viewRev === rev ? S.view : null; }
/** The block buttons of an error or finding, for the config the check is about (`rev`, by default the newest: the
 *  pending compile, else the deployed config). They name the blocks of that revision, so while another one is
 *  shown (or loading) there are none: its names are not these blocks' (#126). */
function goButtons(uuids, rev = editRev()?.id) {
  const v = viewFor(rev);
  return (uuids || []).filter((u) => v?.objects[u]).map((u) =>
    `<button type="button" class="go" data-go="${esc(u)}" title="Show this block">${esc(displayTitle(v.objects[u]))}</button>`).join("");
}
// ── lox logic test (#67) ──────────────────────────────────────────────
// The same check runs the module's tests (test "…" … end) in lox's simulator when the module compiles, beside
// lint. The Tests box above Apply lists each test with pass, fail or couldn't run, the expect that did not hold
// (expected and actual) and the blocks it checks; a block a failing test checks gets a red badge, and the test
// in Properties. Apply is off while a test fails, and the server refuses it at its Tests pass step.
// While the tests run again, the last results stay up; a lox without lox logic test says so once, quietly.
/** lox logic test's tests (#67), when the last check has them; else none. */
function testAll() { return S.check?.available && S.check.test?.available ? S.check.test.tests || [] : []; }
/** The tests that failed or could not run. */
function testsBad() { return testAll().filter((t) => t.status !== "pass"); }
/** The blocks a test that did not pass is about: those of the expects that did not hold (else all it checks),
 *  and those the simulator could not model. */
function testBlocks(t) {
  const f = (t.failures || []).flatMap((x) => x.blocks || []);
  return [...new Set([...(f.length ? f : t.blocks || []), ...(t.unmodeled || []).map((u) => u.uuid)])];
}
/** The tests that failed on block `uuid`, on the newest config only (as lint's findings). */
function testFindings(uuid) { return S.rev === editRev()?.id ? testsBad().filter((t) => testBlocks(t).includes(uuid)) : []; }
/** Why Apply is off for the tests: some fail, by the run of the files as they are (not one still running). */
function testsFailing() {
  const t = S.check?.available && !checkBroken() && S.check.current !== false ? S.check.test : null;
  if (!t?.available || t.running) return "";
  const n = testsBad().length;
  return n ? `${plural(n, "test")} of lox logic test ${n === 1 ? "fails" : "fail"}` : "";
}
/** A failed expect: `step 1: expect both.Q == 1 — expected 1, actual 0`. */
function expectLine(f) {
  return `${f.step ? `step ${f.step}: ` : ""}${f.expect}${f.expected != null ? ` — expected ${f.cmp && f.cmp !== "==" ? f.cmp + " " : ""}${f.expected}, actual ${f.actual}` : ` — actual ${f.actual}`}`;
}
/** One test that did not pass, in a line (about block `uuid`, when given: only its expects). */
function testLine(t, uuid) {
  const fs = (t.failures || []).filter((f) => !uuid || !f.blocks?.length || f.blocks.includes(uuid));
  const um = (t.unmodeled || []).filter((u) => !uuid || u.uuid === uuid);
  const why = t.status === "error"
    ? (um.length ? `couldn't run: the simulator has no model for ${um.map((u) => `${u.type} "${u.title}"`).join(", ")}` : `couldn't run: ${t.error || ""}`)
    : `fails: ${fs.map(expectLine).join("; ")}`;
  return `Test "${t.name}"${t.file ? ` (${checkAt(t)})` : ""} ${why}`;
}
function renderTests() {
  const box = $("deploy-tests"), c = S.check;
  const t = c?.available && !checkBroken() && S.rev === editRev()?.id ? c.test : null;
  const list = t?.available ? t.tests || [] : [];
  box.classList.toggle("hidden", !t || (t.available && !list.length && !t.running));
  box.classList.toggle("quiet", !!t?.unsupported);
  const bad = list.filter((x) => x.status !== "pass");
  box.classList.toggle("error", !!bad.length);
  box.classList.toggle("hint", !bad.length);
  if (!t) { box.innerHTML = ""; return; }
  if (t.unsupported) { box.innerHTML = `<div class="ck-sub">No tests run: ${esc(t.reason || "this lox has no lox logic test")}.</div>`; return; }
  if (!t.available) { box.innerHTML = `<div class="ck-h">Tests didn't run</div><div class="ck-sub">${esc(t.reason || "")}</div>`; return; }
  if (!list.length) { box.innerHTML = `<div class="ck-sub">Running the module's tests…</div>`; return; }
  const was = box.querySelector("details");
  // Open while a test fails, unless the user folded it.
  const open = was ? was.open || (bad.length && !was.dataset.bad) : !!bad.length;
  const icon = { pass: "✓", fail: "✕", error: "!" };
  const items = list.map((x) => {
    const lines = [];
    for (const f of x.failures || []) lines.push(`<li>${f.line ? `<span class="ck-at">${esc(checkAt({ file: x.file, line: f.line }))}</span>` : ""}${esc(expectLine(f))} ${goButtons(f.blocks)}</li>`);
    if (x.status === "error") {
      lines.push(`<li>${esc(x.error || "")}</li>`);
      if (x.unmodeled?.length) lines.push(`<li>Not simulated: ${x.unmodeled.map((u) => `${esc(u.type)} "${esc(u.title)}"`).join(", ")} ${goButtons(x.unmodeled.map((u) => u.uuid))}</li>`);
    }
    const checks = x.status === "pass" ? "" : goButtons(x.blocks);
    return `<li class="ts-${esc(x.status)}"><span class="ts-ic" aria-label="${esc(x.status)}">${icon[x.status] || "·"}</span>` +
      `${x.file ? `<span class="ck-at">${esc(checkAt(x))}</span>` : ""}<span class="ts-n">${esc(x.name)}</span>` +
      `${x.status === "error" ? " couldn't run" : ""} ${checks}${lines.length ? `<ul>${lines.join("")}</ul>` : ""}</li>`;
  });
  const n = list.length, failed = list.filter((x) => x.status === "fail").length, errs = list.filter((x) => x.status === "error").length;
  const head = `Tests: ${list.length - bad.length} of ${n} passed` + (failed ? `, ${failed} failed` : "") + (errs ? `, ${errs} couldn't run` : "");
  const sub = t.running ? " · testing the latest change…"
    : bad.length ? " · Apply is off until they pass" : " · lox logic test, in lox's simulator";
  box.innerHTML = `<details${open ? " open" : ""}${bad.length ? ' data-bad="1"' : ""}><summary><span class="ck-h">${esc(head)}</span>${esc(sub)}</summary><ul class="ts-list">${items.join("")}</ul>` +
    `<div class="ck-sub ts-cover">A pass covers only what the simulator models: a test that needs another block says so and names it.</div></details>`;
}
$("deploy-tests").addEventListener("click", whenReady(goClicked));

// ── lox logic lint (#66) ──────────────────────────────────────────────
// The same check runs lox logic lint on the project when the module compiles: unused names, blocks whose
// outputs feed nothing, users whose tag opens no Code Touch, and more. The Deploy pane lists every finding
// by kind with its file, line and blocks; each block gets a warning badge and the finding in Properties.
// Findings are hints: Apply never waits for them.
// Like the badges, the list is about the newest config only: it shows while that revision is shown.
// While lint runs again, the last findings stay up; a lox without lint says so once, quietly.
function renderLint() {
  const box = $("deploy-lint"), c = S.check;
  const l = c?.available && !checkBroken() && S.rev === editRev()?.id ? c.lint : null;
  const list = l?.available ? l.findings || [] : [];
  box.classList.toggle("hidden", !l || (l.available && !list.length));
  box.classList.toggle("quiet", !!l?.unsupported);
  if (!l) { box.innerHTML = ""; return; }
  if (l.unsupported) { box.innerHTML = `<div class="ck-sub">Lint is unavailable: ${esc(l.reason || "this lox has no lox logic lint")}.</div>`; return; }
  if (!l.available) { box.innerHTML = `<div class="ck-h">Lint didn't run</div><div class="ck-sub">${esc(l.reason || "")}</div>`; return; }
  const open = box.querySelector("details")?.open ?? false;
  const kinds = [...new Set(list.map((f) => f.kind))];
  const items = kinds.map((k) => {
    const of = list.filter((f) => f.kind === k);
    return `<li class="lt-kind"><span class="lt-k">${esc(k)}</span> (${of.length})<ul>${of.map((f) =>
      `<li>${f.file ? `<span class="ck-at">${esc(checkAt(f))}</span>` : ""}<span class="mono">${esc(f.name)}</span>: ${esc(f.message)} ${goButtons(f.blocks)}</li>`).join("")}</ul></li>`;
  });
  box.innerHTML = `<details${open ? " open" : ""}><summary><span class="ck-h">Lint: ${plural(list.length, "finding")}</span>${l.running ? " · linting the latest change…" : ""} · hints from lox logic lint; Apply doesn't wait for them</summary><ul>${items.join("")}</ul></details>`;
}
$("deploy-lint").addEventListener("click", whenReady(goClicked));
$("deploy-check").addEventListener("click", whenReady(async (e) => {
  if (e.target.closest("#btn-compile")) {
    deployMsg("");
    try { const j = await act("/api/compile"); deployMsg(j.message); } catch (err) { deployMsg(err.message, true); }
    return;
  }
  if (e.target.closest("#btn-removals")) {
    deployMsg("");
    try { await removalsPlan(); } catch (err) { deployMsg(`Cannot work out the removals: ${err.message}`, true); }
    return;
  }
  goClicked(e);
}));
/** A block's title, else its slug, for the blocks a default reaches. */
function dfltName(b) { const o = b.uuid && viewFor(editRev()?.id)?.objects[b.uuid]; return o ? displayTitle(o) : b.slug; }
function dfltList(blocks) { return blocks.length ? blocks.map(dfltName).join(", ") : "none"; }
/** Apply's review of the module's constants and defaults that change (#81): each, and the blocks a default changes. */
function constApplyText() {
  const p = constPending();
  if (!p.length) return "";
  const line = (x) => x.kind === "let"
    ? `  • let ${x.name} = ${x.value} (was ${x.deployed ?? "none"}), ${constAt(x.file, x.line)}: ${plural(x.uses, "use")}`
    : `  • defaults ${x.name}(${x.port}: ${x.value ?? "none"}) (was ${x.deployed ?? "none"}), ${constAt(x.file, x.line)}: changes ${plural(x.blocks.length, "block")}: ${dfltList(x.blocks)}`;
  return `Constants and defaults that change (each sets every port that takes it):\n${p.map(line).join("\n")}\n\n`;
}
