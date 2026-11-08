/* exported $, AB, EDIT_ATTR, KIND, MV, PREVIEW_TTL, S, accessPending, api, appIssues, applyFocus, autoTitle, awaitedAppRead, calApplyWarning, ciEq, claimAt, claimsOf, connect, constAt, constLet, constPending, constUse, displayTitle, editsMoved, editsWires, esc, fmtWhen, goClicked, gotoPage, hideTip, isDeployed, liveHere, liveOn, liveStart, liveStreaming, liveVal, loadApp, loadAppFromMs, loadCal, loadMs, loadRunning, loadView, logStale, lvSpan, managedRefresh, managedSet, managedSrc, membersPending, moodsPending, msWarn, narrowText, nodeById, pageCreated, pagesCreated, plural, portName, prettyType, renderAddHud, renderChanges, renderHistory, renderLiveSwitch, renderLog, renderPage, renderProps, renderPull, renderRights, renderSelects, renderStale, renderTree, rulesPending, running, sameEnd, schedsPending, scheduleConflicts, seUuid, select, setLive, setStatus, showTip, srcPath, staleText, svg, tilesPending, wcardRefresh */
"use strict";
// vislox — Loxone Config style viewer: read-only by default, with optional editing and deployment.
// Canvas coordinates are the config's own units: one port row is 192.

const ROW = 192, LABEL_W = 2112, SVGNS = "http://www.w3.org/2000/svg";
/** Below this zoom the page is an overview: names only, drawn at a constant screen size.
 *  Port text (104 units) switches off below ~5 px on high-density screens and ~6.5 px elsewhere. */
const FAR_K = (window.devicePixelRatio >= 2 ? 5 : 6.5) / 104;
/** "100 %": one port row is about 19 px, as in Loxone Config at its default zoom. */
const K100 = 0.1;
const $ = (id) => document.getElementById(id);
const S = {
  hist: null, view: null, rev: null, base: null, page: null, sel: null,
  follow: true, marks: true, vp: {}, open: new Set(["__pages"]), chgOff: new Set(), chgHw: false,
  objPage: new Map(), pageOf: new Map(), loadSeq: 0, onlyChanged: false, chgIdx: -1,
  closed: new Set(["Advanced"]), dim: true, onlyManaged: false, treeFocus: null, hash: null, applyingHash: false,
  app: null, appBusy: false, appErr: "", applyReads: 0, deployed: null, rollbackRev: null, rollbackAt: 0, removals: null, removalsAt: 0,
  live: { on: pref("live", false), dim: pref("liveDim", true), query: pref("liveQuery", false), qes: null, qkey: "", qstat: null, es: null, status: null, values: new Map(), controls: {}, globals: {}, periods: null },
};
function pref(k, d) { try { const v = localStorage.getItem("vislox." + k); return v === null ? d : v === "1"; } catch { return d; } }
function setPref(k, v) { try { localStorage.setItem("vislox." + k, v ? "1" : "0"); } catch { /* private mode */ } }

// ── helpers ────────────────────────────────────────────────────────────
function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
function el(tag, attrs, parent, text) {
  const e = document.createElementNS(SVGNS, tag);
  for (const k in attrs || {}) if (attrs[k] !== undefined && attrs[k] !== null) e.setAttribute(k, attrs[k]);
  if (text !== undefined) e.textContent = text;
  if (parent) parent.appendChild(e);
  return e;
}
function clip(s, n) { s = String(s ?? ""); return n < 2 ? "" : s.length > n ? s.slice(0, n - 1) + "…" : s; }
function plural(n, w) { return `${n} ${w}${n === 1 ? "" : "s"}`; }

const FRIENDLY = {
  And: "AND", Or: "OR", Not: "NOT", Xor: "XOR", Equal: "Equal", NotEqual: "Not equal", Greater: "Greater",
  GreaterEqual: "Greater or equal", Less: "Less", LessEqual: "Less or equal", Minmax: "Min / Max", AMinmax: "Min / Max",
  AnalogThresholdTrigger: "Threshold switch", Div: "Divide", Sub: "Subtract", Mult: "Multiply", Add: "Add", Avg: "Average",
  MultiClick: "Multiclick", LongClick: "Long click", TextGenerator: "Text generator", OnOffDelay: "On/off delay",
  EdgeDetection: "Edge detection", HeatIRoomController2: "Intelligent room controller", LightController2: "Lighting controller",
  AutoJalousie: "Automatic blinds", Text: "Note", PushButton: "Push button", PulseAt: "Pulse at", AutopilotRule: "Automatic rule",
  MeterAbsUni: "Meter", MeterAbsBi: "Meter (bidirectional)", Radio: "Radio buttons", OnPulseDelay: "On-pulse delay",
  OffDelay: "Off delay", OnDelay: "On delay", PButtonT: "Push button +", AnalogMultiplexer2: "Analog multiplexer",
};
function prettyType(t) { return FRIENDLY[t] || String(t || "").replace(/([a-z])([A-Z0-9])/g, "$1 $2"); }
/** Loxone numbers untitled objects `O12345`; those names mean nothing to a reader. */
function autoTitle(t) { return !t || /^O\d+$/.test(t); }
function displayTitle(o) { return autoTitle(o.title) || o.title === o.ty ? prettyType(o.ty) : o.title; }

const MONTH = { month: "short", day: "numeric" };
function fmtWhen(t, withYear) {
  if (!t) return "";
  const d = new Date(t * 1000), now = new Date(), ago = Date.now() / 1000 - t;
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return `today ${time}`;
  const y = withYear || d.getFullYear() !== now.getFullYear() ? { ...MONTH, year: "numeric" } : MONTH;
  return `${d.toLocaleDateString([], y)}, ${time}`;
}
async function api(path) {
  const r = await fetch(path);
  const j = await r.json();
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
function setStatus(text, cls) {
  $("status-text").textContent = text;
  $("status").className = "status " + (cls || "");
  $("status").title = text;
}
/** How long a Roll back's or a removal's plan (#69, #70) is shown: the server keeps it as long. */
const PREVIEW_TTL = 15 * 60 * 1000;
function lightColor(rgb) {
  const m = /^(\d+),(\d+),(\d+)$/.exec(rgb || "");
  if (!m) return false;
  const [r, g, b] = m.slice(1).map(Number);
  return 0.299 * r + 0.587 * g + 0.114 * b > 170;
}
function copyText(text) {
  if (navigator.clipboard && window.isSecureContext) return navigator.clipboard.writeText(text);
  const ta = document.createElement("textarea");
  ta.value = text; document.body.appendChild(ta); ta.select();
  document.execCommand("copy"); ta.remove();
  return Promise.resolve();
}

/** Short, readable names for revisions: `Backup v274`, not `[192_168_1_120] Config backup 2026-10-31 …`. */
function revInfo(r) {
  let title = r.label;
  const m = /Config backup .*?\(v(\d+)\)/i.exec(r.label);
  if (r.kind === "pending") title = r.current ? "Compiled, not deployed" : "Earlier compile";
  else if (r.kind === "rollback" || r.kind === "removals") title = r.label;
  else if (r.kind === "live") title = r.label.replace(/ \(same as an older commit\)/, "");
  else if (r.kind === "backup") title = r.label;
  else if (m) title = `Backup v${m[1]}`;
  else title = title.replace(/^\[[^\]]+\]\s*/, "");
  const sub = r.kind === "git" ? `${fmtWhen(r.time, true)} · ${r.detail}` : fmtWhen(r.time, true);
  return { title, sub, short: `${title} · ${fmtWhen(r.time)}` };
}

let streamRetryTimer = null;

/** What the Miniserver runs: `deployed`, as the server says (the last Apply, Roll back or Pull, else the newest
 *  commit; one of the history, never a compile). Not `current`, which is only what the watched file holds: an edit
 *  on disk nothing committed is current, not run. */
function isDeployed(r) { return !!r?.deployed; }

function revButton(r, newest) {
  const info = revInfo(r);
  const b = document.createElement("button");
  b.type = "button";
  b.className = `hrev k-${r.kind}` + (r.id === S.rev ? " sel" : "") + (r.id === S.base ? " base" : "") + (r.current ? " current" : "") + (isDeployed(r) ? " deployed" : "");
  b.setAttribute("aria-current", r.id === S.rev ? "true" : "false");
  const deployed = isDeployed(r);
  const also = r.backups?.length ? `\n\nThe Miniserver's backup${r.backups.length > 1 ? "s" : ""} ${r.backups.join(", ")} ${r.backups.length > 1 ? "hold" : "holds"} this config.` : "";
  b.title = `${r.label}\n${r.detail}\n${new Date(r.time * 1000).toLocaleString()}${also}` +
    (deployed ? "\n\nThis is the config on the Miniserver." : "") +
    (!deployed && r.current && r.kind !== "pending" ? "\n\nThis is the file on disk, not the config on the Miniserver. Only Apply deploys it; a Pull would overwrite it." : "") +
    `\n\nClick: show it\nShift+click: compare what you see with it${r.id === S.base ? " (again: stop comparing)" : ""}`;
  const pills = [];
  if (r.kind === "pending" && r.current) pills.push('<span class="pill pend">would be deployed</span>');
  if (r.kind === "rollback") pills.push('<span class="pill pend">would be deployed by Roll back</span>');
  if (r.kind === "removals") pills.push('<span class="pill pend">would be deployed with the removals</span>');
  if (deployed) pills.push('<span class="pill cur">on Miniserver</span>');
  if (r.kind === "live") pills.push('<span class="pill new">not in git</span>');
  if (r.kind === "backup") pills.push('<span class="pill ms">not pulled</span>');
  if (r.same_as) pills.push(`<span class="pill">same as ${esc(revById(r.same_as) ? revInfo(revById(r.same_as)).title : r.same_as)}</span>`);
  if (r.backups?.length) pills.push(`<span class="pill ms">Miniserver backup${r.backups.length > 1 ? ` ×${r.backups.length}` : ""}</span>`);
  if (r.id === S.base) pills.push('<span class="pill">compared with</span>');
  b.innerHTML = `<span class="node"></span><span class="l1"><span class="lbl">${esc(info.title)}</span></span>
    <span class="dt">${esc(info.sub)}</span>${pills.length ? `<span class="tags">${pills.join("")}</span>` : ""}`;
  b.onclick = (ev) => {
    if (ev.shiftKey) {
      if (r.id === S.rev) return;
      S.base = r.id === S.base ? null : r.id;
    } else { S.rev = r.id; S.base = null; }
    S.follow = !ev.shiftKey && r.id === newest ? S.follow : false;
    $("chk-follow").checked = S.follow;
    renderHistory(); renderSelects(); loadView(); writeHash(true);
  };
  return b;
}

function renderHistory() {
  const box = $("hist");
  box.innerHTML = "";
  const sec = (title, list) => {
    if (title) {
      const h = document.createElement("div");
      h.className = "hist-sec";
      h.textContent = title;
      box.appendChild(h);
    }
    for (const r of list) {
      box.appendChild(revButton(r, list[0].id));
      const rb = S.hist.rollbacks?.[r.id];
      if (rb) box.appendChild(rollbackRow(r, rb));
    }
  };
  renderOpLog(box);
  if (rollbackPreview()) sec("Roll back, not deployed", [S.rollbackRev]);
  if (removalsPreview()) sec("Removals allowed, not deployed", [S.removals.revision]);
  if (S.hist.pending.length) { sec("Pending deployment", S.hist.pending); sec("Deployed history", S.hist.history); }
  else sec(null, S.hist.history);
  if (!S.hist.history.length) box.insertAdjacentHTML("beforeend", '<div class="empty">No revisions yet.</div>');
  renderBackups(box);
}

// ── the Miniserver's own backups ───────────────────────────────────────
// Every config the Miniserver loads leaves /prog/sps_<ver>_<stamp>.zip behind. vislox lists and
// downloads them (FTP, read-only) only when asked, once each; one with the bytes of a Pull shows
// as that Pull (its "Miniserver backup" pill), the others here, grouped by the Pulls around them.
let backupsBusy = false;
// A Read the server refused, with the state it refused in: once that state moves on (a read ran,
// from here or after a Pull), the newer state counts.
let backupsErr = null;
function backupsState(info) { return `${info?.listed}|${info?.busy}|${info?.error}`; }
function renderBackups(box) {
  const info = S.hist.ms_backups;
  const list = S.hist.backups || [];
  const broken = info?.broken || [];
  if (!info || (!info.available && !list.length && !broken.length)) return;
  if (backupsErr && backupsErr.state !== backupsState(info)) backupsErr = null;
  const busy = backupsBusy || info.busy;
  const h = document.createElement("div");
  h.className = "hist-sec bk-sec";
  const tip = info.available ? `List the backups on the Miniserver (/prog, FTP, read-only) and download the new ones${msWarn("pull")}` : `Cannot read the backups: ${info.reason}`;
  h.innerHTML = `<span>Miniserver backups</span><button type="button" class="bk-read"${busy || !info.available ? " disabled" : ""}
    title="${esc(tip)}">${busy ? "Reading…" : "Read"}</button>`;
  h.querySelector("button").onclick = readBackups;
  box.appendChild(h);
  const p = info.progress;
  const err = backupsErr?.text || info.error;
  const note = info.stuck ? `${info.error || "An FTP call took too long"}. Waiting for its FTP session to end before the next read.`
    : busy ? (p?.[1] ? `Downloading ${Math.min(p[0] + 1, p[1])} of ${p[1]}…` : "Listing the backups on the Miniserver…")
    : err ? `Cannot read the backups: ${err}`
    : !info.available ? `Downloaded before. Not read now: ${info.reason}.`
    : info.listed == null ? "The Miniserver keeps a backup of each config it loads. Read lists them and downloads each one once."
    : `${info.on_miniserver} on the Miniserver, read ${fmtWhen(info.listed, true)}.` + (list.length ? "" : " Each one is a Pull above, or the same as the save before it.");
  box.insertAdjacentHTML("beforeend", `<div class="bk-note${(err && !busy) || info.stuck ? " error" : ""}" role="status">${esc(note)}</div>`);
  const pulls = S.hist.history.some((r) => r.kind === "git");
  const short = (id) => { const r = revById(id); return r ? revInfo(r).short : id; };
  let group;
  for (const r of list) {
    const key = r.before || "";
    if (key !== group) {
      group = key;
      const t = !pulls ? "On the Miniserver"
        : !r.before ? "Since the last Pull"
        : r.after ? `Between ${short(r.after)} and ${short(r.before)}`
        : `Before ${short(r.before)}`;
      box.insertAdjacentHTML("beforeend", `<div class="bk-gap">${esc(t)}</div>`);
    }
    box.appendChild(revButton(r, null));
  }
  if (broken.length) {
    box.insertAdjacentHTML("beforeend", `<div class="bk-gap">Could not be read (tried again on the next Read)</div>` +
      broken.map((b) => `<div class="bk-broken" title="${esc(b.file)}"><span class="l1">Miniserver backup v${esc(String(b.version))} · ${esc(fmtStamp(b.stamp))}</span>
        <span class="dt">${esc(b.error)}</span></div>`).join(""));
  }
}
/** `YYYYMMDDHHMMSS` (the Miniserver's local time) as `2026-11-05 23:55`. */
function fmtStamp(s) { return s.length >= 12 ? `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)} ${s.slice(8, 10)}:${s.slice(10, 12)}` : s; }
async function readBackups() {
  if (backupsBusy) return;
  backupsBusy = true; backupsErr = null; renderHistory();
  let text = "";
  try {
    const r = await fetch("/api/backups/read", { method: "POST", headers: { "X-Vislox": "backups" } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) text = j.error || r.statusText;
  } catch (e) { text = e.message; }
  backupsBusy = false;
  if (text) backupsErr = { text, state: backupsState(S.hist?.ms_backups) };
  await loadHistory();
}

function renderSelects() {
  const label = (r) => (r.kind === "pending" ? "⏳ " : "") + revInfo(r).short;
  const rs = $("sel-rev"), bs = $("sel-base");
  rs.innerHTML = allRevs().map((r) => `<option value="${esc(r.id)}"${r.id === S.rev ? " selected" : ""}>${esc(label(r))}</option>`).join("");
  bs.innerHTML = `<option value="">— nothing —</option>` +
    allRevs().filter((r) => r.id !== S.rev).map((r) => `<option value="${esc(r.id)}"${r.id === S.base ? " selected" : ""}>${esc(label(r))}</option>`).join("");
  bs.disabled = allRevs().length < 2;
  rs.title = revById(S.rev)?.label || "";
  bs.title = revById(S.base)?.label || "";
}

// ── view ───────────────────────────────────────────────────────────────
async function loadView() {
  if (!S.rev) { showMsg("No config found."); return; }
  const seq = ++S.loadSeq, rev = S.rev;
  showMsg("Loading…");
  // Whatever names blocks from the view (the links of the Deploy pane, the defaults line) stops doing so while
  // the view shown is not this revision's: a load that fails or is superseded leaves no old names behind (#126).
  renderActions();
  let v;
  try {
    v = await api(`/api/view?rev=${encodeURIComponent(S.rev)}&base=${encodeURIComponent(S.base || "")}`);
  } catch (e) {
    if (seq === S.loadSeq) showMsg("Cannot show this revision: " + e.message);
    return;
  }
  if (seq !== S.loadSeq) return;
  S.view = v;
  S.viewRev = rev;
  S.objPage = new Map();
  S.pageOf = new Map(v.pages.map((p) => [p.uuid, p]));
  for (const p of v.pages) for (const o of p.objects) if (!o.moved_to && (!S.objPage.has(o.uuid) || o.status !== "removed")) S.objPage.set(o.uuid, p.uuid);
  $("doc-title").textContent = v.title || "Loxone config";
  $("doc-title").title = v.title || "";
  document.title = `${v.title || "Loxone config"} — vislox`;
  const h = S.hash; S.hash = null;
  if (h?.page && S.pageOf.has(h.page)) S.page = h.page;
  if (!S.pageOf.has(S.page)) S.page = (v.pages.find((p) => p.changes > 0) || v.pages[0])?.uuid || null;
  if (h) S.sel = h.sel && v.objects[h.sel] ? h.sel : null;
  if (S.sel && !v.objects[S.sel]) S.sel = null;
  S.chgIdx = -1;
  const compared = v.compared;
  $("sw-changes").classList.toggle("disabled", !compared);
  $("chk-changes").disabled = !compared;
  $("sw-changes").title = compared ? "Colour what changed against the compared revision" : "Nothing to mark: no revision to compare with";
  $("only-changed-w").classList.toggle("hidden", !compared);
  if (!compared) S.onlyChanged = false;
  renderLegend();
  renderTabs();
  renderTree();
  renderChanges();
  renderPage();
  renderProps();
  // What names blocks from the view (goButtons, dfltName) is drawn again from this one: the check's answer can
  // come before the view of a new pending revision (#126). An open palette lists this view's names too.
  renderActions();
  if (!$("palette").classList.contains("hidden")) renderPalette();
  if (h) select(h.sel && v.objects[h.sel] ? h.sel : null);
  else if (S.sel) applyFocus();
  // A link or Back/Forward says whether the view is open; a reload of the revision keeps it as it is.
  if (h?.view === "modes") modesOpen({ quiet: true });
  else if (h) modesClose(true);
  else renderModes();
  writeHash(false);
}
function renderLegend() {
  $("legend").classList.toggle("diff", !!(S.view?.compared && S.marks));
  const m = hasManaged();
  $("legend").classList.toggle("managed", m);
  if (!m) S.onlyManaged = false;
  $("chk-only-managed").checked = S.onlyManaged;
}
// The blocks the project's Lox Logic module manages (#64): the lockfile pins their UUIDs, and the module's
// .loxl files declare them. GET /api/edits says where (`sources`), for a lox.toml project only.
/** Where the module declares the block `uuid`: { slug, file, line, how }, or null when it does not manage it. */
// They are today's lockfile and module, so they are shown on the newest config only (the pending compile, else
// the deployed config on disk), never on an older revision of the History.
function managedShown() { const r = editRev(); return !!(S.edits?.sources && r && S.rev === r.id); }
function managedSrc(uuid) { return managedShown() ? S.edits.sources[uuid] || null : null; }
function hasManaged() { return managedShown() && Object.keys(S.edits.sources).length > 0; }
/** The managed blocks shown: what the canvas and the legend are drawn from. */
function managedSet() { return hasManaged() ? Object.keys(S.edits.sources).sort().join(",") : ""; }
function managedTip(m) {
  return `Managed by the Lox Logic module: ${m.slug}${srcPath(m) ? ` (${srcPath(m)})` : ""}. Change it there; an edit made in Loxone Config is overwritten on the next push.`;
}
/** After /api/edits: redraw the page when the managed blocks shown changed; else only their badges' tooltips. */
function managedRefresh(was) {
  if (!S.view) return;
  if (managedSet() !== was) { renderLegend(); renderPage(); applyFocus(); return; }
  vpG.querySelectorAll(".blk.managed[data-uuid] .mbadge title").forEach((t) => {
    const m = managedSrc(t.closest("[data-uuid]").dataset.uuid);
    if (m) t.textContent = managedTip(m);
  });
}
/** `pages/licht.loxl:12`, to paste into an editor or a prompt; null when the module has no line for it. */
function srcPath(m) { return m?.file ? `${m.file}${m.line ? ":" + m.line : ""}` : null; }
function srcHow(m) {
  if (m?.how === "ambiguous") return "Declared more than once in the module, which lox refuses: no line is given.";
  if (!m?.file) return "Not found in the module's files: the lockfile pins it, but no .loxl file declares it now.";
  if (m.how === "expression") return "Made by lox for the expression on this line.";
  if (m.how === "template") return "Made by the template instance on this line.";
  return "Declared on this line.";
}
function showMsg(t) {
  const m = $("canvas-msg");
  m.innerHTML = t ? `<span>${esc(t)}</span>` : "";
  m.classList.toggle("show", !!t);
}

// ── tabs ───────────────────────────────────────────────────────────────
function visiblePages() {
  const ps = S.view?.pages || [];
  return S.onlyChanged ? ps.filter((p) => p.changes > 0 || p.uuid === S.page) : ps;
}
function renderTabs() {
  const list = $("tab-list");
  list.innerHTML = "";
  const marks = S.view.compared && S.marks;
  const total = S.view.pages.length, changedPages = S.view.pages.filter((p) => p.changes).length;
  $("pages-count").textContent = marks && changedPages ? `${plural(total, "page")} · ${changedPages} changed` : plural(total, "page");
  for (const p of visiblePages()) {
    const t = document.createElement("button");
    t.type = "button";
    t.setAttribute("role", "tab");
    t.setAttribute("aria-selected", p.uuid === S.page ? "true" : "false");
    t.tabIndex = p.uuid === S.page ? 0 : -1;
    t.className = "tab" + (p.status === "added" || p.status === "removed" ? " " + p.status : "");
    t.dataset.uuid = p.uuid;
    const badge = marks && p.changes ? ` <span class="badge ${p.status === "removed" ? "removed" : p.status === "added" ? "added" : "changed"}" aria-label="${p.changes} changes">${p.changes}</span>` : "";
    t.innerHTML = esc(p.title || "(untitled page)") + badge;
    t.title = (p.old_title ? `${p.old_title} → ${p.title}` : p.title) + ` — ${plural(p.objects.filter((o) => o.shape !== "tag").length, "block")}`;
    t.onclick = () => gotoPage(p.uuid);
    list.appendChild(t);
    if (p.uuid === S.page) requestAnimationFrame(() => t.scrollIntoView({ block: "nearest", inline: "nearest" }));
  }
}
$("tab-list").addEventListener("keydown", whenReady((e) => {
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home" && e.key !== "End") return;
  e.preventDefault();
  const tabs = [...$("tab-list").querySelectorAll(".tab")];
  let i = tabs.indexOf(document.activeElement);
  i = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : Math.max(0, Math.min(tabs.length - 1, i + (e.key === "ArrowRight" ? 1 : -1)));
  gotoPage(tabs[i].dataset.uuid);
  $("tab-list").querySelector(`[data-uuid="${CSS.escape(tabs[i].dataset.uuid)}"]`)?.focus();
}));
function gotoPage(uuid, opts) {
  if (!uuid || !S.pageOf.has(uuid)) return;
  // Going to a page leaves the Operating modes view.
  const left = MD.open;
  modesClose(true);
  if (S.page !== uuid) {
    S.page = uuid;
    renderTabs(); renderPage();
    if (S.sel) applyFocus();
    writeHash(true);
  } else if (left) writeHash(true);
  if (opts && opts.focus) focusObj(opts.focus, opts.zoom);
}
function stepPage(d) {
  const pages = visiblePages();
  const i = pages.findIndex((p) => p.uuid === S.page);
  const j = Math.max(0, Math.min(pages.length - 1, i + d));
  if (pages[j]) gotoPage(pages[j].uuid);
}
document.querySelectorAll(".tab-nav button").forEach((b) => (b.onclick = whenReady(() => stepPage(b.dataset.nav === "next" ? 1 : -1))));
$("chk-only-changed").onchange = whenReady((e) => { S.onlyChanged = e.target.checked; renderTabs(); });
$("btn-pages").onclick = whenReady(() => openPalette(""));

// ── canvas ─────────────────────────────────────────────────────────────
const svg = $("canvas"), vpG = $("viewport");
let L = {}; // layers of the current page
function curPage() { return S.pageOf.get(S.page); }

function pageExtent(p) {
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity;
  for (const o of p.objects) {
    const [x, y, w, h] = o.rect;
    x1 = Math.min(x1, x); y1 = Math.min(y1, y); x2 = Math.max(x2, x + w); y2 = Math.max(y2, y + h);
  }
  for (const l of p.labels) { x1 = Math.min(x1, l.x - LABEL_W - 200); x2 = Math.max(x2, l.x + LABEL_W + 200); }
  if (!isFinite(x1)) return p.sheet ? p.sheet : [0, 0, 24000, 16000];
  return [x1, y1, x2 - x1, y2 - y1];
}

let rafLod = 0;
function applyVp() {
  const v = S.vp[S.page];
  if (!v) return;
  vpG.setAttribute("transform", `translate(${v.x},${v.y}) scale(${v.k})`);
  $("btn-zoom-pct").textContent = Math.round((v.k / K100) * 100) + "%";
  updateMinimap();
  if (!rafLod) rafLod = requestAnimationFrame(() => { rafLod = 0; renderFar(); });
}
function fit() {
  const p = curPage();
  if (!p) return;
  const [x, y, w, h] = pageExtent(p);
  const W = svg.clientWidth, H = svg.clientHeight, pad = 30;
  const k = Math.min((W - 2 * pad) / Math.max(w, 1), (H - 2 * pad) / Math.max(h, 1), 0.12);
  S.vp[S.page] = { k, x: (W - w * k) / 2 - x * k, y: pad - y * k };
  applyVp();
}
function zoomAt(f, cx, cy) {
  const v = S.vp[S.page];
  if (!v) return;
  const k = Math.min(0.6, Math.max(0.006, v.k * f));
  v.x = cx - (cx - v.x) * (k / v.k);
  v.y = cy - (cy - v.y) * (k / v.k);
  v.k = k;
  applyVp();
}
function zoomCenter(f) { zoomAt(f, svg.clientWidth / 2, svg.clientHeight / 2); }
function pan(dx, dy) { const v = S.vp[S.page]; if (v) { v.x += dx; v.y += dy; applyVp(); } }
svg.addEventListener("wheel", whenReady((e) => {
  e.preventDefault();
  const r = svg.getBoundingClientRect();
  if (e.ctrlKey || !e.shiftKey) zoomAt(Math.pow(1.0018, -e.deltaY), e.clientX - r.left, e.clientY - r.top);
  else pan(-e.deltaY, 0);
}), { passive: false });
let drag = null;
svg.addEventListener("mousedown", whenReady((e) => {
  if (e.button !== 0 && e.button !== 1) return;
  drag = { x: e.clientX, y: e.clientY, moved: false };
  svg.classList.add("panning");
}));
window.addEventListener("mousemove", whenReady((e) => {
  // A wire being drawn holds the page still; so does a move with no button down (the pan's mouse-up
  // clears it only after a timer).
  if (!drag || WR) return;
  if (!(e.buttons & 5)) { if (!drag.moved) drag = null; return; }
  const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
  if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
  if (drag.moved) { pan(dx, dy); drag.x = e.clientX; drag.y = e.clientY; hideTip(); }
}));
window.addEventListener("mouseup", whenReady(() => { svg.classList.remove("panning"); setTimeout(() => (drag = null), 0); }));
svg.addEventListener("click", whenReady((e) => {
  if (drag && drag.moved) return;
  if (e.target === svg || e.target.classList.contains("sheet") || e.target.classList.contains("sheet-dots")) { select(null); wcardClose(); }
}));
$("btn-fit").onclick = whenReady(fit);
$("btn-zoom-in").onclick = whenReady(() => zoomCenter(1.25));
$("btn-zoom-out").onclick = whenReady(() => zoomCenter(0.8));
$("btn-zoom-pct").onclick = whenReady(() => { const v = S.vp[S.page]; if (v) zoomCenter(K100 / v.k); });
window.addEventListener("resize", whenReady(() => { if (S.page && !S.vp[S.page]) fit(); updateMinimap(); }));

const tip = $("tooltip");
function showTip(e, html) {
  if (drag?.moved) return;
  tip.innerHTML = html;
  tip.classList.remove("hidden");
  const w = tip.offsetWidth, h = tip.offsetHeight;
  tip.style.left = Math.min(window.innerWidth - w - 8, e.clientX + 16) + "px";
  tip.style.top = (e.clientY + 18 + h > window.innerHeight ? e.clientY - h - 12 : e.clientY + 18) + "px";
}
function hideTip() { tip.classList.add("hidden"); }
function hoverTip(node, html) {
  node.addEventListener("mousemove", (e) => showTip(e, typeof html === "function" ? html() : html));
  node.addEventListener("mouseleave", hideTip);
}

function renderPage() {
  renderAddHud();
  vpG.innerHTML = "";
  hideTip();
  const p = curPage();
  const marks = S.marks && !!S.view?.compared;
  svg.classList.toggle("marks-on", marks);
  svg.classList.toggle("marks-off", !marks);
  svg.classList.toggle("dim-unchanged", marks && S.dim && !!p && p.changes > 0);
  svg.classList.toggle("only-managed", S.onlyManaged && hasManaged());
  if (!p) { showMsg(S.view && !S.view.pages.length ? "This config has no logic pages." : ""); renderMinimap(); return; }
  showMsg(p.objects.length ? "" : "This page is empty.");
  svg.setAttribute("aria-label", `Logic page ${p.title}: ${plural(p.objects.filter((o) => o.shape !== "tag").length, "block")}. Drag or use the arrow keys to pan, scroll or plus and minus to zoom.`);

  const ext = pageExtent(p);
  const sh = p.sheet || [0, 0, 23808, 16032];
  const sx1 = Math.min(sh[0], ext[0] - 480), sy1 = Math.min(sh[1], ext[1] - 480);
  const sx2 = Math.max(sh[0] + sh[2], ext[0] + ext[2] + 480), sy2 = Math.max(sh[1] + sh[3], ext[1] + ext[3] + 480);
  el("rect", { class: "sheet", x: sx1, y: sy1, width: sx2 - sx1, height: sy2 - sy1 }, vpG);
  el("rect", { class: "sheet-dots", x: sx1, y: sy1, width: sx2 - sx1, height: sy2 - sy1 }, vpG);

  L = { ghost: el("g", {}, vpG), wires: el("g", {}, vpG), refs: el("g", {}, vpG), objs: el("g", {}, vpG), pills: el("g", {}, vpG), badges: el("g", {}, vpG), far: el("g", {}, vpG), xrefs: [], wired: new Map(), kind: new Map() };
  const wire = (uuid, key) => { if (!L.wired.has(uuid)) L.wired.set(uuid, new Set()); L.wired.get(uuid).add(key); };
  for (const w of p.wires) {
    wire(w.from[0], w.from[1]); wire(w.to[0], w.to[1]);
    for (const [u, k] of [w.from, w.to]) L.kind.set(u + "|" + k, !!L.kind.get(u + "|" + k) || w.analog);
  }
  for (const l of p.labels) wire(l.obj, l.port);
  const objByUuid = new Map(p.objects.map((o) => [o.uuid, o]));
  const logged = logMarks();
  S.log.drawn = marksKey(logged);

  const hops = wireHops(p.wires);
  p.wires.forEach((w, wi) => {
    const d = wirePath(hops[wi]);
    const mw = managedSrc(w.from[0]) || managedSrc(w.to[0]) ? " m-wire" : "";
    const line = el("path", { class: `wire${w.analog ? " analog" : ""}${w.status ? " d-" + w.status : ""}${mw}`, d, "data-f": w.from[0], "data-t": w.to[0], "data-src": w.src }, L.wires);
    const hit = el("path", { class: "wire-hit", d }, L.wires);
    const fo = objByUuid.get(w.from[0]), to = objByUuid.get(w.to[0]);
    hoverTip(hit, () => `<b>${esc(fo ? displayTitle(fo) : "")}</b>.${esc(portName(fo, w.from[1]))} → <b>${esc(to ? displayTitle(to) : "")}</b>.${esc(portName(to, w.to[1]))}<br><span class="tm">${w.analog ? "analog" : "digital"}${w.status ? " · " + w.status : ""}</span>${liveTip(w.src)}`);
    hit.addEventListener("mouseenter", () => line.classList.add("hl"));
    hit.addEventListener("mouseleave", () => line.classList.remove("hl"));
    hit.addEventListener("click", (e) => {
      e.stopPropagation();
      if (drag?.moved) return;
      select(w.to[0]);
      // On the newest revision, the wire's card: Delete, Revert, or why not.
      if (S.hist?.actions?.shown && S.edits) wcardOpen(e, w);
    });
  });

  const stack = new Map();
  for (const l of p.labels) {
    const key = l.obj + "|" + l.port;
    const n = stack.get(key) || 0;
    stack.set(key, n + 1);
    drawXref(L.refs, l, n);
  }

  for (const o of p.objects) {
    if (o.shape === "tag") drawTag(L.objs, o, p);
    else drawBlock(L.objs, o);
    if (o.status && S.view.compared) drawBadge(L.badges, o);
    if (logged.has(o.uuid)) drawLogBadge(L.badges, o, logged.get(o.uuid));
    if (o.moved_to) {
      const g = el("g", { class: "xref ghost-note" }, L.badges);
      const t = el("text", { class: "pg", x: o.rect[0], y: o.rect[1] + o.rect[3] + 160 }, g, `→ moved to page “${o.moved_to.page_title}”`);
      t.style.fontSize = "120px";
      g.addEventListener("click", (e) => { e.stopPropagation(); followTarget(o.moved_to); });
    }
    if (o.old_rect && S.view.compared) {
      const [x, y, w, h] = o.old_rect;
      el("rect", { class: "ghost", x, y, width: w, height: h, rx: 40 }, L.ghost);
      el("line", { class: "ghost-line ghost", x1: x + w / 2, y1: y + h / 2, x2: o.rect[0] + o.rect[2] / 2, y2: o.rect[1] + o.rect[3] / 2 }, L.ghost);
    }
  }
  drawInstances(p);
  drawExprs(p);
  wrDraw(p);
  L.live = liveBuild(p);
  liveApply(null);
  renderMinimap();
  if (!S.vp[S.page]) fit(); else applyVp();
  markCurrentChange();
}

/** Overview zoom: hide the small print, write each block's name at a readable screen size. */
function renderFar() {
  const v = S.vp[S.page], p = curPage();
  if (!v || !p || !L.far) return;
  const far = v.k < FAR_K;
  svg.classList.toggle("lod-far", far);
  L.far.innerHTML = "";
  if (!far) return;
  const px = 12.5 / v.k; // 12.5 px on screen
  // Reference bars: their name, as big as the bar allows.
  const tagPx = Math.min(9.5 / v.k, ROW * 0.78);
  if (ROW * v.k >= 7) for (const o of p.objects) {
    if (o.shape !== "tag" || o.moved_to) continue;
    const [x, y, w] = o.rect;
    const t = o.target || {};
    const wr = L.wired?.get(o.uuid);
    const right = o.ports.some((q) => q.side === "R" && wr?.has(q.key));
    // A flag wired on both sides is two tags: both ways show.
    const mem = memSplit(o) ? `${tagAddr(o, false)} ${tagAddr(o, true)}` : o.ty === "Memory" || t.ty === "Memory" ? tagAddr(o, right, !wr) : "";
    const addr = t.iname ? ` · ${t.iname}` : mem ? ` · ${mem}` : "";
    el("text", { class: "far-sub", x: x + w / 2, y: y + ROW / 2 + tagPx * 0.36, "text-anchor": "middle", "font-size": tagPx }, L.far, clip(o.title + addr, Math.floor((w * v.k) / (tagPx * v.k * 0.56))));
  }
  // Links to other pages: name and page, as big as the bar allows.
  if (ROW * v.k >= 7) for (const r of L.xrefs) {
    const n = Math.floor((LABEL_W * v.k) / (tagPx * v.k * 0.56));
    el("text", { class: "far-sub", x: r.x + LABEL_W / 2, y: r.y + tagPx * 0.36, "text-anchor": "middle", "font-size": tagPx }, L.far, clip(r.text + (r.page ? " ↗ " + r.page : ""), n));
  }
  for (const o of p.objects) {
    if (o.shape === "tag" || o.moved_to) continue;
    const [x, y, w, h] = o.rect;
    const sw = w * v.k;
    if (sw < 26) continue;
    const n = Math.floor(sw / 7.4);
    const note = o.ty === "Text" ? noteText(o).split("\n").find((l) => l.trim()) : null;
    // The name gets up to two lines (three for notes), never cut mid-word if it fits.
    const room = Math.max(1, Math.floor((h * v.k) / 16));
    const name = wrapWords(note || displayTitle(o), n, Math.min(note ? 3 : 2, room));
    const lines = name.map((t) => [t, "far-t"]);
    if (!note && o.shape === "large" && h * v.k > 16 * (name.length + 1) && !autoTitle(o.title)) lines.push([prettyType(o.ty), "far-sub"]);
    const cy = y + Math.min(h / 2, 900) - ((lines.length - 1) * px * 1.15) / 2 + px * 0.35;
    const st = S.marks && S.view.compared && o.status ? " st-" + o.status : " unch";
    lines.forEach(([t, cls], i) => {
      el("text", { class: cls + st, x: x + w / 2, y: cy + i * px * 1.15, "text-anchor": "middle", "font-size": cls === "far-sub" ? px * 0.88 : px }, L.far, t);
    });
  }
}

/** Greedy word wrap into at most `max` lines of `n` characters; the last line ends in … if text is left. */
function wrapWords(text, n, max) {
  const words = String(text || "").split(/\s+/).filter(Boolean);
  const lines = [];
  for (const w of words) {
    const last = lines[lines.length - 1];
    if (last !== undefined && (last + " " + w).length <= n) lines[lines.length - 1] = last + " " + w;
    else lines.push(w);
  }
  if (lines.length <= max) return lines.map((l) => clip(l, n));
  const out = lines.slice(0, max - 1).map((l) => clip(l, n));
  const rest = lines.slice(max - 1).join(" ");
  out.push(rest.length > n ? clip(rest, n) : rest.slice(0, Math.max(1, n - 1)) + "…");
  return out;
}

function portName(o, key) { return o?.ports.find((q) => q.key === key)?.name || key; }

function wrapObj(parent, o, cls) {
  const movable = !o.moved_to && !!moveState(o.uuid);
  const g = el("g", { class: `${cls}${o.status ? " d-" + o.status : ""}${o.moved_to ? " ghosted" : ""}${o.uuid === S.sel ? " sel" : ""}${movable ? " movable" : ""}`, "data-uuid": o.moved_to ? "" : o.uuid }, parent);
  if (movable) g.addEventListener("mousedown", (e) => mvStart(e, o));
  g.addEventListener("click", (e) => {
    e.stopPropagation();
    if (drag?.moved) return;
    select(o.uuid);
  });
  g.addEventListener("dblclick", (e) => {
    e.stopPropagation();
    if (o.target?.page && o.target.page !== S.page) gotoPage(o.target.page, { focus: o.target.uuid });
    else focusObj(o.uuid, 0.12);
  });
  hoverTip(g, () => objTip(o));
  return g;
}
/** What a tooltip says of a memory flag, or of a reference to one: which side of the flag it is. */
function flagTip(o, f) {
  const n = (l) => l.length ? `${l.length}` : "nothing";
  const role = { flag: "Memory flag", writes: "Writes memory flag", reads: "Reads memory flag" }[f.role];
  const what = f.role === "flag" ? "" : ` <b>${esc(f.title)}</b>`;
  const where = o.target?.page_title ? ` on page “${esc(o.target.page_title)}”` : "";
  const go = o.target?.page && o.target.page !== S.page ? "<br><span class='tm'>double-click to go there</span>" : "";
  return `${role}${what}${where}<br><span class="tm">written by ${n(f.written)} · read by ${n(f.read)}</span>${go}`;
}
function objTip(o) {
  const info = S.view.objects[o.uuid] || {};
  const parts = [`<b>${esc(displayTitle(o))}</b> <span class="tm">${esc(prettyType(o.ty))}</span>`];
  if (autoTitle(o.title) && o.title) parts.push(`<span class="tm">${esc(o.title)}</span>`);
  if (info.old_title) parts.push(`renamed from <i>${esc(info.old_title)}</i>`);
  if (info.flag) parts.push(flagTip(o, info.flag));
  else if (o.target) parts.push(`mirrors <b>${esc(o.target.title)}</b>${o.target.page_title ? ` on page “${esc(o.target.page_title)}”` : o.target.path ? ` in ${esc(o.target.path)}` : ""}${o.target.page && o.target.page !== S.page ? "<br><span class='tm'>double-click to go there</span>" : ""}`);
  if (info.room || info.category) parts.push(esc([info.room, info.category].filter(Boolean).join(" · ")));
  if (o.ty === "Text") parts.push(esc(clip(noteText(o), 300)).replace(/\n/g, "<br>"));
  if (o.status) parts.push(`<i>${o.status}</i>`);
  return parts.join("<br>");
}

/** Wire corners are rounded with this radius (page units; a port row is 192). */
const WIRE_R = 64;
/** A wire hops over another wire's vertical run with a half circle of this radius. */
const HOP_R = 36;

/** Each wire's points without repeats or straight-through corners, and where its
 *  horizontal runs cross another wire's vertical run: `{ pts, hops: Map(segment → [x…]) }`. */
function wireHops(wires) {
  const clean = wires.map((w) => {
    const out = [];
    for (const q of w.points) {
      const a = out[out.length - 1], b = out[out.length - 2];
      if (a && a[0] === q[0] && a[1] === q[1]) continue;
      if (a && b && ((b[0] === a[0] && a[0] === q[0]) || (b[1] === a[1] && a[1] === q[1]))) out.pop();
      out.push(q);
    }
    return out;
  });
  const verts = [];
  clean.forEach((pts, wi) => {
    for (let i = 0; i + 1 < pts.length; i++) {
      const [a, b] = [pts[i], pts[i + 1]];
      if (a[0] === b[0]) verts.push({ wi, x: a[0], y0: Math.min(a[1], b[1]), y1: Math.max(a[1], b[1]) });
    }
  });
  return clean.map((pts, wi) => {
    const hops = new Map();
    for (let i = 0; i + 1 < pts.length; i++) {
      const [a, b] = [pts[i], pts[i + 1]];
      if (a[1] !== b[1]) continue;
      const y = a[1], x0 = Math.min(a[0], b[0]), x1 = Math.max(a[0], b[0]);
      // Clear of the corners, so a hop never runs into a bend.
      const pad = WIRE_R + HOP_R;
      const xs = [];
      for (const v of verts) {
        if (v.wi === wi || wires[v.wi].src === wires[wi].src) continue;
        if (v.x > x0 + pad && v.x < x1 - pad && y > v.y0 + HOP_R && y < v.y1 - HOP_R) xs.push(v.x);
      }
      if (!xs.length) continue;
      xs.sort((p, q) => (b[0] > a[0] ? p - q : q - p));
      // Hops closer than a hop's width merge into one.
      const merged = [];
      for (const x of xs) if (!merged.length || Math.abs(x - merged[merged.length - 1]) >= 2 * HOP_R + 8) merged.push(x);
      hops.set(i, merged);
    }
    return { pts, hops };
  });
}

/** An SVG path for a wire: rounded corners, a half-circle hop at each crossing. */
function wirePath({ pts, hops }) {
  const n = pts.length;
  const len = (i) => Math.abs(pts[i + 1][0] - pts[i][0]) + Math.abs(pts[i + 1][1] - pts[i][1]);
  // The radius at corner i, at most half of either leg.
  const r = (i) => (i <= 0 || i >= n - 1 ? 0 : Math.min(WIRE_R, len(i - 1) / 2, len(i) / 2));
  const toward = (p, q, d) => {
    const dx = Math.sign(q[0] - p[0]), dy = Math.sign(q[1] - p[1]);
    return [p[0] + dx * d, p[1] + dy * d];
  };
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 0; i + 1 < n; i++) {
    const a = pts[i], b = pts[i + 1];
    const dir = Math.sign(b[0] - a[0]);
    for (const x of hops.get(i) || []) {
      d += `L${x - dir * HOP_R},${a[1]}A${HOP_R},${HOP_R} 0 0 ${dir > 0 ? 1 : 0} ${x + dir * HOP_R},${a[1]}`;
    }
    const e = toward(b, a, r(i + 1));
    d += `L${e[0]},${e[1]}`;
    if (i + 2 < n) {
      const s = toward(b, pts[i + 2], r(i + 1));
      d += `Q${b[0]},${b[1]} ${s[0]},${s[1]}`;
    }
  }
  return d;
}

/** Loxone Config 16 port stubs: a small grey tab on the block edge. The wire ends under it. */
function drawStub(g, q) {
  const left = q.side === "L";
  const cls = `stub${q.inv ? " inv" : ""}`;
  el("rect", { class: cls, x: left ? q.x - 112 : q.x + 12, y: q.y - 40, width: 100, height: 80, rx: 14 }, g);
  if (q.inv) el("circle", { class: "inv-dot", cx: q.x - 6, cy: q.y, r: 34 }, g);
  if (q.status) el("circle", { class: "pin-chg", cx: left ? q.x - 62 : q.x + 62, cy: q.y, r: 50 }, g);
}

function drawPins(g, o, inner) {
  const wired = L.wired?.get(o.uuid);
  for (const q of o.ports) {
    drawStub(g, q);
    // An unwired input with a constant shows the constant, as Loxone Config does.
    if (q.side === "L" && q.def != null && !wired?.has(q.key)) {
      el("text", { class: "def-t", x: q.x - 160, y: q.y + 36, "text-anchor": "end" }, g, clip(q.def, 10));
    }
    if (inner) {
      const twin = o.ports.some((r) => r !== q && r.side !== q.side && Math.abs(r.y - q.y) < 10);
      const n = twin ? Math.floor(inner * 0.55) : inner;
      const t = el("text", { class: "port-t", x: q.side === "L" ? q.x + 70 : q.x - 70, y: q.y + 36, "text-anchor": q.side === "L" ? "start" : "end" }, g, clip(q.name, n));
      el("title", {}, t, `${q.name} (${q.key})${q.def != null ? ` = ${q.def}` : ""}`);
    }
  }
}

/** The block's border: its header colour, a shade darker. */
function edgeColor(rgb) {
  const m = /^(\d+),(\d+),(\d+)$/.exec(rgb || "");
  if (!m) return "#5f7f68";
  const [r, g, b] = m.slice(1).map((v) => Math.round(+v * 0.62 + 40 * 0.38));
  return `rgb(${r},${g},${b})`;
}

function noteText(o) { return S.view.objects[o.uuid]?.attrs.find((a) => a.name === "Text")?.value || ""; }

/** A note (`Text` object): a sticky note in its own colour, carrying its text. */
function drawNote(parent, o) {
  const [x, y, w, h] = o.rect;
  const g = wrapObj(parent, o, "blk note");
  const color = o.color && o.color !== "0,0,0" ? `rgb(${o.color})` : "#fff9c4";
  el("rect", { class: "halo", x: x - 50, y: y - 50, width: w + 100, height: h + 100, rx: 60 }, g);
  el("rect", { class: "body note-body", x, y, width: w, height: h, style: `fill:${color}`, stroke: "#d8cf8a" }, g);
  el("path", { class: "note-fold", d: `M${x + w - 240},${y + h} L${x + w},${y + h - 240} L${x + w},${y + h} Z` }, g);
  const fo = el("foreignObject", { x: x + 90, y: y + 70, width: Math.max(0, w - 180), height: Math.max(0, h - 140) }, g);
  const div = document.createElement("div");
  div.className = "note-t";
  div.textContent = noteText(o) || o.title;
  fo.appendChild(div);
}

function drawBlock(parent, o) {
  if (o.ty === "Text") return drawNote(parent, o);
  const [x, y, w, h] = o.rect;
  const g = wrapObj(parent, o, "blk");
  const rgb = o.color && o.color !== "0,0,0" ? o.color : "141,255,112";
  const color = `rgb(${rgb})`;
  const large = o.shape === "large";
  const hh = large ? 264 : 288, r = 60;
  el("rect", { class: "halo", x: x - 50, y: y - 50, width: w + 100, height: h + 100, rx: 90 }, g);
  el("rect", { class: "body", x, y, width: w, height: h, rx: r, stroke: edgeColor(rgb) }, g);
  el("path", { class: "hdr", d: `M${x},${y + hh} V${y + r} Q${x},${y} ${x + r},${y} H${x + w - r} Q${x + w},${y} ${x + w},${y + r} V${y + hh} Z`, fill: color }, g);
  el("rect", { class: "edge", x, y, width: w, height: h, rx: r, stroke: edgeColor(rgb) }, g);
  const dark = lightColor(rgb);
  const named = !autoTitle(o.title) && o.title !== o.ty;
  // Header: the name, left-aligned, with the settings and info buttons on the right.
  const icons = w >= 1100 ? 2 : 0;
  const title = large ? displayTitle(o) : named ? o.title : prettyType(o.ty);
  el("text", { class: "hdr-t" + (dark ? " dark" : ""), x: x + 90, y: y + hh / 2 + 42 }, g, clip(title, Math.floor((w - 140 - icons * 190) / 66)));
  for (let i = 0; i < icons; i++) {
    const cx = x + w - 130 - i * 180;
    el("circle", { class: "icon", cx, cy: y + hh / 2, r: 70 }, g);
    el("text", { class: "icon-t", x: cx, y: y + hh / 2 + 34, "text-anchor": "middle" }, g, i ? "⚙" : "i");
  }
  if (large) {
    el("rect", { class: "sub", x: x + 1, y: y + hh, width: w - 2, height: 216 }, g);
    el("text", { class: "sub-t", x: x + w / 2, y: y + hh + 146, "text-anchor": "middle" }, g, clip(prettyType(o.ty), Math.floor(w / 60)));
    const info = S.view.objects[o.uuid] || {};
    [info.room, info.category].filter(Boolean).forEach((m, i) => {
      if (480 + ROW * (i + 1) <= h) el("text", { class: "meta-t", x: x + w / 2, y: y + 480 + ROW * i + 136, "text-anchor": "middle" }, g, clip(m, 15));
    });
    drawPins(g, o, 20);
  } else {
    // A named logic block keeps its operation in grey in the middle, as Loxone Config shows it.
    if (named) {
      const n = Math.floor((w - 2 * 380) / 56);
      if (n >= 3) el("text", { class: "meta-t", x: x + w / 2, y: y + hh + 96 + 36, "text-anchor": "middle" }, g, clip(prettyType(o.ty), n));
    }
    drawPins(g, o, 7);
  }
  if (S.view.objects[o.uuid]?.app?.shown) drawAppBadge(g, x, y + h);
  const m = !o.moved_to && managedSrc(o.uuid);
  if (m) { g.classList.add("managed"); drawManagedBadge(g, x, y, m); }
  const lf = !o.moved_to ? lintFindings(o.uuid) : [];
  if (lf.length) { g.classList.add("linted"); drawLintBadge(g, x + w, y, lf); }
  // A test that fails on it (#67): left of the lint badge.
  const tf = !o.moved_to ? testFindings(o.uuid) : [];
  if (tf.length) { g.classList.add("test-failed"); drawTestBadge(g, x + w - (lf.length ? 280 : 0), y, tf); }
}
/** A red badge at the block's upper right: a test of lox logic test that checks it fails (#67). Apply is off then. */
function drawTestBadge(g, x, y, list) {
  const b = el("g", { class: "tbadge" }, g);
  el("title", {}, b, list.map((t) => testLine(t)).join("\n"));
  el("circle", { cx: x, cy: y, r: 105 }, b);
  el("text", { x, y: y + 45, "text-anchor": "middle" }, b, "✕");
}
/** A warning badge at the block's upper right: lox logic lint has findings on it (#66). Never blocks Apply. */
function drawLintBadge(g, x, y, list) {
  const b = el("g", { class: "lbadge" }, g);
  el("title", {}, b, list.map((f) => `Lint (${f.kind}): ${f.message}`).join("\n"));
  el("path", { d: `M ${x} ${y - 110} L ${x + 115} ${y + 80} L ${x - 115} ${y + 80} Z` }, b);
  el("text", { x, y: y + 60, "text-anchor": "middle" }, b, "!");
}
/** A small badge at the block's upper left: the project's Lox Logic module manages it. */
function drawManagedBadge(g, x, y, m) {
  const b = el("g", { class: "mbadge" }, g);
  el("title", {}, b, managedTip(m));
  // On the corner, clear of the name in the header.
  el("rect", { x: x - 130, y: y - 100, width: 220, height: 160, rx: 40 }, b);
  el("text", { x: x - 20, y: y + 24, "text-anchor": "middle" }, b, "λ");
}
/** A small phone at the block's lower left: the block is shown in the app. */
function drawAppBadge(g, x, y) {
  const b = el("g", { class: "abadge" }, g);
  el("title", {}, b, "Shown in the app: the Miniserver sends its live values");
  el("circle", { cx: x, cy: y, r: 100 }, b);
  el("rect", { x: x - 38, y: y - 62, width: 76, height: 124, rx: 14 }, b);
  el("line", { x1: x - 14, y1: y + 40, x2: x + 14, y2: y + 40 }, b);
}

/** Analog references get a black "#" chip, digital ones a ring, as in Loxone Config 16. */
function drawKind(g, cx, cy, analog) {
  if (analog) {
    el("rect", { class: "kind", x: cx - 52, y: cy - 52, width: 104, height: 104, rx: 18 }, g);
    el("text", { class: "kind-t", x: cx, y: cy + 30, "text-anchor": "middle" }, g, "#");
  } else {
    el("circle", { class: "kind-ring", cx, cy, r: 54 }, g);
    el("circle", { class: "kind", cx, cy, r: 30 }, g);
  }
}

/** The address a tag shows in its pill, and which way the flag goes through it: a memory flag is written
 *  through its input (`→M`) and read through its outputs (`M→`), as an output reference and an input
 *  reference of it show it; a memory flag wired on neither side is just `M`. */
function tagAddr(o, right, unwired) {
  const t = o.target || {};
  if (t.iname) return t.iname;
  if (o.ty === "Memory") return unwired ? "M" : right ? "M→" : "→M";
  if (t.ty === "Memory") return o.ty === "InputRef" ? "M→" : "→M";
  return o.ty === "OutputRefLM" ? "✉" : "";
}

/** A memory flag wired on both sides of this page is two tags, as Loxone Config shows it: the one it is written
 *  through (the input's wire) and the one it is read through (the outputs' wires). */
function memSplit(o) {
  const wired = L.wired?.get(o.uuid);
  const sideWired = (side) => o.ports.some((q) => q.side === side && wired?.has(q.key));
  return o.ty === "Memory" && sideWired("L") && sideWired("R");
}

function drawTag(parent, o, page) {
  const [x, y, w, h] = o.rect;
  const g = wrapObj(parent, o, "tag");
  const wired = L.wired?.get(o.uuid);
  const sideWired = (side) => o.ports.some((q) => q.side === side && wired?.has(q.key));
  // The side the wire leaves from: where it is wired, else what the type says.
  const feedsRight = sideWired("R") || (!sideWired("L") && o.ty === "InputRef");
  const split = memSplit(o);
  const gap = h / 2, fw = split ? Math.floor((w - gap) / 2) : w;
  const rx = h / 2, cy = y + h / 2;
  const t = o.target || {};
  const offPage = t.page && t.page !== page.uuid;
  for (const right of split ? [false, true] : [feedsRight]) {
    const bx = split && right ? x + w - fw : x;
    // Analog if the wires on its feeding side are; an unwired reference goes by its address (AI, AQ, VI…).
    const fed = o.ports.filter((q) => (q.side === "R") === right && L.kind.has(o.uuid + "|" + q.key));
    const analog = fed.length ? fed.some((q) => L.kind.get(o.uuid + "|" + q.key)) : /^(A[QI]|VI|VA)/.test(t.iname || "");
    el("rect", { class: "halo", x: bx - 50, y: y - 50, width: fw + 100, height: h + 100, rx: rx + 50 }, g);
    el("rect", { class: "bar", x: bx, y, width: fw, height: h, rx, ry: rx }, g);
    const addr = tagAddr(o, right, !sideWired("L") && !sideWired("R"));
    const addrW = addr ? (split ? 300 : Math.max(400, addr.length * 76 + 180)) : 0;
    // The address sits in its own pill at the wired end, with the kind chip beside it.
    const ax = right ? bx + fw - addrW : bx;
    if (addr) {
      el("rect", { class: "addr", x: ax, y, width: addrW, height: h, rx, ry: rx }, g);
      el("text", { class: "addr-t", x: ax + addrW / 2, y: cy + 38, "text-anchor": "middle" }, g, clip(addr, 7));
    }
    const kx = right ? ax - 100 : ax + addrW + 100;
    drawKind(g, kx, cy, analog);
    const icX = right ? bx + 110 : bx + fw - 110;
    el("text", { class: "ic-t", x: icX, y: cy + 36, "text-anchor": "middle" }, g, t.page ? "⚙" : "i");
    // Half a tag has less room: tighter padding and narrower letters, the name clipped to what is left.
    const pad = split ? 170 : 210, nameW = split ? fw - addrW - 108 - pad - 40 : fw - addrW - 220 - 200 - 60;
    const name = clip(o.title + (offPage ? " ↗" : ""), Math.floor(nameW / (split ? 50 : 60)));
    if (right) el("text", { class: "cell-t", x: bx + pad, y: cy + 36 }, g, name);
    else el("text", { class: "cell-t", x: bx + fw - pad, y: cy + 36, "text-anchor": "end" }, g, name);
  }
  for (const side of ["L", "R"]) {
    const q = o.ports.find((p) => p.side === side && wired?.has(p.key)) || (split || (side === "R") === feedsRight ? o.ports.find((p) => p.side === side) : null);
    if (q) drawStub(g, q);
  }
}

function drawXref(parent, l, n) {
  const left = l.side === "L";
  const y = l.y + n * (ROW + 24);
  const bx = left ? l.x - 150 - LABEL_W : l.x + 150;
  const t = l.target;
  const g = el("g", { class: `xref${l.status ? " d-" + l.status : ""}`, "data-obj": l.obj }, parent);
  const label = `${t.port ? t.port + ": " : ""}${autoTitle(t.title) ? prettyType(t.ty) : t.title}`;
  L.xrefs.push({ x: bx, y, text: label, page: t.page_title || "" });
  const analog = /^A[QI]/.test(l.port) || /^A[QI]/.test(t.port || "");
  const wx = left ? bx + LABEL_W : bx;
  el("path", { class: `xref-wire${analog ? " analog" : ""}${l.status ? " d-" + l.status : ""}`, "data-f": l.obj, "data-src": l.src, d: wirePath(wireHops([{ points: [[l.x, l.y], [(l.x + wx) / 2, l.y], [(l.x + wx) / 2, y], [wx, y]] }])[0]) }, g);
  const top = y - ROW / 2, rx = ROW / 2;
  el("rect", { class: "bar", x: bx, y: top, width: LABEL_W, height: ROW, rx, ry: rx }, g);
  // The link icon in its own pill at the wired end, like Loxone Config's connection references.
  const pw = 300, px = left ? bx + LABEL_W - pw : bx;
  el("rect", { class: "addr", x: px, y: top, width: pw, height: ROW, rx, ry: rx }, g);
  el("text", { class: "ic-t", x: px + pw / 2, y: y + 36, "text-anchor": "middle" }, g, l.incoming ? "⇥" : "↦");
  const tx = left ? bx + 90 : bx + pw + 70, tEnd = left ? px - 60 : bx + LABEL_W - 90;
  el("text", { class: "t", x: tx, y: y + 36 }, g, clip(label, 19));
  el("text", { class: "pg", x: tEnd, y: y + 34, "text-anchor": "end" }, g, clip(t.page_title ? "↗ " + t.page_title : t.path ? "⌂ " + (t.path.split(" / ").pop() || "") : "", 10));
  g.addEventListener("click", (e) => { e.stopPropagation(); if (!drag?.moved) followTarget(t); });
  hoverTip(g, () => `${l.incoming ? "Comes from" : "Goes to"} <b>${esc(t.title)}</b>${t.port ? "." + esc(t.port) : ""} <span class="tm">${esc(prettyType(t.ty))}</span><br>${t.page_title ? `on page “${esc(t.page_title)}” — click to go there` : t.path ? `in ${esc(t.path)} — click to select` : ""}${l.status ? `<br><i>${l.status}</i>` : ""}${liveTip(l.src)}`);
}

function drawBadge(parent, o) {
  const sym = { added: "+", removed: "−", changed: "~", moved: "↔" }[o.status];
  const col = { added: "#1b8a3a", removed: "#c62828", changed: "#e89400", moved: "#1f6fd1" }[o.status];
  if (!sym) return;
  const g = el("g", { class: "dbadge" }, parent);
  const cx = o.rect[0] + o.rect[2], cy = o.rect[1];
  el("circle", { cx, cy, r: 110, fill: col }, g);
  el("text", { x: cx, y: cy + 52, "text-anchor": "middle" }, g, sym);
}

/** A block the Miniserver's log named since the last program change: a small mark on its top-left corner. */
function drawLogBadge(parent, o, e) {
  const g = el("g", { class: "lbadge " + e.kind }, parent);
  const cx = o.rect[0], cy = o.rect[1];
  el("circle", { cx, cy, r: 100 }, g);
  el("text", { x: cx, y: cy + 46, "text-anchor": "middle" }, g, "!");
  hoverTip(g, () => `<b>Miniserver log</b> ${esc(e.at)}<br>${esc(e.text)}`);
  g.addEventListener("click", (ev) => { ev.stopPropagation(); if (!drag?.moved) openLog(e); });
}

function followTarget(t) {
  if (t.page) gotoPage(t.page, { focus: t.uuid });
  select(t.uuid);
}

function focusObj(uuid, minK) {
  const p = curPage();
  const o = p?.objects.find((x) => x.uuid === uuid && !x.moved_to);
  if (!o) return;
  if (!S.vp[S.page]) fit();
  const v = S.vp[S.page];
  v.k = Math.max(v.k, minK || K100);
  const [x, y, w, h] = o.rect;
  v.x = svg.clientWidth / 2 - (x + w / 2) * v.k;
  v.y = svg.clientHeight / 2 - (y + h / 2) * v.k;
  applyVp();
  const g = vpG.querySelector(`[data-uuid="${CSS.escape(uuid)}"]`);
  if (g) { g.classList.remove("flash"); void g.getBBox(); g.classList.add("flash"); setTimeout(() => g.classList.remove("flash"), 2000); }
}

// ── minimap ────────────────────────────────────────────────────────────
const mm = $("mm");
let mmExt = null;
function renderMinimap() {
  const p = curPage(), objs = $("mm-objs");
  objs.innerHTML = "";
  $("minimap").classList.toggle("hidden", !p || !p.objects.length);
  if (!p) return;
  const [x, y, w, h] = pageExtent(p);
  const pad = 300;
  mmExt = [x - pad, y - pad, w + 2 * pad, h + 2 * pad];
  mm.setAttribute("viewBox", mmExt.join(" "));
  mm.setAttribute("preserveAspectRatio", "xMidYMid meet");
  el("rect", { x: mmExt[0], y: mmExt[1], width: mmExt[2], height: mmExt[3], fill: "#fff" }, objs);
  const col = { added: "#1b8a3a", removed: "#c62828", changed: "#e89400", moved: "#1f6fd1" };
  const marks = S.marks && S.view.compared;
  for (const o of p.objects) {
    const [ox, oy, ow, oh] = o.rect;
    const c = marks && o.status ? col[o.status] : o.shape === "tag" ? "#9aa09a" : o.color && o.color !== "0,0,0" ? `rgb(${o.color})` : "#5cb83f";
    el("rect", { x: ox, y: oy, width: ow, height: Math.max(oh, 120), fill: c, opacity: marks && o.status ? 1 : 0.8 }, objs);
  }
  updateMinimap();
}
function updateMinimap() {
  const v = S.vp[S.page], r = $("mm-view");
  if (!v || !mmExt) return;
  r.setAttribute("x", -v.x / v.k); r.setAttribute("y", -v.y / v.k);
  r.setAttribute("width", svg.clientWidth / v.k); r.setAttribute("height", svg.clientHeight / v.k);
}
function minimapGo(e) {
  const v = S.vp[S.page];
  if (!v) return;
  const pt = mm.createSVGPoint();
  pt.x = e.clientX; pt.y = e.clientY;
  const q = pt.matrixTransform(mm.getScreenCTM().inverse());
  v.x = svg.clientWidth / 2 - q.x * v.k;
  v.y = svg.clientHeight / 2 - q.y * v.k;
  applyVp();
}
let mmDrag = false;
$("minimap").addEventListener("mousedown", whenReady((e) => { mmDrag = true; minimapGo(e); e.preventDefault(); }));
window.addEventListener("mousemove", whenReady((e) => { if (mmDrag) minimapGo(e); }));
window.addEventListener("mouseup", whenReady(() => (mmDrag = false)));

// ── selection, focus mode & properties ────────────────────────────────
function select(uuid, opts) {
  // A block of an expression drawn folded (#82): open it, so the block shows.
  const fx = uuid && exprOf(uuid);
  if (fx && !S.exOpen.has(fx.id) && curPage()?.objects.some((o) => o.uuid === uuid)) { S.exOpen.add(fx.id); S.sel = uuid; renderPage(); }
  S.sel = uuid || null;
  vpG.querySelectorAll(".sel").forEach((n) => n.classList.remove("sel"));
  if (uuid) vpG.querySelectorAll(`[data-uuid="${CSS.escape(uuid)}"]`).forEach((n) => n.classList.add("sel"));
  applyFocus();
  renderProps();
  renderModes();
  if (!opts || !opts.noReveal) revealInTree(uuid);
  writeHash(false);
}
/** Light up the selected block, its wires and its direct neighbours; dim the rest. */
function applyFocus() {
  vpG.querySelectorAll(".lit").forEach((n) => n.classList.remove("lit"));
  const sel = S.sel;
  const on = !!sel && !!vpG.querySelector(`[data-uuid="${CSS.escape(sel)}"]`);
  vpG.classList.toggle("focus", on);
  if (!on) return;
  const lit = new Set([sel]);
  vpG.querySelectorAll(".wire").forEach((w) => {
    if (w.dataset.f === sel || w.dataset.t === sel) { w.classList.add("lit"); lit.add(w.dataset.f); lit.add(w.dataset.t); }
  });
  vpG.querySelectorAll(".xref").forEach((x) => { if (x.dataset.obj === sel) { x.classList.add("lit"); x.querySelector(".xref-wire")?.classList.add("lit"); } });
  for (const u of lit) vpG.querySelectorAll(`[data-uuid="${CSS.escape(u)}"]`).forEach((n) => n.classList.add("lit"));
  vpG.querySelectorAll(".live-pill").forEach((n) => { if (lit.has(n.dataset.f)) n.classList.add("lit"); });
}

function linkTarget(t) {
  const name = autoTitle(t.title) ? `${prettyType(t.ty)} (${t.title})` : t.title;
  const where = t.page_title ? (t.page === S.page ? "this page" : t.page_title) : t.path ? t.path.split(" / ").pop() : "";
  return `<button type="button" class="lnk" data-go="${esc(t.uuid)}" data-page="${esc(t.page || "")}">${esc(name)}${t.port ? "." + esc(t.port) : ""}</button>${where ? `<span class="where">${esc(where)}</span>` : ""}`;
}

/** A calendar entry in one line, for a tooltip: “Every year on 24 Dec · switches on Holiday”. */
function calTip(cal) {
  if (!cal) return "";
  return `${cal.when ?? "Date cannot be read"} · switches on ${cal.mode.title}`;
}

const propRow = (k, v, cls, tip) => `<div class="pr ${cls || ""}"><div class="k${tip ? " hinted" : ""}"${tip ? ` title="${esc(tip)}"` : ""}>${esc(k)}</div><div class="v">${v}</div></div>`;
function renderProps() {
  // Live values re-render Properties often: keep the edit field being typed in.
  const f = document.activeElement?.closest?.("#props [data-ed-field], #props [data-sch-field], #props [data-mo-field], #props [data-mo-name], #props [data-ex-field], #props [data-tp-field], #props [data-ru-field], #props [data-dflt-field]");
  const attr = ["data-ed-field", "data-sch-field", "data-mo-field", "data-mo-name", "data-ex-field", "data-tp-field", "data-ru-field", "data-dflt-field"].find((a) => f?.hasAttribute(a));
  const keep = f && { field: f.getAttribute(attr), a: f.selectionStart, b: f.selectionEnd };
  renderPropsBody();
  if (keep) {
    const g = $("props").querySelector(`[${attr}="${CSS.escape(keep.field)}"]`);
    if (g) { g.focus(); try { g.setSelectionRange(keep.a, keep.b); } catch { /* not a text field */ } }
  }
}
function renderPropsBody() {
  meSync();
  ruSync();
  acSync();
  tiSync();
  const box = $("props");
  const o = S.sel && S.view?.objects[S.sel];
  if (!o) {
    $("props-h").textContent = "Properties";
    box.innerHTML = '<div class="empty">Select a block on the page or in the project tree to see its configuration and connections.</div>';
    return;
  }
  $("props-h").textContent = "Properties";
  const marks = S.marks && S.view.compared;
  const val = (v) => (v == null || v === "" ? '<span class="none-v">—</span>' : esc(v));
  const chg = (oldV, newV, changed) => (marks && changed ? `<span class="old">${esc(oldV ?? "—")}</span><span class="new">${esc(newV ?? "—")}</span>` : val(newV));
  const sections = [];
  const section = (name, rows, count) => { if (rows.length) sections.push({ name, rows, count }); };
  const row = propRow;

  const ed = editRows(o, row);
  if (ed) section("Edit", ed.rows, ed.count);
  const se = seRows(o, row);
  if (se) section("Switching times", se.rows, se.count);
  const mo = moRows(o, row, marks);
  if (mo) section("Moods", mo.rows, mo.count);
  const me = meRows(o, row, marks);
  if (me) section("Members", me.rows, me.count);
  const ru = ruRows(o, row, marks);
  if (ru) section("Rule", ru.rows, ru.count);
  const ti = tiRows(o, row, marks);
  if (ti) section("Tiles", ti.rows, ti.count);
  const tl = tiLinkedRows(o, row);
  if (tl) section("Touch Pure Flex", tl);
  const ex = exRows(o, row, marks);
  if (ex) section("Expression", ex.rows, ex.count);
  const tp = tpRows(o, row, marks);
  if (tp) section("Template", tp.rows, tp.count);
  const ms = managedSrc(o.uuid);
  // The errors of lox logic check whose line names this block (#65): on the newest config only.
  const ce = S.rev === editRev()?.id ? checkErrors(o.uuid) : [];
  // lox logic lint's findings on it (#66); on an NFC Code Touch also the users whose tag or code opens none.
  const lf = lintFindings(o.uuid);
  const access = o.ty === "NfcCodeTouch" && S.rev === editRev()?.id ? lintAll().filter((f) => f.kind === "no-access-right") : [];
  // The tests of lox logic test that fail on it, or could not run for it (#67).
  const tf = testFindings(o.uuid);
  if (ms || ce.length || lf.length || access.length || tf.length) {
    const path = srcPath(ms);
    const rows = ce.map((e) => `<div class="ed-note ck-err">⚠ Doesn't compile${e.file ? ` (${esc(checkAt(e))})` : ""}: ${esc(e.message)}</div>`);
    rows.push(...tf.map((t) => `<div class="ed-note ck-err tf-note">✕ ${esc(testLine(t, o.uuid))}</div>`));
    rows.push(...lf.map((f) => `<div class="ed-note lt-note">⚠ Lint (${esc(f.kind)}${f.file ? `, ${esc(checkAt(f))}` : ""}): ${esc(f.message)}</div>`));
    rows.push(...access.map((f) => `<div class="ed-note lt-note">⚠ Lint (${esc(f.kind)}): ${esc(f.name)}, ${esc(f.message)}</div>`));
    if (ms) rows.push(
      row("Slug", `<span class="mono">${esc(ms.slug)}</span>`),
      row("Source", path
        ? `<span class="mono src-path">${esc(path)}</span><button type="button" class="copy" data-copy="${esc(path)}" data-label="Copy path" title="Copy the file and line, for an editor or a prompt">Copy path</button><div class="src-how">${esc(srcHow(ms))}</div>`
        : `<span class="none-v">${esc(srcHow(ms))}</span>`),
      `<div class="ed-note">Change it in the module: an edit made in Loxone Config is overwritten on the next push.</div>`);
    section("Lox Logic", rows, ce.length + lf.length + access.length + tf.length || undefined);
  }
  const msg = msgRows(o, marks);
  if (msg) section("Message", msg.rows, msg.count);
  if (o.rights) {
    const rr = rightsRows(o, marks);
    // Editing who may open an NFC Code Touch (#76), and who gets a push notification (#77).
    rr.rows.push(...acRows(o));
    section(o.rights.kind === "access" ? "Access" : "Recipients", rr.rows, rr.count + (accessEdit(o.uuid, o.rights.kind)?.pending ? 1 : 0));
  }
  const opT = opTarget(o);
  if (opT) section("Operate", [opControls(opT)]);
  if (liveHere()) {
    const rows = [];
    const ctl = S.live.controls[o.uuid];
    for (const [name, su] of Object.entries(ctl?.states || {})) rows.push(row(name, lvSpan(su)));
    for (const q of o.ports) {
      const inc = q.conns.find((c) => c.dir === "in");
      // What the block receives: its own value if the Miniserver streams it, else its wire's.
      const src = inc && !S.live.values.has(q.uuid) ? inc.src : q.uuid;
      if (!inc && !q.conns.length && !liveVal(q.uuid)) continue;
      rows.push(row(q.name, lvSpan(src) + `<span class="lv-of">${inc ? "in" : "out"}</span>`));
    }
    section("Live values", rows.length ? rows : [`<div class="empty">The Miniserver sends no values for this block.</div>`], rows.length);
  }
  for (const sec of liveSections(o, row, marks)) section(sec.name, sec.rows, sec.count);
  const cal = o.calendar;
  if (cal) {
    const modeV = (m) => (m.target ? linkTarget(m.target) : `<span class="none-v">${esc(m.title)}</span>`);
    const when = cal.when == null ? `<span class="none-v">Cannot read: ${esc(cal.why || "unknown")}. See Advanced.</span>` : esc(cal.when);
    section("Calendar entry", [
      row("When", marks && cal.old_when != null ? `<span class="old">${esc(cal.old_when)}</span><span class="new">${when}</span>` : when, marks && cal.old_when != null ? "pr-chg" : ""),
      row("Switches on", marks && cal.old_mode ? `<span class="old">${esc(cal.old_mode.title)}</span><span class="new">${modeV(cal.mode)}</span>` : modeV(cal.mode), marks && cal.old_mode ? "pr-chg" : ""),
      ...(() => {
        const d = calDriftOf(o.uuid);
        if (!d) return [];
        const what = d.kind === "deleted" ? "Deleted in the app since the deployed config. Pull or Sync to take that over." : `Changed in the app since the deployed config: ${d.changes.join("; ")}. Pull or Sync to take it over.`;
        return [row("In the app", `<span class="cal-app">${esc(what)}</span>`, "pr-chg")];
      })(),
    ]);
  }
  const periods = mdPeriodRows(o, row);
  if (periods) section("Heating and cooling", periods);
  const inApp = appRows(o, row, chg, marks);
  if (inApp) section("In the app", inApp.rows, inApp.count);
  const note = o.ty === "Text" ? o.attrs.find((a) => a.name === "Text") : null;
  if (note) section("Note", [`<div class="note-p${marks && note.changed ? " pr-chg" : ""}">${marks && note.changed ? chg(note.old, note.value, true) : esc(note.value || "")}</div>`]);
  const dv = o.device;
  if (dv) {
    const rows = [row("Kind", esc(dv.kind) + (dv.code ? ` <span class="mono none-v">${esc(dv.code)}</span>` : ""))];
    if (!dv.own) rows.push(row("Device", `<button type="button" class="lnk" data-go="${esc(dv.uuid)}">${esc(dv.title)}</button>`));
    else if (!o.attrs.some((a) => a.hint && /^(Serial|IP)$/.test(a.name))) rows.push(row("Serial", val(dv.serial)));
    if (marks && dv.how) rows.push(row("This change", `<span class="hw-how">${esc(HW_HOW[dv.how] || dv.how)}</span>`, "pr-chg"));
    // Whether a Tree or Air input or output records statistics (#85), as the config has it.
    if (o.statistics) {
      const was = marks ? o.statistics_old : null;
      rows.push(row("Statistics", was ? chg(was, o.statistics, true) : esc(o.statistics), was ? "pr-chg" : "", STATS_TIP));
    }
    // A Tree device created here (#85): not paired until a Tree search in Loxone Config finds it.
    const made = hwCreated(o.uuid);
    if (made && !made.paired) rows.push(row("Pairing", `<span class="hw-how">${esc(HW_UNPAIRED)}</span>`, "pr-chg"));
    // The settings matched to the device's own configuration, with what they do there.
    for (const a of o.attrs.filter((x) => x.hint)) {
      rows.push(row(HW_LABEL[a.name] || a.name, marks && a.changed ? chg(a.old, a.value, true) : val(a.value), marks && a.changed ? (a.hw ? "pr-chg pr-hw" : "pr-chg") : "", a.hint));
    }
    if (dv.note) rows.push(`<div class="note-p hw-note">${esc(dv.note)}</div>`);
    section("Device", rows);
  }
  // A constant with the unit Lox Logic writes it with (#79): `Time 30min`, not 1800.
  // From a `let` constant or a `defaults` statement (#81): `28 (temp_schwelle)`.
  const constOf = (q) => { const u = constUse(o.uuid, q.key); return u ? ` <span class="const-of" title="${esc(constTip(u))}">(${esc(constName(u))})</span>` : ""; };
  const params = o.ports.filter((q) => q.def != null || q.def_changed || q.inv)
    .map((q) => row(q.name + (q.inv ? " (inverted)" : ""), chg(q.old_def_text ?? q.old_def, q.def_text ?? q.def, q.def_changed) + constOf(q) + dfltButton(o, q), marks && q.def_changed ? "pr-chg" : ""));
  // The module's `let` and `defaults` values this block takes that are not deployed, and a default being made or
  // changed (#81).
  params.push(...constPendingRows(o, row), ...dfltRows(o));
  section("Parameters", params, params.length);
  const sets = settingsRows(o, row, marks, chg);
  if (sets) section("Settings", sets.rows, sets.count);

  const connRows = (dir) => o.ports.filter((q) => q.conns.some((c) => c.dir === dir)).map((q) => {
    const cs = q.conns.filter((c) => c.dir === dir);
    const add = marks && cs.some((c) => c.status === "added"), rem = marks && cs.some((c) => c.status === "removed");
    return row(q.name, cs.map((c) => `<div class="conn ${marks ? c.status : ""}"><span class="arr">${dir === "in" ? "←" : "→"}</span>${linkTarget(c.target)}</div>`).join(""),
      add && !rem ? "pr-add" : rem && !add ? "pr-rem" : add && rem ? "pr-chg" : "");
  });
  const ins = connRows("in"), outs = connRows("out");
  section("Inputs", ins, ins.length);
  section("Outputs", outs, outs.length);

  if (o.flag) {
    // A memory flag is written through its input and read through its outputs; on other pages Loxone
    // Config shows it as an output reference (writes it) and an input reference (reads it) (#104).
    const f = o.flag, self = f.role === "flag";
    const sites = (l, none) => l.length ? l.map((t) => `<div class="conn">${REF_TYPES.has(t.ty) ? `<button type="button" class="lnk" data-go="${esc(t.uuid)}" data-page="${esc(t.page || "")}">${t.ty === "InputRef" ? "Input" : "Output"} reference on ${t.page === S.page ? "this page" : esc(t.page_title || "a page")}</button>` : linkTarget(t)}</div>`).join("") : `<span class="none-v">${none}</span>`;
    const rows = [];
    if (!self) rows.push(row(f.role === "writes" ? "Writes" : "Reads", `<div class="conn">${linkTarget(o.target)}</div>`));
    rows.push(row("Written", sites(f.written, "nowhere: nothing sets it"), "", "Where the flag is set: the blocks wired to its input and the output references of it"));
    rows.push(row("Read", sites(f.read, "nowhere: nothing uses it"), "", "Where the flag is used: the blocks wired to its outputs and the input references of it"));
    section("Memory flag", rows, f.written.length + f.read.length);
  } else if (o.mirrored_by.length) {
    section("Shown on pages", o.mirrored_by.map(([pg, ref]) => {
      const p = S.pageOf.get(pg);
      return row("Page", `<button type="button" class="lnk" data-page="${esc(pg)}" data-go="${esc(ref)}">${esc(p?.title || pg)}</button>`);
    }), o.mirrored_by.length);
  }

  const details = [];
  details.push(row("Type", esc(o.ty)));
  const iname = o.attrs.find((a) => a.name === "IName");
  if (iname && iname.value !== o.uuid) details.push(row("Short name", val(iname.value)));
  if (autoTitle(o.title) && o.title) details.push(row("Object number", esc(o.title)));
  if (o.path && !o.page) details.push(row("Location", esc(o.path)));
  details.push(row("UUID", `<span class="mono">${esc(o.uuid)}</span><button type="button" class="copy" data-copy="${esc(o.uuid)}" data-label="Copy">Copy</button>`));
  section("Details", details);

  // The block settings (#79) are in Settings, as Lox Logic writes them.
  const inSettings = new Set((o.settings || []).map((x) => x.attr));
  const raw = o.attrs.filter((a) => !inSettings.has(a.name) && (!["Title", "Type", "IName", ...(note ? ["Text"] : [])].includes(a.name) || (a.changed && a !== note))).map((a) => {
    let v;
    if (a.name === "Cl" && a.value && /^\d+,\d+,\d+$/.test(a.value) && !(marks && a.changed)) v = `<span class="swatch" style="background:rgb(${esc(a.value)})"></span> ${esc(a.value)}`;
    else if (marks && a.changed) v = chg(a.old, a.value, true);
    else if (a.value && a.value.length > 160) v = `<span class="long">${esc(a.value)}</span><button type="button" class="more">Show all</button>`;
    else v = val(a.value);
    return row(a.name, v, marks && a.changed ? (a.value == null ? "pr-rem" : "pr-chg") : "", a.hint);
  });
  const rawChanged = marks && o.attrs.some((a) => a.changed);
  section("Advanced", raw, raw.length);

  const color = o.attrs.find((a) => a.name === "Cl")?.value;
  const facts = [];
  if (o.page) facts.push(`<button type="button" class="chip" data-page="${esc(o.page)}" data-go="${esc(o.uuid)}" title="Show on its page">▦ ${esc(S.pageOf.get(o.page)?.title || "page")}</button>`);
  if (o.room) facts.push(`<span class="chip" title="Room">⌂ ${esc(o.room)}</span>`);
  if (o.category) facts.push(`<span class="chip" title="Category">◆ ${esc(o.category)}</span>`);
  if (ms) facts.push(`<span class="chip managed" title="Managed by the project's Lox Logic module">λ ${esc(ms.slug)}</span>`);
  if (o.target) facts.push(`<span class="chip" title="Mirrors">↪ ${linkTarget(o.target)}</span>`);
  if (marks && dv?.how) facts.push(`<span class="chip hw" title="${esc(HW_HOW[dv.how] || dv.how)}">⚙ hardware</span>`);
  const title = autoTitle(o.title) ? prettyType(o.ty) : o.title;
  if (note) S.closed.delete("Note");
  const st = marks && o.status ? ` <span class="st ${o.status}">${o.status}</span>` : "";
  const renamed = marks && o.old_title ? `<div class="ph-s"><span class="old">${esc(o.old_title)}</span>→ <span class="new">${esc(o.title)}</span></div>` : "";
  box.innerHTML = `<div class="ph"><div class="ph-top">${color && /^\d+,\d+,\d+$/.test(color) && color !== "0,0,0" ? `<span class="swatch" style="background:rgb(${esc(color)})"></span>` : ""}<span class="ph-t">${esc(title)}</span>${st}</div>
    <div class="ph-s">${esc(prettyType(o.ty))}</div>${renamed}${facts.length ? `<div class="ph-facts">${facts.join("")}</div>` : ""}</div>` +
    sections.map((s) => {
      const open = !S.closed.has(s.name) || (s.name === "Advanced" && rawChanged);
      return `<section><button type="button" class="ps-h" aria-expanded="${open}" data-sec="${esc(s.name)}">${esc(s.name)}${s.count ? `<span class="count">${s.count}</span>` : ""}</button><div class="ps-b" data-body="${esc(s.name)}"${open ? "" : " hidden"}>${s.rows.join("")}</div></section>`;
    }).join("");
}
$("props").addEventListener("keydown", whenReady((e) => {
  if (e.target.id === "ad-slug" && e.key === "Enter") { e.preventDefault(); adoptSave(S.sel); return; }
  if (e.target.id === "ad-slug" && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); adoptClose(); return; }
  if (e.target.id === "rn-slug" && e.key === "Enter") { e.preventDefault(); renameSave(S.sel); return; }
  if (e.target.id === "rn-slug" && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); renameClose(); return; }
  if (e.target.id === "ed-mk" && e.key === "Enter") { e.preventDefault(); constMake(); return; }
  if (e.target.id === "ed-mk" && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); constMkClose(); return; }
  const xf = e.target.closest("[data-ex-field]");
  if (xf && e.key === "Enter") { e.preventDefault(); exAct(xf.dataset.exEnter, xf.dataset.id); return; }
  const f = e.target.closest("[data-ed-field]");
  // In a program (#79) Enter is a line break; Ctrl+Enter (⌘ on a Mac) saves.
  if (e.target.id === "dflt-v" && e.key === "Enter") { e.preventDefault(); dfltSave(); return; }
  if (e.target.id === "dflt-v" && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); dfltClose(); return; }
  if (f && e.key === "Enter" && (f.tagName !== "TEXTAREA" || e.ctrlKey || e.metaKey)) { e.preventDefault(); editSave(); return; }
  if (f && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); editClose(); return; }
  const i = e.target.closest(".op input");
  if (i && e.key === "Enter") { e.preventDefault(); operate(i.dataset.opUuid, i.value); }
}));
$("props").addEventListener("toggle", whenReady((e) => { if (e.target.classList?.contains("ex-new")) { exSync(); S.ex.draft["new:open"] = e.target.open; } }), true);
$("props").addEventListener("input", whenReady((e) => {
  if (e.target.dataset.exField) { exSync(); S.ex.draft[e.target.dataset.exField] = e.target.value; return; }
  if (e.target.id === "ad-slug") { AD.slug = e.target.value; return; }
  if (e.target.dataset.dfltField) { DF.value = e.target.value; DF.err = ""; return; }
  if (e.target.id === "rn-slug") { RN.slug = e.target.value; return; }
  if (e.target.dataset.edScope && S.edit.uuid === S.sel) { (S.edit.scope ||= {})[e.target.dataset.edScope] = e.target.value; return; }
  const f = e.target.closest("[data-ed-field]");
  if (!f || S.edit.uuid !== S.sel) return;
  S.edit.draft[f.dataset.edField] = f.value;
  delete S.edit.bad[f.dataset.edField];
}));
$("props").addEventListener("click", whenReady((e) => {
  const xb = e.target.closest("[data-ex]");
  if (xb) { exAct(xb.dataset.ex, xb.dataset.id); return; }
  const db = e.target.closest("[data-dflt]");
  if (db) {
    const what = db.dataset.dflt;
    if (what === "open") dfltOpen(db.dataset.key);
    else if (what === "save") dfltSave();
    else if (what === "cancel") dfltClose();
    else if (what === "revert") dfltRevert(db.dataset.key);
    return;
  }
  const eb = e.target.closest("[data-ed]");
  if (eb) {
    const what = eb.dataset.ed;
    if (what === "open") editOpen(S.sel);
    else if (what === "save") editSave();
    else if (what === "cancel") editClose();
    else if (what === "revert") editRevert(eb.dataset.key);
    else if (what === "remove") editRemove(S.sel);
    else if (what === "delete") editDelete(S.sel);
    else if (what === "rename-open") renameOpen(S.sel);
    else if (what === "rename-cancel") renameClose();
    else if (what === "rename") renameSave(S.sel);
    else if (what === "unmove") mvRevert(S.sel);
    else if (what === "adopt-open") adoptOpen(S.sel);
    else if (what === "adopt") adoptSave(S.sel);
    else if (what === "adopt-cancel") adoptClose();
    else if (what === "unadopt") adoptHandBack(S.sel);
    else if (what === "newest") editGoNewest();
    else if (what === "mk-open") constMkOpen(eb.dataset.field);
    else if (what === "mk-cancel") constMkClose();
    else if (what === "mk-save") constMake();
    return;
  }
  const ob = e.target.closest("[data-op]");
  if (ob) { operate(ob.dataset.opUuid, ob.dataset.op === "set" ? ob.closest(".op").querySelector("input").value : ob.dataset.op); return; }
  const h = e.target.closest(".ps-h");
  if (h) {
    const open = h.getAttribute("aria-expanded") !== "true";
    h.setAttribute("aria-expanded", open);
    h.nextElementSibling.hidden = !open;
    open ? S.closed.delete(h.dataset.sec) : S.closed.add(h.dataset.sec);
    return;
  }
  if (e.target.closest(".app-load")) { loadAppFromMs(); return; }
  const c = e.target.closest(".copy");
  if (c) { const label = c.dataset.label ??= c.textContent; copyText(c.dataset.copy).then(() => { c.textContent = "Copied"; setTimeout(() => (c.textContent = label), 1200); }); return; }
  const m = e.target.closest(".more");
  if (m) { m.previousElementSibling.classList.add("open"); m.remove(); return; }
  const a = e.target.closest("[data-go]");
  if (!a) return;
  if (a.dataset.page) gotoPage(a.dataset.page, { focus: a.dataset.go });
  select(a.dataset.go);
}));

/** How a change reconfigures a device (`device.how`, `hardware[].how`). */
const HW_HOW = {
  added: "Added: the Miniserver configures this device once it finds it on the bus",
  removed: "Removed: the Miniserver no longer configures this device",
  replaced: "Device replaced: another serial is another physical device",
  paired: "Paired: the config now names this device's serial, so the Miniserver configures the device with it",
  unpaired: "Unpaired: the config no longer names a serial, so no physical device gets this configuration",
  changed: "Reconfigures the device: the Miniserver sends it a new configuration, which the device stores",
};
/** Statistics on a Tree or Air input or output (#85). */
const STATS_TIP = "Statistics: the Miniserver records the value (StatsType=). on is the kind Lox Logic writes for this type; another kind (StatsType n) is one Loxone Config set.";
/** A Tree device created here and not paired yet (#85). */
const HW_UNPAIRED = "Created here, not paired: pair it with a Tree search in Loxone Config. Until then no device gets its configuration.";
/** The Tree device created here (#85) with UUID `uuid`, from GET /api/edits' blocks. */
function hwCreated(uuid) { return (S.edits?.blocks || []).find((b) => b.hardware && b.uuid && ciEq(b.uuid, uuid)) || null; }
/** Whether `uuid` hangs below a Tree device created here and not deployed: lox mints its UUIDs anew on every compile. */
function hwBelowNew(uuid) {
  const path = findPath(uuid);
  return (S.edits?.blocks || []).some((b) => b.hardware && b.uuid && !b.deployed && path.some((p) => ciEq(p, b.uuid)));
}
const HW_LABEL = { AC: "Audible acknowledgement", SSC: "Send interval", MinChange: "Threshold", MinTime: "Minimal time between sends", AverageTime: "Mean value time", TreeType: "Device type", SubType: "Device subtype", HwVersion: "Hardware version" };

// ── tree ───────────────────────────────────────────────────────────────
const ICON = { Pages: ["#4f7fc4", "▦"], Page: ["#4f7fc4", "P"], Refs: ["#8a908a", "⇄"], Place: ["#8b6cc4", "R"], Category: ["#b5891f", "C"],
  VirtualIn: ["#0f8fb0", "V"], VirtualOut: ["#0f8fb0", "V"], InputRef: ["#8a908a", "→"], OutputRef: ["#8a908a", "←"], Memory: ["#6b726b", "M"], User: ["#6b726b", "U"] };
function icon(n) {
  // Rooms and categories get the app's icon, once the structure file is read.
  const place = (n.ty === "Place" || n.ty === "Category") && (n.ty === "Place" ? S.app?.rooms : S.app?.cats)?.[n.uuid];
  if (place?.image) return `<span class="tn-ic app" aria-hidden="true">${appIcon(place.image)}</span>`;
  const [c, t] = ICON[n.ty] || (n.children.length ? ["#a7ada7", "▤"] : ["#4ea432", "•"]);
  return `<span class="tn-ic" style="background:${c}" aria-hidden="true">${t}</span>`;
}
const REF_TYPES = new Set(["InputRef", "OutputRef", "OutputRefLM"]);
const changeCount = new WeakMap();
function subtreeChanges(n) {
  if (changeCount.has(n)) return changeCount.get(n);
  let c = n.status ? 1 : 0;
  for (const k of n.children) c += subtreeChanges(k);
  changeCount.set(n, c);
  return c;
}
/** Pages list their blocks directly and fold their many input/output references into one group. */
function treeChildren(n) {
  if (n.ty !== "Page" || n.synthetic) return n.children;
  if (!n._kids) {
    const refs = n.children.filter((c) => REF_TYPES.has(c.ty));
    const rest = n.children.filter((c) => !REF_TYPES.has(c.ty));
    rest.sort((a, b) => (autoTitle(a.title) - autoTitle(b.title)) || a.title.localeCompare(b.title));
    n._kids = refs.length ? [...rest, { uuid: n.uuid + "::refs", title: "Inputs & outputs", ty: "Refs", status: refs.some((r) => r.status) ? "changed" : "", synthetic: true, children: refs }] : rest;
  }
  return n._kids;
}
function treeRoots() {
  const pages = [];
  const collect = (n) => { if (n.page) pages.push(n); else n.children.forEach(collect); };
  S.view.tree.forEach(collect);
  const nested = pages.some((pg) => !S.view.tree.includes(pg));
  return nested ? [{ uuid: "__pages", title: "Pages", ty: "Pages", status: "", synthetic: true, children: pages }, ...S.view.tree.filter((n) => !n.page)] : S.view.tree;
}
function renderTree() {
  const box = $("tree");
  const q = $("tree-filter").value.trim().toLowerCase();
  box.innerHTML = "";
  if (!S.view) return;
  const marks = S.marks && S.view.compared;
  const rows = [];
  const rowHtml = (n, depth, kids, open, extra) => {
    const cnt = marks && kids ? subtreeChanges(n) : 0;
    const badge = cnt && !n.status ? `<span class="badge changed" aria-label="${cnt} changes">${cnt}</span>` : "";
    const name = autoTitle(n.title) && n.ty !== "Page" ? prettyType(n.ty) : n.title;
    const ty = extra ?? (!kids && name !== prettyType(n.ty) ? prettyType(n.ty) : "");
    const tipText = calTip(S.view.objects[n.uuid]?.calendar);
    const year = MODES_FOLDERS.has(n.ty) ? '<button type="button" class="tn-year" data-modes tabindex="-1" title="Operating modes: a year calendar of when each mode is on">Year</button>' : "";
    // A folder or device of the object tree takes new virtual inputs, outputs and Modbus devices (#84).
    const pst = n.synthetic ? null : periphAddState(n);
    const add = pst ? `<button type="button" class="tn-year tn-add" data-add-under tabindex="-1"${pst.ok ? "" : " disabled"} title="${esc(pst.ok ? `Add to ${name} (it is compiled, not deployed)` : pst.why)}">Add</button>` : "";
    // A Tree device created here and not paired yet (#85).
    const hwNew = !n.synthetic && hwCreated(n.uuid);
    const unpaired = hwNew && !hwNew.paired ? `<span class="tn-tag" title="${esc(HW_UNPAIRED)}">unpaired</span>` : "";
    const num = n.ty === "Page" || n.ty === "Refs" ? `<span class="tn-n">${n.ty === "Page" ? treeChildren(n).reduce((a, c) => a + (c.synthetic ? 0 : 1), 0) : n.children.length}</span>` : "";
    return `<div class="tn-row ${marks ? n.status : ""}${n.uuid === S.sel ? " sel" : ""}${n.uuid === S.treeFocus ? " kfocus" : ""}" role="treeitem" aria-level="${depth + 1}"${kids ? ` aria-expanded="${open}"` : ""} aria-selected="${n.uuid === S.sel}" data-uuid="${esc(n.uuid)}"${tipText ? ` title="${esc(tipText)}"` : ""} style="padding-left:${4 + depth * 14}px">` +
      `<span class="tn-tw${open ? " open" : ""}" aria-hidden="true">${kids ? "▶" : ""}</span>${icon(n)}<span class="tn-t">${esc(name)}</span>${ty ? `<span class="tn-ty">${esc(ty)}</span>` : ""}${year}${add}${unpaired}${badge}${num}</div>`;
  };
  if (q) {
    const out = [];
    const walk = (n, path) => {
      if (out.length > 400) return;
      const name = autoTitle(n.title) ? prettyType(n.ty) : n.title;
      if (!n.synthetic && (n.title.toLowerCase().includes(q) || name.toLowerCase().includes(q) || n.ty.toLowerCase().includes(q))) out.push([n, path]);
      for (const c of n.children) walk(c, n.synthetic ? path : path ? path + " / " + n.title : n.title);
    };
    S.view.tree.forEach((n) => walk(n, ""));
    for (const [n, path] of out) rows.push(rowHtml(n, 0, false, false, path));
    box.innerHTML = rows.join("") || '<div class="empty">Nothing matches.</div>';
    return;
  }
  const build = (n, depth) => {
    const kids = treeChildren(n);
    const open = kids.length > 0 && S.open.has(n.uuid);
    rows.push(rowHtml(n, depth, kids.length > 0, open));
    if (open) for (const c of kids) build(c, depth + 1);
  };
  for (const n of treeRoots()) build(n, 0);
  box.innerHTML = rows.join("");
}
function findPath(uuid) {
  const path = [];
  const walk = (n) => {
    if (n.uuid === uuid) return true;
    for (const c of treeChildren(n)) if (walk(c)) { path.push(n.uuid); return true; }
    return false;
  };
  for (const n of S.view ? treeRoots() : []) if (walk(n)) break;
  return path;
}
function revealInTree(uuid) {
  if (uuid) for (const p of findPath(uuid)) S.open.add(p);
  if (uuid) S.treeFocus = uuid;
  renderTree();
  const row = uuid && $("tree").querySelector(`.tn-row[data-uuid="${CSS.escape(uuid)}"]`);
  if (row) row.scrollIntoView({ block: "nearest" });
}
function nodeById(uuid) {
  let found = null;
  const walk = (n) => { if (found) return; if (n.uuid === uuid) { found = n; return; } treeChildren(n).forEach(walk); };
  if (S.view) treeRoots().forEach(walk);
  return found;
}
/** The Calendar and Operating modes folders have a button that opens the year view of the modes. */
const MODES_FOLDERS = new Set(["CalendarCaption", "ModeCaption"]);
function activateNode(uuid) {
  const n = nodeById(uuid);
  if (!n) return;
  if (n.synthetic || (n.children.length && n.ty !== "Page" && !S.view.objects[uuid])) { toggleNode(uuid); return; }
  if (S.pageOf.has(uuid)) { gotoPage(uuid); S.open.add(uuid); select(null, { noReveal: true }); S.treeFocus = uuid; renderTree(); return; }
  const pg = S.objPage.get(uuid);
  if (pg) gotoPage(pg, { focus: uuid });
  select(uuid);
}
function toggleNode(uuid) {
  S.open.has(uuid) ? S.open.delete(uuid) : S.open.add(uuid);
  S.treeFocus = uuid;
  renderTree();
}
$("tree").addEventListener("click", whenReady((e) => {
  const row = e.target.closest(".tn-row");
  if (!row) return;
  const uuid = row.dataset.uuid;
  S.treeFocus = uuid;
  if (e.target.closest("[data-modes]")) { modesOpen(); return; }
  if (e.target.closest("[data-add-under]")) { abOpenUnder(uuid); return; }
  if (e.target.closest(".tn-tw") && row.hasAttribute("aria-expanded")) { toggleNode(uuid); return; }
  activateNode(uuid);
}));
$("tree").addEventListener("dblclick", whenReady((e) => {
  if (e.target.closest("[data-modes], [data-add-under]")) return;
  const row = e.target.closest(".tn-row");
  if (row && row.hasAttribute("aria-expanded")) toggleNode(row.dataset.uuid);
}));
$("tree").addEventListener("keydown", whenReady((e) => {
  const rows = [...$("tree").querySelectorAll(".tn-row")];
  if (!rows.length) return;
  let i = rows.findIndex((r) => r.dataset.uuid === S.treeFocus);
  const cur = rows[i];
  const go = (j) => {
    j = Math.max(0, Math.min(rows.length - 1, j));
    S.treeFocus = rows[j].dataset.uuid;
    rows.forEach((r) => r.classList.remove("kfocus"));
    rows[j].classList.add("kfocus");
    rows[j].scrollIntoView({ block: "nearest" });
  };
  switch (e.key) {
    case "ArrowDown": go(i + 1); break;
    case "ArrowUp": go(i < 0 ? 0 : i - 1); break;
    case "Home": go(0); break;
    case "End": go(rows.length - 1); break;
    case "ArrowRight":
      if (cur?.getAttribute("aria-expanded") === "false") toggleNode(cur.dataset.uuid); else go(i + 1);
      break;
    case "ArrowLeft":
      if (cur?.getAttribute("aria-expanded") === "true") toggleNode(cur.dataset.uuid);
      else if (cur) { const lvl = +cur.getAttribute("aria-level"); for (let j = i - 1; j >= 0; j--) if (+rows[j].getAttribute("aria-level") < lvl) { go(j); break; } }
      break;
    case "Enter": case " ": if (cur) activateNode(cur.dataset.uuid); break;
    default: return;
  }
  e.preventDefault();
}));
$("tree").addEventListener("focus", whenReady(() => {
  if (!S.treeFocus) { const r = $("tree").querySelector(".tn-row"); if (r) { S.treeFocus = r.dataset.uuid; r.classList.add("kfocus"); } }
}));
$("tree-filter").addEventListener("input", whenReady(renderTree));

// ── changes ────────────────────────────────────────────────────────────
const KIND = { added: ["+", "added"], removed: ["−", "removed"], renamed: ["~", "renamed"], param: ["~", "parameters"], attr: ["~", "settings"], rights: ["~", "rights"], "wire+": ["+", "wires added"], "wire-": ["−", "wires removed"], moved: ["↔", "moved"] };
/** The `let` or `defaults` a parameter change comes from (#81), on the newest config: `let x`, `defaults T`. */
function chgConst(c) {
  if (c.kind !== "param" || !c.obj || !c.port) return null;
  const u = constUse(c.obj, c.port), l = constLet(u);
  return u ? (l ? `let ${l}` : `defaults ${u.name}`) : null;
}
/** The expression a change is to (#82), on the newest config: `expr <id>`, for a block of its chain now or in the
 *  deployed revision, and for the wire into its sink. */
function chgExpr(c) {
  if (!c.obj) return null;
  const e = exprList().find((x) => x.blocks.some((b) => ciEq(b, c.obj)) || x.deployed?.blocks.some((b) => ciEq(b, c.obj)));
  return e ? `expr ${e.id}` : null;
}
/** What groups a change: an expression (even one change: it reads as the expression), else a `let` or `defaults`. */
function chgKey(c) { return chgTpl(c) || chgExpr(c) || chgConst(c); }
/** Whether the changes of key `k` (`n` of them) go under a heading: an expression's and a template's or
 *  instance's even alone (#82, #83). */
function chgGrouped(k, n) { return !!k && (n > 1 || /^(expr|tpl|inst) /.test(k)); }
/** The changes listed: those the filters let through, and the changes one expression causes, and the parameter
 *  changes one `let` or `defaults` causes (two or more), together where the first of them is. Next and previous go
 *  in this order. */
function shownChanges() {
  const list = (S.view?.changes || []).filter((c) => !S.chgOff.has(c.kind) && (!S.chgHw || c.hw));
  const key = new Map(), n = new Map();
  for (const c of list) { const k = chgKey(c); if (k) { key.set(c, k); n.set(k, (n.get(k) || 0) + 1); } }
  if (![...n].some(([k, x]) => chgGrouped(k, x))) return list;
  const by = new Map();
  for (const c of list) { const k = key.get(c); if (chgGrouped(k, n.get(k))) (by.get(k) || by.set(k, []).get(k)).push(c); }
  const out = [], done = new Set();
  for (const c of list) {
    const k = key.get(c);
    if (!chgGrouped(k, n.get(k))) out.push(c);
    else if (!done.has(k)) { done.add(k); out.push(...by.get(k)); }
  }
  return out;
}
/** The heading over the changes one expression causes: what it feeds, and its text before → now. Before is the
 *  deployed revision's, shown when that is what is compared. */
function chgExprHead(k, count) {
  const e = exprList().find((x) => `expr ${x.id}` === k);
  if (!e) return "";
  const vsDeployed = S.base && S.base === deployedRev()?.id;
  const old = vsDeployed ? exprOld(e) : null;
  const what = e.kind === "signal" ? `signal ${e.sink}` : `expression on ${e.sink}`;
  const text = old ? `<span class="old">${esc(old)}</span> → <span class="new">${esc(e.now)}</span>`
    : vsDeployed && e.pending && !e.deployed ? `<span class="new">${esc(e.text)}</span> (new)`
    : vsDeployed && e.pending ? `${esc(e.text)} (changed)` : esc(e.text);
  const at = e.file ? constAt(e.file, e.line) : "";
  return `<div class="chg-grp chg-expr" role="listitem" data-expr="${esc(e.id)}"><span class="mono">${esc(what)}: ${text}</span> · ${plural(count, "change")}${at ? `<span class="p">${esc(at)}</span>` : ""}</div>`;
}
/** The heading over the changes one `let` or `defaults` causes. */
function chgConstHead(k, count) {
  const [kind, name] = k.split(" ");
  const l = kind === "let" && constLets().find((x) => x.name === name);
  const d = kind === "defaults" && (S.edits?.constants?.defaults || []).find((x) => x.type === name);
  const at = l ? constAt(l.file, l.line) : d ? constAt(d.file, d.line) : "";
  return `<div class="chg-grp" role="listitem"><span class="mono">${esc(l ? `let ${l.name} = ${l.value}` : k)}</span> · ${plural(count, "change")}${at ? `<span class="p">${esc(at)}</span>` : ""}</div>`;
}
function renderChanges() {
  const v = S.view, box = $("changes"), filt = $("chg-filter");
  box.innerHTML = ""; filt.innerHTML = "";
  renderRights();
  const adopted = adoptedUncommitted();
  const adoptionRows = () => {
    for (const b of adopted) {
      const d = document.createElement("div"); d.className = "chg adoption";
      const go = document.createElement("button"); go.type = "button";
      go.textContent = `Adopt into vislox: ${v.objects[b.uuid]?.title || b.slug}`;
      go.onclick = () => select(b.uuid); d.appendChild(go);
      const revert = document.createElement("button"); revert.type = "button"; revert.textContent = "Revert";
      revert.dataset.adoptRevert = b.uuid;
      revert.onclick = () => { select(b.uuid); adoptHandBack(b.uuid); }; d.appendChild(revert); box.appendChild(d);
    }
    adoptRevertSync();
  };
  const compact = (!v.compared || !v.changes.length) && !adopted.length;
  $("right").classList.toggle("compact", compact);
  $("chg-nav").classList.toggle("hidden", !v.compared || !v.changes.length);
  $("chg-dim").classList.toggle("hidden", !v.compared || !v.changes.length);
  if (!v.compared) {
    $("chg-count").textContent = "";
    const only = allRevs().length < 2;
    const base = revById(S.rev)?.base;
    box.innerHTML = only
      ? '<div class="empty"><b>Only one revision so far.</b><br>When the config changes — after <code>lox config pull</code>, a compile, or an edit on disk — the new revision appears in History and its changes are listed here.</div>'
      : `<div class="empty">Plain view: nothing is being compared.${base ? ' <button type="button" class="lnk" id="cmp-prev">Compare with the revision before</button>' : ""}</div>`;
    $("cmp-prev")?.addEventListener("click", () => { S.base = base; renderHistory(); renderSelects(); loadView(); writeHash(true); });
    adoptionRows();
    return;
  }
  $("chg-count").textContent = v.changes.length + adopted.length;
  const counts = {};
  for (const c of v.changes) counts[c.kind] = (counts[c.kind] || 0) + 1;
  for (const k of Object.keys(KIND)) {
    if (!counts[k]) continue;
    const p = document.createElement("button");
    p.type = "button";
    p.className = "pill";
    p.setAttribute("aria-pressed", S.chgOff.has(k) ? "false" : "true");
    p.textContent = `${KIND[k][1]} ${counts[k]}`;
    p.onclick = () => { S.chgOff.has(k) ? S.chgOff.delete(k) : S.chgOff.add(k); S.chgIdx = -1; renderChanges(); };
    filt.appendChild(p);
  }
  // Changes that reconfigure a device (an extension, a Tree or Air device): pressed, only those show.
  const hwCount = v.changes.filter((c) => c.hw).length;
  if (!hwCount) S.chgHw = false;
  if (hwCount) {
    const p = document.createElement("button");
    p.type = "button";
    p.className = "pill hw";
    p.setAttribute("aria-pressed", String(S.chgHw));
    p.textContent = `hardware ${hwCount}`;
    p.title = `Show only the changes that reconfigure a device (${plural(v.stats.devices, "device")}): the Miniserver sends it a new configuration, which it stores itself`;
    p.onclick = () => { S.chgHw = !S.chgHw; S.chgIdx = -1; renderChanges(); };
    filt.appendChild(p);
  }
  if (!v.changes.length) { box.innerHTML = adopted.length ? '' : '<div class="empty">No differences between these revisions.</div>'; adoptionRows(); return; }
  const list = shownChanges().slice(0, 2000);
  const groups = new Map();
  for (const c of list) { const k = chgKey(c); if (k) groups.set(k, (groups.get(k) || 0) + 1); }
  let inGroup = null;
  list.forEach((c, i) => {
    const k = chgKey(c), grouped = chgGrouped(k, groups.get(k));
    if (grouped && k !== inGroup) box.insertAdjacentHTML("beforeend", k.startsWith("expr ") ? chgExprHead(k, groups.get(k)) : /^(tpl|inst) /.test(k) ? chgTplHead(k, groups.get(k), list) : chgConstHead(k, groups.get(k)));
    inGroup = grouped ? k : null;
    const d = document.createElement("button");
    d.type = "button";
    d.className = "chg" + (i === S.chgIdx ? " cur" : "") + (grouped ? " in-grp" : "");
    d.setAttribute("role", "listitem");
    const where = c.page ? S.pageOf.get(c.page)?.title : c.obj ? v.objects[c.obj]?.path : "";
    d.innerHTML = `<span class="k ${esc(c.kind)}${c.how ? " " + esc(c.how) : ""}" aria-label="${esc(KIND[c.kind]?.[1] || c.kind)}">${c.how === "added" ? "+" : c.how === "removed" ? "−" : KIND[c.kind]?.[0] || "~"}</span><span class="x"><span class="t">${esc(c.text)}${k && !grouped ? ` <span class="const-of">(${esc(k)})</span>` : ""}</span>${where ? `<span class="p">${esc(where)}</span>` : ""}</span>${c.hw ? '<span class="hw-tag" title="Reconfigures a device">hardware</span>' : ""}`;
    d.onclick = () => gotoChange(i);
    box.appendChild(d);
  });
  adoptionRows();
  updateChangePos();
}
function updateChangePos() {
  const n = shownChanges().length;
  $("chg-pos").textContent = S.chgIdx >= 0 ? `${S.chgIdx + 1} / ${n}` : `${n}`;
}
function gotoChange(i) {
  const list = shownChanges();
  if (!list.length) return;
  S.chgIdx = (i + list.length) % list.length;
  const c = list[S.chgIdx];
  $("changes").querySelectorAll(".chg").forEach((d, j) => d.classList.toggle("cur", j === S.chgIdx));
  $("changes").querySelectorAll(".chg")[S.chgIdx]?.scrollIntoView({ block: "nearest" });
  updateChangePos();
  if (c.page) gotoPage(c.page, { focus: c.obj });
  if (c.obj) select(c.obj, { noReveal: false });
  markCurrentChange();
}
function markCurrentChange() {
  vpG.querySelectorAll(".cur-change").forEach((n) => n.classList.remove("cur-change"));
  const c = shownChanges()[S.chgIdx];
  if (c?.obj) vpG.querySelectorAll(`[data-uuid="${CSS.escape(c.obj)}"]`).forEach((n) => n.classList.add("cur-change"));
}
$("chg-prev").onclick = whenReady(() => gotoChange(S.chgIdx < 0 ? -1 : S.chgIdx - 1));
$("chg-next").onclick = whenReady(() => gotoChange(S.chgIdx + 1));

// ── command palette ────────────────────────────────────────────────────
// The palette also picks a block for an editor (`pal.pick`, #75: a central block's new member): only blocks of
// the types it takes, none it excludes, and choosing one hands it over instead of going there.
const pal = { items: [], idx: 0, pick: null };
const PAL_PLACEHOLDER = "Go to a page or block…";
function openPicker(pick) {
  if (!S.view) return;
  pal.pick = pick;
  $("pal-input").placeholder = pick.placeholder;
  $("palette").setAttribute("aria-label", pick.title);
  openPalette("");
}
function openPalette(q) {
  if (!S.view) return;
  $("palette").classList.remove("hidden");
  const inp = $("pal-input");
  inp.value = q || "";
  renderPalette();
  inp.focus();
}
function closePalette() {
  $("palette").classList.add("hidden");
  if (!pal.pick) return;
  const back = pal.pick.back;
  pal.pick = null;
  $("pal-input").placeholder = PAL_PLACEHOLDER;
  $("palette").setAttribute("aria-label", "Go to page or block");
  back?.();
}
/** Subsequence match: every query character in order; earlier and contiguous hits score higher. */
function fuzzy(q, s) {
  const t = s.toLowerCase();
  const at = t.indexOf(q);
  if (at >= 0) return { score: 1000 - at - (t.length - q.length) * 0.1, hits: [[at, at + q.length]] };
  let i = 0, score = 0, last = -2;
  const hits = [];
  for (let j = 0; j < t.length && i < q.length; j++) {
    if (t[j] === q[i]) {
      score += j === last + 1 ? 6 : 1;
      if (hits.length && hits[hits.length - 1][1] === j) hits[hits.length - 1][1]++; else hits.push([j, j + 1]);
      last = j; i++;
    }
  }
  return i === q.length ? { score, hits } : null;
}
function mark(s, hits) {
  let out = "", p = 0;
  for (const [a, b] of hits || []) { out += esc(s.slice(p, a)) + "<mark>" + esc(s.slice(a, b)) + "</mark>"; p = b; }
  return out + esc(s.slice(p));
}
function renderPalette() {
  const q = $("pal-input").value.trim().toLowerCase();
  const v = S.view, marks = S.marks && v.compared;
  const pages = v.pages.map((p) => ({ kind: "page", title: p.title || "(untitled)", meta: `${plural(p.objects.filter((o) => o.shape !== "tag").length, "block")}${marks && p.changes ? ` · ${p.changes} changed` : ""}`, page: p.uuid }));
  const blocks = [];
  for (const p of v.pages) for (const o of p.objects) {
    if (o.moved_to || o.shape === "tag") continue;
    // A block the module manages is found by its slug too (#64).
    blocks.push({ kind: "block", title: displayTitle(o), meta: `${prettyType(o.ty)} · ${p.title}`, page: p.uuid, uuid: o.uuid, slug: managedSrc(o.uuid)?.slug });
  }
  // Views match a whole word they start with (“mod”, “calendar”), not any subsequence, and come after
  // pages and blocks, so that a short query still opens the best page or block.
  if (pal.pick) {
    const pk = pal.pick, seen = new Set();
    // The server's rule: a block the module manages is never offered (vislox's files name only their own blocks
    // and Loxone Config's).
    const managed = new Set((S.edits?.managed || []).map((u) => u.toLowerCase()));
    const cands = blocks.filter((b) => {
      const o = v.objects[b.uuid], u = b.uuid.toLowerCase();
      if (!o || seen.has(u) || !pk.types.includes(o.ty) || pk.exclude.has(u) || managed.has(u)) return false;
      seen.add(u);
      return true;
    });
    const scored = q ? cands.map((it) => { const m = fuzzy(q, it.title); return m ? { ...it, score: m.score, hits: m.hits } : null; }).filter(Boolean).sort((a, b) => b.score - a.score)
      : cands.sort((a, b) => a.title.localeCompare(b.title));
    pal.items = scored.length ? [{ sec: pk.title }, ...scored] : [];
    pal.idx = pal.items.findIndex((x) => !x.sec);
    $("pal-list").innerHTML = pal.items.length ? pal.items.map((it, i) => it.sec
      ? `<li class="sec" role="presentation">${esc(it.sec)}</li>`
      : `<li role="option" id="pal-${i}" data-i="${i}" aria-selected="${i === pal.idx}"><span class="pk">add</span><span class="pt">${mark(it.title, it.hits)}</span><span class="pm">${esc(it.meta)}</span></li>`).join("")
      : `<li class="sec" role="presentation">${esc(pk.none)}</li>`;
    $("pal-input").setAttribute("aria-activedescendant", pal.idx >= 0 ? `pal-${pal.idx}` : "");
    return;
  }
  const views = mdEntries().length || v.modes.length ? [{ kind: "view", title: "Operating modes", meta: "year calendar of the calendar entries", names: ["operating modes", "modes", "calendar"] }] : [];
  let items;
  if (!q) items = [{ sec: "Pages" }, ...pages, ...(views.length ? [{ sec: "Views" }, ...views] : [])];
  else {
    const score = (it) => {
      const m = fuzzy(q, it.title), s = it.slug ? fuzzy(q, it.slug) : null;
      if (s && (!m || s.score > m.score)) return { ...it, score: s.score, hits: [], slugHits: s.hits };
      return m ? { ...it, score: m.score + (it.kind === "page" ? 50 : 0), hits: m.hits } : null;
    };
    const ps = pages.map(score).filter(Boolean).sort((a, b) => b.score - a.score).slice(0, 12);
    const bs = blocks.map(score).filter(Boolean).sort((a, b) => b.score - a.score).slice(0, 40);
    const vs = views.filter((it) => it.names.some((w) => w.startsWith(q))).map((it) => {
      const at = it.title.toLowerCase().indexOf(q);
      return { ...it, hits: at >= 0 ? [[at, at + q.length]] : [] };
    });
    items = [...(ps.length ? [{ sec: "Pages" }, ...ps] : []), ...(bs.length ? [{ sec: "Blocks" }, ...bs] : []), ...(vs.length ? [{ sec: "Views" }, ...vs] : [])];
  }
  pal.items = items;
  pal.idx = items.findIndex((x) => !x.sec);
  const ul = $("pal-list");
  ul.innerHTML = items.length ? items.map((it, i) => it.sec
    ? `<li class="sec" role="presentation">${esc(it.sec)}</li>`
    : `<li role="option" id="pal-${i}" data-i="${i}" aria-selected="${i === pal.idx}"><span class="pk">${it.kind}</span><span class="pt">${mark(it.title, it.hits)}</span><span class="pm">${esc(it.meta)}${it.slug ? ` · <span class="mono pal-slug">${mark(it.slug, it.slugHits)}</span>` : ""}</span></li>`).join("")
    : '<li class="sec" role="presentation">Nothing matches.</li>';
  $("pal-input").setAttribute("aria-activedescendant", pal.idx >= 0 ? `pal-${pal.idx}` : "");
}
function palMove(d) {
  const opts = pal.items.map((x, i) => (x.sec ? -1 : i)).filter((i) => i >= 0);
  if (!opts.length) return;
  const k = Math.max(0, Math.min(opts.length - 1, opts.indexOf(pal.idx) + d));
  pal.idx = opts[k];
  $("pal-list").querySelectorAll("[role=option]").forEach((li) => li.setAttribute("aria-selected", +li.dataset.i === pal.idx));
  $(`pal-${pal.idx}`)?.scrollIntoView({ block: "nearest" });
  $("pal-input").setAttribute("aria-activedescendant", `pal-${pal.idx}`);
}
function palOpen(i) {
  const it = pal.items[i];
  if (!it || it.sec) return;
  if (pal.pick) {
    const take = pal.pick.onPick;
    pal.pick.back = null;
    closePalette();
    take(it.uuid);
    return;
  }
  closePalette();
  if (it.kind === "view") { modesOpen(); return; }
  if (it.kind === "page") { gotoPage(it.page); select(null, { noReveal: true }); }
  else { gotoPage(it.page, { focus: it.uuid }); select(it.uuid); }
  svg.focus({ preventScroll: true });
}
$("pal-input").addEventListener("input", whenReady(renderPalette));
$("pal-input").addEventListener("keydown", whenReady((e) => {
  if (e.key === "ArrowDown") { palMove(1); e.preventDefault(); }
  else if (e.key === "ArrowUp") { palMove(-1); e.preventDefault(); }
  else if (e.key === "Enter") { palOpen(pal.idx); e.preventDefault(); }
  else if (e.key === "Escape") { closePalette(); e.preventDefault(); }
}));
$("pal-list").addEventListener("click", whenReady((e) => { const li = e.target.closest("[data-i]"); if (li) palOpen(+li.dataset.i); }));
$("palette").addEventListener("mousedown", whenReady((e) => { if (e.target.id === "palette") closePalette(); }));
$("btn-search").onclick = whenReady(() => openPalette(""));

// ── help ───────────────────────────────────────────────────────────────
function openHelp() { $("help").classList.remove("hidden"); $("help-close").focus(); }
function closeHelp() { $("help").classList.add("hidden"); }
$("btn-help").onclick = whenReady(openHelp);
$("help-close").onclick = whenReady(closeHelp);
$("help").addEventListener("mousedown", whenReady((e) => { if (e.target.id === "help") closeHelp(); }));

// ── controls ───────────────────────────────────────────────────────────
$("sel-rev").onchange = whenReady((e) => {
  S.rev = e.target.value;
  S.base = revById(S.rev)?.base || null;
  S.follow = false; $("chk-follow").checked = false;
  renderHistory(); renderSelects(); loadView(); writeHash(true);
});
$("sel-base").onchange = whenReady((e) => { S.base = e.target.value || null; S.follow = false; $("chk-follow").checked = false; renderHistory(); loadView(); writeHash(true); });
$("chk-changes").onchange = whenReady((e) => {
  S.marks = e.target.checked;
  if (!S.view) return;
  renderLegend(); renderTabs(); renderTree(); renderPage(); renderProps(); applyFocus(); renderModes(); renderTests(); renderLint();
});
$("chk-live").onchange = whenReady((e) => setLive(e.target.checked));
$("chk-live-dim").checked = S.live.dim;
$("chk-live-dim").onchange = whenReady((e) => { S.live.dim = e.target.checked; setPref("liveDim", S.live.dim); renderLiveSwitch(); });
$("chk-live-query").checked = S.live.query;
$("chk-live-query").onchange = whenReady((e) => { S.live.query = e.target.checked; setPref("liveQuery", S.live.query); renderLiveSwitch(); });
$("chk-dim").onchange = whenReady((e) => { S.dim = e.target.checked; renderPage(); applyFocus(); });
$("chk-only-managed").onchange = whenReady((e) => { S.onlyManaged = e.target.checked; renderPage(); applyFocus(); });
$("chk-follow").onchange = whenReady((e) => { if (e.target.checked) followLatest(); else S.follow = false; writeHash(false); });

window.addEventListener("keydown", whenReady((e) => {
  if (e.defaultPrevented) return;
  if (!$("palette").classList.contains("hidden") || !$("help").classList.contains("hidden")) {
    if (e.key === "Escape") { closePalette(); closeHelp(); }
    return;
  }
  if (AB.open) { if (e.key === "Escape") abClose(); return; }
  if (e.key === "Escape" && !$("undo-pop").classList.contains("hidden")) { undoListToggle(false); $("btn-undo-list").focus(); return; }
  if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
    e.preventDefault();
    // With a central block's members open in the editor, Ctrl+K picks a member to add (#75).
    if ($("props").querySelector('[data-me="add"]:not([disabled])')) meAdd(); else openPalette("");
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key === "f") { e.preventDefault(); $("tree-filter").focus(); $("tree-filter").select(); return; }
  const tag = e.target.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") { if (e.key === "Escape") e.target.blur(); return; }
  // Undo and redo the edits of this tab; in a text field they are the field's own.
  if ((e.ctrlKey || e.metaKey) && !e.altKey && (e.key === "z" || e.key === "Z" || (!IS_MAC && e.ctrlKey && (e.key === "y" || e.key === "Y")))) {
    if (!S.hist?.actions?.shown) return;
    e.preventDefault();
    undoStep(e.shiftKey || e.key === "y" || e.key === "Y");
    return;
  }
  if (e.altKey && !e.ctrlKey && !e.metaKey && MV_KEYS[e.key] && mvNudge(MV_KEYS[e.key], e.shiftKey ? 10 : 1)) { e.preventDefault(); return; }
  if (!e.ctrlKey && !e.metaKey && !e.altKey && (S.wsel || S.wmsg)) {
    if (e.key === "Escape" && wcardClose()) { e.preventDefault(); return; }
    if ((e.key === "Delete" || e.key === "Backspace") && wrDelete()) { e.preventDefault(); return; }
  }
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const inTree = e.target.id === "tree", inTabs = e.target.closest?.("#tab-list");
  // The Operating modes view handles its year and date navigation; the hidden page is not panned or zoomed.
  if (!inTree && !inTabs && modesKey(e)) return;
  const step = e.shiftKey ? 240 : 60;
  switch (e.key) {
    case "+": case "=": zoomCenter(1.25); break;
    case "-": case "_": zoomCenter(0.8); break;
    case "0": fit(); break;
    case "1": { const v = S.vp[S.page]; if (v) zoomCenter(K100 / v.k); break; }
    case "PageDown": stepPage(1); break;
    case "PageUp": stepPage(-1); break;
    case "]": case "n": gotoChange(S.chgIdx + 1); break;
    case "[": case "p": gotoChange(S.chgIdx < 0 ? -1 : S.chgIdx - 1); break;
    case "?": openHelp(); break;
    case "l": case "L": if (S.hist?.live?.available) setLive(!S.live.on); break;
    case "/": openPalette(""); break;
    case "Escape": select(null, { noReveal: true }); break;
    case "ArrowLeft": if (inTree || inTabs) return; pan(step, 0); break;
    case "ArrowRight": if (inTree || inTabs) return; pan(-step, 0); break;
    case "ArrowUp": if (inTree) return; pan(0, step); break;
    case "ArrowDown": if (inTree) return; pan(0, -step); break;
    default: return;
  }
  e.preventDefault();
}));

for (const [sid, a, b] of [["split-left", "tree-pane", "props-pane"], ["split-right", "hist-pane", "chg-pane"]]) {
  $(sid).addEventListener("mousedown", whenReady((e) => {
    e.preventDefault();
    const A = $(a), B = $(b), total = A.offsetHeight + B.offsetHeight, y0 = e.clientY, h0 = A.offsetHeight;
    const mv = (ev) => {
      const h = Math.min(total - 72, Math.max(72, h0 + ev.clientY - y0));
      A.style.flex = `0 0 ${h}px`; B.style.flex = `1 1 ${total - h}px`;
    };
    const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", mv);
    window.addEventListener("mouseup", up);
  }));
}

// ── pull from the Miniserver ───────────────────────────────────────────
// `lox config pull` on the server: the new backup arrives as a git commit.
let pulling = false;
function renderPull() {
  const info = S.hist?.pull || { available: false };
  const b = $("btn-pull");
  // A config repository without a usable lox setup still gets the button, disabled, to say why.
  b.classList.toggle("hidden", !info.shown);
  b.disabled = pulling || !info.available;
  b.classList.toggle("busy", pulling);
  b.querySelector(".lbl").textContent = pulling ? "Pulling…" : "Pull from Miniserver";
  b.title = !info.available ? `Cannot pull: ${info.reason}` : "Download the Miniserver's current config and commit it (lox config pull)" + msWarn("pull");
  renderStale();
}
function pullMsg(text, cls) {
  const m = $("pull-msg");
  m.textContent = text || "";
  m.className = "pull-msg" + (cls ? " " + cls : "") + (text ? "" : " hidden");
  clearTimeout(m._t);
  if (text && cls !== "error") m._t = setTimeout(() => pullMsg(""), 8000);
}
async function pull() {
  if (pulling) return;
  pulling = true; renderPull(); pullMsg("");
  try {
    const r = await fetch("/api/pull", { method: "POST", headers: { "X-Vislox": "pull" } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    // Which file that was, and whether the Miniserver runs it, shows above (it takes a moment).
    if (S.hist?.running) loadRunning(true);
    if (j.committed) {
      pullMsg("Pulled: the Miniserver's config is committed as a new backup.");
      await loadHistory(true);
    } else pullMsg("Up to date: the last backup holds the Miniserver's newest config.", "info");
  } catch (e) {
    pullMsg(e.message, "error");
  } finally {
    pulling = false; renderPull(); loadMs(true);
  }
}
$("btn-pull").onclick = whenReady(pull);

// ── which config the Miniserver runs ───────────────────────────────────
// Pull and Apply take the newest backup in /prog; the Miniserver may run another file (an upload
// still waiting, its emergency config, a file its firmware cannot run). def.log names the one it loaded.
const running = { loaded: false, busy: false, report: null };
async function loadRunning(fresh) {
  if (running.busy) return running.report;
  running.loaded = true; running.busy = true; renderRunning();
  try { running.report = await api("/api/running" + (fresh ? "?fresh=true" : "")); }
  catch (e) { running.report = { error: e.message, warnings: [] }; }
  finally { running.busy = false; renderRunning(); }
  return running.report;
}
function renderRunning() {
  const box = $("ms-run");
  const r = running.report;
  box.classList.toggle("hidden", !S.hist?.running || (!r && !running.busy));
  if (!r) { box.innerHTML = running.busy ? '<div class="ms-line"><span>Asking the Miniserver which config it runs…</span></div>' : ""; return; }
  const again = `<button type="button" class="ms-again" title="Check again" aria-label="Check again which config the Miniserver runs"${running.busy ? " disabled" : ""}>↻</button>`;
  if (r.error) {
    box.innerHTML = `<div class="ms-line"><span class="ms-err" title="${esc(r.error)}">Cannot tell which config the Miniserver runs: ${esc(r.error)}</span>${again}</div>`;
    return;
  }
  const how = r.from === "def.log"
    ? `def.log: loaded ${r.start?.loaded ? r.start.loaded + ", " : ""}program started ${r.start?.at || "?"}`
    : "guessed from the files in /prog by the Miniserver's load order";
  const tip = [how, r.firmware ? `firmware ${r.firmware}` : "", r.pull ? `Pull downloads ${r.pull}` : ""].filter(Boolean).join("\n");
  box.innerHTML = `<div class="ms-line" title="${esc(tip)}"><span>${r.from === "def.log" ? "Runs" : "Probably runs"} <code>${esc(r.running || "?")}</code>${r.firmware ? ` · fw ${esc(r.firmware)}` : ""}</span>${again}</div>` +
    (r.warnings || []).map((w) => `<div class="ms-warn" data-kind="${esc(w.kind)}">⚠ ${esc(w.text)}</div>`).join("");
}
$("ms-run").addEventListener("click", whenReady((e) => { if (e.target.closest(".ms-again")) loadRunning(true); }));
$("pull-msg").onclick = whenReady(() => pullMsg(""));

// ── the Miniserver: status chip and preflight ──────────────────────────
// The server checks the Miniserver lox talks to (its web port, FTP, the unauthenticated
// jdev/cfg/apiKey, lox's token) and refuses Pull, Apply and Sync when they could not work.
S.ms = null;
// What the last check found against an action. It only warns: the server checks again when the
// button is clicked, so a cause that has cleared does not keep the button off.
function msBlocked(what) { return S.ms?.available ? S.ms.blocked?.[what] || "" : ""; }
function msWarn(what) { const b = msBlocked(what); return b ? `\n\n⚠ The last check found: ${b}. It is checked again when you click.` : ""; }
// `fresh`: check again now; `poll`: the regular poll, which asks only for the config dates again.
async function loadMs(fresh, poll) {
  try { S.ms = await api("/api/miniserver" + (fresh ? "?fresh=1" : poll ? "?poll=1" : "")); } catch { return; }
  renderMs(); renderPull(); renderActions();
}
function renderMs() {
  const m = S.ms, chip = $("ms-chip");
  chip.classList.toggle("hidden", !m?.available);
  if (!m?.available) { closeMsPop(); return; }
  const blocked = ["pull", "apply", "sync"].filter((w) => m.blocked?.[w]);
  const why = [...new Set(blocked.map((w) => m.blocked[w]))];
  const state = blocked.length ? "bad" : m.warnings.length ? "warn" : "ok";
  chip.className = "ms-chip " + state;
  $("ms-lbl").textContent = m.identity === "wrong" ? `Wrong Miniserver (${m.serial})`
    : (m.reachable ? [m.serial, m.firmware].filter(Boolean).join(" · ") || "Miniserver" : "Miniserver unreachable")
      + (m.route?.kind === "remote" ? " · Remote Connect" : "");
  chip.title = why.length ? `Miniserver: ${why.join("\n")}`
    : `Miniserver ${m.serial || ""} at ${m.host}` + (m.warnings.length ? `\n${m.warnings.join("\n")}` : "");
  const tls = { on: "available", none: "none", expired: "certificate expired", unknown: "unknown" }[m.tls] || "unknown";
  const tok = m.tokenValidUntil ? new Date(m.tokenValidUntil * 1000).toLocaleString([], { ...MONTH, year: "numeric", hour: "2-digit", minute: "2-digit" }) + (m.tokenValidUntil * 1000 < Date.now() ? " (expired)" : "") : "no token yet";
  const row = (k, v) => v == null || v === "" ? "" : `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`;
  // How vislox reaches it (route.rs): its CloudDNS name, Remote Connect, or lox's host as it is.
  const r = m.route;
  const route = r && ({ clouddns: "local, under its CloudDNS name", remote: "Remote Connect", local: "lox's host" }[r.kind]
    + (r.url.startsWith("https://") ? (r.verified ? " · certificate checked" : " · certificate not checked") : " · no TLS"));
  $("ms-body").innerHTML = `<h3>Miniserver</h3><dl>
      ${row("Address", m.host)}${row("Route", route)}${r && r.url !== r.loxHost ? row("lox's host (Pull, Apply, Sync)", r.loxHost || "none (a serial only)") : ""}
      ${r?.note && r.verified ? row("Why", r.note) : ""}${row("Serial", m.serial)}
      ${m.expected ? row("Config belongs to", `${m.expected.serial} (${m.expected.from})` + ({ confirmed: " ✓", wrong: " ✗ not this one" }[m.identity] || "")) : ""}
      ${row("Firmware", m.firmware)}
      ${row("Deployed config", m.config?.deployed && `Loxone Config ${m.config.deployed}`)}
      ${row("Pending config", m.config?.pending && `Loxone Config ${m.config.pending}`)}
      ${row("Config last changed", m.configDate?.error ? `can't tell (${m.configDate.error})`
        : m.configDate?.miniserver && msDate(m.configDate.miniserver) + ({ after: " (after the base)", before: " (before the base: an older config runs)" }[m.configDate.order] || ""))}
      ${row("Base taken at", m.configDate?.base && msDate(m.configDate.base))}
      ${row("Calendar entries", !S.cal?.available || (!S.cal.read_at && !S.cal.failed) ? "" : S.cal.error ? `can't tell (${S.cal.error})`
        : calDrift().length ? `${calDrift().length} changed in the app` : `${S.cal.entries}, as in the deployed config`)}
      ${m.reachable ? row("Connection", m.local === false ? "remote" : m.local ? "local" : "unknown") + row("TLS", tls)
        + row("Live-update slot", m.eventSlots == null ? "" : m.eventSlots ? "free" : "none free") : ""}
      ${row("lox's token valid until", tok)}
      ${(() => { const st = S.mon?.info?.stats; return st ? row("Load (Debug-Monitor)", `CPU ${Math.round(st.cpu)} % · heap ${st.heapKb} of ${st.maxKb} kB` + (st.linkErrors ? ` · ${st.linkErrors} Link errors` : "")) : ""; })()}</dl>
    ${m.checks.map((c) => `<div class="chk ${c.ok ? "ok" : "bad"}"><span class="dot" aria-hidden="true"></span><span><b>${esc(c.name)}</b> <span class="d">${esc(c.ok ? (c.detail === "answers" ? "answers" : "answers: " + c.detail) : c.detail)}</span></span></div>`).join("")}
    ${m.discovered?.length ? `<div class="chk"><span><b>On the local network</b> <span class="d">${m.discovered.map((d) => esc(`${d.serial} at ${d.ip} (${d.firmware})`)).join(", ")}</span></span></div>` : ""}
    ${m.discoveryNote ? `<div class="chk bad"><span class="dot" aria-hidden="true"></span><span><b>Discovery</b> <span class="d">${esc(m.discoveryNote)}</span></span></div>` : ""}
    ${why.map((y) => `<div class="blocked">${esc(blocked.filter((w) => m.blocked[w] === y).map((w) => w[0].toUpperCase() + w.slice(1)).join(", "))} would fail: ${esc(y)}</div>`).join("")}
    ${m.warnings.length ? `<ul class="warns">${m.warnings.map((w) => `<li>${esc(w)}</li>`).join("")}</ul>` : ""}`;
  renderStale();
  $("ms-when").textContent = "Checked " + new Date(m.checked * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
// The Miniserver's config changed after the base (LoxAPPversion3 against the date in the
// download the base came from): someone saved from Loxone Config, or the like.
/** "2026-11-06 14:32:10" (the Miniserver's local time) → "14:32" today, else "2026-11-06 14:32". */
function msDate(t) {
  const m = /^(\d{4})-(\d\d)-(\d\d) (\d\d):(\d\d)/.exec(t || "");
  if (!m) return t || "";
  const d = new Date();
  const today = +m[1] === d.getFullYear() && +m[2] === d.getMonth() + 1 && +m[3] === d.getDate();
  return today ? `${m[4]}:${m[5]}` : `${m[1]}-${m[2]}-${m[3]} ${m[4]}:${m[5]}`;
}
function staleText() {
  const c = S.ms?.available && S.ms.configDate;
  return c?.changed ? `The Miniserver's config changed at ${msDate(c.miniserver)}, after the base vislox shows (taken at ${msDate(c.base)}).` : "";
}
function renderStale() {
  renderCalDrift();
  const text = staleText();
  $("stale").classList.toggle("hidden", !text);
  if (!text) return;
  const pullOk = !!S.hist?.pull?.available;
  const a = S.hist?.actions;
  const syncOk = !!a?.shown && !!a.apply?.available && !pendingChange() && !a.running;
  $("stale-text").textContent = text + (pullOk || syncOk ? " Take it in before you change anything:" : " Changes made now start from an old base.");
  $("stale-text").title = `Miniserver: ${S.ms.configDate.miniserver} · base: ${S.ms.configDate.base}`;
  $("stale-pull").classList.toggle("hidden", !pullOk);
  $("stale-pull").disabled = pulling;
  $("stale-sync").classList.toggle("hidden", !syncOk);
  $("stale-row").classList.toggle("hidden", !pullOk && !syncOk);
}
$("stale-pull").onclick = whenReady(() => $("btn-pull").click());
$("stale-sync").onclick = whenReady(() => $("btn-sync").click());

// ── calendar entries changed in the Loxone app ─────────────────────────
// The server reads the Miniserver's entries (calendargetentries, admin only) when live values
// start, after Pull, Apply and Sync, and before Apply, and compares them with the deployed
// revision. Without admin rights nothing shows.
S.cal = null;
// Requests can answer out of order: only the newest one's answer is kept.
S.calSeq = 0;
/** Ask for the drift (`fresh`: have the server read the Miniserver again first). Returns this
 * request's answer; a failed request is an answer too (`failed`, with `error`), never the last one. */
async function loadCal(fresh) {
  const seq = ++S.calSeq;
  let c;
  try {
    const r = await fetch("/api/calendar" + (fresh ? "?fresh=1" : ""), { headers: fresh ? { "X-Vislox": "calendar" } : {} });
    const j = await r.json().catch(() => ({}));
    c = r.ok ? j : { available: true, failed: true, error: j.error || r.statusText || `HTTP ${r.status}` };
  } catch (e) {
    c = { available: true, failed: true, error: e.message || "vislox did not answer" };
  }
  if (seq !== S.calSeq) return c;
  S.cal = c;
  renderCalDrift();
  if (S.ms) renderMs();
  if (S.view && S.sel && S.view.objects[S.sel]?.calendar) renderProps();
  return c;
}
/** The entries the app changed, against the deployed revision. */
function calDrift(c = S.cal) { return c?.available && Array.isArray(c.drift) ? c.drift : []; }
/** How entry `uuid` was changed in the app, when it was and the deployed revision is viewed. */
// The drift is against the deployed config (`S.cal.rev`: the last commit, not an edit on disk); it
// shows on that revision and on what the file on disk holds now.
function calDriftOf(uuid) {
  const r = revById(S.rev);
  return S.rev === S.cal?.rev || (r?.current && r.kind !== "pending") ? calDrift().find((d) => d.uuid === uuid) : undefined;
}
function calDriftText(c = S.cal) {
  const d = calDrift(c);
  if (!d.length) return "";
  const n = d.length;
  const by = ["added", "changed", "deleted"].map((k) => [k, d.filter((x) => x.kind === k).length]).filter(([, c]) => c);
  return `${n} calendar ${n === 1 ? "entry was" : "entries were"} ${by.length === 1 ? by[0][0] : `changed (${by.map(([k, c]) => `${c} ${k}`).join(", ")})`} in the Loxone app since the deployed config.`;
}
/** What Apply's confirmation says about the calendar entries, from the read just before it:
 * the entries the app changed, or that they could not be checked. A Miniserver that refuses
 * (no admin rights) is skipped quietly. */
function calApplyWarning(c) {
  if (!c?.available) return "";
  if (c.error && !c.refused) return `⚠ Couldn't check calendar entries changed in the app: ${c.error}. Applying overwrites any there are.\n\n`;
  const d = calDrift(c);
  if (!d.length) return "";
  return `⚠ ${calDriftText(c)} Applying overwrites ${d.length === 1 ? "it" : "them"}:\n${d.map((x) => `  ${x.name}: ${x.kind === "changed" ? x.changes.join("; ") : x.kind}`).join("\n")}\n\n`;
}
function calDriftLine(d) {
  const has = !!S.view?.objects[d.uuid];
  const name = has ? `<button type="button" class="lnk" data-go="${esc(d.uuid)}">${esc(d.name)}</button>` : `<b>${esc(d.name)}</b>`;
  if (d.kind === "added") return `${name}: added, ${esc(d.what)}`;
  if (d.kind === "deleted") return `${name}: deleted (was ${esc(d.what)})`;
  return `${name}: ${esc(d.changes.join("; "))}`;
}
function renderCalDrift() {
  const text = calDriftText();
  $("cal-drift").classList.toggle("hidden", !text);
  if (!text) return;
  const pullOk = !!S.hist?.pull?.available;
  const a = S.hist?.actions;
  const syncOk = !!a?.shown && !!a.apply?.available && !pendingChange() && !a.running;
  $("cal-drift-text").textContent = text + (pullOk || syncOk ? " Take them over before you change anything:" : " An Apply would overwrite them.");
  $("cal-drift-text").title = S.cal.read_at ? `Read from the Miniserver ${new Date(S.cal.read_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "";
  $("cal-drift-list").innerHTML = calDrift().map((d) => `<li>${calDriftLine(d)}</li>`).join("");
  $("cal-drift-pull").classList.toggle("hidden", !pullOk);
  $("cal-drift-pull").disabled = pulling;
  $("cal-drift-sync").classList.toggle("hidden", !syncOk);
  $("cal-drift-row").classList.toggle("hidden", !pullOk && !syncOk);
}
$("cal-drift-pull").onclick = whenReady(() => $("btn-pull").click());
$("cal-drift-sync").onclick = whenReady(() => $("btn-sync").click());
$("cal-drift-list").addEventListener("click", whenReady((e) => {
  const g = e.target.closest("[data-go]");
  if (!g) return;
  const u = g.dataset.go, pg = S.view?.objects[u]?.page;
  if (pg) gotoPage(pg, { focus: u });
  select(u);
}));
function closeMsPop() { $("ms-pop").classList.add("hidden"); $("ms-chip").setAttribute("aria-expanded", "false"); }
$("ms-chip").onclick = whenReady(() => {
  const open = $("ms-pop").classList.toggle("hidden") === false;
  $("ms-chip").setAttribute("aria-expanded", String(open));
  if (open) loadMs();
});
$("ms-recheck").onclick = whenReady(async () => {
  const b = $("ms-recheck");
  b.disabled = true; b.textContent = "Checking…";
  await loadMs(true);
  b.disabled = false; b.textContent = "Check again";
});
document.addEventListener("keydown", whenReady((e) => {
  if (e.key === "Escape" && !$("ms-pop").classList.contains("hidden")) { closeMsPop(); $("ms-chip").focus(); e.stopPropagation(); }
}), true);
document.addEventListener("click", whenReady((e) => { if (!e.target.closest(".ms-wrap")) closeMsPop(); }));

// ── in the app ─────────────────────────────────────────────────────────
// The Miniserver's structure file (LoxAPP3.json) lists what the app shows:
// controls (by block UUID) with their type, room, category and icon.
// vislox reads it with live values, or when asked; never on its own.

function awaitedAppRead() { return !!S.app?.loaded && (S.app.reads ?? 0) > S.applyReads; }
async function loadApp() {
  try { S.app = await api("/api/app"); } catch { return; }
  renderTree();
  if (S.view) { renderProps(); renderPage(); if (S.sel) applyFocus(); }
  if (S.deployed) renderDeployed();
}
async function loadAppFromMs() {
  if (S.appBusy) return;
  S.appBusy = true; S.appErr = ""; renderProps();
  try {
    const r = await fetch("/api/app/load", { method: "POST", headers: { "X-Vislox": "app" } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) S.appErr = j.error || r.statusText;
  } catch (e) { S.appErr = e.message; }
  S.appBusy = false;
  await loadApp();
}
function appIcon(name) {
  return name ? `<img class="app-ic" src="/api/app/icon?name=${encodeURIComponent(name)}" alt="" loading="lazy" onerror="this.remove()">` : "";
}
/** The viewed revision is what the Miniserver runs (its config can be compared with the app). */
function viewingDeployed() {
  return isDeployed(revById(S.rev));
}
/** Where the config and the app disagree about block `o`, in words. */
function appIssues(o) {
  const a = S.app, ctl = a?.controls?.[o.uuid], cfg = o.app;
  if (!a?.loaded || !cfg) return [];
  // Removed from the config: only the app still showing it is wrong.
  if (o.status === "removed") return ctl && !ctl.parent ? ["removed, but still in the app"] : [];
  const out = [];
  const name = (m, id) => m?.[id]?.name || id;
  if (cfg.shown && !ctl) out.push("not in the app");
  if (!cfg.shown && ctl && !ctl.parent) out.push("still in the app");
  if (cfg.shown && ctl && !ctl.parent) {
    if (cfg.room_id && ctl.room && cfg.room_id !== ctl.room) out.push(`in room ${name(a.rooms, ctl.room)} in the app, ${cfg.room || cfg.room_id} in the config`);
    if (cfg.category_id && ctl.cat && cfg.category_id !== ctl.cat) out.push(`in category ${name(a.cats, ctl.cat)} in the app, ${cfg.category || cfg.category_id} in the config`);
  }
  return out;
}
/** Rows of the In the app section: the config's visualization settings, then what the Miniserver's app shows. */
function appRows(o, row, chg, marks) {
  const a = S.app, ctl = a?.loaded ? a.controls?.[o.uuid] : null, cfg = o.app, old = o.app_old;
  if (!cfg && !ctl) return null;
  const rows = [];
  const changed = marks && o.app_changed;
  const yn = (v) => (v == null ? null : v ? "Yes" : "No");
  const place = (m, id, title) => {
    const p = id && a?.[m]?.[id];
    return `${p ? appIcon(p.image) : ""}${esc(title || p?.name || id || "")}`;
  };
  if (cfg) {
    const c = (k) => changed && (old?.[k] ?? null) !== (cfg[k] ?? null);
    rows.push(row("Shown", c("shown") ? chg(yn(old?.shown ?? false), yn(cfg.shown), true) : esc(yn(cfg.shown)), c("shown") ? "pr-chg" : ""));
    const pl = (label, m, k, idk) => {
      if (!cfg[idk] && !(changed && old?.[idk])) return;
      const ch = c(idk);
      rows.push(row(label, ch ? chg(old?.[k] ?? old?.[idk], cfg[k] ?? cfg[idk], true) : place(m, cfg[idk], cfg[k]), ch ? "pr-chg" : ""));
    };
    pl("Room", "rooms", "room", "room_id");
    pl("Category", "cats", "category", "category_id");
  }
  if (a?.loaded) {
    if (ctl) {
      rows.push(row("Control", `${appIcon(ctl.image)}${esc(prettyType(ctl.type) || "—")}` + (ctl.name && ctl.name !== o.title ? ` <span class="none-v">as “${esc(ctl.name)}”</span>` : "")));
      if (ctl.parent) rows.push(row("Part of", esc(a.controls[ctl.parent]?.name || ctl.parent)));
      if (!cfg) {
        if (ctl.room) rows.push(row("Room", place("rooms", ctl.room)));
        if (ctl.cat) rows.push(row("Category", place("cats", ctl.cat)));
      }
    }
    const issues = viewingDeployed() ? appIssues(o) : [];
    for (const i of issues) rows.push(`<div class="app-note warn">⚠ The Miniserver's app: ${esc(i)}.</div>`);
    if (!ctl && !(cfg?.shown && issues.length)) rows.push('<div class="app-note">Not in the app, so the Miniserver sends no live values for it. Only blocks shown in the app, and the periphery, carry live values.</div>');
    const when = a.fetched ? fmtWhen(a.fetched) : "";
    rows.push(`<div class="app-note">What the app shows comes from the Miniserver's structure file (LoxAPP3.json)${a.lastModified ? `, saved ${esc(a.lastModified)}` : ""}, read ${esc(when)}${a.from === "cache" ? " by an earlier run of vislox" : ""}${viewingDeployed() ? "" : ". It belongs to the running config, not to this revision"}.` +
      (a.available ? ` <button type="button" class="app-load lnk"${S.appBusy ? " disabled" : ""}>${S.appBusy ? "Reading…" : "Read again"}</button>` : "") + "</div>");
  } else if (a?.available) {
    rows.push(`<div class="app-note">The control type, icons and live values come from the Miniserver's structure file (LoxAPP3.json). It is read with live values, or now: <button type="button" class="app-load lnk"${S.appBusy ? " disabled" : ""}>${S.appBusy ? "Reading…" : "Read from the Miniserver"}</button></div>`);
  }
  const err = S.appErr || a?.error;
  if (err) rows.push(`<div class="app-note warn">${esc(err)}</div>`);
  return { rows, count: 0 };
}
// ── access rights, recipients and switching times (#68) ────────────────
// Who may open an NFC Code Touch, who gets a push notification and a Daytimer's switching times are on no logic
// page, yet a Lox Logic claim (`haustuer.access = bewohner`, `frost.recipients = familie`, `uhr.schedule = abend`)
// owns them: whoever a set leaves out loses the right on the next push, and a schedule replaces what the app
// changed. The view compares them (`view.rights`: per block, who was added, removed or rescheduled, and the users
// who gain or lose the right in effect, through groups too); GET /api/edits says which the module claims
// (`set_claims`). Names stay on the page: never in a URL, the title or a log.
/** The module's claims on block `uuid` (today's module: shown on the newest config only, like `sources`). */
function setClaims(uuid) { return managedShown() ? claimsOf(uuid) : []; }
/** The same, whatever revision is shown: for Apply's confirmation, which is about the newest config. */
function claimsOf(uuid) { return (S.edits?.set_claims || []).filter((c) => c.blocks?.includes(uuid)); }
function claimAt(c) { return `${c.file}:${c.line}`; }
function claimWhat(c) {
  return c.kind === "schedule" ? "The push replaces these switching times, edits made in the app among them"
    : c.kind === "tiles" ? "The push writes the set's tile programs onto the device"
    : c.kind === "access" ? "Whoever the set leaves out loses access on the next push"
    : "Whoever the set leaves out stops getting it on the next push";
}
function claimSet(c) { return `${c.kind} ${c.set}${c.set_line ? `, ${c.set_file}:${c.set_line}` : ""}`; }
/** The claim in plain text, for a tooltip. */
function claimTip(c) { return `Managed by ${claimAt(c)} (${claimSet(c)}). ${claimWhat(c)}.`; }
function claimNote(c) {
  return `<div class="ed-note rt-claim">Managed by <span class="mono">${esc(claimAt(c))}</span> <span class="none-v">(${esc(claimSet(c))})</span>. ${esc(claimWhat(c))}.</div>`;
}
const RIGHT_KIND = { access: "access", recipients: "recipients", schedule: "switching times" };
function whoText(p) { return `${p.kind === "group" ? "group " : ""}${p.title}`; }
function whoTip(p) { return p.members?.length ? `Members: ${p.members.join(", ")}` : ""; }
/** One holder of a right as a line: `+ group Bewohner (Workdays)`. */
function holderLi(p, cls, sign, extra) {
  const tip = whoTip(p);
  return `<li class="${cls}"${tip ? ` title="${esc(tip)}"` : ""}><span class="rt-s">${sign}</span>${esc(whoText(p))}${p.schedule ? ` <span class="none-v">(${esc(p.schedule)})</span>` : ""}${extra || ""}</li>`;
}
/** A user who may still open a door, but not under every schedule: "bob may open Haustür only at Workdays, no
 *  longer at Always". vislox cannot compare what the schedules' times cover, so any schedule taken away counts. */
function narrowText(n, door) {
  const gone = n.from.filter((x) => !n.to.includes(x));
  return `${n.title} may open ${door} only at ${n.to.join(", ") || "—"}, no longer at ${gone.join(", ") || n.from.join(", ")}`;
}
function lossText(c, users) {
  const names = users.map((u) => u.title).join(", ");
  return c.kind === "access" ? `${names} ${users.length === 1 ? "loses" : "lose"} access` : `${names} ${users.length === 1 ? "stops" : "stop"} getting it`;
}
function gainText(c, users) {
  const names = users.map((u) => u.title).join(", ");
  return c.kind === "access" ? `${names} ${users.length === 1 ? "gains" : "gain"} access` : `${names} now ${users.length === 1 ? "gets" : "get"} it`;
}
/** A Daytimer's changed modes, the compared revision's week next to this one's. */
function scheduleSideBySide(o) {
  const si = o?.schedule;
  if (!si) return "";
  const rows = schedRows(si, null).filter((r) => r.changed);
  if (!rows.length) return "";
  return `<div class="rt-sch"><div class="rt-sch-h"><span></span><span>before</span><span>after</span></div>` + rows.map((r) =>
    `<div class="rt-sch-r"><span class="sch-mode" title="${esc(r.title)}">${esc(r.title)}</span><div title="${esc(schedLine(r.old || [], si.analog))}">${schedBar(r.old || [], si, "was")}</div><div title="${esc(schedLine(r.mine, si.analog))}">${schedBar(r.mine, si, "cfg")}</div></div>`).join("") + "</div>";
}
function renderRights() {
  const box = $("rights-box"), v = S.view;
  const list = v?.compared ? v.rights || [] : [];
  box.classList.toggle("hidden", !list.length);
  if (!list.length) { box.innerHTML = ""; return; }
  const open = box.querySelector("details")?.open ?? true;
  const lost = new Set(list.flatMap((c) => [...(c.lose || []), ...(c.narrowed || [])].map((u) => u.uuid))).size;
  const items = list.map((c) => {
    const o = v.objects[c.uuid];
    const claims = setClaims(c.uuid);
    const head = `<div class="rt-head"><button type="button" class="lnk" data-go="${esc(c.uuid)}">${esc(c.title)}</button> <span class="rt-kind">${esc(RIGHT_KIND[c.kind] || c.kind)}</span>` +
      claims.map((x) => ` <span class="rt-src" title="${esc(claimTip(x))}">λ <span class="mono">${esc(claimAt(x))}</span></span>`).join("") + "</div>";
    if (c.kind === "schedule") return `<li class="rt-item">${head}${scheduleSideBySide(o)}</li>`;
    const lines = [
      ...(c.lose?.length ? [`<li class="rt-lose">⚠ ${esc(lossText(c, c.lose))}</li>`] : []),
      ...(c.narrowed || []).map((n) => `<li class="rt-lose">⚠ ${esc(narrowText(n, c.title))}</li>`),
      ...c.removed.map((p) => holderLi(p, "rt-rem", "−")),
      ...c.added.map((p) => holderLi(p, "rt-add", "+")),
      ...c.rescheduled.map((r) => holderLi(r.principal, "rt-chg", "~", ` <span class="none-v">was ${esc(r.from || "—")}</span>`)),
      ...(c.gain?.length ? [`<li class="rt-gain">${esc(gainText(c, c.gain))}</li>`] : []),
    ];
    return `<li class="rt-item">${head}${c.new_block ? '<div class="none-v">A new block.</div>' : ""}${c.removed_block ? '<div class="rt-lose">Removed, with its rights.</div>' : ""}<ul class="rt-lines">${lines.join("")}</ul></li>`;
  });
  box.innerHTML = `<details${open ? " open" : ""}><summary><span class="rt-h">Rights and schedules</span> ${list.length}${lost ? ` <span class="rt-lose">· ${plural(lost, "user")} lose${lost === 1 ? "s" : ""} a right</span>` : ""}</summary><ul class="rt-list">${items.join("")}</ul></details>`;
}
$("rights-box").addEventListener("click", whenReady(goClicked));
/** How two lists of rights differ, as the server compares them (rights.rs): a right is a principal and a
 *  schedule, both by UUID. A principal with one right before and one after, under another schedule, is
 *  rescheduled (new entry → old entry); anything else is added or removed. */
function rightsDiff(was, now) {
  const key = (p) => `${p.uuid}|${p.schedule_uuid || ""}`;
  const wk = new Set(was.map(key)), nk = new Set(now.map(key));
  let added = now.filter((p) => !wk.has(key(p))), removed = was.filter((p) => !nk.has(key(p)));
  const once = (l, u) => l.filter((p) => p.uuid === u).length === 1;
  const rescheduled = new Map();
  for (const a of added) {
    const r = removed.find((x) => x.uuid === a.uuid);
    if (r && once(added, a.uuid) && once(removed, a.uuid) && once(now, a.uuid) && once(was, a.uuid)) rescheduled.set(a, r);
  }
  added = added.filter((p) => !rescheduled.has(p));
  const gone = new Set(rescheduled.values());
  removed = removed.filter((p) => !gone.has(p));
  return { added, removed, rescheduled };
}
/** Properties of an NFC Code Touch or a push notification: who holds the right, marked against the compared
 *  revision, who holds it in effect, and the module's claim on it. */
function rightsRows(o, marks) {
  const r = o.rights;
  const old = marks ? o.rights_old : null;
  const rows = setClaims(o.uuid).map(claimNote);
  const d = old ? rightsDiff(old.list, r.list) : null;
  const items = r.list.map((p) => {
    if (!d) return holderLi(p, "", "");
    const was = d.rescheduled.get(p);
    if (was) return holderLi(p, "rt-chg", "~", ` <span class="none-v">was ${esc(was.schedule || "—")}</span>`);
    return d.added.includes(p) ? holderLi(p, "rt-add", "+") : holderLi(p, "", "");
  });
  if (d) for (const p of d.removed) items.push(holderLi(p, "rt-rem", "−"));
  rows.push(items.length ? `<ul class="rt-lines rt-props">${items.join("")}</ul>`
    : `<div class="empty">${r.kind === "access" ? "No user or group may open it." : "No user or group is a recipient."}</div>`);
  const users = r.users.map((u) => u.title);
  const label = r.kind === "access" ? "Opens for" : "Sent to";
  rows.push(propRow(label, users.length ? esc(users.join(", ")) : '<span class="none-v">nobody</span>', "",
    r.kind === "access" ? "The users who may open it: directly, or through a group" : "The users who get it: directly, through a group, or as members of Full access"));
  // Members of Full access get every notification (#77): inherited, as Loxone Config shows them; no set takes it away.
  if (r.inherited?.length) rows.push(propRow("Inherited", `${esc(r.inherited.map((u) => u.title).join(", "))} <span class="none-v">(Full access)</span>`, "",
    "Members of Full access get every push notification, whatever a set says"));
  if (old) {
    const ids = (l) => new Set(l.map((u) => u.uuid));
    const now = ids(r.users), then = ids(old.users);
    const lose = old.users.filter((u) => !now.has(u.uuid)), gain = r.users.filter((u) => !then.has(u.uuid));
    if (lose.length) rows.push(`<div class="ed-note ck-err">⚠ ${esc(lossText({ kind: r.kind }, lose))} here.</div>`);
    for (const n of (S.view?.rights || []).find((c) => c.uuid === o.uuid)?.narrowed || []) rows.push(`<div class="ed-note ck-err">⚠ ${esc(narrowText(n, displayTitle(o)))} here.</div>`);
    if (gain.length) rows.push(`<div class="ed-note">${esc(gainText({ kind: r.kind }, gain))} here.</div>`);
  }
  return { rows, count: r.list.length };
}
/** What a push notification says, and when (#77): on its block on a page (`OutputRefLM`), its texts and minimum
 *  time, marked against the compared revision; on the notification, the blocks that send it. The texts are shown
 *  as the review shows a title: in Properties and the change list, never in a URL, the page title or a log. Its
 *  message is edited on a block vislox added or adopted (*Edit values*); one Loxone Config drew keeps it there. */
function msgRows(o, marks) {
  if (o.ty === "Notification") {
    const refs = (o.mirrored_by || []).map(([, u]) => S.view?.objects[u]).filter((x) => x?.message);
    if (!refs.length) return null;
    return { rows: refs.map((x) => propRow("Sent by", `<button type="button" class="lnk" data-go="${esc(x.uuid)}" data-page="${esc(x.page || "")}">${esc(S.pageOf.get(x.page)?.title || displayTitle(x))}</button>` +
      `<span class="where">${esc(msgSummary(x.message))}</span>`)) };
  }
  const m = o.message;
  if (!m) return null;
  const old = marks ? o.message_old : null;
  const txt = (v) => (v == null ? '<span class="none-v">no message</span>' : esc(v));
  const dur = (v) => (v == null ? '<span class="none-v">5s (default)</span>' : esc(msgDuration(v)));
  const cell = (k, f) => (old && old[k] !== m[k] ? [`<span class="old">${f(old[k])}</span><span class="new">${f(m[k])}</span>`, "pr-chg"] : [f(m[k]), ""]);
  const n = m.notification && S.view?.objects[m.notification];
  const rows = [
    propRow("Notification", n ? `<button type="button" class="lnk" data-go="${esc(n.uuid)}">${esc(displayTitle(n))}</button>` : '<span class="none-v">none</span>', "", "The push notification it sends: made in Loxone Config (Messages)"),
    propRow("On", ...cell("on", txt), "Sent when its input turns on"),
    propRow("Off", ...cell("off", txt), "Sent when its input turns off"),
    propRow("Minimum time", ...cell("minimum_time", dur), "The least time between two messages"),
  ];
  if (S.rev === editRev()?.id && !editsBlock(o.uuid) && !S.edits?.managed?.includes(o.uuid))
    rows.push(`<div class="ed-note">Drawn in Loxone Config: its message is changed there. A push notification added here (Add block) is edited here.</div>`);
  return { rows, count: old ? ["on", "off", "minimum_time"].filter((k) => old[k] !== m[k]).length || undefined : undefined };
}
/** Milliseconds as Lox Logic writes a duration: 12h, 30min, 5s. */
function msgDuration(ms) {
  return ms > 0 && ms % 3600000 === 0 ? `${ms / 3600000}h` : ms > 0 && ms % 60000 === 0 ? `${ms / 60000}min` : ms % 1000 === 0 ? `${ms / 1000}s` : `${ms}ms`;
}
function msgSummary(m) { return [m.on != null ? `on: ${m.on}` : "", m.off != null ? `off: ${m.off}` : ""].filter(Boolean).join(" · ") || "no message"; }
// ── block settings (#79) ───────────────────────────────────────────────
// The settings Loxone Config stores on a block's element, by their Lox Logic name and as Lox Logic writes them
// (`At: 18:58`, `Pulse: sunset`, `Type: text`, the program of a Code16). They are edited on a block the module
// manages through vislox (added here or adopted: *Edit values*); on any other block they are read-only, since
// lox takes them only in a managed block's declaration.
const SET_LABEL = { Desc: "Desc (name in the app)", Code: "Program" };
/** Lines of `a` and `b`, the changed middle marked: [kind, line] with kind " ", "-" or "+". */
function lineDiff(a, b) {
  const x = (a ?? "").split("\n"), y = (b ?? "").split("\n");
  let i = 0, j = 0;
  while (i < x.length && i < y.length && x[i] === y[i]) i++;
  while (j < x.length - i && j < y.length - i && x[x.length - 1 - j] === y[y.length - 1 - j]) j++;
  return [...x.slice(0, i).map((l) => [" ", l]), ...x.slice(i, x.length - j).map((l) => ["-", l]),
    ...y.slice(i, y.length - j).map((l) => ["+", l]), ...x.slice(x.length - j).map((l) => [" ", l])];
}
/** A program as Properties shows it: folded, its lines counted; a change as a line diff around what changed. */
function programHtml(s, marks) {
  const lines = (v) => (v == null ? 0 : v.split("\n").length - (v.endsWith("\n") ? 1 : 0));
  if (marks && s.changed) {
    const d = lineDiff(s.old, s.value);
    // Three lines of context around each change.
    const near = (k) => d.slice(Math.max(0, k - 3), k + 4).some(([m]) => m !== " ");
    const shown = d.map((l, k) => (near(k) ? l : null));
    const out = [];
    shown.forEach((l, k) => { if (l) out.push(`<span class="pd${l[0] === "-" ? " old" : l[0] === "+" ? " new" : ""}">${esc(l[0] + " " + l[1])}</span>`); else if (shown[k - 1]) out.push('<span class="pd gap">…</span>'); });
    return `<pre class="prog prog-diff">${out.join("\n")}</pre>`;
  }
  if (s.value == null) return '<span class="none-v">— (Loxone Config\'s program)</span>';
  return `<details class="prog-d"><summary>${plural(lines(s.value), "line")}</summary><pre class="prog">${esc(s.value)}</pre></details>`;
}
function settingsRows(o, row, marks, chg) {
  const list = (o.settings || []).filter((x) => x.value != null || (marks && x.changed));
  if (!list.length) return null;
  const rows = list.map((x) => row(SET_LABEL[x.name] || x.name, x.kind === "program" ? programHtml(x, marks) : chg(x.old, x.value, x.changed),
    marks && x.changed ? (x.value == null ? "pr-rem" : "pr-chg") : ""));
  // Who edits them: vislox (added or adopted here), the module's own file, or nobody until it is managed.
  if (S.hist?.actions?.shown && !editsBlock(o.uuid)) {
    const m = managedSrc(o.uuid);
    rows.push(`<div class="ed-note set-note">${m ? `The module manages this block: change its settings in ${esc(srcPath(m) || "its .loxl file")}.`
      : "Read-only: lox changes these only on a block the module manages. Choose Adopt into vislox in Edit above to change them here."}</div>`);
  }
  return { rows, count: marks ? list.filter((x) => x.changed).length || undefined : undefined };
}

/** Show the block a button of the check or lint list names, on its page. */
function goClicked(e) {
  const g = e.target.closest("[data-go]");
  if (!g) return;
  const u = g.dataset.go, pg = S.view?.objects[u]?.page;
  if (pg) gotoPage(pg, { focus: u });
  select(u);
}
const EDIT_ATTR = { title: "Name", room: "Room", category: "Category", statistics: "Statistics", entry_mode: "Switches on", entry_date: "Date" };
function editLabel(o, c) { return EDIT_ATTR[c.op] || (c.op === "param" ? portName(o, c.port) : SET_LABEL[c.name] || c.name); }
/** A claimed value as the Edit section lists it: a program (#79) by its size, the rest as written. */
function editValue(c) {
  if (c.value == null) return '<span class="none-v">back to Loxone Config\'s value</span>';
  if (c.op === "setting" && c.name === "Code") return `<span class="none-v">edited here, ${plural(c.value.split("\n").length - (c.value.endsWith("\n") ? 1 : 0), "line")}</span>`;
  // A calendar entry's mode (#86), by its name.
  if (c.op === "entry_mode") return esc(mdModeTitle(c.value));
  return esc(c.value);
}
/** The config's rooms or categories by title; a title two of them share cannot be picked (lox claims by title). */
function editPlaces(ty) {
  const n = new Map();
  for (const x of Object.values(S.view?.objects || {})) if (x.ty === ty && x.status !== "removed") n.set(x.title, (n.get(x.title) || 0) + 1);
  return [...n].sort((a, b) => a[0].localeCompare(b[0]));
}
/** A constant as the form starts it: with the unit Lox Logic writes it with (`30min`), else as stored. */
function portNow(o, key) { const q = o.ports.find((x) => x.key === key); return q?.def_text ?? q?.def ?? "0"; }
// ── expressions and named signals (#82) ───────────────────────────────
// lox desugars an expression (`x.Port <- a.Q and b.AQ >= 28`, `Port: …` in a declaration, or a named signal
// `warm = …`) into And, Or, Not, comparator and Formula blocks, which it re-derives on every compile, so they are
// never edited as blocks. GET /api/edits' `expressions` names each one's blocks (the top one first), where it is
// written and what the deployed revision had. The canvas draws a chain as one group titled with its expression,
// folded into one block until it is opened; Properties shows the expression, and edits those of vislox's own files
// (the `expr` and `signal` ops); the Changes list puts an expression's changes under its old → new text.
S.exOpen = new Set(); // the expressions (by id) drawn open
S.ex = { key: null, draft: {}, err: "" }; // the Expression section's draft, for one block of one pending config
function exprList() { return managedShown() ? S.edits.expressions || [] : []; }
function ciEq(a, b) { return !!a && !!b && a.toLowerCase() === b.toLowerCase(); }
/** The expression whose blocks `uuid` is one of. */
function exprOf(uuid) { return uuid ? exprList().find((e) => e.blocks.some((b) => ciEq(b, uuid))) || null : null; }
/** The expressions that feed an input of `uuid`. */
function exprsInto(uuid) { return exprList().filter((e) => e.to && ciEq(e.to[0], uuid)); }
/** What the deployed config had, when the expression changed since and reads otherwise: the top labels of both
 *  configs read back the same way (constants resolved, a bare name's output, a signal's label), so an expression
 *  that did not change never shows an old text. Null otherwise. */
function exprOld(e) {
  return e.pending && e.deployed && e.now != null && e.deployed.text !== e.now ? e.deployed.text : null;
}
/** What an expression feeds, said after its text: `jal.AutoShade`, or a signal's output. */
function exprWhat(e) { return e.kind === "signal" ? `${e.sink} =` : `${e.sink} ←`; }
/** Each expression with a block on page `p`: a dashed frame titled with it when open, else one block showing it
 *  in place of its blocks and the wires between them. */
function drawExprs(p) {
  const list = exprList();
  if (!list.length) return;
  const objs = new Map(p.objects.filter((o) => o.shape !== "tag" && !o.moved_to).map((o) => [o.uuid.toLowerCase(), o]));
  for (const e of list) {
    const here = e.blocks.map((b) => objs.get(b.toLowerCase())).filter(Boolean);
    if (!here.length) continue;
    const pad = 120;
    const x1 = Math.min(...here.map((o) => o.rect[0])) - pad, y1 = Math.min(...here.map((o) => o.rect[1])) - pad;
    const x2 = Math.max(...here.map((o) => o.rect[0] + o.rect[2])) + pad, y2 = Math.max(...here.map((o) => o.rect[1] + o.rect[3])) + pad;
    const w = x2 - x1, h = y2 - y1;
    const top = here.find((o) => ciEq(o.uuid, e.blocks[0])) || here[0];
    const title = `${exprWhat(e)} ${e.text}`;
    const tip = () => `<b>${esc(e.kind === "signal" ? `Signal ${e.sink}${e.title ? ` (${e.title})` : ""}` : `Expression on ${e.sink}`)}</b><br><span class="mono">${esc(e.text)}</span><br><span class="tm">${plural(e.blocks.length, "block")} lox made for it${e.file ? ` · ${esc(constAt(e.file, e.line))}` : ""}</span>`;
    if (S.exOpen.has(e.id)) {
      const g = el("g", { class: "expr-grp" + (e.pending ? " pending" : ""), "data-expr": e.id }, L.ghost);
      el("rect", { class: "expr-frame", x: x1, y: y1, width: w, height: h, rx: 90 }, g);
      const t = el("text", { class: "expr-t", x: x1 + 40, y: y1 - 60 }, L.badges, clip(`▾ ${title}`, Math.max(12, Math.floor(w / 62))));
      t.setAttribute("data-expr", e.id);
      t.setAttribute("role", "button");
      hoverTip(t, () => `${tip()}<br><span class="tm">Click to fold it into one block.</span>`);
      t.addEventListener("click", (ev) => { ev.stopPropagation(); S.exOpen.delete(e.id); renderPage(); applyFocus(); });
      continue;
    }
    const inside = new Set(here.map((o) => o.uuid));
    for (const u of inside) vpG.querySelectorAll(`.blk[data-uuid="${CSS.escape(u)}"]`).forEach((n) => n.classList.add("expr-hidden"));
    L.wires.querySelectorAll(".wire").forEach((n) => {
      if (inside.has(n.dataset.f) && inside.has(n.dataset.t)) { n.classList.add("expr-hidden"); n.nextElementSibling?.classList.add("expr-hidden"); }
    });
    const g = el("g", { class: "expr-fold" + (e.pending ? " pending" : "") + (inside.has(S.sel) ? " sel" : ""), "data-expr": e.id, "data-top": top.uuid, role: "button", "aria-label": title }, L.objs);
    el("rect", { class: "expr-body", x: x1, y: y1, width: w, height: h, rx: 90 }, g);
    el("text", { class: "expr-h", x: x1 + 90, y: y1 + 190 }, g, clip(e.kind === "signal" ? `Signal ${e.title || e.sink}` : `Expression ▸ ${e.sink}`, Math.floor((w - 180) / 66)));
    // The text, wrapped at its spaces.
    const per = Math.max(8, Math.floor((w - 180) / 62)), words = e.text.split(" "), lines = [];
    for (const wd of words) { const last = lines[lines.length - 1]; if (last != null && (last + " " + wd).length <= per) lines[lines.length - 1] = last + " " + wd; else lines.push(wd); }
    const room = Math.max(1, Math.floor((h - 300) / 150));
    lines.slice(0, room).forEach((l, i) => el("text", { class: "expr-x", x: x1 + 90, y: y1 + 400 + i * 150 }, g, i === room - 1 && lines.length > room ? clip(l + " …", per) : clip(l, per)));
    hoverTip(g, () => `${tip()}<br><span class="tm">Click to open it.</span>`);
    g.addEventListener("click", (ev) => { ev.stopPropagation(); if (drag?.moved) return; select(top.uuid); });
    // Blocks of something else under its area stay on top, and take their own clicks.
    for (const o of p.objects) {
      if (inside.has(o.uuid) || o.moved_to) continue;
      const [ox, oy, ow, oh] = o.rect;
      if (ox > x2 || oy > y2 || ox + ow < x1 || oy + oh < y1) continue;
      L.objs.querySelectorAll(`:scope > [data-uuid="${CSS.escape(o.uuid)}"]`).forEach((n) => L.objs.appendChild(n));
    }
  }
}
/** The Expression section's draft follows the selected block and the pending config: a new one starts afresh. */
function exSync() {
  const key = `${S.sel}|${editRev()?.blob || ""}`;
  if (S.ex.key !== key) S.ex = { key, draft: {}, err: "" };
}
/** Whether vislox can edit expressions now: the newest config shown, editing on, nothing compiling. */
function exState() {
  const a = S.hist?.actions;
  if (!a?.shown) return null;
  if (!a.edit?.available) return { why: `Editing is off: ${a.edit?.reason || "unknown"}.` };
  const r = editRev();
  if (!r || S.rev !== r.id) return null;
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy || S.edit.busy) return { why: "An edit is compiling." };
  return { ok: true, rev: r };
}
/** The pending claim of the `expr` op on an input (`claims`), for its Revert. */
function exClaim(e) { return e.to ? (S.edits?.claims || []).find((c) => c.op === "expr" && ciEq(c.uuid, e.to[0]) && c.port === e.to[1]) || null : null; }
function exRows(o, row, marks) {
  exSync();
  const mine = exprOf(o.uuid);
  const list = [mine, ...exprsInto(o.uuid).filter((e) => e !== mine)].filter(Boolean);
  const st = exState();
  const d = S.ex.draft;
  const rows = [];
  let count = 0;
  for (const e of list) {
    const id = esc(e.id);
    const old = exprOld(e);
    const sink = e.to && S.view.objects[e.to[0]];
    rows.push(row(e.kind === "signal" ? "Signal" : "Expression", `<span class="mono ex-text">${esc(e.text)}</span>`, marks && e.pending ? "pr-chg" : ""));
    if (e.kind === "signal") {
      rows.push(row("Output", `<span class="mono">${esc(e.sink)}.${e.top_type === "Formula" ? "AQ" : "Q"}</span>`));
      rows.push(row("Label", e.title ? esc(e.title) : '<span class="none-v">— (its expression)</span>'));
    } else rows.push(row("Feeds", sink ? `<button type="button" class="lnk" data-go="${esc(sink.uuid)}" data-page="${esc(sink.page || "")}">${esc(displayTitle(sink))}.${esc(portName(sink, e.to[1]))}</button>` : `<span class="mono">${esc(e.sink)}</span>`));
    rows.push(row("Blocks", `${plural(e.blocks.length, "block")} lox made for it`, "", "lox makes these blocks from the expression on every compile: they are changed through the expression, never one by one."));
    const path = constAt(e.file, e.line);
    if (path) rows.push(row("Source", `<span class="mono src-path">${esc(path)}</span><button type="button" class="copy" data-copy="${esc(path)}" data-label="Copy path" title="Copy the file and line">Copy path</button>`));
    if (e.pending) {
      count++;
      rows.push(`<div class="ed-note ex-pending">${!e.deployed ? "Not deployed: new since the deployed config." : old ? `Not deployed: the deployed config has <span class="mono">${esc(old)}</span>.` : "Not deployed: its blocks changed since the deployed config."}</div>`);
    }
    if (!e.ours) {
      rows.push(`<div class="ed-note">Written in the module's own file: change it there (vislox writes only its own files).</div>`);
      continue;
    }
    if (!st) continue;
    if (!st.ok) { rows.push(`<div class="ed-note">${esc(st.why)}</div>`); continue; }
    const tv = d[`${e.id}:text`] ?? e.text;
    rows.push(`<div class="ex-form"><label class="ed-l">Expression <input type="text" class="ed-in mono" data-ex-field="${id}:text" data-ex-enter="save" data-id="${id}" value="${esc(tv)}" spellcheck="false" aria-label="Expression"></label>` +
      (e.kind === "signal" ? `<label class="ed-l">Label <input type="text" class="ed-in" data-ex-field="${id}:title" data-ex-enter="save" data-id="${id}" value="${esc(d[`${e.id}:title`] ?? e.title ?? "")}" placeholder="its expression" aria-label="Label"></label>` : "") +
      `<div class="ed-bar"><button type="button" class="ed-btn" data-ex="save" data-id="${id}">Save</button>` +
      (e.pending && (e.kind === "signal" ? e.revert?.length : exClaim(e)?.pending) ? `<button type="button" class="ed-btn" data-ex="revert" data-id="${id}" title="Back to what the deployed revision has">Revert</button>` : "") + `</div>` +
      (e.kind !== "signal" ? `<label class="ed-l">Name this expression <input type="text" class="ed-in mono" data-ex-field="${id}:name" data-ex-enter="name" data-id="${id}" value="${esc(d[`${e.id}:name`] ?? "")}" placeholder="vx_name" spellcheck="false" aria-label="Signal name"></label><div class="ed-bar"><button type="button" class="ed-btn" data-ex="name" data-id="${id}" title="Turn it into a named signal of this page, and feed the input from it">Name it</button></div>` : "") +
      `</div>`);
  }
  // A new expression on an unwired input of a block vislox edits.
  const fresh = st?.ok && !mine && (o.edit?.params || []).filter((q) => !exprsInto(o.uuid).some((e) => e.to[1] === q.key));
  if (fresh?.length && !S.edits?.managed?.includes(o.uuid)) {
    const port = d["new:port"] ?? fresh[0].key;
    rows.push(`<details class="ex-new"${d["new:open"] ? " open" : ""}><summary>Feed an input from an expression…</summary><div class="ex-form"><label class="ed-l">Input <select class="ed-in" data-ex-field="new:port" aria-label="Input">${fresh.map((q) => `<option value="${esc(q.key)}"${q.key === port ? " selected" : ""}>${esc(portName(o, q.key))}</option>`).join("")}</select></label>` +
      `<label class="ed-l">Expression <input type="text" class="ed-in mono" data-ex-field="new:text" data-ex-enter="add" data-id="new" value="${esc(d["new:text"] ?? "")}" placeholder="vx_….Q and vx_….AQ >= 28" spellcheck="false" aria-label="New expression"></label>` +
      `<div class="ed-bar"><button type="button" class="ed-btn" data-ex="add" data-id="new">Add expression</button></div>` +
      `<div class="ed-note">Names: a block of vislox's by its slug, one Loxone Config drew as vx_ and its UUID (- as _), a constant. lox makes the blocks; the compile says what it refuses.</div></div></details>`);
  }
  if (S.ex.err) rows.push(`<div class="ed-msg err ex-err" role="alert">${esc(S.ex.err)}</div>`);
  return rows.length ? { rows, count: count || undefined } : null;
}
/** A button of the Expression section: `save`, `revert`, `name` (Name this expression) or `add`. */
async function exAct(what, id) {
  exSync();
  const st = exState();
  if (!st?.ok) return;
  const d = S.ex.draft, uuid = S.sel;
  const e = exprList().find((x) => x.id === id);
  let ops;
  if (what === "add") {
    const o = S.view.objects[uuid];
    const port = d["new:port"] ?? (o?.edit?.params || [])[0]?.key, text = (d["new:text"] || "").trim();
    if (!port || !text) { S.ex.err = "Type the expression first."; renderProps(); return; }
    ops = [{ op: "expr", uuid, port, value: text }];
  } else if (!e) return;
  else if (what === "save") {
    const text = (d[`${id}:text`] ?? e.text).trim();
    if (!text) { S.ex.err = "An expression cannot be empty."; renderProps(); return; }
    ops = e.kind === "signal"
      ? [{ op: "signal", slug: e.sink, page: S.page, value: text, title: ((d[`${id}:title`] ?? e.title ?? "").trim()) || null }]
      : [{ op: "expr", uuid: e.to[0], port: e.to[1], value: text }];
  } else if (what === "revert") {
    // A signal's: first each input it feeds back to what fed it, then the signal (the server's list).
    if (e.kind === "signal") ops = e.revert;
    else { const c = exClaim(e); if (!c) return; ops = [{ op: "expr", uuid: c.uuid, port: c.port, value: c.deployed }]; }
  } else if (what === "name") {
    const name = (d[`${id}:name`] || "").trim();
    if (!/^vx_[a-z0-9_]+$/.test(name)) { S.ex.err = "A signal of vislox's is named vx_ and then lowercase letters, digits and _."; renderProps(); return; }
    ops = [{ op: "signal", slug: name, page: S.page, value: e.text }, { op: "expr", uuid: e.to[0], port: e.to[1], value: name }];
  } else return;
  S.ex.err = "";
  const r = await editSend(uuid, st.rev, ops);
  if (r === "failed") { S.ex.err = S.edit.err; S.edit.err = ""; renderProps(); return; }
  // The expression's blocks may be new ones: select its top block once /api/edits names it (the one refresh the
  // History started for this compile).
  if (e || what === "add") {
    await S.editsLoad;
    const want = what === "name" ? exprList().find((x) => x.kind === "signal" && x.sink === (d[`${id}:name`] || "").trim())
      : e ? exprList().find((x) => x.id === e.id) : exprsInto(uuid).find((x) => x.to[1] === ops[0].port);
    const top = want?.blocks[0];
    if (top && S.view?.objects[top]) select(top, { noReveal: true });
    else if (top) { const t = Object.keys(S.view?.objects || {}).find((u) => ciEq(u, top)); if (t) select(t, { noReveal: true }); }
  }
}

// ── template instances (#83) ──────────────────────────────────────────
// A `template` is a block type made of blocks; each instance (`sued = Fassade(…)`) stamps out a copy whose lockfile
// keys are `sued_hoch`, which source spells `sued.hoch`. GET /api/edits' `templates` lists the module's templates
// (parameters, outputs, file and line), every instance (nested ones by their path, `sued.hitze`) with its arguments
// as written and the blocks it made, and lox's library. The canvas frames an instance's blocks, titled `sued:
// Fassade` with its arguments and outputs (nested instances inside); Properties names a block's instance, template
// and body name, and edits an instance of vislox's own files (the `instance` op); the Changes list puts the changes
// a template's body caused under the template, and those of one call under its instance.
S.tp = { key: null, draft: {}, err: "", busy: false }; // the Template section's draft, for one block of one pending config
function tplData() { return managedShown() ? S.edits.templates || null : null; }
function tplInstances() { return tplData()?.instances || []; }
function tplTemplate(name) { return (tplData()?.templates || []).find((t) => t.name === name) || null; }
/** The instances by path. */
function tplByPath(path) { return tplInstances().find((x) => x.path === path) || null; }
let TPL_IX = { of: null, map: new Map() };
/** The instance (innermost) and block of `uuid`: { inst, block }, or null. */
function instOf(uuid) {
  const d = tplData();
  if (!uuid || !d) return null;
  if (TPL_IX.of !== d) {
    TPL_IX = { of: d, map: new Map() };
    for (const inst of d.instances) for (const b of inst.blocks) TPL_IX.map.set(b.uuid.toLowerCase(), { inst, block: b });
  }
  return TPL_IX.map.get(uuid.toLowerCase()) || null;
}
/** An instance, its parent, … up to the top-level one. */
function instChain(inst) {
  const out = [];
  for (let x = inst; x && out.length < 10; x = x.parent ? tplByPath(x.parent) : null) out.push(x);
  return out;
}
/** `sued: Fassade`, and the arguments as written. */
function instTitle(x) { return `${x.path}: ${x.template}${x.label ? ` "${x.label}"` : ""}`; }
function instArgs(x) { return x.args.map(([k, v]) => `${k}: ${v}`).join(" · "); }
function instOuts(x) { return x.outputs.length ? `→ ${x.outputs.map((o) => o.name).join(", ")}` : ""; }
/** Each instance with a block on page `p`: a dashed frame around its blocks and its nested instances' frames,
 *  headed by `sued: Fassade` and its arguments and outputs. Behind the blocks; the header takes clicks. */
function drawInstances(p) {
  const list = tplInstances();
  if (!list.length) return;
  const objs = new Map(p.objects.filter((o) => o.shape !== "tag" && !o.moved_to).map((o) => [o.uuid.toLowerCase(), o]));
  const H = 380, pad = 140, frames = new Map();
  // Bottom up: a frame takes its own blocks and its children's frames.
  const frame = (x, depth) => {
    if (frames.has(x.path) || depth > 10) return frames.get(x.path) || null;
    const rs = x.blocks.map((b) => objs.get(b.uuid.toLowerCase())?.rect).filter(Boolean);
    for (const c of list.filter((y) => y.parent === x.path)) { const f = frame(c, depth + 1); if (f) rs.push(f); }
    if (!rs.length) { frames.set(x.path, null); return null; }
    const x1 = Math.min(...rs.map((r) => r[0])) - pad, y1 = Math.min(...rs.map((r) => r[1])) - pad - H;
    const x2 = Math.max(...rs.map((r) => r[0] + r[2])) + pad, y2 = Math.max(...rs.map((r) => r[1] + r[3])) + pad;
    const f = [x1, y1, x2 - x1, y2 - y1];
    frames.set(x.path, f);
    return f;
  };
  for (const x of list) frame(x, 0);
  for (const x of list) {
    const f = frames.get(x.path);
    if (!f) continue;
    const [x1, y1, w, h] = f;
    const t = tplTemplate(x.template);
    const pending = !!(x.change || (t && t.changed));
    const g = el("g", { class: "tpl-grp" + (pending ? " pending" : "") + (x.parent ? " nested" : ""), "data-inst": x.path }, L.ghost);
    el("rect", { class: "tpl-frame", x: x1, y: y1, width: w, height: h, rx: 60 }, g);
    const per = Math.max(10, Math.floor((w - 120) / 62));
    const head = el("g", { class: "tpl-head", "data-inst": x.path, role: "button", "aria-label": instTitle(x) }, L.badges);
    el("text", { class: "tpl-t", x: x1 + 60, y: y1 + 150 }, head, clip(instTitle(x), per));
    const sub = [instArgs(x), instOuts(x)].filter(Boolean).join("  ");
    if (sub) el("text", { class: "tpl-a", x: x1 + 60, y: y1 + 290 }, head, clip(sub, Math.floor(per * 1.15)));
    const first = x.blocks.find((b) => objs.has(b.uuid.toLowerCase()) && !b.expression) || x.blocks[0];
    hoverTip(head, () => `<b>${esc(instTitle(x))}</b><br>Instance of the template <span class="mono">${esc(x.template)}</span>${t?.file ? ` (${esc(constAt(t.file, t.line))})` : ""}` +
      `${x.args.length ? `<br><span class="mono">${esc(instArgs(x))}</span>` : ""}<br><span class="tm">${plural(x.blocks.length, "block")} of its own · ${esc(constAt(x.file, x.line))}${x.change === "new" ? " · new since the deployed config" : x.change === "call" ? " · its call changed since the deployed config" : t?.changed ? " · its template changed since the deployed config" : ""}</span>` +
      (first ? '<br><span class="tm">Click to select its first block.</span>' : ""));
    head.addEventListener("click", (ev) => {
      ev.stopPropagation();
      if (drag?.moved || !first) return;
      const u = Object.keys(S.view?.objects || {}).find((k) => ciEq(k, first.uuid));
      if (u) select(u);
    });
  }
}
/** The Template section's draft follows the selected block and the pending config. */
function tpSync() {
  const key = `${S.sel}|${editRev()?.blob || ""}`;
  if (S.tp.key !== key) S.tp = { key, draft: {}, err: "", busy: false };
}
/** A parameter's hint: its kind, what it takes, its default. */
function tpParamHint(p) {
  const what = p.kind === "object" ? `an object${p.type && p.type !== "any" ? ` (${p.type})` : ""}: vx_ and its UUID, or a block of vislox's`
    : p.kind === "input" ? "a source: a port, a block, or an expression; open when not given"
    : p.kind === "set" ? `a ${p.type} set of vislox's, by its name`
    : p.type === "string" ? 'a text ("…"; typed without quotes it is quoted, unless it names a constant)' : p.type === "duration" ? "a duration, like 3min" : p.type === "bool" ? "on or off" : "a number";
  return `${what}${p.default != null ? `; default ${p.default}` : p.required ? "; required" : ""}`;
}
/** The fields of a call of template `t`: its label (when it takes one), then each parameter. */
function tpFields(t, call, prefix) {
  const d = S.tp.draft;
  const rows = [];
  const lp = t.params.find((p) => p.name === "label");
  if (lp) rows.push(`<label class="ed-l">Label <input type="text" class="ed-in" data-tp-field="${prefix}label" value="${esc(d[`${prefix}label`] ?? call?.label ?? "")}" placeholder="${esc(lp.default ?? "required")}" aria-label="Label"></label>`);
  for (const p of t.params.filter((q) => q.name !== "label")) {
    const v = d[`${prefix}a:${p.name}`] ?? call?.args?.[p.name] ?? "";
    rows.push(`<label class="ed-l">${esc(p.name)} <span class="tm">${esc(p.kind === "value" ? p.type : p.kind)}</span><input type="text" class="ed-in mono" data-tp-field="${prefix}a:${esc(p.name)}" value="${esc(v)}" placeholder="${esc(p.default ?? (p.required ? "required" : "open"))}" spellcheck="false" title="${esc(tpParamHint(p))}" aria-label="${esc(p.name)}"></label>`);
  }
  return rows.join("");
}
/** The call a draft makes (`prefix`: the fields' prefix). */
function tpCall(t, call, prefix) {
  const d = S.tp.draft, args = {};
  for (const p of t.params.filter((q) => q.name !== "label")) {
    const v = (d[`${prefix}a:${p.name}`] ?? call?.args?.[p.name] ?? "").trim();
    if (v) args[p.name] = v;
  }
  const label = (d[`${prefix}label`] ?? call?.label ?? "").trim();
  return { template: t.name, ...(label ? { label } : {}), args };
}
function tpRows(o, row, marks) {
  tpSync();
  const hit = instOf(o.uuid);
  if (!hit) return null;
  const { inst, block } = hit, chain = instChain(inst), top = chain[chain.length - 1];
  const t = tplTemplate(inst.template);
  const rows = [];
  rows.push(row("Block", `<span class="mono">${esc(block.name)}</span>${block.expression ? ' <span class="tm">(lox made it for an expression)</span>' : ""}`, "", "How source names this block: the instance, then the name in the template's body."));
  rows.push(row("Instance", `<span class="mono">${esc(inst.path)}</span>${inst.label ? ` “${esc(inst.label)}”` : ""}`));
  rows.push(row("Template", `<span class="mono">${esc(inst.template)}</span>${t?.file ? ` <span class="tm">${esc(constAt(t.file, t.line))}</span>` : ""}${t?.library ? ' <span class="tm">(lox\'s library)</span>' : ""}`, marks && t?.changed ? "pr-chg" : "", t?.description || ""));
  if (chain.length > 1) rows.push(row("Inside", chain.slice(1).map((x) => `<span class="mono">${esc(x.path)}: ${esc(x.template)}</span>`).join(" ⊂ ")));
  for (const [k, v] of inst.args) rows.push(row(k, `<span class="mono">${esc(v)}</span>`, "", t?.params.find((p) => p.name === k) ? tpParamHint(t.params.find((p) => p.name === k)) : ""));
  for (const p of (t?.params || []).filter((q) => q.name !== "label" && !inst.args.some(([k]) => k === q.name))) rows.push(row(p.name, `<span class="none-v">${esc(p.default ?? "open")}</span>`, "", tpParamHint(p)));
  if (inst.outputs.length) rows.push(row("Outputs", inst.outputs.map((x) => {
    const u = x.uuid && Object.keys(S.view.objects).find((k) => ciEq(k, x.uuid));
    return u ? `<button type="button" class="lnk" data-go="${esc(u)}">${esc(inst.path)}.${esc(x.name)}</button>` : `<span class="mono">${esc(inst.path)}.${esc(x.name)}</span>`;
  }).join(" ")));
  const path = constAt(top.file, top.line);
  if (path) rows.push(row("Source", `<span class="mono src-path">${esc(path)}</span><button type="button" class="copy" data-copy="${esc(path)}" data-label="Copy path" title="Copy the file and line">Copy path</button>${inst.body_line ? `<div class="src-how">Called in ${esc(chain[1]?.template || "")}'s body, line ${inst.body_line}.</div>` : ""}`));
  let count = 0;
  if (t?.changed && t.deployed) { count++; rows.push(`<div class="ed-note tp-pending">Not deployed: the template ${esc(t.name)} changed since the deployed config, so every instance of it did.</div>`); }
  if (top.change === "new") { count++; rows.push(`<div class="ed-note tp-pending">Not deployed: ${esc(top.path)} is new since the deployed config.</div>`); }
  else if (top.change === "call") { count++; rows.push(`<div class="ed-note tp-pending">Not deployed: ${esc(top.path)}'s call changed since the deployed config.</div>`); }
  if (!top.ours) {
    rows.push(`<div class="ed-note">Written in the module's own file: change it there (vislox writes only its own files).</div>`);
    return { rows, count: count || undefined };
  }
  const st = exState();
  if (!st) return { rows, count: count || undefined };
  if (!st.ok) { rows.push(`<div class="ed-note">${esc(st.why)}</div>`); return { rows, count: count || undefined }; }
  const tt = tplTemplate(top.template);
  if (!tt) { rows.push(`<div class="ed-note">The template ${esc(top.template)} is not in the module.</div>`); return { rows, count: count || undefined }; }
  const busy = S.tp.busy ? " disabled" : "";
  const revert = top.revert && JSON.stringify(top.revert.value) !== JSON.stringify(top.edit);
  rows.push(`<div class="tp-form" data-inst="${esc(top.path)}"><div class="tm">${esc(top.path)}: its arguments</div>${tpFields(tt, top.edit, "e:")}` +
    `<div class="ed-bar"><button type="button" class="ed-btn" data-tp="save"${busy}>Save</button>` +
    (revert && top.committed ? `<button type="button" class="ed-btn" data-tp="revert"${busy} title="Back to what the deployed revision has">Revert</button>` : "") +
    (!top.committed ? `<button type="button" class="ed-btn" data-tp="drop"${busy} title="Take this instance out again: it is not deployed">Remove instance</button>` : "") + `</div></div>`);
  if (S.tp.err) rows.push(`<div class="ed-msg err tp-err" role="alert">${esc(S.tp.err)}</div>`);
  return { rows, count: count || undefined };
}
/** A button of the Template section: `save`, `revert` or `drop`. */
async function tpAct(what) {
  tpSync();
  const st = exState(), hit = instOf(S.sel);
  if (!st?.ok || !hit || S.tp.busy) return;
  const top = instChain(hit.inst).pop();
  const t = tplTemplate(top.template);
  const page = top.revert?.page || top.page || S.page;
  let ops;
  if (what === "save") ops = [{ op: "instance", slug: top.path, page, value: tpCall(t, top.edit, "e:") }];
  else if (what === "revert") ops = [top.revert];
  else if (what === "drop") ops = [{ op: "instance", slug: top.path, page, value: null }];
  else return;
  S.tp.err = "";
  const uuid = S.sel;
  const r = await editSend(uuid, st.rev, ops);
  if (r === "failed") { S.tp.err = S.edit.err; S.edit.err = ""; renderProps(); return; }
  if (what !== "drop") {
    await S.editsLoad;
    const x = tplByPath(top.path), b = x?.blocks.find((y) => !y.expression) || x?.blocks[0];
    const u = b && Object.keys(S.view?.objects || {}).find((k) => ciEq(k, b.uuid));
    if (u) select(u, { noReveal: true });
  }
}
$("props").addEventListener("input", whenReady((e) => {
  if (e.target.dataset.tpField) { tpSync(); S.tp.draft[e.target.dataset.tpField] = e.target.value; }
}));
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-tp]");
  if (b) { e.stopPropagation(); tpAct(b.dataset.tp); }
}));
$("props").addEventListener("keydown", whenReady((e) => {
  if (e.target.dataset?.tpField && e.key === "Enter") { e.preventDefault(); tpAct("save"); }
}));
/** The template group a change goes under (#83), on the newest config: `tpl <Template>` when the change is to a
 *  block of an instance whose template (or an enclosing one) changed since the deployed config, which is what is
 *  compared; else `inst <path>`, the top-level instance. */
function chgTpl(c) {
  if (!c.obj) return null;
  const hit = instOf(c.obj);
  if (!hit) return null;
  const chain = instChain(hit.inst);
  if (S.base && S.base === deployedRev()?.id) {
    const changed = chain.find((x) => tplTemplate(x.template)?.changed && tplTemplate(x.template)?.deployed);
    if (changed) return `tpl ${changed.template}`;
  }
  return `inst ${chain[chain.length - 1].path}`;
}
/** The heading over the changes of one template's body (how many instances they are in), or of one call. */
function chgTplHead(k, count, list) {
  const [kind, name] = k.split(" ");
  if (kind === "tpl") {
    const t = tplTemplate(name);
    const inst = new Set(list.filter((c) => chgKey(c) === k).map((c) => instChain(instOf(c.obj).inst).pop().path));
    return `<div class="chg-grp chg-tpl" role="listitem" data-tpl="${esc(name)}"><span class="mono">${esc(name)}: ${plural(inst.size, "instance")} changed</span> · ${plural(count, "change")}${t?.file ? `<span class="p">${esc(constAt(t.file, t.line))}</span>` : ""}</div>`;
  }
  const x = tplByPath(name);
  if (!x) return "";
  const vsDeployed = S.base && S.base === deployedRev()?.id;
  const what = vsDeployed && x.change === "new" ? " (new)" : vsDeployed && x.change === "call" ? " (call changed)" : "";
  return `<div class="chg-grp chg-tpl" role="listitem" data-inst="${esc(x.path)}"><span class="mono">${esc(x.path)}: ${esc(x.template)}${what}</span> · ${plural(count, "change")}${x.file ? `<span class="p">${esc(constAt(x.file, x.line))}</span>` : ""}</div>`;
}
/** The templates the Add dialog offers (#83): the module's, then lox's library's it does not have. */
function tplOffered() {
  const d = tplData();
  if (!d) return [];
  const mine = d.templates.map((t) => ({ ...t, from: t.library ? `lox's library (${t.library}.loxl)` : constAt(t.file, t.line) }));
  const lib = d.library.filter((l) => !d.templates.some((t) => t.name === l.name)).map((l) => ({ ...l, from: `lox's library (${l.file}.loxl)`, lib: true }));
  return [...mine, ...lib];
}
/** A free name for a new instance of `t`: vx_ and the template's name, numbered. */
function tplNewSlug(t) {
  const stem = `vx_${t.name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "")}`;
  const taken = (s) => tplInstances().some((x) => x.path === s) || Object.values(S.edits?.sources || {}).some((m) => m.slug === s || m.slug.startsWith(s + "_"));
  for (let i = 1; ; i++) if (!taken(`${stem}_${i}`)) return `${stem}_${i}`;
}

// ── let constants and defaults (#81) ──────────────────────────────────
// lox writes a `let` constant's value, and a type's `defaults`, into each port that takes it, so the config keeps
// no trace of either. GET /api/edits says which port of which block takes its value from one (`constants.uses`,
// UUID lowercase → port key), on the newest config only, as `sources` (#64).
/** Where port `key` of block `uuid` takes its value from: { kind: "let"|"defaults", name, constant?, value, file,
 *  line, at_file, at_line, ours, at_ours }, or null. */
function constUse(uuid, key) { return managedShown() ? S.edits.constants?.uses?.[(uuid || "").toLowerCase()]?.[key] || null : null; }
function constLets() { return managedShown() ? S.edits.constants?.lets || [] : []; }
/** The `let` a port's value comes from, through a default too. */
function constLet(u) { return u && (u.kind === "let" ? u.name : u.constant) || null; }
/** How Properties names where a value comes from: `temp_schwelle`, `defaults ModbusASensor`. */
function constName(u) { return u.kind === "let" ? u.name : `defaults ${u.name}${u.constant ? `, ${u.constant}` : ""}`; }
function constAt(file, line) { return file ? `${file}${line ? ":" + line : ""}` : ""; }
/** How many ports take the value `u` names: the constant's (also through defaults), else the statement's. */
function constCount(u) {
  const l = constLet(u);
  if (l) return constLets().find((x) => x.name === l)?.uses || 0;
  return (S.edits?.constants?.defaults || []).find((d) => d.type === u.name)?.uses || 0;
}
function constTip(u) {
  const at = constAt(u.file, u.line);
  return u.kind === "let"
    ? `From the constant ${u.name} = ${u.value}${at ? ` (${at})` : ""}, which ${plural(constCount(u), "port")} take. lox writes its value into each one.`
    : `From defaults ${u.name}${at ? ` (${at})` : ""}: every block of the type that does not bind the port takes it${u.constant ? `, here the constant ${u.constant} = ${u.value}` : ""}.`;
}
/** The module's own `let` the server lists, with its line as the file has it (`source`). */
function constLetOf(name) { return constLets().find((l) => l.name === name) || null; }
/** Why this field takes no value at all: the server refuses both the constant (a text constant) and only this block
 *  (the module's own file binds the port); or null. */
function constAllRefused(o, field) {
  const [k, key] = field.split(/:(.*)/s);
  const u = k === "p" ? constUse(o.uuid, key) : null;
  const l = u?.kind === "let" ? constLetOf(u.name) : null;
  return l?.refusal && u.block_refusal ? `${l.refusal}; ${u.block_refusal}` : null;
}
/** The note and the choice under a field whose value a constant or default gives (#81). */
function constFieldRows(o, field, mine, kind) {
  const [k, key] = field.split(/:(.*)/s);
  if (k !== "p") return "";
  const u = constUse(o.uuid, key), id = `ed-${field.replace(/[^\w-]/g, "_")}`;
  const busy = mine.busy ? " disabled" : "";
  if (mine.mk?.field === field) {
    return `<div class="const-mk"><label for="ed-mk">Constant</label><input id="ed-mk" data-ed-field="mk" value="${esc(mine.draft.mk ?? mine.mk.name)}" autocomplete="off" spellcheck="false"${busy}>` +
      `<button type="button" class="primary small" data-ed="mk-save"${busy}>Make it</button><button type="button" class="ed-btn small" data-ed="mk-cancel"${busy}>Cancel</button>` +
      `<div class="const-why">vislox writes <span class="mono">let ${esc(mine.draft.mk ?? mine.mk.name)} = ${esc(mine.draft[field] ?? portNow(o, key))}</span> into ${esc(S.edits?.file || "its file")} and binds this port to it. Type its name into another block's field to bind that one too.</div></div>`;
  }
  if (!u) {
    return kind === "quantity" && S.edits?.dir ? `<button type="button" class="lnk const-mk-open" data-ed="mk-open" data-field="${esc(field)}"${busy} title="Name this value: a let constant in vislox's file that other blocks can take too">Make a constant</button>` : "";
  }
  const n = constCount(u), at = constAt(u.file, u.line);
  const from = `From <span class="mono">${esc(constName(u))}</span>${at ? ` (${esc(at)})` : ""}, ${plural(n, "use")}.`;
  // A constant, vislox's or the module's own (#81): the constant (its line rewritten in the module's file), or only
  // this block. The server says when this block cannot take a value of its own, and when the constant is not changed.
  if (u.kind === "let") {
    const l = constLetOf(u.name), whyConst = l?.refusal || null, whyBlock = u.block_refusal || null;
    const scope = whyConst ? "block" : whyBlock ? "constant" : mine.scope?.[field] || "constant";
    const radio = (v, label, why) => `<label class="const-scope"${why ? ` title="${esc(why)}"` : ""}><input type="radio" name="${id}-scope" data-ed-scope="${esc(field)}" value="${v}"${scope === v ? " checked" : ""}${busy || why ? " disabled" : ""}> ${label}</label>`;
    const where = u.ours ? "" : ` vislox rewrites its value on ${esc(constAt(u.file, u.line))} and nothing else.`;
    return `<div class="const-why">${from}${where}</div>` + radio("constant", `Change the constant (${plural(n, "use")})`, whyConst) +
      radio("block", "Only this block", whyBlock) + (whyBlock ? `<div class="const-why">${esc(whyBlock)}</div>` : whyConst ? `<div class="const-why">${esc(whyConst)}</div>` : "");
  }
  return `<div class="const-why">${from} Saving binds only this block's port; <b>Default…</b> under Parameters changes the default for every block of the type.</div>`;
}
/** A constant: a number, or one with a unit (#79); the server checks that the port takes the unit. */
const EDIT_QUANTITY = /^-?\d+(\.\d+)?(ms|s|min|h|%|K|°)?$/;
/** The fields of the edit form: [field, label, value now, kind, unit]. */
function editFields(o) {
  const f = [];
  // A block added here: its declaration takes the name, the constants and the settings lox takes there
  // (a Formula's text, a PulseAt's time, …); lox has no room or category for it.
  const nb = editsBlock(o.uuid);
  if (nb) {
    // A push notification's block (#77) takes the notification's name, and no constant: only its message.
    if (o.ty === "OutputRefLM") {
      for (const st of nb.settings || []) f.push([`s:${st.name}`, st.name === "MinimumTime" ? "Minimum time" : st.name, st.value ?? "", st.kind]);
      return f;
    }
    // An object of the object tree (#84): its name and settings; a number may carry its unit, and a setting lox
    // needs stays (the server refuses an empty one).
    if (nb.periphery) {
      f.push(["title", "Name", o.title || "", "text"]);
      for (const st of nb.settings || []) f.push([`s:${st.name}`, st.required ? `${st.name} (required)` : st.name, st.value ?? "", st.kind === "number" ? "quantity" : st.kind, st.unit, st.choices]);
      return f;
    }
    // A block added here may have nothing `caps()` offers (`o.edit` null): its declaration still takes these.
    f.push(["title", "Name", o.title || "", "text"]);
    for (const p of o.edit?.params || []) f.push([`p:${p.key}`, portName(o, p.key), portNow(o, p.key), "quantity", p.unit]);
    // A program (#79) starts from the one the block runs: Loxone Config's until the declaration names a file.
    for (const st of nb.settings || []) f.push([`s:${st.name}`, SET_LABEL[st.name] || st.name, st.value ?? (st.kind === "program" ? o.settings?.find((x) => x.name === st.name)?.value ?? "" : ""), st.kind, null, st.choices]);
    return f;
  }
  if (!o.edit) return f;
  // Statistics on a Tree or Air input or output (#85): on or off; another kind Loxone Config set shows as it is.
  const stats = o.edit.statistics ? ["statistics", "Statistics", o.edit.statistics, "stats"] : null;
  // Below a Tree device created here and not deployed, only the statistics: lox mints the UUIDs anew on every compile.
  if (hwBelowNew(o.uuid)) return stats ? [stats] : f;
  if (o.edit.title) f.push(["title", "Name", o.title || "", "text"]);
  if (o.edit.room) f.push(["room", "Room", o.room || "", "Place"]);
  if (o.edit.category) f.push(["category", "Category", o.category || "", "Category"]);
  for (const p of o.edit.params) f.push([`p:${p.key}`, portName(o, p.key), portNow(o, p.key), "quantity", p.unit]);
  for (const st of o.edit.settings) f.push([`s:${st.name}`, st.name, st.value ?? "", st.text ? "text" : "number"]);
  if (stats) f.push(stats);
  return f;
}
function editRows(o, row) {
  const mv = moveRows(o, row);
  const ad = adoptRows(o, row);
  const st = editState(o);
  if (!st) return mv.rows.length || ad.length ? { rows: [...ad, ...mv.rows], count: mv.count } : null;
  const rows = [...ad, ...mv.rows];
  const mine = S.edit.uuid === o.uuid ? S.edit : null;
  rows.push(...lifecycleRows(o, row, st));
  // A block added here and not deployed: Remove takes it out again.
  for (const b of editsAdded(o.uuid)) {
    rows.push(row("Block", `<span class="ed-v">added here</span><span class="ed-tag" title="Declared in ${esc(b.file)} and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      (st.ok ? `<button type="button" class="ed-btn small" data-ed="remove"${b.delete ? ` disabled title="${esc(b.delete)}"` : ' title="Take the block out again"'}>Remove</button>` : ""), "pr-chg"));
  }
  // What was edited here and is not deployed yet, each with a way back.
  for (const c of editsPending(o.uuid)) {
    const v = editValue(c);
    rows.push(row(editLabel(o, c), `<span class="ed-v">${v}</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      (st.ok ? `<button type="button" class="ed-btn small" data-ed="revert" data-key="${esc(editKey(c))}" title="${c.deployed == null ? "Hand the value back to Loxone Config" : c.name === "Code" ? "Back to the program as deployed" : `Back to ${esc(c.op === "entry_mode" ? mdModeTitle(c.deployed) : c.deployed)}, as deployed`}">Revert</button>` : ""), "pr-chg"));
  }
  // A constant of vislox's this block takes, changed and not deployed (#81): Revert puts back the deployed value;
  // one made here is dropped by reverting the ports bound to it.
  for (const c of constsPending(o)) {
    const back = c.deployed != null && st.ok
      ? `<button type="button" class="ed-btn small" data-ed="revert" data-key="${esc(editKey(c))}" title="Back to ${esc(c.deployed)}, as deployed, in every port bound to it">Revert</button>` : "";
    rows.push(row(`Constant ${c.name}`, `<span class="ed-v">${esc(c.value)}</span><span class="ed-tag" title="${c.deployed == null ? "Made here, not deployed: revert the ports bound to it to drop it" : `Deployed as ${esc(c.deployed)}`}">not deployed</span>${back}`, "pr-chg"));
  }
  if (!st.ok) {
    rows.push(`<div class="ed-note">${esc(st.why)}${st.newest ? ' <button type="button" class="lnk" data-ed="newest">View it</button>' : ""}</div>`);
  } else if (!mine?.open) {
    rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-ed="open">Edit values</button>` +
      (mine?.msg ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.msg)}</span>` : "") + "</div>");
  } else {
    for (const [field, label, now, kind, unit, choices] of editFields(o)) {
      const v = mine.draft[field] ?? now, bad = mine.bad[field];
      const id = `ed-${field.replace(/[^\w-]/g, "_")}`;
      // Neither the constant nor this block can take a value (the server's refusals, #81): read-only.
      const locked = constAllRefused(o, field);
      const attrs = `id="${id}" data-ed-field="${esc(field)}"${bad ? ` aria-invalid="true" aria-describedby="${id}-e"` : ""}${mine.busy || locked ? " disabled" : ""}${locked ? ` title="${esc(locked)}"` : ""}`;
      const input = kind === "Place" || kind === "Category"
        ? `<select ${attrs}>${editPlaces(kind).map(([t, n]) => `<option value="${esc(t)}"${t === v ? " selected" : ""}${n > 1 ? ' disabled title="Two of them have this title: rename one in Loxone Config to pick it"' : ""}>${esc(t)}</option>`).join("")}</select>`
        : kind === "choice" || kind === "on_off"
        ? `<select ${attrs}><option value="">as Loxone Config places it</option>${(kind === "on_off" ? ["on", "off"] : choices || []).map((c) => `<option${c === v ? " selected" : ""}>${esc(c)}</option>`).join("")}</select>`
        : kind === "stats"
        ? `<select ${attrs} title="${esc(STATS_TIP)}">${v === "on" || v === "off" ? "" : `<option value="${esc(v)}" selected disabled>${esc(v)}, as Loxone Config set it</option>`}${["on", "off"].map((c) => `<option${c === v ? " selected" : ""}>${c}</option>`).join("")}</select>`
        // After a line break: HTML drops the first one after <textarea>, so a program starting with a blank line keeps it.
        : kind === "program"
        ? `<textarea ${attrs} class="prog" rows="14" autocomplete="off" spellcheck="false" placeholder="Empty: Loxone Config's program">\n${esc(v)}</textarea>`
        : `<input ${attrs} value="${esc(v)}" ${kind === "number" || kind === "time" ? `inputmode="decimal" class="num"${kind === "time" ? ' placeholder="HH:MM"' : ""}` : kind === "quantity" ? `class="num" list="ed-consts" title="A number, or one with its unit: 30min, 250ms, 70%; or the name of a constant"` : kind === "duration" ? 'class="num" placeholder="5s"' : ""} autocomplete="off" spellcheck="false">`;
      rows.push(`<div class="pr ed-row${kind === "program" ? " ed-prog" : ""}"><label class="k" for="${id}">${esc(label)}</label><div class="v">${input}` +
        (unit ? `<span class="ed-unit">${esc(unit)}</span>` : "") + (bad ? `<div class="ed-bad" id="${id}-e">${esc(bad)}</div>` : "") + constFieldRows(o, field, mine, kind) + "</div></div>");
    }
    // The constants a port may be bound to (#81): typed by name into a field.
    if (constLets().length) rows.push(`<datalist id="ed-consts">${constLets().map((l) => `<option value="${esc(l.name)}">${esc(`= ${l.value}`)}</option>`).join("")}</datalist>`);
    rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-ed="save"${mine.busy ? " disabled" : ""}>${mine.busy ? "Compiling…" : "Save"}</button>` +
      `<button type="button" class="ed-btn small" data-ed="cancel"${mine.busy ? " disabled" : ""}>Cancel</button>` +
      `<span class="ed-msg${mine.err ? " err" : ""}" role="status" aria-live="polite">${esc(mine.err || mine.msg || "Saving compiles the module; nothing is deployed.")}</span></div>`);
  }
  return { rows, count: editsPending(o.uuid).length + constsPending(o).length + editsAdded(o.uuid).length + mv.count };
}
function editOpen(uuid) {
  S.edit = { uuid, open: true, draft: {}, bad: {}, busy: false, msg: "", err: "" };
  S.closed.delete("Edit");
  renderProps();
  $("props").querySelector("[data-ed-field]")?.focus();
}
function editClose() {
  if (S.edit.busy) return;
  S.edit = { uuid: S.edit.uuid, open: false, draft: {}, bad: {}, busy: false, msg: "", err: "" };
  renderProps();
  $("props").querySelector('[data-ed="open"]')?.focus();
}
const EDIT_NUMBER = /^-?\d+(\.\d+)?$/;
async function editSave() {
  const o = S.view?.objects[S.edit.uuid], st = o && editState(o);
  if (!st?.ok || S.edit.busy || !S.edit.open) return;
  const ops = [], bad = {}, added = !!editsBlock(o.uuid), lets = new Set(constLets().map((l) => l.name)), changed = new Map();
  for (const [field, , now, kind] of editFields(o)) {
    const v = S.edit.draft[field];
    if (v == null || v === now || constAllRefused(o, field)) continue;
    const t = kind === "number" || kind === "quantity" || kind === "time" || kind === "duration" ? v.trim() : v;
    const [k, key] = field.split(/:(.*)/s);
    // A constant's name binds the port to it (#81).
    if (k === "p" && lets.has(t)) { ops.push({ op: "param", uuid: o.uuid, port: key, value: t }); continue; }
    // A value of a constant changes the constant (vislox's, or the module's own on its line, #81), unless only this
    // block is to change; the server's reasons pick the scope where one of them is off.
    const u = k === "p" ? constUse(o.uuid, key) : null;
    const l = u?.kind === "let" ? constLetOf(u.name) : null;
    const scope = l?.refusal ? "block" : u?.block_refusal ? "constant" : S.edit.scope?.[field] || "constant";
    if (u?.kind === "let" && scope === "constant") {
      if (!EDIT_QUANTITY.test(t) && !AB_TIME.test(t) && !/^(on|off)$/.test(t)) { bad[field] = "A number, one with its unit (30min, 70%), a time of day (19:00), or on or off."; continue; }
      // Two fields of one constant: one value, or say which differ.
      const was = changed.get(u.name);
      if (was && was.value !== t) { bad[field] = `${u.name} is ${was.value} in ${was.label}: give both the same value, or change one for this block only.`; continue; }
      if (!was) ops.push(u.ours ? { op: "let", name: u.name, value: t } : { op: "let", name: u.name, value: t, source: l?.source ?? "" });
      changed.set(u.name, { value: t, label: portName(o, key) });
      continue;
    }
    // On a block added here an empty setting is dropped: Loxone Config's value again.
    if (added && k === "s" && !t.trim()) { ops.push({ op: "setting", uuid: o.uuid, name: key, value: null }); continue; }
    if (kind === "number" && !EDIT_NUMBER.test(t)) { bad[field] = /,/.test(t) ? "Write a decimal point, not a comma." : "A number, like 12 or -2.5."; continue; }
    if (kind === "quantity" && !EDIT_QUANTITY.test(t)) { bad[field] = /,/.test(t) ? "Write a decimal point, not a comma." : "A number, like 12 or -2.5, or one with its unit, like 30min."; continue; }
    if (kind === "time" && !AB_TIME.test(t)) { bad[field] = "A time of day, like 19:00."; continue; }
    if (kind === "duration" && !AB_DURATION.test(t.trim())) { bad[field] = "A duration, like 12h, 30min, 5s or 90 (seconds)."; continue; }
    if (field === "title" && !t.trim()) { bad[field] = "The name cannot be empty."; continue; }
    if (kind === "stats" && t !== "on" && t !== "off") continue;
    // A program the block already runs, sent back unchanged but for line endings, changes nothing.
    if (kind === "program" && t.replace(/\r\n/g, "\n") === (now ?? "").replace(/\r\n/g, "\n")) continue;
    ops.push(EDIT_ATTR[field] ? { op: field, uuid: o.uuid, value: t } : k === "p" ? { op: "param", uuid: o.uuid, port: key, value: t } : { op: "setting", uuid: o.uuid, name: key, value: t });
  }
  S.edit.bad = bad; S.edit.err = ""; S.edit.msg = "";
  if (Object.keys(bad).length) { S.edit.err = "Fix the marked values."; renderProps(); $("props").querySelector('[aria-invalid="true"]')?.focus(); return; }
  if (!ops.length) { S.edit.msg = "Nothing changed."; renderProps(); return; }
  await editSend(o.uuid, st.rev, ops);
}
/** Send the edits; on success show the new pending revision. Returns "ok", "failed" (the server refused
 *  or did not answer: nothing changed), or "shown" (compiled, but showing the new revision failed). */
async function editSend(uuid, rev, ops, label) {
  S.edit.busy = true; S.edit.uuid = uuid; renderProps();
  let res;
  try {
    res = await editAct(rev.blob, ops, label);
  } catch (err) {
    S.edit.busy = false; S.edit.err = err.message;
    renderProps();
    return "failed";
  }
  S.edit = { uuid, open: false, draft: {}, bad: {}, busy: false, msg: "Saved and compiled: pending, not deployed. Review it, then Apply or Discard.", err: "" };
  try {
    await editShowNewest(res.blob);
    if (S.sel === uuid && !S.view?.objects[uuid]) select(null, { noReveal: true });
  } catch (err) {
    S.edit.err = `Saved and compiled, but the page could not show it: ${err.message}`;
    renderProps();
    return "shown";
  }
  return "ok";
}
/** Make a constant (#81): a name for this port's value, `let vx_… = value` in vislox's file, and the port bound to
 *  it, in one request (undone together). */
function constMkOpen(field) {
  const o = S.view?.objects[S.edit.uuid];
  if (!o || S.edit.busy) return;
  const [, key] = field.split(/:(.*)/s);
  const taken = new Set(constLets().map((l) => l.name));
  const stem = `vx_${(portName(o, key) || key).toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") || "wert"}`;
  let name = stem;
  for (let i = 2; taken.has(name); i++) name = `${stem}_${i}`;
  S.edit.mk = { field, name };
  delete S.edit.draft.mk;
  renderProps();
  $("ed-mk")?.focus();
}
function constMkClose() {
  if (S.edit.busy) return;
  S.edit.mk = null;
  delete S.edit.draft.mk;
  renderProps();
}
async function constMake() {
  const o = S.view?.objects[S.edit.uuid], st = o && editState(o), mk = S.edit.mk;
  if (!st?.ok || !mk || S.edit.busy) return;
  const [, key] = mk.field.split(/:(.*)/s);
  const name = (S.edit.draft.mk ?? mk.name).trim(), value = (S.edit.draft[mk.field] ?? portNow(o, key)).trim();
  S.edit.bad = {};
  if (!/^vx_[a-z0-9_]+$/.test(name)) S.edit.bad[mk.field] = "vislox's constants are named vx_ and then lowercase letters, digits and _.";
  else if (!EDIT_QUANTITY.test(value)) S.edit.bad[mk.field] = "A number, like 12 or -2.5, or one with its unit, like 30min.";
  if (S.edit.bad[mk.field]) { S.edit.err = "Fix the marked values."; renderProps(); return; }
  await editSend(o.uuid, st.rev, [{ op: "let", name, value }, { op: "param", uuid: o.uuid, port: key, value: name }]);
}
// ── the module's own `let` and `defaults` values, and defaults made here (#81) ─────────────
// vislox rewrites one value of the module's own `let` or `defaults` line and nothing else, naming the line as the
// server listed it (`source`): a file changed since is refused. A default changes every managed block of the type
// that does not bind the port, the module's own among them, so *Default…* lists them first (GET /api/defaults) and
// the op names exactly those blocks (`blocks`), as Apply's consent to removals does.
const DF = { uuid: null, key: null, info: null, value: "", busy: false, err: "" };
/** The module's values not deployed (`constants.pending`): a constant's, or a default's per port. */
function constPending() { return managedShown() ? S.edits.constants?.pending || [] : []; }
/** What names one of them across a refresh of the list: its kind, constant or type, and port. */
function constPendingKey(p) { return [p.kind, p.name, p.port || ""].join("|"); }
/** *Default…* on a parameter of a managed block whose value no constant gives. */
function dfltButton(o, q) {
  if (!S.edits?.sources?.[o.uuid] || !constEditState().ok || DF.uuid === o.uuid && DF.key === q.key) return "";
  const u = constUse(o.uuid, q.key);
  if (u && u.kind !== "defaults") return "";
  return ` <button type="button" class="lnk const-dflt-open" data-dflt="open" data-key="${esc(q.key)}" title="${u ? `Change defaults ${esc(u.name)} for every block of the type that takes it` : `Make this value the default of every ${esc(o.ty)} that does not set ${esc(q.name)} itself`}">Default…</button>`;
}
/** The pending values of the module's constants and defaults this block takes, each with Revert. */
function constPendingRows(o, row) {
  const rows = [], st = constEditState();
  for (const p of constPending()) {
    const takes = p.kind === "let" ? o.ports.some((q) => constLet(constUse(o.uuid, q.key)) === p.name)
      : p.blocks.some((b) => ciEq(b.uuid, o.uuid));
    if (!takes) continue;
    const key = constPendingKey(p);
    const what = p.kind === "let" ? `Constant ${p.name}` : `Default ${p.name} ${p.port}`;
    const was = p.deployed == null ? "new, not deployed" : `deployed as ${p.deployed}`;
    const reach = p.kind === "defaults" ? ` · changes ${plural(p.blocks.length, "block")}: ${dfltList(p.blocks)}` : ` · ${plural(p.uses, "use")}`;
    const back = st.ok && p.revert ? `<button type="button" class="ed-btn small" data-dflt="revert" data-key="${esc(key)}" title="${p.deployed == null ? "Take the default out of vislox's file" : `Back to ${esc(p.deployed)}, as deployed`}">Revert</button>` : "";
    rows.push(row(what, `<span class="ed-v">${esc(p.value ?? "(none)")}</span><span class="ed-tag" title="${esc(`${was}${reach}`)}">not deployed</span>${back}<div class="const-why">${esc(`${constAt(p.file, p.line)}${reach}`)}</div>`, "pr-chg"));
  }
  return rows;
}
/** The open *Default…* panel of this block. */
function dfltRows(o) {
  if (DF.uuid !== o.uuid) return [];
  const q = o.ports.find((x) => x.key === DF.key), i = DF.info, busy = DF.busy || !constEditState().ok;
  if (!q) return [];
  const head = `<span class="mono">defaults ${esc(o.ty)}(${esc(i?.port || q.name)}: …)</span>`;
  if (!i) return [`<div class="const-dflt" role="group" aria-label="Default">${head} <span class="const-why">${esc(DF.err || "Working out which blocks it changes…")}</span>${DF.err ? ' <button type="button" class="ed-btn small" data-dflt="cancel">Close</button>' : ""}</div>`];
  const st = i.statement;
  const where = st ? `${st.ours ? "vislox's statement" : "the module's own statement, rewritten on its line"} in ${constAt(st.file, st.line)}` : `a new statement in ${S.edits?.file || "vislox's file"}`;
  const label = st && st.value != null ? `Change the default (${plural(i.blocks.length, "block")})` : `Make it the default (${plural(i.blocks.length, "block")})`;
  return [`<div class="const-dflt" role="group" aria-label="Default">${head}` +
    `<div class="const-why">${esc(`${where}. It changes every ${o.ty} that does not set ${i.port} itself, the module's own blocks among them: ${plural(i.blocks.length, "block")}: ${dfltList(i.blocks)}.`)}</div>` +
    (i.refusal ? `<div class="const-why err">${esc(i.refusal)}</div>` : "") +
    `<label for="dflt-v">Default</label><input id="dflt-v" class="num" data-dflt-field="value" value="${esc(DF.value)}" autocomplete="off" spellcheck="false"${busy || i.refusal ? " disabled" : ""}>` +
    `<button type="button" class="primary small" data-dflt="save"${busy || i.refusal ? " disabled" : ""}>${DF.busy ? "Compiling…" : esc(label)}</button>` +
    `<button type="button" class="ed-btn small" data-dflt="cancel"${DF.busy ? " disabled" : ""}>Cancel</button>` +
    `<span class="ed-msg${DF.err ? " err" : ""}" role="status" aria-live="polite">${esc(DF.err || "Saving compiles the module; nothing is deployed.")}</span></div>`];
}
async function dfltOpen(key) {
  const o = S.view?.objects[S.sel];
  if (!o) return;
  Object.assign(DF, { uuid: o.uuid, key, info: null, value: portNow(o, key), busy: false, err: "" });
  renderProps();
  try {
    const i = await api(`/api/defaults?type=${encodeURIComponent(o.ty)}&port=${encodeURIComponent(key)}`);
    if (DF.uuid !== o.uuid || DF.key !== key) return;
    DF.info = i;
    if (i.statement?.value != null) DF.value = i.statement.value;
  } catch (err) {
    if (DF.uuid === o.uuid && DF.key === key) DF.err = err.message;
  }
  renderProps();
  $("dflt-v")?.focus();
}
function dfltClose() { if (DF.busy) return; Object.assign(DF, { uuid: null, key: null, info: null, busy: false, err: "" }); renderProps(); }
async function dfltSend(ops, label) {
  const st = constEditState();
  if (!st.ok) { DF.err = st.why; renderProps(); return false; }
  DF.busy = true; DF.err = ""; renderProps();
  try {
    const res = await editAct(st.rev.blob, ops, label);
    await editShowNewest(res.blob);
    return true;
  } catch (err) {
    DF.err = err.message;
    return false;
  } finally {
    DF.busy = false;
    renderProps();
  }
}
async function dfltSave() {
  const o = S.view?.objects[DF.uuid], i = DF.info;
  if (!o || !i || DF.busy || i.refusal) return;
  const value = DF.value.trim();
  if (!value) { DF.err = "A value: a number, one with its unit (30min), on or off, or a constant's name."; renderProps(); return; }
  const op = { op: "defaults", type: i.type, port: i.key, value, blocks: i.blocks.map((b) => b.slug) };
  if (i.statement && !i.statement.ours) op.source = i.statement.source;
  if (await dfltSend([op], `Default ${i.type} ${i.port} = ${value}`)) {
    Object.assign(DF, { uuid: null, key: null, info: null });
  } else if (/Confirm exactly these/.test(DF.err)) {
    // The blocks it changes are no longer the ones listed: ask the server again and show them for consent.
    const was = i.blocks.map((b) => b.slug).join(), uuid = DF.uuid, key = DF.key;
    try {
      const fresh = await api(`/api/defaults?type=${encodeURIComponent(i.type)}&port=${encodeURIComponent(i.key)}`);
      if (DF.uuid === uuid && DF.key === key) {
        DF.info = fresh;
        DF.err = fresh.blocks.map((b) => b.slug).join() === was ? DF.err
          : `The blocks it changes are not the ones listed before: now ${plural(fresh.blocks.length, "block")}: ${dfltList(fresh.blocks)}. Save again to confirm these.`;
      }
    } catch (err) {
      DF.err = err.message;
    }
  }
  renderProps();
}
async function dfltRevert(key) {
  const p = constPending().find((x) => constPendingKey(x) === key);
  if (!p?.revert) return;
  await dfltSend([p.revert], p.kind === "let" ? `Revert the constant ${p.name}` : `Revert the default ${p.name} ${p.port}`);
}
/** Back to the deployed value: claim what the deployed revision claimed, or nothing. */
async function editRevert(key) {
  const c = (S.edits?.claims || []).find((x) => editKey(x) === key && x.pending);
  // A constant (#81) is reverted from a block that takes it: the one selected.
  const o = c && S.view?.objects[c.op === "let" ? S.sel : c.uuid], st = o && editState(o);
  if (!st?.ok) return;
  if (c.op === "let") { await editSend(o.uuid, st.rev, [{ op: "let", name: c.name, value: c.deployed }], `Revert the constant ${c.name}`); return; }
  const op = EDIT_ATTR[c.op] ? { op: c.op, uuid: c.uuid } : c.op === "param" ? { op: "param", uuid: c.uuid, port: c.port } : { op: "setting", uuid: c.uuid, name: c.name };
  await editSend(c.uuid, st.rev, [{ ...op, value: c.deployed }], `Revert ${editLabel(o, c)} of ${displayTitle(o)}`);
}
// ── switching times (#73) ──────────────────────────────────────────────
// A DayTimer's switching times are a Lox Logic `schedule` set and the timer's `.schedule` claim, written into
// vislox's file of claims. The editor shows one bar per operating mode, as the Schedule section does: drag an
// interval to move it, its ends to resize it, click the empty bar to add one; the selected mode's intervals
// below take exact times, a value on an analog timer, and *activate*. Saving sends one `schedule` op: every
// column, by its mode's UUID. It is an edit like any other: compiled, never deployed, undone through `undo`.
const SE = { uuid: null, open: false, cols: [], sel: null, shareAll: true, busy: false, msg: "", err: "", bad: {} };
/** The built-in operating modes: the `N` of their fixed UUID → name and day word. */
const SE_BUILTIN = {
  4: ["Monday", "mon"], 5: ["Tuesday", "tue"], 6: ["Wednesday", "wed"], 7: ["Thursday", "thu"], 8: ["Friday", "fri"],
  9: ["Saturday", "sat"], 10: ["Sunday", "sun"], 1: ["Holiday", "holiday"], 2: ["Vacation", "vacation"], 3: ["Day off", "dayoff"],
  11: ["Heating period", "heating"], 12: ["Cooling period", "cooling"],
};
const seUuid = (n) => `00000000-0000-${n.toString(16).padStart(4, "0")}-1500000000000000`;
const SE_WEEK = [4, 5, 6, 7, 8, 9, 10].map(seUuid);
const SE_GROUPS = { weekdays: [4, 5, 6, 7, 8].map(seUuid), weekend: [9, 10].map(seUuid), daily: SE_WEEK };
/** The built-in mode number of `uuid`, or null. */
function seBuiltin(uuid) { const m = /^00000000-0000-([0-9a-f]{4})-1500000000000000$/i.exec(uuid || ""); const n = m && parseInt(m[1], 16); return n && SE_BUILTIN[n] ? n : null; }
/** A column's name: the config's title of its mode, else the built-in name; and the word lox writes. */
function seName(uuid) {
  const n = seBuiltin(uuid), o = S.view?.objects[uuid];
  if (n) return { title: o?.title && o.title !== SE_BUILTIN[n][0] ? `${SE_BUILTIN[n][0]} (${o.title})` : SE_BUILTIN[n][0], word: SE_BUILTIN[n][1] };
  return { title: o?.title || uuid, word: o ? `"${o.title}"` : uuid };
}
/** The operating modes a column can be added for: the other built-in modes (heating and cooling only when the
 *  config has them, as lox requires), then the config's own modes. */
function seModes() {
  const out = [1, 2, 3].map(seUuid);
  for (const n of [11, 12]) if (S.view?.objects[seUuid(n)]) out.push(seUuid(n));
  const own = Object.values(S.view?.objects || {}).filter((o) => o.ty === "Mode" && !seBuiltin(o.uuid) && o.status !== "removed");
  own.sort((a, b) => a.title.localeCompare(b.title));
  return [...out, ...own.map((o) => o.uuid)];
}
/** What `GET /api/edits` says of this timer's switching times (#73), or null. */
function schedEdit(uuid) { return (S.edits?.schedules || []).find((s) => s.uuid === uuid) || null; }
/** The timers whose switching times were edited here and are not deployed. */
function schedsPending() { return (S.edits?.schedules || []).filter((s) => s.pending); }
/** The minutes of `hh:mm` (`24:00` is 1440), or null. */
function seMin(t) {
  const m = /^(\d{1,2}):(\d{2})$/.exec((t || "").trim());
  if (!m) return null;
  const v = +m[1] * 60 + +m[2];
  return +m[2] < 60 && v <= 1440 ? v : null;
}
/** The draft from the schedule the config shown has: the seven weekdays always (a weekday without a column is
 *  off all day once claimed), then the timer's other columns. */
function seDraft(si, from) {
  const cols = [...SE_WEEK];
  for (const c of si.cols) if (!cols.includes(c.col.toLowerCase()) && !c.col.startsWith("#")) cols.push(c.col.toLowerCase());
  return cols.map((uuid) => ({
    uuid,
    spans: (from || si).entries.filter((e) => e.col.toLowerCase() === uuid).sort(byTime)
      .map((e) => ({ from: e.from, to: e.to, value: si.analog ? String(e.value) : undefined, activate: !!e.activate })),
  }));
}
/** Whether this timer's switching times can be edited here now: `{ ok, rev }`, or null (nothing to show). */
function seState(o) {
  if (o.ty !== "DayTimer" || !o.schedule) return null;
  const st = editState(o);
  return st?.ok ? st : null;
}
function seOpen(uuid) {
  const o = S.view?.objects[uuid];
  if (!o?.schedule) return;
  Object.assign(SE, { uuid, open: true, cols: seDraft(o.schedule), sel: SE_WEEK[0], shareAll: true, busy: false, msg: "", err: "", bad: {} });
  S.closed.delete("Switching times");
  renderProps();
  $("props").querySelector(".se-row.sel .se-mode")?.focus();
}
function seClose() {
  if (SE.busy) return;
  Object.assign(SE, { open: false, cols: [], msg: "", err: "", bad: {} });
  renderProps();
  $("props").querySelector('[data-se="open"]')?.focus();
}
const seCol = (uuid) => SE.cols.find((c) => c.uuid === uuid);
/** One column's bar: each interval a segment with a handle at either end. */
function seBar(c, analog) {
  return c.spans.map((s, i) => {
    const l = (s.from / 1440) * 100, w = Math.max(((s.to - s.from) / 1440) * 100, 0.6);
    const tip = `${hhmm(s.from)}–${hhmm(s.to)}${analog ? ` · ${s.value ?? "?"}` : ""}${s.activate ? " · needs activation" : ""}`;
    return `<span class="se-seg${s.activate ? " act" : ""}" data-col="${esc(c.uuid)}" data-i="${i}" style="left:${l.toFixed(2)}%;width:${w.toFixed(2)}%" title="${esc(tip)}"><i class="se-h l" data-end="from"></i>${analog && w > 9 ? esc(s.value ?? "") : ""}<i class="se-h r" data-end="to"></i></span>`;
  }).join("");
}
function seRefreshBars(analog) {
  for (const el of $("props").querySelectorAll("[data-se-bar]")) {
    const c = seCol(el.dataset.seBar);
    if (c) el.innerHTML = seBar(c, analog);
  }
}
/** The rows of the *Switching times* section. */
function seRows(o, row) {
  const st = seState(o);
  if (!st) return null;
  // The rest of the module claims them: changed there (a timer takes one schedule).
  const other = claimsOf(o.uuid).find((c) => c.kind === "schedule" && !c.file.startsWith(`${S.edits?.dir}/`));
  if (other) {
    const at = `${other.file}:${other.line}`, mine = schedEdit(o.uuid)?.set ? SE.uuid === o.uuid ? SE : {} : null;
    if (!mine) return { rows: [`<div class="ed-note">${esc(at)} claims its switching times: change them there.</div>`] };
    // Claimed twice: lox compiles neither until one claim goes. vislox's goes with one click.
    return { rows: [`<div class="sch-note warn">⚠ Claimed twice: ${esc(at)} and vislox's ${esc(S.edits?.file || "")} both claim its switching times, and lox compiles neither until one goes.</div>`,
      `<div class="ed-bar"><button type="button" class="ed-btn small" data-se="drop"${mine.busy ? " disabled" : ""} title="Take vislox's claim out and compile: ${esc(at)} keeps the timer">Remove vislox's claim</button>` +
      (mine.msg || mine.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "") + "</div>"], count: 1 };
  }
  const si = o.schedule, e = schedEdit(o.uuid), mine = SE.uuid === o.uuid ? SE : null, rows = [];
  const live = liveStreaming() ? liveSchedule(si) : undefined, cmp = live ? schedCompare(si, live) : null;
  const title = (u) => { const x = S.view?.objects[u]; return x ? displayTitle(x) : u; };
  if (e?.set) {
    rows.push(`<div class="ed-note">Written by vislox as <span class="mono">${esc(e.set)}</span> in ${esc(S.edits?.file || "")}${e.shared?.length ? `, shared with ${esc(e.shared.map(title).join(", "))}` : ""}.</div>`);
    rows.push(...staleRows((S.edits?.schedule_sets || []).find((x) => x.name === e.set), "se", mine));
  }
  if (e?.pending) {
    const back = e.deployed ? "Back to the switching times deployed" : "Back to Loxone Config's switching times";
    rows.push(row("Switching times", `<span class="ed-v">edited here</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      `<button type="button" class="ed-btn small" data-se="revert" title="${esc(back)}">Revert</button>`, "pr-chg"));
  }
  // The Miniserver runs other switching times: edited in the app, or not applied yet. A save, once applied, replaces them.
  if (cmp && !cmp.same) rows.push(`<div class="sch-note warn">⚠ The Miniserver runs other switching times (${esc(cmp.what)}), maybe changed in the app. Saving here and applying replaces them.</div>`);
  if (!mine?.open) {
    // Only sets of timers of the same kind: the server refuses an analog timer on a digital set and back.
    const kind = (x) => S.view?.objects[x.timers[0]]?.schedule?.analog;
    const sets = (S.edits?.schedule_sets || []).filter((x) => x.name !== e?.set && x.timers.length && !x.timers.includes(o.uuid) && kind(x) === si.analog);
    rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-se="open">Edit switching times</button>` +
      (sets.length ? `<label class="se-same">Use the same times as <select data-se="same"${mine?.busy ? " disabled" : ""}><option value="">…</option>${sets.map((x) => `<option value="${esc(x.name)}">${esc(x.timers.map(title).join(", "))}</option>`).join("")}</select></label>` : "") +
      (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "") + "</div>");
    return { rows, count: e?.pending ? 1 : 0 };
  }
  const analog = si.analog, dis = mine.busy ? " disabled" : "";
  const used = new Set(SE.cols.map((c) => c.uuid));
  rows.push(`<div class="se" data-analog="${analog}"><div class="sch-axis"><span></span><span class="sch-hours"><i>0</i><i>6</i><i>12</i><i>18</i><i>24 h</i></span></div>` +
    SE.cols.map((c) => {
      const n = seName(c.uuid), week = SE_WEEK.includes(c.uuid);
      return `<div class="se-row sch-row${SE.sel === c.uuid ? " sel" : ""}${mine.bad[c.uuid] ? " bad" : ""}"><button type="button" class="se-mode sch-mode lnk" data-se="pick" data-col="${esc(c.uuid)}" title="${esc(`${n.title} (${n.word} in Lox Logic)`)}" aria-pressed="${SE.sel === c.uuid}">${esc(n.title)}</button>` +
        `<div class="se-cell"><div class="se-bar sch-bar" data-se-bar="${esc(c.uuid)}" title="Click to add an interval; drag one to move it, its ends to resize it">${seBar(c, analog)}</div>` +
        (week ? "" : `<button type="button" class="se-x" data-se="dropcol" data-col="${esc(c.uuid)}" title="No column for this mode: on such a day the timer follows the weekday"${dis}>✕</button>`) + `</div></div>`;
    }).join("") + `</div>`);
  const extra = seModes().filter((u) => !used.has(u));
  if (extra.length) rows.push(`<div class="ed-bar"><label class="se-same">Add a column for <select data-se="addcol"${dis}><option value="">…</option>${extra.map((u) => `<option value="${esc(u)}">${esc(seName(u).title)}</option>`).join("")}</select></label></div>`);
  const c = seCol(SE.sel);
  if (c) {
    const n = seName(c.uuid);
    const spans = c.spans.map((s, i) => {
      const k = `${c.uuid}|${i}`;
      const f = (end) => `<input class="num se-t" data-sch-field="${esc(k)}|${end}" value="${esc(hhmm(s[end]))}" aria-label="${end === "from" ? "From" : "To"}" placeholder="HH:MM" inputmode="numeric" autocomplete="off"${dis}>`;
      return `<div class="se-iv${mine.bad[k] ? " bad" : ""}">${f("from")}<span>–</span>${f("to")}` +
        (analog ? `<input class="num se-v" data-sch-field="${esc(k)}|value" value="${esc(s.value ?? "")}" aria-label="Value" placeholder="value" inputmode="decimal" autocomplete="off"${dis}>` : "") +
        `<label class="se-act" title="It switches on only after a pulse on the timer's activation input (Loxone Config's 'Activation required')"><input type="checkbox" data-se="act" data-col="${esc(c.uuid)}" data-i="${i}"${s.activate ? " checked" : ""}${dis}> activate</label>` +
        `<button type="button" class="ed-btn small" data-se="del" data-col="${esc(c.uuid)}" data-i="${i}" aria-label="Delete ${esc(hhmm(s.from))}–${esc(hhmm(s.to))}"${dis}>Delete</button>` +
        (mine.bad[k] ? `<div class="ed-bad">${esc(mine.bad[k])}</div>` : "") + `</div>`;
    }).join("");
    const copy = Object.keys(SE_GROUPS).map((g) => `<button type="button" class="ed-btn small" data-se="copy" data-to="${g}"${dis}>${g === "daily" ? "Every day" : g === "weekdays" ? "Weekdays" : "Weekend"}</button>`).join("");
    rows.push(`<div class="se-detail"><div class="se-dh">${esc(n.title)} <span class="none-v">${c.spans.length ? "" : "off all day"}</span></div>${spans}` +
      `<div class="ed-bar"><button type="button" class="ed-btn small" data-se="add"${dis}>Add interval</button><span class="se-copy">Copy to ${copy}</span></div></div>`);
  }
  const others = Object.values(S.view?.objects || {}).filter((x) => x.ty === "DayTimer" && x.uuid !== o.uuid && x.schedule && x.schedule.analog === analog && x.status !== "removed");
  if (others.length) rows.push(`<div class="ed-bar"><label class="se-same">Start from the times of <select data-se="copyfrom"${dis}><option value="">…</option>${others.map((x) => `<option value="${esc(x.uuid)}">${esc(displayTitle(x))}</option>`).join("")}</select></label></div>`);
  if (e?.shared?.length) rows.push(`<label class="se-share"><input type="checkbox" data-se="shareall"${SE.shareAll ? " checked" : ""}${dis}> Change ${esc(e.shared.map(title).join(", "))} too: ${e.shared.length === 1 ? "it shares" : "they share"} these switching times (${esc(e.set)}). Off: this timer gets a set of its own.</label>`);
  rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-se="save"${dis}>${mine.busy ? "Compiling…" : "Save"}</button>` +
    `<button type="button" class="ed-btn small" data-se="cancel"${dis}>Cancel</button>` +
    `<span class="ed-msg${mine.err ? " err" : ""}" role="status" aria-live="polite">${esc(mine.err || mine.msg || "Saving writes a schedule set into the module and compiles it; nothing is deployed. The claim replaces the timer's whole schedule.")}</span></div>`);
  return { rows, count: e?.pending ? 1 : 0 };
}
/** Check the draft as lox would: times, order, overlaps, values. Marks what is wrong; true when all is well. */
function seCheck(analog) {
  const bad = {};
  for (const c of SE.cols) {
    c.spans.forEach((s, i) => {
      const k = `${c.uuid}|${i}`;
      if (s.from == null || s.to == null) bad[k] = "A time of day, like 06:30 (24:00 ends the day).";
      else if (s.from >= s.to) bad[k] = "It ends before it starts: split an interval across midnight into two.";
      else if (analog && !EDIT_NUMBER.test((s.value ?? "").trim())) bad[k] = /,/.test(s.value || "") ? "Write a decimal point, not a comma." : "An analog timer needs a value, like 21.5.";
    });
    const v = c.spans.map((s, i) => [s, i]).filter(([s]) => s.from != null && s.to != null).sort((a, b) => a[0].from - b[0].from);
    for (let j = 1; j < v.length; j++) if (v[j][0].from < v[j - 1][0].to) bad[`${c.uuid}|${v[j][1]}`] ??= `Overlaps ${hhmm(v[j - 1][0].from)}–${hhmm(v[j - 1][0].to)}.`;
    if (Object.keys(bad).some((k) => k.startsWith(c.uuid + "|"))) bad[c.uuid] = true;
  }
  SE.bad = bad;
  return !Object.keys(bad).length;
}
async function seSave() {
  const o = S.view?.objects[SE.uuid], st = o && seState(o);
  if (!st || SE.busy || !SE.open) return;
  const analog = o.schedule.analog;
  if (!seCheck(analog)) {
    SE.err = "Fix the marked intervals.";
    const bad = Object.keys(SE.bad).find((k) => k.includes("|"));
    if (bad) SE.sel = bad.split("|")[0];
    renderProps();
    $("props").querySelector(".se-iv.bad input")?.focus();
    return;
  }
  const modes = {};
  for (const c of SE.cols) modes[c.uuid] = [...c.spans].sort((a, b) => a.from - b.from)
    .map((s) => ({ from: s.from, to: s.to, ...(analog ? { value: s.value.trim() } : {}), ...(s.activate ? { activate: true } : {}) }));
  const e = schedEdit(o.uuid);
  const value = { modes, ...(e?.set && e.shared?.length && SE.shareAll ? { set: e.set } : {}) };
  await seSend(o, st, { op: "schedule", uuid: o.uuid, value });
}
async function seSend(o, st, op, label) {
  SE.uuid = o.uuid; SE.busy = true; SE.err = ""; SE.msg = ""; renderProps();
  try {
    const res = await editAct(st.rev.blob, [op], label);
    Object.assign(SE, { open: false, cols: [], busy: false, bad: {}, err: "", msg: res.unchanged ? "Nothing changed." : "Saved and compiled: pending, not deployed. Review it, then Apply or Discard." });
    await editShowNewest(res.blob);
  } catch (err) {
    SE.busy = false; SE.err = err.message;
  }
  renderProps();
}
/** What Revert sends for a block's set claim of `kind` (`schedule`, `moods`, `access`): `{ value }`, or `{ why }`.
 *  `e` is the block's entry of `GET /api/edits`, `sets` that kind's sets; the keys come from the server
 *  (`set_kinds`). Back to the deployed content, for this block only: the deployed set with its content while no
 *  other block claims it, else the deployed content in a set of its own, never a rewrite of the others'. No claim
 *  deployed: vislox's claim goes. When the deployed content cannot be read against this config, Revert is refused
 *  with the reason: sending the set's name alone would claim it as it is now, and change nothing. */
function setRevertValue(kind, e, sets, uuid) {
  const k = S.edits?.set_kinds?.[kind];
  const d = e?.deployed ?? null;
  if (!d) return { value: null };
  if (!k) return { why: "The server did not say how to revert this." };
  const others = (sets.find((x) => x.name === d.set)?.[k.members] || []).filter((u) => u !== uuid);
  if (d[k.content] == null) {
    const names = others.map((u) => { const x = S.view?.objects[u]; return x ? displayTitle(x) : u; });
    return { why: `The deployed ${k.what} (${d.set}) cannot be read against this config${names.length ? `, and ${names.join(", ")} ${others.length === 1 ? "claims" : "claim"} that set now` : ""}: revert in the module, or Discard.` };
  }
  return { value: others.length ? { [k.content]: d[k.content] } : { set: d.set, [k.content]: d[k.content] } };
}
async function seRevert(uuid) {
  const o = S.view?.objects[uuid], st = o && seState(o), e = schedEdit(uuid);
  if (!st || !e) return;
  const r = setRevertValue("schedule", e, S.edits?.schedule_sets || [], uuid);
  if (r.why) { SE.uuid = uuid; SE.err = r.why; SE.msg = ""; renderProps(); return; }
  await seSend(o, st, { op: "schedule", uuid, value: r.value }, `Revert the switching times of ${displayTitle(o)}`);
}
/** Take vislox's claim out of a timer the module's own file claims too. */
async function seDrop(uuid) {
  const o = S.view?.objects[uuid], st = o && seState(o);
  if (!st) return;
  await seSend(o, st, { op: "schedule", uuid, value: null }, `Remove vislox's claim on the switching times of ${displayTitle(o)}`);
}
async function seSame(uuid, set) {
  const o = S.view?.objects[uuid], st = o && seState(o);
  if (!st || !set) return;
  await seSend(o, st, { op: "schedule", uuid, value: { set } });
}
/** A free hour (or what is left of it before the next interval) from minute `at` of column `c`, no later than
 *  23:00; null when that start is inside an interval or no minute is free after it. */
function seFree(c, at) {
  const from = Math.max(0, Math.min(at, 1380));
  if (c.spans.some((s) => s.from <= from && from < s.to)) return null;
  const next = Math.min(...c.spans.filter((s) => s.from > from).map((s) => s.from), 1440);
  const to = Math.min(from + 60, next);
  return to > from ? { from, to } : null;
}
function seAdd(c, at, analog) {
  const free = seFree(c, at) ?? (() => { for (let m = 0; m < 1440; m += 60) { const f = seFree(c, m); if (f) return f; } return null; })();
  if (!free) { SE.err = "This day has no free hour left."; return null; }
  const last = c.spans.find((s) => s.value != null)?.value;
  const s = { ...free, value: analog ? (last ?? "") : undefined, activate: false };
  c.spans.push(s);
  c.spans.sort((a, b) => a.from - b.from);
  return s;
}
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-se]");
  if (!b || b.tagName === "SELECT" || b.type === "checkbox") return;
  e.stopPropagation();
  const o = S.view?.objects[S.sel], analog = !!o?.schedule?.analog, what = b.dataset.se;
  if (what === "open") return seOpen(S.sel);
  if (what === "cancel") return seClose();
  if (what === "save") return seSave();
  if (what === "revert") return seRevert(S.sel);
  if (what === "drop") return seDrop(S.sel);
  if (what === "dropstale") { const o = S.view?.objects[S.sel], st = o && seState(o); if (st) seSend(o, st, { op: "schedule", uuid: b.dataset.u, value: null }, "Drop a stale claim on switching times"); return; }
  if (SE.busy || !SE.open) return;
  const c = b.dataset.col ? seCol(b.dataset.col) : seCol(SE.sel);
  if (what === "pick") SE.sel = b.dataset.col;
  else if (what === "dropcol") { SE.cols = SE.cols.filter((x) => x.uuid !== b.dataset.col); if (SE.sel === b.dataset.col) SE.sel = SE_WEEK[0]; }
  else if (what === "del" && c) c.spans.splice(+b.dataset.i, 1);
  else if (what === "add" && c) seAdd(c, 360, analog);
  else if (what === "copy" && c) for (const u of SE_GROUPS[b.dataset.to]) { const t = seCol(u); if (t && t !== c) t.spans = c.spans.map((s) => ({ ...s })); }
  SE.bad = {}; SE.err = "";
  renderProps();
}), true);
$("props").addEventListener("change", whenReady((e) => {
  const t = e.target, what = t.dataset?.se;
  if (!what) return;
  const o = S.view?.objects[S.sel];
  if (what === "same") return seSame(S.sel, t.value);
  if (SE.busy || !SE.open || !o) return;
  if (what === "act") { const c = seCol(t.dataset.col); if (c?.spans[+t.dataset.i]) c.spans[+t.dataset.i].activate = t.checked; }
  else if (what === "shareall") SE.shareAll = t.checked;
  else if (what === "addcol" && t.value) { SE.cols.push({ uuid: t.value, spans: [] }); SE.sel = t.value; }
  else if (what === "copyfrom" && t.value) {
    // Only timers of the same kind are offered: an analog one's values, or a digital one's on.
    const from = S.view.objects[t.value]?.schedule;
    if (from) { SE.cols = seDraft(from); if (!seCol(SE.sel)) SE.sel = SE_WEEK[0]; }
  }
  renderProps();
}));
$("props").addEventListener("input", whenReady((e) => {
  const f = e.target.closest?.("[data-sch-field]");
  if (!f || !SE.open || SE.busy) return;
  const [uuid, i, end] = f.dataset.schField.split("|");
  const s = seCol(uuid)?.spans[+i];
  if (!s) return;
  if (end === "value") s.value = f.value;
  else s[end] = seMin(f.value);
  const ok = end === "value" || s[end] != null;
  f.setAttribute("aria-invalid", ok ? "false" : "true");
  seRefreshBars(!!S.view?.objects[SE.uuid]?.schedule?.analog);
}));
$("props").addEventListener("keydown", whenReady((e) => {
  if (!e.target.closest?.("[data-sch-field]")) return;
  if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); seSave(); }
  else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); seClose(); }
}), true);
// Dragging on a bar: a segment moves, a handle resizes, the empty bar adds an hour there. Snapped to 15 minutes
// (Alt: to the minute), never past the day or into the next interval.
$("props").addEventListener("pointerdown", whenReady((e) => {
  const bar = e.target.closest?.("[data-se-bar]");
  if (!bar || !SE.open || SE.busy || e.button !== 0) return;
  const c = seCol(bar.dataset.seBar), analog = !!S.view?.objects[SE.uuid]?.schedule?.analog;
  if (!c) return;
  e.preventDefault();
  const rect = bar.getBoundingClientRect();
  const at = (x, alt) => { const m = Math.round(((x - rect.left) / rect.width) * 1440); const q = alt ? 1 : 15; return Math.max(0, Math.min(1440, Math.round(m / q) * q)); };
  const seg = e.target.closest(".se-seg");
  let s, mode;
  if (seg) { s = c.spans[+seg.dataset.i]; mode = e.target.closest(".se-h")?.dataset.end || "move"; }
  else { s = seAdd(c, Math.min(at(e.clientX, e.altKey), 1439), analog); mode = "new"; if (!s) { renderProps(); return; } }
  SE.sel = c.uuid;
  const start = at(e.clientX, e.altKey), orig = { from: s.from, to: s.to };
  const others = c.spans.filter((x) => x !== s);
  const lo = Math.max(0, ...others.filter((x) => x.to <= orig.from).map((x) => x.to));
  const hi = Math.min(1440, ...others.filter((x) => x.from >= orig.to).map((x) => x.from));
  const move = (ev) => {
    const d = at(ev.clientX, ev.altKey) - start;
    if (mode === "move") { const len = orig.to - orig.from; s.from = Math.max(lo, Math.min(hi - len, orig.from + d)); s.to = s.from + len; }
    else if (mode === "from") s.from = Math.max(lo, Math.min(s.to - 1, orig.from + d));
    // A new interval ends where the pointer is, once it moved.
    else if (mode === "new") { if (d) s.to = Math.min(hi, Math.max(s.from + 1, at(ev.clientX, ev.altKey))); }
    else s.to = Math.min(hi, Math.max(s.from + 1, orig.to + d));
    seRefreshBars(analog);
  };
  const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); SE.bad = {}; renderProps(); };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}));

/** A shared set's claimants the config shown no longer has (deleted in Loxone Config): the server refuses
 *  to write the set while their claims stand, so each gets *Drop its claim* (`attr`: `se` or `mo`). */
function staleRows(set, attr, mine) {
  return (set?.stale || []).map((u) => `<div class="sch-note warn">⚠ ${esc(set.name)} is also claimed for ${esc(u)}, which this config does not have (deleted in Loxone Config?). The set cannot be written until that claim goes. ` +
    `<button type="button" class="ed-btn small" data-${attr}="dropstale" data-u="${esc(u)}"${mine?.busy ? " disabled" : ""}>Drop its claim</button></div>`);
}
// ── lighting moods (#74) ───────────────────────────────────────────────
// A lighting controller's moods are a Lox Logic `moods` set and the controller's `.moods` claim, written into
// vislox's file of claims. Properties shows the circuits (AQ1 … AQ18, each of one kind: switch, dimmer, colour or
// tunable white) and the moods in order; the editor adds, renames, reorders and deletes moods and sets each
// circuit's value, offering only the values its kind takes. Saving sends one `moods` op: every mood but the off
// mood, in order, its values in the language's terms. It is an edit like any other: compiled, never deployed,
// undone through `undo`.
const MO = { uuid: null, open: false, rows: [], busy: false, msg: "", err: "", bad: {}, shareAll: true, all: false, extra: new Set(), warned: "" };
/** What each kind of circuit is called, and the values it takes. */
const MO_KIND = { switch: "switch", dimmer: "dimmer", rgb: "colour (RGB)", colour: "colour (RGB or tunable white)", white: "tunable white", raw: "a kind Lox Logic does not name", mixed: "two kinds of values" };
const MO_TAKES = { switch: ["off", "on", "pct"], dimmer: ["off", "pct"], rgb: ["off", "rgb"], colour: ["off", "rgb", "temp"], white: ["off", "temp", "rgb"], raw: ["raw"], mixed: ["raw"] };
/** The tag of a mood a set must keep, from the server's one rule (`role`). */
const MO_ROLE = { builtin: "built-in", motion: "motion settings" };
const MO_T = { off: "off", on: "on", pct: "%", rgb: "rgb", temp: "white", raw: "as stored" };
const MO_HINT = { pct: "0 to 100", rgb: "red, green, blue: 0 to 100 each", temp: "brightness %, Kelvin", raw: "the stored number" };
/** What `GET /api/edits` says of this controller's moods (#74), or null. */
function moodsEdit(uuid) { return (S.edits?.moods || []).find((m) => m.uuid === uuid) || null; }
/** The controllers whose moods were edited here and are not deployed. */
function moodsPending() { return (S.edits?.moods || []).filter((m) => m.pending); }
/** A circuit value in the language's terms, split for the editor: its kind of value and its numbers. */
function moParse(v) {
  let m;
  if (v === "off" || v === "on") return { t: v, a: "" };
  if ((m = /^(\d+)%$/.exec(v))) return { t: "pct", a: m[1] };
  if ((m = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(v))) return { t: "rgb", a: `${m[1]}, ${m[2]}, ${m[3]}` };
  if ((m = /^temp\((\d+)%, (\d+)K\)$/.exec(v))) return { t: "temp", a: `${m[1]}, ${m[2]}` };
  return { t: "raw", a: v };
}
/** A cell back in the language's terms, or null when its numbers are not ones the circuit takes. */
function moValue(c) {
  const nums = (s, n) => { const p = (s || "").split(",").map((x) => x.trim()); return p.length === n && p.every((x) => /^\d+$/.test(x)) ? p.map(Number) : null; };
  const pct = (x) => x >= 0 && x <= 100;
  if (c.t === "off" || c.t === "on") return c.t;
  if (c.t === "pct") { const n = nums(c.a.replace(/%$/, ""), 1); return n && pct(n[0]) ? `${n[0]}%` : null; }
  if (c.t === "rgb") { const n = nums(c.a, 3); return n && n.every(pct) ? `rgb(${n.join(", ")})` : null; }
  if (c.t === "temp") { const n = nums(c.a.replace(/[%K]/g, ""), 2); return n && pct(n[0]) && n[1] > 0 && n[1] < 10000 ? `temp(${n[0]}%, ${n[1]}K)` : null; }
  return /^\d+$/.test(c.a.trim()) ? c.a.trim() : null;
}
/** Whether this controller's moods can be edited here now: `{ ok, rev }`, or null (nothing to edit). */
function moState(o) {
  if (o.ty !== "LightController2" || !o.moods || o.moods.error) return null;
  const st = editState(o);
  return st?.ok ? st : null;
}
/** The circuits the section shows: those named, and those a mood other than "Viel Licht" (which switches every
 *  circuit on) uses; in the editor also those added to it; all of them with *Show all circuits*. A circuit not
 *  shown keeps its values. */
function moCircuits(o) {
  const mine = MO.open && MO.uuid === o.uuid;
  return o.moods.circuits.filter((c) => MO.all || c.name || c.used || (mine && MO.extra.has(c.n)));
}
/** The T5 input that selects mood `m` (scene id n: `Sel<n>`), or null. */
function moButton(mi, sid) { return sid && mi.selected.includes(+sid) ? `T5/${sid}` : null; }
function moOpen(uuid) {
  const o = S.view?.objects[uuid];
  if (!o?.moods) return;
  const rows = o.moods.moods.filter((m) => !m.off).map((m) => ({ orig: m.name, name: m.name, sid: m.sid, required: m.required, cells: m.values.map(moParse) }));
  Object.assign(MO, { uuid, open: true, rows, busy: false, msg: "", err: "", bad: {}, shareAll: true, extra: new Set(), warned: "" });
  S.closed.delete("Moods");
  renderProps();
  $("props").querySelector('[data-mo-name="0"]')?.focus();
}
function moClose() {
  if (MO.busy) return;
  Object.assign(MO, { open: false, rows: [], msg: "", err: "", bad: {}, warned: "" });
  renderProps();
  $("props").querySelector('[data-mo="open"]')?.focus();
}
/** The lint findings on circuit `n` of the controller: `no-actuator` (#66), a percentage on a circuit without
 *  an actuator, which the Miniserver only switches. */
function moLint(uuid, n) { return lintFindings(uuid).filter((f) => f.kind === "no-actuator" && new RegExp(`\\bAQ${n}\\b`).test(f.message)); }
/** The rows of the *Moods* section. */
function moRows(o, row, marks) {
  const mi = o.moods;
  if (o.ty !== "LightController2" || !mi) return null;
  const rows = [], st = moState(o), e = moodsEdit(o.uuid), mine = MO.uuid === o.uuid ? MO : null;
  const title = (u) => { const x = S.view?.objects[u]; return x ? displayTitle(x) : u; };
  const circuits = moCircuits(o);
  const managed = editsBlock(o.uuid) && st;
  const circ = circuits.map((c) => {
    const lint = moLint(o.uuid, c.n);
    // A controller added or adopted here declares its switch and dimmer circuits (Circuit1 … Circuit5); colour
    // circuits and the rest stay with Loxone Config.
    // With vislox's moods claim, the claim writes the circuit's values anew, so they must fit the new kind.
    const tip = e?.set
      ? `Declares Circuit${c.n} in the block's declaration. The moods vislox claims (${e.set}) write this circuit's values on every compile, so they must fit the new kind: change them in Edit moods first`
      : `Declares Circuit${c.n} in the block's declaration; changing it clears the circuit's values in every mood, as Loxone Config's mood editor does`;
    const decl = managed && c.n <= 5 && (c.kind === "switch" || c.kind === "dimmer")
      ? ` <select class="mo-decl" data-mo="circuit" data-n="${c.n}" aria-label="AQ${c.n} is a" title="${esc(tip)}"${S.edit.busy ? " disabled" : ""}>${["switch", "dimmer"].map((k) => `<option${k === c.kind ? " selected" : ""}>${k}</option>`).join("")}</select>`
      : "";
    return row(`AQ${c.n}`, `${c.name ? esc(c.name) : '<span class="none-v">no actuator</span>'} <span class="mo-kind">${esc(MO_KIND[c.kind])}</span>${decl}` +
      lint.map((f) => `<div class="ed-note lt-note">⚠ Lint (no-actuator${f.file ? `, ${esc(checkAt(f))}` : ""}): ${esc(f.message)}</div>`).join(""), lint.length ? "pr-chg" : "");
  });
  rows.push(`<div class="mo-sub">Circuits</div>`, ...circ);
  if (mi.error) rows.push(`<div class="sch-note warn">⚠ lox cannot write these moods: ${esc(mi.error)}</div>`);
  if (circuits.length < mi.circuits.length) rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-mo="all">${MO.all ? "Show the circuits in use" : `Show all ${mi.circuits.length} circuits`}</button></div>`);
  // The claim: vislox's set, or the rest of the module's.
  const other = claimsOf(o.uuid).find((c) => c.kind === "moods" && !c.file.startsWith(`${S.edits?.dir}/`));
  if (other && st) {
    const at = claimAt(other);
    if (e?.set) {
      rows.push(`<div class="sch-note warn">⚠ Claimed twice: ${esc(at)} and vislox's ${esc(S.edits?.file || "")} both claim its moods, and lox compiles neither until one goes.</div>`,
        `<div class="ed-bar"><button type="button" class="ed-btn small" data-mo="drop"${mine?.busy ? " disabled" : ""} title="Take vislox's claim out and compile: ${esc(at)} keeps the controller">Remove vislox's claim</button>` +
        (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "") + "</div>");
    } else rows.push(`<div class="ed-note">${esc(at)} claims its moods: change them there.</div>`);
  } else if (e?.set && st) {
    rows.push(`<div class="ed-note">Written by vislox as <span class="mono">${esc(e.set)}</span> in ${esc(S.edits?.file || "")}${e.shared?.length ? `, shared with ${esc(e.shared.map(title).join(", "))}` : ""}.</div>`);
    rows.push(...staleRows((S.edits?.mood_sets || []).find((x) => x.name === e.set), "mo", mine));
  }
  if (e?.pending && st && !other) {
    const back = e.deployed ? "Back to the moods deployed" : "Back to Loxone Config's moods";
    rows.push(row("Moods", `<span class="ed-v">edited here</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      `<button type="button" class="ed-btn small" data-mo="revert" title="${esc(back)}">Revert</button>`, "pr-chg"));
  }
  if (!mine?.open || !st || other) {
    // The moods as the config shown has them, marked against the compared revision.
    const old = marks ? o.moods_old : null;
    const oldOf = (n) => old?.moods.find((m) => m.name === n);
    rows.push(`<div class="mo-sub">Moods</div>`);
    for (const m of mi.moods) {
      const was = oldOf(m.name), btn = moButton(mi, m.sid);
      const tags = [m.off ? "off mood" : "", MO_ROLE[m.role] || "", btn ? `${btn} selects it` : ""].filter(Boolean);
      const v = m.off ? '<span class="none-v">every circuit off, always there</span>' : old && !was ? `<span class="new">${esc(m.text)}</span>` : was && was.text !== m.text ? `<span class="old">${esc(was.text)}</span><span class="new">${esc(m.text)}</span>` : esc(m.text);
      rows.push(row(m.name, `${v}${tags.map((t) => ` <span class="mo-tag">${esc(t)}</span>`).join("")}`, old && (!was || was.text !== m.text) ? "pr-chg" : ""));
    }
    for (const m of old?.moods || []) if (!mi.moods.some((x) => x.name === m.name)) rows.push(row(m.name, `<span class="old">${esc(m.text)}</span> <span class="mo-tag">removed</span>`, "pr-chg"));
    if (old) {
      const order = (a, b) => a.moods.filter((m) => b.moods.some((x) => x.name === m.name)).map((m) => m.name).join(" · ");
      if (order(old, mi) !== order(mi, old)) rows.push(`<div class="ed-note">Order: <span class="old">${esc(order(old, mi))}</span> <span class="new">${esc(order(mi, old))}</span></div>`);
    }
    if (st && !other) {
      const sets = (S.edits?.mood_sets || []).filter((x) => x.name !== e?.set && x.controllers.length && !x.controllers.includes(o.uuid));
      rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-mo="open">Edit moods</button>` +
        (sets.length ? `<label class="se-same">Use the same moods as <select data-mo="same"${mine?.busy ? " disabled" : ""}><option value="">…</option>${sets.map((x) => `<option value="${esc(x.name)}">${esc(x.controllers.map(title).join(", "))}</option>`).join("")}</select></label>` : "") +
        (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "") + "</div>");
    }
    return { rows, count: (e?.pending ? 1 : 0) + mi.circuits.filter((c) => moLint(o.uuid, c.n).length).length || undefined };
  }
  // The editor: a row per mood, a cell per circuit shown.
  const dis = mine.busy ? " disabled" : "";
  const off = mi.moods.find((m) => m.off);
  const cell = (r, i, c) => {
    const v = r.cells[c.n - 1], k = `${i}|${c.n}`, takes = MO_TAKES[c.kind];
    const opts = (takes.includes(v.t) ? takes : [v.t, ...takes]).map((t) => `<option value="${t}"${t === v.t ? " selected" : ""}>${esc(MO_T[t])}</option>`).join("");
    const arg = v.t === "off" || v.t === "on" ? "" : `<input class="num mo-a" data-mo-field="${esc(k)}" value="${esc(v.a)}" placeholder="${esc(MO_HINT[v.t])}" title="${esc(MO_HINT[v.t])}" aria-label="AQ${c.n} ${esc(MO_T[v.t])}" autocomplete="off"${dis}>`;
    return `<td class="${mine.bad[k] ? "bad" : ""}" title="${esc(mine.bad[k] || `AQ${c.n}: ${MO_KIND[c.kind]}`)}"><select data-mo="t" data-k="${esc(k)}" aria-label="${esc(`${r.name || "new mood"}, AQ${c.n}`)}"${dis}>${opts}</select>${arg}</td>`;
  };
  const head = `<tr><th>Mood</th>${circuits.map((c) => `<th title="${esc(`${c.name || "no actuator"}: ${MO_KIND[c.kind]}`)}">AQ${c.n}</th>`).join("")}<th></th></tr>`;
  const body = MO.rows.map((r, i) => {
    const btn = moButton(mi, r.sid);
    const lock = r.required ? ` title="${esc(`${r.required}: it stays in the set, and keeps its name`)}" readonly` : "";
    return `<tr class="${mine.bad[`n${i}`] ? "bad" : ""}"><td class="mo-name"><input data-mo-name="${i}" value="${esc(r.name)}" aria-label="Mood name" placeholder="name" autocomplete="off"${lock}${dis}>` +
      `${btn && r.orig ? ` <span class="mo-tag" title="A button on ${esc(btn)} selects this mood by its scene id">${esc(btn)}</span>` : ""}${mine.bad[`n${i}`] ? `<div class="ed-bad">${esc(mine.bad[`n${i}`])}</div>` : ""}</td>` +
      circuits.map((c) => cell(r, i, c)).join("") +
      `<td class="mo-ops"><button type="button" class="se-x" data-mo="up" data-i="${i}" aria-label="Move ${esc(r.name)} up"${i ? "" : " disabled"}${dis}>↑</button>` +
      `<button type="button" class="se-x" data-mo="down" data-i="${i}" aria-label="Move ${esc(r.name)} down"${i < MO.rows.length - 1 ? "" : " disabled"}${dis}>↓</button>` +
      `<button type="button" class="se-x" data-mo="del" data-i="${i}" aria-label="Delete ${esc(r.name)}"${r.required ? ` disabled title="${esc(r.required)}: it stays in the set"` : ""}${dis}>✕</button></td></tr>`;
  }).join("");
  rows.push(`<div class="mo-wrap"><table class="mo-grid">${head}${body}</table></div>`);
  if (off) rows.push(`<div class="ed-note">"${esc(off.name)}", the off mood, is always there and keeps its place.</div>`);
  const extra = mi.circuits.filter((c) => !circuits.includes(c));
  rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-mo="add"${dis}>Add mood</button>` +
    (extra.length ? `<label class="se-same">Add a circuit <select data-mo="addc"${dis}><option value="">…</option>${extra.map((c) => `<option value="${c.n}">AQ${c.n} (${esc(MO_KIND[c.kind])})</option>`).join("")}</select></label>` : "") + "</div>");
  if (e?.shared?.length) rows.push(`<label class="se-share"><input type="checkbox" data-mo="shareall"${MO.shareAll ? " checked" : ""}${dis}> Change ${esc(e.shared.map(title).join(", "))} too: ${e.shared.length === 1 ? "it shares" : "they share"} these moods (${esc(e.set)}). Off: this controller gets a set of its own.</label>`);
  if (mine.warned) rows.push(`<div class="sch-note warn" role="alert">⚠ ${esc(mine.warned)}</div>`);
  rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-mo="save"${dis}>${mine.busy ? "Compiling…" : mine.warned ? "Save anyway" : "Save"}</button>` +
    `<button type="button" class="ed-btn small" data-mo="cancel"${dis}>Cancel</button>` +
    `<span class="ed-msg${mine.err ? " err" : ""}" role="status" aria-live="polite">${esc(mine.err || mine.msg || "Saving writes a moods set into the module and compiles it; nothing is deployed. A mood is known by its name: renaming one is a delete and an add.")}</span></div>`);
  return { rows, count: e?.pending ? 1 : 0 };
}
/** Check the draft as lox would: names, values each circuit takes. Marks what is wrong; true when all is well. */
function moCheck(o) {
  const bad = {}, seen = new Map(), offName = o.moods.moods.find((m) => m.off)?.name;
  MO.rows.forEach((r, i) => {
    const n = r.name.trim();
    if (!n) bad[`n${i}`] = "A mood needs a name.";
    else if (n === offName) bad[`n${i}`] = `"${n}" is the off mood, which is always there.`;
    else if (seen.has(n)) bad[`n${i}`] = `Two moods are named "${n}": a mood is known by its name.`;
    seen.set(n, i);
    r.cells.forEach((c, j) => { if (c.t !== "off" && moValue(c) == null) bad[`${i}|${j + 1}`] = `AQ${j + 1}: ${MO_HINT[c.t] || "a value"}.`; });
  });
  for (const m of o.moods.moods.filter((x) => x.required)) if (!MO.rows.some((r) => r.name.trim() === m.name)) bad.req = `"${m.name}" is ${m.required}: it stays in the set.`;
  MO.bad = bad;
  return !Object.keys(bad).length;
}
/** The moods a button selects that the draft deletes or renames: their names and T5 inputs. */
function moButtonsLost(o) {
  const mi = o.moods, lost = [];
  for (const m of mi.moods.filter((x) => !x.off)) {
    const btn = moButton(mi, m.sid);
    if (!btn) continue;
    const r = MO.rows.find((x) => x.orig === m.name);
    if (!r) lost.push(`${btn} selects "${m.name}", which this deletes`);
    else if (r.name.trim() !== m.name) lost.push(`${btn} selects "${m.name}", which this renames to "${r.name.trim()}": a new mood with a new scene id`);
  }
  return lost;
}
async function moSave() {
  const o = S.view?.objects[MO.uuid], st = o && moState(o);
  if (!st || MO.busy || !MO.open) return;
  if (!moCheck(o)) {
    MO.err = MO.bad.req || "Fix the marked moods and values.";
    renderProps();
    $("props").querySelector(".mo-grid .bad input, .mo-grid .bad select")?.focus();
    return;
  }
  // Warned once: deleting or renaming a mood a button selects leaves that button selecting nothing, or a new
  // mood that takes its scene id. A second click saves anyway.
  const lost = moButtonsLost(o);
  if (lost.length && !MO.warned) {
    MO.warned = `${lost.join("; ")}. The button then selects nothing, or a new mood that takes the freed scene id.`;
    renderProps();
    return;
  }
  const moods = MO.rows.map((r) => ({
    name: r.name.trim(),
    values: r.cells.map((c, j) => (c.t === "off" ? null : `AQ${j + 1} ${moValue(c)}`)).filter(Boolean).join(", "),
  }));
  const e = moodsEdit(o.uuid);
  const value = { moods, ...(e?.set && e.shared?.length && MO.shareAll ? { set: e.set } : {}) };
  await moSend(o, st, { op: "moods", uuid: o.uuid, value });
}
async function moSend(o, st, op, label) {
  MO.uuid = o.uuid; MO.busy = true; MO.err = ""; MO.msg = ""; renderProps();
  try {
    const res = await editAct(st.rev.blob, [op], label);
    Object.assign(MO, { open: false, rows: [], busy: false, bad: {}, err: "", warned: "", msg: res.unchanged ? "Nothing changed." : "Saved and compiled: pending, not deployed. Review it, then Apply or Discard." });
    await editShowNewest(res.blob);
  } catch (err) {
    MO.busy = false; MO.err = err.message; MO.warned = "";
  }
  renderProps();
}
async function moRevert(uuid) {
  const o = S.view?.objects[uuid], st = o && moState(o), e = moodsEdit(uuid);
  if (!st || !e) return;
  const r = setRevertValue("moods", e, S.edits?.mood_sets || [], uuid);
  if (r.why) { MO.uuid = uuid; MO.err = r.why; MO.msg = ""; renderProps(); return; }
  await moSend(o, st, { op: "moods", uuid, value: r.value }, `Revert the moods of ${displayTitle(o)}`);
}
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-mo]");
  if (!b || b.tagName === "SELECT" || b.type === "checkbox") return;
  e.stopPropagation();
  const o = S.view?.objects[S.sel], what = b.dataset.mo;
  if (!o) return;
  if (what === "all") { MO.all = !MO.all; return renderProps(); }
  if (what === "open") return moOpen(S.sel);
  if (what === "cancel") return moClose();
  if (what === "save") return moSave();
  if (what === "revert") return moRevert(S.sel);
  if (what === "drop") { const st = moState(o); if (st) moSend(o, st, { op: "moods", uuid: o.uuid, value: null }, `Remove vislox's claim on the moods of ${displayTitle(o)}`); return; }
  if (what === "dropstale") { const st = moState(o); if (st) moSend(o, st, { op: "moods", uuid: b.dataset.u, value: null }, "Drop a stale claim on moods"); return; }
  if (MO.busy || !MO.open) return;
  const i = +b.dataset.i;
  if (what === "add") {
    let n = 1;
    while (MO.rows.some((r) => r.name === `Mood ${n}`)) n++;
    MO.rows.push({ orig: null, name: `Mood ${n}`, sid: null, required: null, cells: o.moods.circuits.map(() => ({ t: "off", a: "" })) });
  } else if (what === "del" && MO.rows[i] && !MO.rows[i].required) MO.rows.splice(i, 1);
  else if (what === "up" && i > 0) MO.rows.splice(i - 1, 0, MO.rows.splice(i, 1)[0]);
  else if (what === "down" && i < MO.rows.length - 1) MO.rows.splice(i + 1, 0, MO.rows.splice(i, 1)[0]);
  MO.bad = {}; MO.err = ""; MO.warned = "";
  renderProps();
  if (what === "add") $("props").querySelector(`[data-mo-name="${MO.rows.length - 1}"]`)?.select();
}), true);
$("props").addEventListener("change", whenReady((e) => {
  const t = e.target, what = t.dataset?.mo;
  if (!what) return;
  const o = S.view?.objects[S.sel];
  if (!o) return;
  if (what === "same") { const st = moState(o); if (st && t.value) moSend(o, st, { op: "moods", uuid: o.uuid, value: { set: t.value } }); return; }
  if (what === "circuit") {
    const st = editState(o);
    if (st?.ok) editSend(o.uuid, st.rev, [{ op: "setting", uuid: o.uuid, name: `Circuit${t.dataset.n}`, value: t.value }], `AQ${t.dataset.n} of ${displayTitle(o)}: ${t.value}`);
    return;
  }
  if (MO.busy || !MO.open) return;
  if (what === "t") {
    const [i, n] = t.dataset.k.split("|").map(Number), c = MO.rows[i]?.cells[n - 1];
    if (c) { c.t = t.value; c.a = t.value === "pct" ? "100" : t.value === "rgb" ? "100, 100, 100" : t.value === "temp" ? "100, 2700" : ""; }
  } else if (what === "shareall") MO.shareAll = t.checked;
  else if (what === "addc" && t.value) MO.extra.add(+t.value);
  MO.bad = {}; MO.err = ""; MO.warned = "";
  renderProps();
}));
$("props").addEventListener("input", whenReady((e) => {
  if (!MO.open || MO.busy) return;
  const f = e.target.closest?.("[data-mo-field], [data-mo-name]");
  if (!f) return;
  if (f.dataset.moName != null) { const r = MO.rows[+f.dataset.moName]; if (r) r.name = f.value; }
  else {
    const [i, n] = f.dataset.moField.split("|").map(Number), c = MO.rows[i]?.cells[n - 1];
    if (c) { c.a = f.value; f.setAttribute("aria-invalid", moValue(c) == null ? "true" : "false"); }
  }
  MO.warned = "";
}));
$("props").addEventListener("keydown", whenReady((e) => {
  if (!e.target.closest?.("[data-mo-field], [data-mo-name]")) return;
  if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); moSave(); }
  else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); moClose(); }
}), true);

// ── members of a central block (#75) ───────────────────────────────────
// A CentralLight drives a list of lighting controllers, a CentralShade a list of automatic blinds: Lox Logic writes
// it as `<central>.members = (a, b, …)`, in the order the app lists them. Properties shows the members, each a link
// to its block; the editor adds one (the Ctrl+K picker, offering only blocks of the type the central block takes),
// removes and reorders them. Saving sends one `members` op with the whole list: compiled, never deployed, undone
// through `undo`. An alarm's members (`Objects=`) are shown read-only: they stay with Loxone Config.
// The draft belongs to one block and one pending config (`blob`): when either changes (another selection, an
// undo, an edit elsewhere) it is dropped, and a save sends the blob it was made on, which the server checks.
const ME = { uuid: null, blob: null, open: false, list: [], dropped: 0, busy: false, msg: "", err: "" };
const ME_WHAT = { LightController2: "lighting controller", AutoJalousie: "automatic blind" };
/** What `GET /api/edits` says of this central block's members (#75), or null. */
function membersEdit(uuid) { return (S.edits?.members || []).find((m) => m.uuid === uuid) || null; }
/** The central blocks whose members were edited here and are not deployed. */
function membersPending() { return (S.edits?.members || []).filter((m) => m.pending); }
/** Whether this block's members can be edited here now: `{ ok, rev }`, or null. */
function meState(o) {
  if (!o.members?.editable) return null;
  const st = editState(o);
  return st?.ok ? st : null;
}
/** The module's own claim on this block's members (a file other than vislox's), if any. */
function meOther(o) { return (S.edits?.members_claims || []).find((c) => c.blocks?.includes(o.uuid) && !c.file.startsWith(`${S.edits?.dir}/`)) || null; }
/** A block as a member link: from the view's objects (a member the draft adds may be listed nowhere yet). */
function meTarget(u) {
  const x = S.view?.objects[u];
  if (!x) return { uuid: u, missing: true };
  return { uuid: u, title: x.title, ty: x.ty, page: x.page, page_title: S.pageOf?.get(x.page)?.title, path: x.page ? null : x.path };
}
function meLink(t) {
  return t.missing ? `<span class="mono">${esc(t.uuid)}</span> <span class="mo-tag" title="The list names it, but this config has no such object">not in this config</span>` : linkTarget(t);
}
function meWhat(mi) { return mi.takes ? ME_WHAT[mi.takes] || prettyType(mi.takes) : "block"; }
function meOpen(uuid) {
  const o = S.view?.objects[uuid];
  if (!o?.members?.editable) return;
  const st = meState(o);
  if (!st) return;
  // A member the config does not have cannot be saved (the server refuses it): the draft leaves it out, and says so.
  const missing = o.members.members.filter((m) => m.missing).length;
  Object.assign(ME, { uuid, blob: st.rev.blob, open: true, list: o.members.members.filter((m) => !m.missing).map((m) => m.uuid), dropped: missing, busy: false, msg: "", err: "" });
  S.closed.delete("Members");
  renderProps();
  $("props").querySelector('[data-me="add"]')?.focus();
}
/** Drop a draft made on another block or another pending config than the one shown now. */
function meSync() {
  if (!ME.open || ME.busy) return;
  if (S.sel !== ME.uuid) Object.assign(ME, { open: false, list: [], dropped: 0, msg: "", err: "" });
  else if (editRev()?.blob !== ME.blob) Object.assign(ME, { open: false, list: [], dropped: 0, err: "", msg: "The pending config changed while editing: the draft was dropped. Edit members again." });
}
function meClose() {
  if (ME.busy) return;
  Object.assign(ME, { open: false, list: [], dropped: 0, msg: "", err: "" });
  renderProps();
  $("props").querySelector('[data-me="open"]')?.focus();
}
/** Add a member: the picker offers the blocks of the type the central block takes, not those listed already. */
function meAdd() {
  const o = S.view?.objects[ME.uuid];
  if (!o?.members || !ME.open || ME.busy) return;
  const what = meWhat(o.members);
  openPicker({
    title: `Add a member to ${displayTitle(o)}`,
    placeholder: `Add a ${what}…`,
    none: `No other ${what} to add.`,
    types: [o.members.takes],
    exclude: new Set([o.uuid, ...ME.list].map((u) => u.toLowerCase())),
    onPick: (u) => { ME.list.push(u); ME.err = ""; ME.msg = ""; renderProps(); $("props").querySelector('[data-me="add"]')?.focus(); },
    back: () => $("props").querySelector('[data-me="add"]')?.focus(),
  });
}
/** The rows of the *Members* section. */
function meRows(o, row, marks) {
  const mi = o.members;
  if (!mi) return null;
  const rows = [], st = meState(o), e = membersEdit(o.uuid), mine = ME.uuid === o.uuid ? ME : null;
  const other = mi.editable ? meOther(o) : null, dis = mine?.busy ? " disabled" : "";
  const msg = () => (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "");
  if (!mi.editable) rows.push(`<div class="ed-note">The members of an alarm (<span class="mono">Objects=</span>) stay with Loxone Config: they name hidden references wired to the alarm, which the list does not own.</div>`);
  if (other && e?.members) {
    rows.push(`<div class="sch-note warn">⚠ Claimed twice: ${esc(claimAt(other))} and vislox's ${esc(S.edits?.file || "")} both claim its members, and lox compiles neither until one goes.</div>`,
      `<div class="ed-bar"><button type="button" class="ed-btn small" data-me="drop"${dis} title="Take vislox's claim out and compile: ${esc(claimAt(other))} keeps the block">Remove vislox's claim</button>${msg()}</div>`);
  } else if (other) rows.push(`<div class="ed-note">${esc(claimAt(other))} claims its members: change them there.</div>`);
  else if (e?.members && st) rows.push(`<div class="ed-note">Claimed by vislox in ${esc(S.edits?.file || "")}: the list is the block's whole member list, in order.</div>`);
  if (e?.stale?.length && st && !other) {
    rows.push(`<div class="sch-note warn">⚠ vislox's claim lists ${plural(e.stale.length, "member")} this config does not have (deleted in Loxone Config?), so lox cannot compile it. ` +
      `<button type="button" class="ed-btn small" data-me="dropstale"${dis}>Drop the missing members</button></div>`);
  }
  if (e?.pending && st && !other) {
    const back = e.deployed ? "Back to the members deployed" : "Back to the members as last written: vislox's claim goes";
    rows.push(row("Members", `<span class="ed-v">edited here</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      `<button type="button" class="ed-btn small" data-me="revert"${dis} title="${esc(back)}">Revert</button>`, "pr-chg"));
  }
  if (!mine?.open || !st || other) {
    // The members as the config shown has them, marked against the compared revision.
    const old = marks ? o.members_old : null;
    const inOld = (u) => old?.members.some((m) => m.uuid === u);
    if (!mi.members.length) rows.push(row(mi.editable ? "Members" : "Objects", `<span class="none-v">none</span>`));
    mi.members.forEach((m, i) => {
      const added = old && !inOld(m.uuid);
      rows.push(row(`${i + 1}`, `${added ? '<span class="new">' : ""}${meLink(m)}${added ? "</span>" : ""}`, added ? "pr-chg" : ""));
    });
    for (const m of old?.members || []) if (!mi.members.some((x) => x.uuid === m.uuid)) rows.push(row("—", `<span class="old">${esc(m.title || m.uuid)}</span> <span class="mo-tag">removed</span>`, "pr-chg"));
    if (old) {
      const order = (a, b) => a.members.filter((m) => b.members.some((x) => x.uuid === m.uuid)).map((m) => m.title || m.uuid).join(" · ");
      if (order(old, mi) !== order(mi, old)) rows.push(`<div class="ed-note">Order: <span class="old">${esc(order(old, mi))}</span> <span class="new">${esc(order(mi, old))}</span></div>`);
    }
    if (st && !other) rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-me="open">Edit members</button>${msg()}</div>`);
    return { rows, count: (e?.pending ? 1 : 0) + (e?.stale?.length ? 1 : 0) || undefined };
  }
  // The editor: the list in order, each with ↑ ↓ ✕, and *Add member*.
  const what = meWhat(mi);
  const items = ME.list.map((u, i) => {
    const t = meTarget(u), name = t.missing ? u : displayTitle(S.view.objects[u]);
    return `<li class="me-item">${meLink(t)}<span class="me-ops">` +
      `<button type="button" class="se-x" data-me="up" data-i="${i}" aria-label="Move ${esc(name)} up"${i ? "" : " disabled"}${dis}>↑</button>` +
      `<button type="button" class="se-x" data-me="down" data-i="${i}" aria-label="Move ${esc(name)} down"${i < ME.list.length - 1 ? "" : " disabled"}${dis}>↓</button>` +
      `<button type="button" class="se-x" data-me="del" data-i="${i}" aria-label="Remove ${esc(name)}"${dis}>✕</button></span></li>`;
  }).join("");
  if (ME.dropped) rows.push(`<div class="sch-note warn">⚠ ${plural(ME.dropped, "member")} not in this config ${ME.dropped === 1 ? "was" : "were"} dropped from the draft: saving leaves ${ME.dropped === 1 ? "it" : "them"} out.</div>`);
  rows.push(ME.list.length ? `<ol class="me-list">${items}</ol>` : `<div class="ed-note">No members: saving empties the list.</div>`);
  rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-me="add"${dis} title="Pick a ${esc(what)} (Ctrl+K)">Add member…</button><span class="none-v">${esc(`${prettyType(mi.takes)} blocks, in the order the app lists them`)}</span></div>`);
  rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-me="save"${dis}>${mine.busy ? "Compiling…" : "Save"}</button>` +
    `<button type="button" class="ed-btn small" data-me="cancel"${dis}>Cancel</button>` +
    `<span class="ed-msg${mine.err ? " err" : ""}" role="status" aria-live="polite">${esc(mine.err || mine.msg || "Saving claims the list in the module and compiles it; nothing is deployed.")}</span></div>`);
  return { rows, count: e?.pending ? 1 : 0 };
}
async function meSend(o, base, op, label) {
  ME.uuid = o.uuid; ME.busy = true; ME.err = ""; ME.msg = ""; renderProps();
  try {
    const res = await editAct(base, [op], label);
    Object.assign(ME, { open: false, list: [], dropped: 0, busy: false, err: "", msg: res.unchanged ? "Nothing changed." : "Saved and compiled: pending, not deployed. Review it, then Apply or Discard." });
    await editShowNewest(res.blob);
  } catch (err) {
    ME.busy = false; ME.err = err.message;
  }
  renderProps();
}
function meSave() {
  const o = S.view?.objects[ME.uuid], st = o && meState(o);
  if (!st || ME.busy || !ME.open) return;
  // Never save a draft over a newer list: the draft goes against the config it was made on.
  if (st.rev.blob !== ME.blob) {
    Object.assign(ME, { open: false, list: [], dropped: 0, err: "The pending config changed while editing: the draft was dropped. Edit members again.", msg: "" });
    renderProps();
    return;
  }
  meSend(o, ME.blob, { op: "members", uuid: o.uuid, value: [...ME.list] });
}
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-me]");
  if (!b) return;
  e.stopPropagation();
  const o = S.view?.objects[S.sel], what = b.dataset.me;
  if (!o) return;
  if (what === "open") return meOpen(S.sel);
  if (what === "cancel") return meClose();
  if (what === "save") return meSave();
  if (what === "add") return meAdd();
  const st = meState(o), ed = membersEdit(o.uuid);
  if (what === "revert") { if (st && ed) meSend(o, st.rev.blob, { op: "members", uuid: o.uuid, value: ed.deployed ?? null }, `Revert the members of ${displayTitle(o)}`); return; }
  if (what === "drop") { if (st) meSend(o, st.rev.blob, { op: "members", uuid: o.uuid, value: null }, `Remove vislox's claim on the members of ${displayTitle(o)}`); return; }
  if (what === "dropstale") { if (st && ed?.members) meSend(o, st.rev.blob, { op: "members", uuid: o.uuid, value: ed.members.filter((u) => !ed.stale.includes(u)) }, `Drop the missing members of ${displayTitle(o)}`); return; }
  if (ME.busy || !ME.open) return;
  const i = +b.dataset.i;
  if (what === "del") ME.list.splice(i, 1);
  else if (what === "up" && i > 0) ME.list.splice(i - 1, 0, ME.list.splice(i, 1)[0]);
  else if (what === "down" && i < ME.list.length - 1) ME.list.splice(i + 1, 0, ME.list.splice(i, 1)[0]);
  ME.err = "";
  renderProps();
  // Keep the keyboard where it was: on the same button of the row that moved, or the next row's.
  const j = what === "up" ? i - 1 : what === "down" ? i + 1 : Math.min(i, ME.list.length - 1);
  ($("props").querySelector(`[data-me="${what}"][data-i="${j}"]:not([disabled])`) || $("props").querySelector(`[data-me="del"][data-i="${j}"]`) || $("props").querySelector('[data-me="add"]'))?.focus();
}), true);

// ── Automatic Designer rules (#87) ─────────────────────────────────────
// A rule (`AutopilotRule`) is made in the app and stored as a JSON payload; Lox Logic writes what it does as a
// `rule` declaration: `active`, its conditions (all must hold; `or` starts another group) and its actions in order.
// The server reads every rule with lox's decompile (`o.rule`: each line in words, the block or mode it names as a
// link, and `value`, the lines with blocks and modes by UUID, which the editor starts from); one Lox Logic cannot
// write (text values, calls, speech) is read-only with the reason. Saving sends one `rule` op with the whole
// rule: compiled, never deployed, undone through `undo`. *New rule* opens the same form for a named rule
// (`new_rule`: its name, a condition and an action at least), which lox creates below the config's last rule; a rule
// created here is renamed in its form and deleted with *Delete rule*.
// lox checks a block's states and commands against the app's capability table when it compiles: the form
// suggests those the config's rules already use (`rule_edit.offers`), and shows lox's refusal, which lists what
// the block offers. The draft belongs to one rule and one pending config (`blob`), as for members.
const RU = { uuid: null, blob: null, open: false, creating: false, active: false, name: "", lines: [], busy: false, msg: "", err: "" };
const RU_WHEN = [["time", "At a time"], ["weekday", "On days of the week"], ["state", "A block's state"], ["mode", "An operating mode"], ["weather", "The weather"], ["day", "A day of the month"], ["daylight", "Day or night"], ["sun", "Sunrise, sunset, dawn, dusk"], ["date", "On a date"]];
const RU_THEN = [["then", "A command to a block"], ["then_mode", "Switch an operating mode"], ["notify", "A push notification"], ["mail", "A mail"], ["wait", "Wait"], ["message", "A message center entry"]];
const RU_OPS = ["==", "!=", ">", "<", ">=", "<="];
const RU_WEATHER = ["temperature", "dewpoint", "felt_temperature", "wind", "gusts", "wind_direction", "irradiance", "relative_irradiance", "solar_irradiance", "precipitation", "pressure", "sunshine"];
const RU_ACTION = new Set(RU_THEN.map(([k]) => k));
/** What `GET /api/edits` says of this rule (#87): what vislox's file states and deployed, or null. */
function ruleEdit(uuid) { return (S.edits?.rules || []).find((r) => ciEq(r.uuid, uuid)) || null; }
/** The rules stated or created here and not deployed. */
function rulesPending() { return (S.edits?.rules || []).filter((r) => r.pending); }
/** Why this rule cannot be edited here (the server's), or null; undefined when the server says nothing of it. */
function ruWhy(o) { const m = S.edits?.rule_edit?.rules; return m ? m[o.uuid.toLowerCase()] : undefined; }
/** Whether rules can be edited here now: `{ ok, rev }`, or `{ why }`. As for any edit, but on a rule (no values). */
function ruState(o) {
  const a = S.hist?.actions;
  if (!a?.shown) return null;
  if (!a.edit?.available) return { why: `Editing is off: ${a.edit?.reason || "unknown"}.` };
  const r = editRev();
  if (!r) return { why: "The config on disk is not in the History yet." };
  if (S.rev !== r.id) return { why: `Edits are made on the newest config (${r.kind === "pending" ? "Pending" : "deployed"}).` };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  if (o) {
    const why = ruWhy(o);
    if (why === undefined && !ruleEdit(o.uuid)) return { why: "Waiting for the server's list of rules." };
    if (why) return { why };
  }
  return { ok: true, rev: r };
}
function ruModes() { return (S.view?.modes || []).map((m) => ({ uuid: m.uuid.toLowerCase(), title: m.title })); }
function ruDefault(kind) {
  const mode = ruModes()[0]?.uuid || "";
  return {
    time: { at: "08:00" }, weekday: { days: ["weekdays"] }, state: { block: "", state: "", op: "==", value: "on" },
    mode: { mode, on: true }, weather: { field: "temperature", op: ">", value: "25" }, day: { op: "==", value: "1" },
    daylight: { on: true }, sun: { at: "sunrise" }, date: { date: "" }, then: { block: "", command: "" },
    then_mode: { mode, on: true }, notify: { subject: "", text: "" }, mail: { to: "", subject: "", text: "" },
    wait: { duration: "5s" }, message: { severity: "info", subject: "", text: "", admins: false },
  }[kind] || {};
}
/** The checks the server makes on the rule's shape, first: nothing is sent while one fails. */
function ruShape(lines) {
  if (!lines.length) return "A rule needs a condition and an action.";
  const first = lines.findIndex((l) => RU_ACTION.has(l.kind));
  if (first < 0) return "A rule does something: add an action.";
  if (lines.slice(first).some((l) => !RU_ACTION.has(l.kind))) return "A rule's conditions come before its actions.";
  if (!first) return "A rule runs on a condition: add one.";
  let n = 0;
  for (const l of lines.slice(0, first)) { if (l.kind === "or") { if (!n) return "Every group of conditions (between “or”s) has one at least."; n = 0; } else n++; }
  if (!n) return "Every group of conditions (between “or”s) has one at least.";
  for (const l of lines) {
    if ((l.kind === "state" || l.kind === "then") && !l.block) return "Pick the block of every condition and command.";
    if (l.kind === "then" && !l.command.trim()) return "Every command to a block names the command.";
    if (l.kind === "message" && /\//.test(l.subject + l.text)) return "A message's subject and text cannot contain “/”.";
  }
  return "";
}
function ruOpen(uuid, creating) {
  const o = S.view?.objects[uuid], st = o && (creating ? ruState(null) : ruState(o));
  if (!o?.rule || !st?.ok || (!creating && !o.rule.value)) return;
  const c = ruleEdit(uuid)?.created;
  const draft = creating
    ? { active: false, name: "", lines: [{ kind: "time", ...ruDefault("time") }, { kind: "then", ...ruDefault("then") }] }
    : { active: o.rule.value.active, name: c ? c.title : "", lines: structuredClone(o.rule.value.lines) };
  Object.assign(RU, { uuid, blob: st.rev.blob, open: true, creating: !!creating, ...draft, busy: false, msg: "", err: "" });
  S.closed.delete("Rule");
  renderProps();
  $("props").querySelector(creating ? '[data-ru-f="name"]' : '[data-ru-f="active"]')?.focus();
}
function ruSync() {
  if (RU.busy) return;
  if (RU.open && S.sel !== RU.uuid) Object.assign(RU, { open: false, creating: false, lines: [], msg: "", err: "" });
  else if (RU.open && editRev()?.blob !== RU.blob) Object.assign(RU, { open: false, creating: false, lines: [], err: "", msg: "The pending config changed while editing: the draft was dropped. Edit the rule again." });
}
function ruClose() {
  if (RU.busy) return;
  Object.assign(RU, { open: false, creating: false, lines: [], msg: "", err: "" });
  renderProps();
}
/** A block of a line, as a link (or the UUID when this config has no such object). */
function ruBlock(u) {
  if (!u) return '<span class="none-v">no block yet</span>';
  const k = Object.keys(S.view?.objects || {}).find((x) => ciEq(x, u));
  return k ? meLink(meTarget(k)) : `<span class="mono">${esc(u)}</span>`;
}
/** The suggestions for a block's states or commands: those the config's rules use on blocks of its type. */
function ruOffers(u, what) {
  const k = Object.keys(S.view?.objects || {}).find((x) => ciEq(x, u || ""));
  return (k && S.edits?.rule_edit?.offers?.[S.view.objects[k].ty]?.[what]) || [];
}
function ruField(i, f, label, value, attrs = "") {
  return `<label class="ru-f"><span>${esc(label)}</span><input type="text" data-ru-f="${f}" data-ru-field="${f}:${i}" data-i="${i}" value="${esc(value ?? "")}"${attrs}></label>`;
}
function ruSelect(i, f, label, value, opts) {
  // A value the config has that the list lacks is still shown, and kept.
  if (value != null && value !== "" && !opts.some(([v]) => v === value)) opts = [...opts, [value, `${value} (as written)`]];
  return `<label class="ru-f"><span>${esc(label)}</span><select data-ru-f="${f}" data-ru-field="${f}:${i}" data-i="${i}">${opts.map(([v, t]) => `<option value="${esc(v)}"${v === value ? " selected" : ""}>${esc(t)}</option>`).join("")}</select></label>`;
}
/** The fields of one line of the draft. */
function ruLineFields(l, i, dis) {
  const onOff = (f, label) => ruSelect(i, f, label, l[f] ? "on" : "off", [["on", "on"], ["off", "off"]]);
  const ops = (f) => ruSelect(i, f, "Compare", l[f], RU_OPS.map((o) => [o, o]));
  const modes = (f) => ruSelect(i, f, "Mode", (l[f] || "").toLowerCase(), [...ruModes().map((m) => [m.uuid, m.title]), ...(l[f] && !ruModes().some((m) => ciEq(m.uuid, l[f])) ? [[l[f], `${l[f]} (not in this config)`]] : [])]);
  const list = (id, items) => items.length ? `<datalist id="${id}">${items.map((x) => `<option value="${esc(x)}">`).join("")}</datalist>` : "";
  const pick = `<span class="ru-f"><span>Block</span>${ruBlock(l.block)} <button type="button" class="ed-btn small" data-ru="pick" data-i="${i}"${dis}>Pick…</button></span>`;
  switch (l.kind) {
    case "or": return '<span class="none-v">— or: the conditions below are another group —</span>';
    case "time": return ruField(i, "at", "Time (HH:MM)", l.at);
    case "weekday": return ruField(i, "days", "Days (mon … sun, weekdays, weekend, daily)", l.days.join(" "));
    case "state": return pick + ruField(i, "state", "State", l.state, ` list="ru-st-${i}"`) + list(`ru-st-${i}`, ruOffers(l.block, "states")) + ops("op") + ruField(i, "value", "Value (number, on, off)", l.value);
    case "mode": return modes("mode") + onOff("on", "While it is");
    case "weather": return ruSelect(i, "field", "Field", l.field, RU_WEATHER.map((w) => [w, w.replace(/_/g, " ")])) + ops("op") + ruField(i, "value", "Value", l.value);
    case "day": return ops("op") + ruField(i, "value", "Day (1 … 31)", l.value);
    case "daylight": return ruSelect(i, "on", "While it is", l.on ? "on" : "off", [["on", "day"], ["off", "night"]]);
    case "sun": return ruSelect(i, "at", "At", l.at, [["sunrise", "sunrise"], ["sunset", "sunset"], ["dawn", "dawn"], ["dusk", "dusk"]]);
    case "date": {
      const why = S.edits?.rule_edit?.date;
      return ruField(i, "date", "Date (YYYY-MM-DD)", l.date, ' placeholder="2026-12-24"') + (why ? `<span class="ed-note">${esc(why)}</span>` : "");
    }
    case "then": return pick + ruField(i, "command", "Command", l.command, ` list="ru-cm-${i}"`) + list(`ru-cm-${i}`, ruOffers(l.block, "commands"));
    case "then_mode": return modes("mode") + onOff("on", "Switch");
    case "notify": return ruField(i, "subject", "Subject", l.subject) + ruField(i, "text", "Text", l.text);
    case "mail": return ruField(i, "to", "To", l.to) + ruField(i, "subject", "Subject", l.subject) + ruField(i, "text", "Text", l.text);
    case "wait": return ruField(i, "duration", "Duration (500ms, 2.5s, 5min, 1h)", l.duration);
    case "message": return ruSelect(i, "severity", "Severity", l.severity, [["info", "info"], ["warning", "warning"], ["important", "important"], ["critical", "critical"]]) + ruField(i, "subject", "Subject", l.subject) + ruField(i, "text", "Text", l.text) +
      `<label class="ru-f ru-chk"><input type="checkbox" data-ru-f="admins" data-i="${i}"${l.admins ? " checked" : ""}> administrators only</label>`;
  }
  return "";
}
/** The rows of the *Rule* section. */
function ruRows(o, row, marks) {
  const ri = o.rule;
  if (!ri) return null;
  const rows = [], st = ruState(o), e = ruleEdit(o.uuid), mine = RU.uuid === o.uuid ? RU : null, dis = mine?.busy ? " disabled" : "";
  const msg = () => (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "");
  const other = (S.edits?.rule_claims || []).find((c) => c.blocks?.includes(o.uuid.toLowerCase()) && !c.file.startsWith(`${S.edits?.dir}/`));
  if (other && e?.value) {
    rows.push(`<div class="sch-note warn">⚠ Declared twice: ${esc(claimAt(other))} and vislox's ${esc(S.edits?.file || "")} both state this rule, and lox compiles neither until one goes.</div>`,
      `<div class="ed-bar"><button type="button" class="ed-btn small" data-ru="drop"${dis}>Remove vislox's declaration</button>${msg()}</div>`);
  } else if (other) rows.push(`<div class="ed-note">${esc(claimAt(other))} states this rule: change it there.</div>`);
  if (e?.created) rows.push(`<div class="ed-note">Created here as <span class="mono">${esc(e.created.slug)}</span> in ${esc(S.edits?.file || "")}: its name and what it does are the module's.</div>`);
  else if (e?.value) rows.push(`<div class="ed-note">Stated by vislox in ${esc(S.edits?.file || "")}: the declaration owns whether it is active, its conditions and its actions; its name stays the app's.</div>`);
  if (e?.pending && st?.ok && !other) {
    const back = e.created ? "Back to the deployed rule" : e.deployed ? "Back to the rule deployed" : "Back to the rule as last written: vislox's declaration goes";
    const can = !e.created || e.deployed;
    rows.push(row("Rule", `<span class="ed-v">edited here</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      (can ? `<button type="button" class="ed-btn small" data-ru="revert"${dis} title="${esc(back)}">Revert</button>` : ""), "pr-chg"));
  }
  if (!mine?.open || (mine.creating ? !ruState(null)?.ok : !st?.ok || other)) {
    const old = marks ? o.rule_old : null;
    rows.push(row("Status", old && old.active !== ri.active ? `<span class="old">${old.active ? "active" : "inactive"}</span><span class="new">${ri.active ? "active" : "inactive"}</span>` : (ri.active ? "active" : '<span class="none-v">inactive</span>'), old && old.active !== ri.active ? "pr-chg" : ""));
    if (ri.why) rows.push(`<div class="ed-note">${esc(ri.why)}</div>`);
    else {
      rows.push(`<div class="ru-sum${old ? " pr-chg" : ""}">${old ? `<span class="old">${esc(old.summary)}</span><br>` : ""}${esc(ri.summary)}</div>`);
      for (const l of ri.lines) {
        if (l.kind === "or") { rows.push('<div class="ru-or">or</div>'); continue; }
        const label = l.kind === "then" ? "Then" : "When";
        rows.push(row(label, `${esc(l.words)}${l.target ? ` <span class="ru-link">${linkTarget(l.target)}</span>` : ""}`));
      }
    }
    const bar = [];
    if (st?.ok && !other && ri.value) bar.push(`<button type="button" class="ed-btn small" data-ru="open"${dis}>Edit rule</button>`);
    if (st?.ok && e?.value && !e.created && !other) bar.push(`<button type="button" class="ed-btn small" data-ru="drop"${dis} title="The rule stays as last written">Remove vislox's declaration</button>`);
    if (st?.ok && e?.created) bar.push(`<button type="button" class="ed-btn small" data-ru="delete"${dis} title="Take the rule out of the config (on Apply, of the Miniserver)">Delete rule</button>`);
    const create = S.edits?.rule_edit?.create;
    const ns = ruState(null);
    if (ns?.ok) bar.push(`<button type="button" class="ed-btn small" data-ru="new"${dis || (create ? " disabled" : "")}${create ? ` title="${esc(create)}"` : ""}>New rule…</button>`);
    if (st && !st.ok && st.why && !ri.why) rows.push(`<div class="ed-note">${esc(st.why)}</div>`);
    if (bar.length || msg()) rows.push(`<div class="ed-bar">${bar.join("")}${msg()}</div>`);
    return { rows, count: e?.pending ? 1 : 0 };
  }
  // The editor: of this rule, or of a new one.
  if (mine.creating) rows.push('<div class="ed-note">A new rule: lox creates it below the config\'s last rule, with this name, its conditions and its actions.</div>');
  rows.push(`<div class="ed-bar"><label class="ru-f ru-chk"><input type="checkbox" data-ru-f="active"${RU.active ? " checked" : ""}${dis}> Active</label>` +
    (e?.created || mine.creating ? `<label class="ru-f"><span>Name</span><input type="text" data-ru-f="name" data-ru-field="name" value="${esc(RU.name)}"${dis}></label>` : "") + "</div>");
  const items = RU.lines.map((l, i) => {
    const kinds = RU_ACTION.has(l.kind) ? RU_THEN : l.kind === "or" ? [["or", "or"]] : RU_WHEN;
    return `<li class="ru-item"><span class="ru-k">${l.kind === "or" ? "" : `<select data-ru-f="kind" data-i="${i}" aria-label="Kind"${dis}>${kinds.map(([k, t]) => `<option value="${k}"${k === l.kind ? " selected" : ""}>${esc(RU_ACTION.has(k) ? `Then: ${t}` : `When: ${t}`)}</option>`).join("")}</select>`}</span>` +
      `<span class="ru-fields">${ruLineFields(l, i, dis)}</span><span class="me-ops">` +
      `<button type="button" class="se-x" data-ru="up" data-i="${i}" aria-label="Move line ${i + 1} up"${i ? "" : " disabled"}${dis}>↑</button>` +
      `<button type="button" class="se-x" data-ru="down" data-i="${i}" aria-label="Move line ${i + 1} down"${i < RU.lines.length - 1 ? "" : " disabled"}${dis}>↓</button>` +
      `<button type="button" class="se-x" data-ru="del" data-i="${i}" aria-label="Remove line ${i + 1}"${dis}>✕</button></span></li>`;
  }).join("");
  rows.push(RU.lines.length ? `<ol class="ru-list">${items}</ol>` : '<div class="ed-note">No conditions and no actions: add a condition and an action.</div>');
  rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-ru="addwhen"${dis}>Add condition</button><button type="button" class="ed-btn small" data-ru="addor"${dis}>Add “or”</button><button type="button" class="ed-btn small" data-ru="addthen"${dis}>Add action</button></div>`);
  const shape = ruShape(RU.lines);
  rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-ru="save"${dis}${shape ? " disabled" : ""}>${mine.busy ? "Compiling…" : "Save"}</button>` +
    `<button type="button" class="ed-btn small" data-ru="cancel"${dis}>Cancel</button>` +
    `<span class="ed-msg${mine.err || shape ? " err" : ""}" role="status" aria-live="polite">${esc(mine.err || shape || mine.msg || "Saving states the rule in the module and compiles it; nothing is deployed. lox checks each block's states and commands.")}</span></div>`);
  return { rows, count: e?.pending ? 1 : 0 };
}
/** After a new rule compiled: show it, on its page. */
function ruShowNew(res) {
  const uuid = res.added?.[0]?.uuid;
  const u = uuid && Object.keys(S.view?.objects || {}).find((k) => ciEq(k, uuid));
  if (!u) return;
  const x = S.view.objects[u];
  if (x.page) gotoPage(x.page, { focus: u });
  select(u);
}
async function ruSend(o, base, ops, label, after) {
  RU.uuid = o.uuid; RU.busy = true; RU.err = ""; RU.msg = ""; renderProps();
  try {
    const res = await editAct(base, ops, label);
    Object.assign(RU, { open: false, creating: false, lines: [], busy: false, err: "", msg: res.unchanged ? "Nothing changed." : "Saved and compiled: pending, not deployed. Review it, then Apply or Discard." });
    await editShowNewest(res.blob);
    after?.(res);
  } catch (err) {
    RU.busy = false; RU.err = err.message;
  }
  renderProps();
}
function ruSave() {
  const o = S.view?.objects[RU.uuid], st = o && (RU.creating ? ruState(null) : ruState(o));
  if (!st?.ok || RU.busy || !RU.open) return;
  if (st.rev.blob !== RU.blob) {
    Object.assign(RU, { open: false, lines: [], err: "The pending config changed while editing: the draft was dropped. Edit the rule again.", msg: "" });
    renderProps();
    return;
  }
  const shape = ruShape(RU.lines);
  if (shape) { RU.err = shape; renderProps(); return; }
  if (RU.creating) {
    const title = RU.name.trim();
    if (!title) { RU.err = "A new rule needs a name."; renderProps(); return; }
    ruSend(o, RU.blob, [{ op: "new_rule", title, value: { active: RU.active, lines: RU.lines } }], `New rule ${title}`, ruShowNew);
    return;
  }
  const ops = [];
  const c = ruleEdit(o.uuid)?.created;
  if (c && RU.name.trim() !== c.title) ops.push({ op: "title", uuid: o.uuid, value: RU.name.trim() });
  ops.push({ op: "rule", uuid: o.uuid, value: { active: RU.active, lines: RU.lines } });
  ruSend(o, RU.blob, ops, `Rule ${displayTitle(o)}`);
}
function ruPick(i) {
  const o = S.view?.objects[RU.uuid];
  if (!o || !RU.open || RU.busy) return;
  const types = [...new Set(Object.values(S.view.objects).map((x) => x.ty))].filter((t) => t !== "AutopilotRule");
  openPicker({
    title: `The block of line ${i + 1}`, placeholder: "Pick a block…", none: "No block to pick.", types, exclude: new Set(),
    onPick: (u) => { RU.lines[i].block = u; RU.err = ""; renderProps(); $("props").querySelector(`[data-ru="pick"][data-i="${i}"]`)?.focus(); },
    back: () => $("props").querySelector(`[data-ru="pick"][data-i="${i}"]`)?.focus(),
  });
}
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-ru]");
  if (!b) return;
  e.stopPropagation();
  const o = S.view?.objects[S.sel], what = b.dataset.ru;
  if (!o) return;
  const st = ruState(o), ed = ruleEdit(o.uuid);
  if (what === "open") return ruOpen(S.sel);
  if (what === "cancel") return ruClose();
  if (what === "save") return ruSave();
  if (what === "new") return ruOpen(S.sel, true);
  if (what === "revert") {
    if (!st?.ok || !ed) return;
    const ops = [];
    if (ed.created && ed.deployed == null) return;
    // A rule created here gets its deployed name back too.
    if (ed.created && ed.deployed_title && ed.deployed_title !== ed.created.title) ops.push({ op: "title", uuid: o.uuid, value: ed.deployed_title });
    ops.push({ op: "rule", uuid: o.uuid, value: ed.deployed ?? null });
    ruSend(o, st.rev.blob, ops, `Revert the rule ${displayTitle(o)}`);
    return;
  }
  if (what === "drop") { if (st?.ok || ed?.value) { const r = editRev(); if (r) ruSend(o, r.blob, [{ op: "rule", uuid: o.uuid, value: null }], `Remove vislox's declaration of ${displayTitle(o)}`); } return; }
  if (what === "delete") { if (st?.ok && ed?.created) ruSend(o, st.rev.blob, [{ op: "remove_block", uuid: o.uuid }], `Delete the rule ${displayTitle(o)}`); return; }
  if (RU.busy || !RU.open) return;
  const i = +b.dataset.i;
  if (what === "pick") return ruPick(i);
  if (what === "addwhen" || what === "addthen" || what === "addor") {
    const first = RU.lines.findIndex((l) => RU_ACTION.has(l.kind));
    const at = what === "addthen" || first < 0 ? RU.lines.length : first;
    const line = what === "addor" ? { kind: "or" } : what === "addthen" ? { kind: "then", ...ruDefault("then") } : { kind: "time", ...ruDefault("time") };
    RU.lines.splice(at, 0, line);
  } else if (what === "del") RU.lines.splice(i, 1);
  else if (what === "up" && i > 0) RU.lines.splice(i - 1, 0, RU.lines.splice(i, 1)[0]);
  else if (what === "down" && i < RU.lines.length - 1) RU.lines.splice(i + 1, 0, RU.lines.splice(i, 1)[0]);
  RU.err = "";
  renderProps();
}), true);
/** A field of the draft: kept as typed, the form redrawn only where its shape changes (kind, block, on/off). */
$("props").addEventListener("input", whenReady((e) => {
  const f = e.target.closest?.("[data-ru-f]");
  if (!f) return;
  const k = f.dataset.ruF, i = +f.dataset.i;
  if (k === "name") { RU.name = f.value; return; }
  if (k === "active") { RU.active = f.checked; return; }
  const l = RU.lines[i];
  if (!l || RU.busy) return;
  if (k === "kind") { RU.lines[i] = { kind: f.value, ...ruDefault(f.value) }; RU.err = ""; renderProps(); return; }
  if (k === "days") l.days = f.value.trim().split(/\s+/).filter(Boolean);
  else if (k === "on") l.on = f.value === "on";
  else if (k === "admins") l.admins = f.checked;
  else l[k] = f.value;
  RU.err = "";
  // The Save button and message follow the draft's shape.
  const shape = ruShape(RU.lines), save = $("props").querySelector('[data-ru="save"]');
  if (save) save.disabled = !!shape;
  const m = save?.parentElement?.querySelector(".ed-msg");
  if (m) { m.textContent = shape || "Saving states the rule in the module and compiles it; nothing is deployed. lox checks each block's states and commands."; m.classList.toggle("err", !!shape); }
}));

// ── who may open an NFC Code Touch (#76), who gets a push notification (#77) ──
// A Code Touch's access rights are a Lox Logic `access` set (users and groups, each with an access schedule) and
// the Code Touch's `.access` claim; a push notification's recipients a `recipients` set (users and groups, no
// schedule) and the notification's `.recipients` claim. Both are written into vislox's file of claims, and the set
// is the whole list: whoever it leaves out loses the right (members of Full access get every notification anyway:
// inherited, never listed). Properties' *Access* or *Recipients* shows who holds the right (the review's, #68);
// *Edit access* / *Edit recipients* adds or removes a user or group (and, for a door, picks its schedule). The
// pickers' lists (every user, group and access schedule of the config, by title) are personal data: they are asked
// for with the admin token (`POST /api/access/choices`), kept in memory only while the editor is open, and never put
// in an address, the page title or storage. The draft and every op name them by UUID. Before a save that takes a
// right away, the editor names who loses it (*Save anyway*); Apply's confirmation says it again (#68). Saving sends
// one `access` or `recipients` op: compiled, never deployed, undone through `undo`. The draft belongs to one block
// and one pending config (`blob`): when either changes, it is dropped, and a save sends the blob it was made on,
// which the server checks.
const AC = { uuid: null, kind: null, blob: null, open: false, rows: [], choices: null, busy: false, msg: "", err: "", warned: "", shareAll: true, dup: [], same: null };
/** Per kind: the block type, the op and `GET /api/edits` keys, and the words. */
const AC_KIND = {
  access: { ty: "NfcCodeTouch", op: "access", sets: "access_sets", members: "code_touches", content: "grants", sec: "Access", sched: true,
    what: "access rights", edit: "Edit access", same: "Same rights as", block: "Code Touch", none: "No user or group: saving revokes everyone's access.",
    leaves: "Whoever the set leaves out loses access." },
  recipients: { ty: "Notification", op: "recipients", sets: "recipient_sets", members: "notifications", content: "recipients", sec: "Recipients", sched: false,
    what: "recipients", edit: "Edit recipients", same: "Same recipients as", block: "notification", none: "No user or group: saving leaves only the members of Full access getting it.",
    leaves: "Whoever the set leaves out stops getting it (Full access aside)." },
};
function acKindOf(o) { return o?.rights && AC_KIND[o.rights.kind] ? o.rights.kind : null; }
/** What `GET /api/edits` says of this block's access rights (#76) or recipients (#77), or null. */
function accessEdit(uuid, kind = "access") { return (S.edits?.[AC_KIND[kind].op] || []).find((m) => m.uuid === uuid) || null; }
/** The blocks whose access rights (or recipients) were edited here and are not deployed. */
function accessPending(kind = "access") { return (S.edits?.[AC_KIND[kind].op] || []).filter((m) => m.pending); }
/** Whether this block's access rights or recipients can be edited here now: `{ ok, rev }`, or null. */
function acState(o) {
  const kind = acKindOf(o);
  if (!kind || o.ty !== AC_KIND[kind].ty) return null;
  const st = editState({ ...o, edit: o.edit || {} });
  return st?.ok ? st : null;
}
/** The module's own claim on it (a file other than vislox's), if any. */
function acOther(o) { const kind = acKindOf(o); return claimsOf(o.uuid).find((c) => c.kind === kind && !c.file.startsWith(`${S.edits?.dir}/`)) || null; }
function acReset(extra) { Object.assign(AC, { open: false, rows: [], choices: null, dup: [], warned: "", same: null, msg: "", err: "" }, extra || {}); }
/** Drop a draft made on another block or another pending config than the one shown now. */
function acSync() {
  if (AC.busy) return;
  if (S.sel !== AC.uuid) { if (AC.open || AC.same || AC.msg || AC.err) acReset(); }
  else if ((AC.open || AC.same) && editRev()?.blob !== AC.blob) acReset({ msg: `The pending config changed while editing: the draft was dropped. ${AC_KIND[AC.kind || "access"].edit} again.` });
}
async function acOpen(uuid) {
  const o = S.view?.objects[uuid], st = o && acState(o), kind = acKindOf(o);
  if (!st || AC.busy) return;
  Object.assign(AC, { uuid, kind, blob: st.rev.blob, busy: true, msg: "", err: "", warned: "", same: null });
  renderProps();
  let choices;
  try { choices = await acChoices(AC.blob); } catch (err) { AC.busy = false; AC.err = `Cannot list the users: ${err.message}`; renderProps(); return; }
  AC.busy = false;
  // Selected elsewhere, or a new pending config, while the list was asked for: no draft.
  if (S.sel !== uuid || editRev()?.blob !== AC.blob) { acReset(); renderProps(); return; }
  // One right per user or group: a second schedule the config holds for one goes on save, and the draft says so.
  const rows = [], dup = [];
  for (const p of o.rights.list) {
    const u = p.uuid.toLowerCase();
    if (rows.some((r) => r.uuid === u)) { dup.push(whoText(p)); continue; }
    rows.push({ kind: p.kind, uuid: u, schedule: (p.schedule_uuid || "").toLowerCase() });
  }
  Object.assign(AC, { open: true, rows, choices, dup, shareAll: true });
  S.closed.delete(AC_KIND[kind].sec);
  renderProps();
  $("props").querySelector('[data-ac-p="0"], [data-ac="add"]')?.focus();
}
/** Titles two objects of a kind share cannot be named in Lox Logic: they cannot be picked. */
function acTitles(list) {
  const n = new Map();
  for (const x of list) n.set(x.title, (n.get(x.title) || 0) + 1);
  return (x) => (!x.title ? "it has no title" : n.get(x.title) > 1 ? "its title is not unique" : "");
}
/** A user or group of the pickers' lists, by UUID. */
function acWho(kind, uuid) { return (kind === "group" ? AC.choices?.groups : AC.choices?.users)?.find((x) => x.uuid === uuid) || null; }
/** The pickers' lists of the config the draft is made on (its blob): the server answers from that revision, or
 *  refuses when it is gone, never from another one. */
function acChoices(blob) { return act("/api/access/choices", { base: blob }); }
/** A block's rights as draft rows: one per right (a user or group may hold two, under two schedules). */
function acRowsOf(x) { return (x?.rights?.list || []).map((p) => ({ kind: p.kind, uuid: p.uuid.toLowerCase(), schedule: (p.schedule_uuid || "").toLowerCase() })); }
/** Each user a set of rights reaches → the access schedules that do (directly, or through a group); for
 *  recipients, the schedule is empty, and the members of Full access always get it. */
function acUserScheds(rows, choices, recipients) {
  const m = new Map();
  const add = (u, sch) => { if (!m.has(u)) m.set(u, new Set()); m.get(u).add(sch); };
  for (const r of rows) {
    if (r.kind === "user") add(r.uuid, r.schedule);
    else for (const u of choices.groups.find((g) => g.uuid === r.uuid)?.users || []) add(u, r.schedule);
  }
  if (recipients) for (const g of choices.groups.filter((x) => x.full_access)) for (const u of g.users) add(u, "");
  return m;
}
/** What going from the rights `was` to `now` takes away on the block `title`, in sentences: who loses access (or
 *  stops getting the notification), and who keeps a door under fewer access schedules. vislox cannot tell whether
 *  one schedule's times cover another's, so any schedule taken from a user counts as a loss. */
function acLossesOf(title, was, now, choices, kind = "access") {
  const rec = kind === "recipients";
  const before = acUserScheds(was, choices, rec), after = acUserScheds(now, choices, rec);
  const name = (u) => choices.users.find((x) => x.uuid === u)?.title || u;
  const sch = (id) => choices.schedules.find((x) => x.uuid === id)?.title || id;
  const out = [];
  const lost = [...before.keys()].filter((u) => !after.has(u)).map(name);
  if (lost.length) out.push(rec ? `${lost.join(", ")} ${lost.length === 1 ? "stops" : "stop"} getting ${title}` : `${lost.join(", ")} ${lost.length === 1 ? "loses" : "lose"} access to ${title}`);
  if (rec) return out;
  for (const [u, b] of before) {
    const a = after.get(u);
    if (!a) continue;
    const gone = [...b].filter((x) => !a.has(x));
    if (gone.length) out.push(`${name(u)} may open ${title} only at ${[...a].map(sch).join(", ")}, no longer at ${gone.map(sch).join(", ")}`);
  }
  return out;
}
/** The losses of saving the draft: on this block, and on those sharing its set when they change too. */
function acLosses(o) {
  const kind = acKindOf(o), e = accessEdit(o.uuid, kind);
  const blocks = [o.uuid, ...(e?.set && e.shared?.length && AC.shareAll ? e.shared : [])];
  return blocks.flatMap((d) => { const x = S.view?.objects[d]; return acLossesOf(x ? displayTitle(x) : d, acRowsOf(x), AC.rows, AC.choices, kind); });
}
/** The rows of *Edit access* or *Edit recipients*, below who holds the right. */
function acRows(o) {
  const rows = [], st = acState(o), kind = acKindOf(o), K = AC_KIND[kind], e = st && accessEdit(o.uuid, kind), mine = AC.uuid === o.uuid ? AC : null;
  if (!st) return rows;
  const other = acOther(o), dis = mine?.busy ? " disabled" : "";
  const title = (u) => { const x = S.view?.objects[u]; return x ? displayTitle(x) : u; };
  const msg = () => (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "");
  if (other && e?.set) {
    rows.push(`<div class="sch-note warn">⚠ Claimed twice: ${esc(claimAt(other))} and vislox's ${esc(S.edits?.file || "")} both claim its ${K.what}, and lox compiles neither until one goes.</div>`,
      `<div class="ed-bar"><button type="button" class="ed-btn small" data-ac="drop"${dis} title="Take vislox's claim out and compile: ${esc(claimAt(other))} keeps the ${K.block}">Remove vislox's claim</button>${msg()}</div>`);
    return rows;
  }
  if (other) { rows.push(`<div class="ed-note">${esc(claimAt(other))} claims its ${K.what}: change them there.</div>`); return rows; }
  if (e?.set) {
    rows.push(`<div class="ed-note">Written by vislox as <span class="mono">${esc(e.set)}</span> in ${esc(S.edits?.file || "")}${e.shared?.length ? `, shared with ${esc(e.shared.map(title).join(", "))}` : ""}. ${K.leaves}</div>`);
    rows.push(...staleRows((S.edits?.[K.sets] || []).find((x) => x.name === e.set), "ac", mine));
  }
  if (e?.pending) {
    const back = e.deployed ? `Back to the ${K.what} deployed` : `Back to the ${K.what} as last written: vislox's claim goes`;
    rows.push(propRow(K.sec, `<span class="ed-v">edited here</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      `<button type="button" class="ed-btn small" data-ac="revert"${dis} title="${esc(back)}">Revert</button>`, "pr-chg"));
  }
  if (mine?.same) {
    rows.push(`<div class="sch-note warn" role="alert">⚠ ${esc(mine.same.lose.join("; "))}.</div>`,
      `<div class="ed-bar"><button type="button" class="primary small" data-ac="sameok"${dis}>${mine.busy ? "Compiling…" : "Use anyway"}</button><button type="button" class="ed-btn small" data-ac="samecancel"${dis}>Cancel</button>${msg()}</div>`);
    return rows;
  }
  if (!mine?.open) {
    const sets = (S.edits?.[K.sets] || []).filter((x) => x.name !== e?.set && x[K.members].length && !x[K.members].includes(o.uuid));
    rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-ac="open"${dis}>${mine?.busy ? "Listing users…" : K.edit}</button>` +
      (sets.length ? `<label class="se-same">${K.same} <select data-ac="same"${dis}><option value="">…</option>${sets.map((x) => `<option value="${esc(x.name)}">${esc(x[K.members].map(title).join(", "))}</option>`).join("")}</select></label>` : "") +
      msg() + "</div>");
    return rows;
  }
  // The editor: a row per user or group, its schedule (a door's), ✕; *Add user or group*.
  const c = AC.choices, uWhy = acTitles(c.users), gWhy = acTitles(c.groups), sWhy = acTitles(c.schedules);
  const taken = new Set(AC.rows.map((r) => r.uuid));
  const opt = (x, k, sel, why) => `<option value="${k}|${esc(x.uuid)}"${sel ? " selected" : ""}${why && !sel ? " disabled" : ""}${!sel && taken.has(x.uuid) ? " disabled" : ""}>${esc(x.title || x.uuid)}${why ? ` (${esc(why)})` : ""}${!K.sched && x.full_access ? " (gets every notification anyway)" : ""}</option>`;
  if (AC.dup.length) rows.push(`<div class="sch-note warn">⚠ ${esc(AC.dup.join(", "))} ${AC.dup.length === 1 ? "holds" : "hold"} a second right under another schedule: a set gives each user or group one schedule, so saving keeps the first.</div>`);
  const items = AC.rows.map((r, i) => {
    const who = acWho(r.kind, r.uuid), name = who?.title || r.uuid;
    const pSel = `<select data-ac-p="${i}" aria-label="User or group ${i + 1}"${dis}>` +
      `<optgroup label="Users">${c.users.map((x) => opt(x, "user", r.kind === "user" && x.uuid === r.uuid, uWhy(x))).join("")}</optgroup>` +
      `<optgroup label="Groups">${c.groups.map((x) => opt(x, "group", r.kind === "group" && x.uuid === r.uuid, gWhy(x))).join("")}</optgroup></select>`;
    const sSel = !K.sched ? "" : `<span class="none-v">at</span><select data-ac-s="${i}" aria-label="Access schedule of ${esc(name)}"${dis}>${r.schedule && !c.schedules.some((x) => x.uuid === r.schedule) ? `<option value="" selected>—</option>` : ""}` +
      c.schedules.map((x) => { const why = sWhy(x), sel = x.uuid === r.schedule; return `<option value="${esc(x.uuid)}"${sel ? " selected" : ""}${why && !sel ? " disabled" : ""}>${esc(x.title || x.uuid)}${why ? ` (${esc(why)})` : ""}</option>`; }).join("") + "</select>";
    const bad = !who ? "not in this config" : (r.kind === "group" ? gWhy(who) : uWhy(who)) || (!K.sched ? "" : !c.schedules.some((x) => x.uuid === r.schedule) ? "no access schedule" : sWhy(c.schedules.find((x) => x.uuid === r.schedule)));
    const tip = r.kind === "group" && who?.users?.length ? ` title="${esc(`Members: ${who.users.map((u) => acWho("user", u)?.title || u).join(", ")}`)}"` : "";
    return `<li class="ac-item${bad ? " bad" : ""}"${tip}>${pSel}${sSel}` +
      `<button type="button" class="se-x" data-ac="del" data-i="${i}" aria-label="Remove ${esc(name)}"${dis}>✕</button>${bad ? `<span class="ed-msg err">${esc(bad)}</span>` : ""}</li>`;
  }).join("");
  rows.push(AC.rows.length ? `<ul class="ac-list">${items}</ul>` : `<div class="sch-note warn">${esc(K.none)}</div>`);
  const free = c.users.some((x) => !taken.has(x.uuid) && !uWhy(x)) || c.groups.some((x) => !taken.has(x.uuid) && !gWhy(x));
  rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-ac="add"${free && (!K.sched || c.schedules.length) ? "" : " disabled"}${dis}>Add user or group</button>` +
    `<span class="none-v">Users, groups${K.sched ? " and access schedules" : ""} come from Loxone Config and the app.</span></div>`);
  if (e?.shared?.length) rows.push(`<label class="se-share"><input type="checkbox" data-ac="shareall"${AC.shareAll ? " checked" : ""}${dis}> Change ${esc(e.shared.map(title).join(", "))} too: ${e.shared.length === 1 ? "it shares" : "they share"} these ${K.what} (${esc(e.set)}). Off: this ${K.block} gets a set of its own.</label>`);
  if (AC.warned) rows.push(`<div class="sch-note warn" role="alert">⚠ ${esc(AC.warned)}</div>`);
  rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-ac="save"${dis}>${AC.busy ? "Compiling…" : AC.warned ? "Save anyway" : "Save"}</button>` +
    `<button type="button" class="ed-btn small" data-ac="cancel"${dis}>Cancel</button>` +
    `<span class="ed-msg${AC.err ? " err" : ""}" role="status" aria-live="polite">${esc(AC.err || AC.msg || "Saving claims the set in the module and compiles it; nothing is deployed.")}</span></div>`);
  return rows;
}
async function acSend(o, base, op, label) {
  AC.uuid = o.uuid; AC.kind = acKindOf(o); AC.busy = true; AC.err = ""; AC.msg = ""; renderProps();
  try {
    const res = await editAct(base, [op], label);
    AC.busy = false;
    acReset({ msg: res.unchanged ? "Nothing changed." : "Saved and compiled: pending, not deployed. Review it, then Apply or Discard." });
    await editShowNewest(res.blob);
  } catch (err) {
    AC.busy = false; AC.err = err.message; AC.warned = "";
  }
  renderProps();
}
async function acSave() {
  const o = S.view?.objects[AC.uuid], st = o && acState(o), kind = acKindOf(o), K = AC_KIND[kind];
  if (!st || AC.busy || !AC.open) return;
  // Never save a draft over a newer config: the draft goes against the config it was made on.
  if (st.rev.blob !== AC.blob) { acReset({ err: `The pending config changed while editing: the draft was dropped. ${K.edit} again.` }); renderProps(); return; }
  if (AC.rows.some((r) => !acWho(r.kind, r.uuid) || (K.sched && !AC.choices.schedules.some((x) => x.uuid === r.schedule)))) {
    AC.err = K.sched ? "Pick a user or group and an access schedule on every row, or remove it." : "Pick a user or group on every row, or remove it.";
    renderProps();
    return;
  }
  // Warned once: taking a right away is said before the save, as Apply's confirmation says it again.
  const lose = acLosses(o);
  if (lose.length && !AC.warned) { AC.warned = `${lose.join("; ")}.`; renderProps(); return; }
  const e = accessEdit(o.uuid, kind);
  const list = AC.rows.map((r) => (K.sched ? { kind: r.kind, uuid: r.uuid, schedule: r.schedule } : { kind: r.kind, uuid: r.uuid }));
  const value = { [K.content]: list, ...(e?.set && e.shared?.length && AC.shareAll ? { set: e.set } : {}) };
  await acSend(o, AC.blob, { op: K.op, uuid: o.uuid, value });
}
/** *Same rights as …* / *Same recipients as …*: claim another block's set as it is; who loses a right is said first. */
async function acSame(o, set) {
  const st = acState(o), kind = acKindOf(o), K = AC_KIND[kind], x = (S.edits?.[K.sets] || []).find((s) => s.name === set);
  if (!st || !x || AC.busy) return;
  const blob = st.rev.blob;
  const label = `${K.same} ${x[K.members].map((u) => { const y = S.view?.objects[u]; return y ? displayTitle(y) : u; }).join(", ")} for ${displayTitle(o)}`;
  Object.assign(AC, { uuid: o.uuid, kind, blob, busy: true, open: false, msg: "", err: "", same: null });
  renderProps();
  let choices;
  try { choices = await acChoices(blob); } catch (err) { AC.busy = false; AC.err = `Cannot list the users: ${err.message}`; renderProps(); return; }
  AC.busy = false;
  if (S.sel !== o.uuid || editRev()?.blob !== blob) { acReset(); renderProps(); return; }
  // The set's rights as a block claiming it has them now.
  const lose = acLossesOf(displayTitle(o), acRowsOf(o), acRowsOf(S.view?.objects[x[K.members][0]]), choices, kind);
  if (!lose.length) return acSend(o, blob, { op: K.op, uuid: o.uuid, value: { set } }, label);
  AC.same = { set, label, lose };
  renderProps();
  $("props").querySelector('[data-ac="sameok"]')?.focus();
}
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-ac]");
  if (!b || b.tagName === "SELECT" || b.type === "checkbox") return;
  e.stopPropagation();
  const o = S.view?.objects[S.sel], what = b.dataset.ac;
  if (!o) return;
  if (what === "open") return acOpen(S.sel);
  if (what === "cancel") { if (!AC.busy) { acReset(); renderProps(); $("props").querySelector('[data-ac="open"]')?.focus(); } return; }
  if (what === "save") return acSave();
  const st = acState(o), kind = acKindOf(o), K = AC_KIND[kind] || AC_KIND.access, ed = kind && accessEdit(o.uuid, kind);
  if (what === "samecancel") { if (!AC.busy) { acReset(); renderProps(); } return; }
  if (what === "sameok") {
    if (!st || AC.busy || !AC.same) return;
    if (st.rev.blob !== AC.blob) { acReset({ err: "The pending config changed: nothing was saved. Pick again." }); renderProps(); return; }
    const { set, label } = AC.same;
    AC.same = null;
    return acSend(o, AC.blob, { op: K.op, uuid: o.uuid, value: { set } }, label);
  }
  if (what === "revert") {
    if (!st || !ed) return;
    const r = setRevertValue(K.op, ed, S.edits?.[K.sets] || [], o.uuid);
    if (r.why) { AC.uuid = o.uuid; AC.kind = kind; AC.err = r.why; AC.msg = ""; renderProps(); return; }
    return acSend(o, st.rev.blob, { op: K.op, uuid: o.uuid, value: r.value }, `Revert the ${K.what} of ${displayTitle(o)}`);
  }
  if (what === "drop") { if (st) acSend(o, st.rev.blob, { op: K.op, uuid: o.uuid, value: null }, `Remove vislox's claim on the ${K.what} of ${displayTitle(o)}`); return; }
  if (what === "dropstale") { if (st) acSend(o, st.rev.blob, { op: K.op, uuid: b.dataset.u, value: null }, `Drop a stale claim on ${K.what}`); return; }
  if (AC.busy || !AC.open) return;
  const i = +b.dataset.i;
  if (what === "del") AC.rows.splice(i, 1);
  else if (what === "add") {
    const c = AC.choices, uWhy = acTitles(c.users), gWhy = acTitles(c.groups), sWhy = acTitles(c.schedules);
    const taken = new Set(AC.rows.map((r) => r.uuid));
    const u = c.users.find((x) => !taken.has(x.uuid) && !uWhy(x)), g = c.groups.find((x) => !taken.has(x.uuid) && !gWhy(x));
    const s = c.schedules.find((x) => !sWhy(x));
    if ((u || g) && (s || !K.sched)) AC.rows.push({ kind: u ? "user" : "group", uuid: (u || g).uuid, schedule: K.sched ? s.uuid : "" });
  }
  AC.err = ""; AC.warned = "";
  renderProps();
  if (what === "add") $("props").querySelector(`[data-ac-p="${AC.rows.length - 1}"]`)?.focus();
  else ($("props").querySelector(`[data-ac="del"][data-i="${Math.min(i, AC.rows.length - 1)}"]`) || $("props").querySelector('[data-ac="add"]'))?.focus();
}), true);
$("props").addEventListener("change", whenReady((e) => {
  const t = e.target;
  if (!t.closest?.("[data-ac], [data-ac-p], [data-ac-s]")) return;
  const o = S.view?.objects[S.sel];
  if (!o) return;
  if (t.dataset.ac === "same") { if (t.value) acSame(o, t.value); return; }
  if (AC.busy || !AC.open) return;
  if (t.dataset.ac === "shareall") AC.shareAll = t.checked;
  else if (t.dataset.acP != null) {
    const r = AC.rows[+t.dataset.acP], [kind, uuid] = t.value.split("|");
    if (r) Object.assign(r, { kind, uuid });
  } else if (t.dataset.acS != null) {
    const r = AC.rows[+t.dataset.acS];
    if (r) r.schedule = t.value;
  }
  AC.err = ""; AC.warned = "";
  renderProps();
  $("props").querySelector(t.dataset.acP != null ? `[data-ac-p="${t.dataset.acP}"]` : t.dataset.acS != null ? `[data-ac-s="${t.dataset.acS}"]` : '[data-ac="shareall"]')?.focus();
}));

// ── tiles of a Touch Pure Flex (#78) ────────────────────────────────────
// A Touch Pure Flex's tiles are a Lox Logic `tiles` set and the device's `.tiles` claim, written into vislox's file
// of claims. Properties' *Tiles* lists the blocks linked to its API connector (the only blocks a command reaches)
// and each tile by index: what its click and its arrows up and down do (the command), the status text and the
// blocks the command addresses (`ctrls=`), marked against the compared revision, with lox logic lint's
// `tile-broadcast` and `tile-dead-command` findings on the tile. *Edit tiles* edits each function's command, text
// and targets, picked from the linked blocks; the layout (which tiles exist) stays with Loxone Config. Saving sends
// one `tiles` op with every tile vislox's set has or the draft changes: compiled, never deployed, undone through
// `undo`. The draft belongs to one device and one pending config (`blob`): when either changes, it is dropped, and a
// save sends the blob it was made on, which the server checks.
const TI = { uuid: null, blob: null, open: false, rows: [], busy: false, msg: "", err: "", shareAll: true };
const TI_FN = { click: "Click", up: "Arrow up", down: "Arrow down" };
/** What `GET /api/edits` says of this device's tiles, or null. */
function tilesEdit(uuid) { return (S.edits?.tiles || []).find((m) => m.uuid === uuid) || null; }
/** The devices whose tiles were edited here and are not deployed. */
function tilesPending() { return (S.edits?.tiles || []).filter((m) => m.pending); }
/** Whether this device's tiles can be edited here now: `{ ok, rev }`, or null. */
function tiState(o) {
  if (o?.ty !== "TreeDevice" || !o.tiles) return null;
  const st = editState({ ...o, edit: o.edit || {} });
  return st?.ok ? st : null;
}
/** The module's own claim on its tiles (a file other than vislox's), if any. */
function tiOther(o) { return claimsOf(o.uuid).find((c) => c.kind === "tiles" && !c.file.startsWith(`${S.edits?.dir}/`)) || null; }
function tiReset(extra) { Object.assign(TI, { open: false, rows: [], msg: "", err: "" }, extra || {}); }
/** Drop a draft made on another device or another pending config than the one shown now. */
function tiSync() {
  if (TI.busy) return;
  if (S.sel !== TI.uuid) { if (TI.open || TI.msg || TI.err) tiReset(); }
  else if (TI.open && editRev()?.blob !== TI.blob) tiReset({ msg: "The pending config changed while editing: the draft was dropped. Edit tiles again." });
}
/** A tile's functions as the draft holds them: command, text, targets (lowercase UUIDs). */
function tiDraftOf(t) { return { index: t.index, fns: t.functions.map((f) => ({ key: f.key, api: f.api || "", text: f.text || "", to: f.to.map((x) => x.uuid.toLowerCase()) })) }; }
/** The lint findings on tile `i` of the device (#66). */
function tiLint(uuid, i) { return lintFindings(uuid).filter((f) => f.kind.startsWith("tile-") && new RegExp(`\\btile ${i}\\b`).test(f.message)); }
function tiOpen(uuid) {
  const o = S.view?.objects[uuid], st = o && tiState(o);
  if (!st || TI.busy) return;
  Object.assign(TI, { uuid, blob: st.rev.blob, open: true, rows: o.tiles.tiles.map(tiDraftOf), msg: "", err: "", shareAll: true });
  S.closed.delete("Tiles");
  renderProps();
  $("props").querySelector('[data-ti-api="0|0"]')?.focus();
}
/** One function, shown: `SET(As;Tg;pulse)` · text · → targets, old → new under marks. */
function tiFnView(f, was) {
  const tgt = (l) => l.length ? l.map((x) => x.missing ? `<span class="none-v" title="No such object in this config">${esc(x.uuid)}</span>` : linkTarget(x)).join(" ") : "";
  const ids = (l) => l.map((x) => x.uuid.toLowerCase()).join(",");
  const part = (now, then, fmt) => (was && now !== then ? `<span class="old">${then ? fmt(then) : "—"}</span><span class="new">${now ? fmt(now) : "—"}</span>` : now ? fmt(now) : "");
  const api = part(f.api, was?.api, (v) => `<span class="mono">${esc(v)}</span>`) || '<span class="none-v">does nothing</span>';
  const text = part(f.text, was?.text, (v) => `<span class="ti-text" title="Status text">${esc(v)}</span>`);
  const to = was && ids(f.to) !== ids(was.to) ? `<span class="old">${tgt(was.to) || "—"}</span><span class="new">${tgt(f.to) || "—"}</span>` : tgt(f.to);
  return `${api}${text ? ` <span class="none-v">·</span> ${text}` : ""}${to ? `<div class="ti-to"><span class="arr">→</span>${to}</div>` : ""}`;
}
/** The rows of the *Tiles* section. */
function tiRows(o, row, marks) {
  const ti = o.tiles;
  if (o.ty !== "TreeDevice" || !ti) return null;
  const rows = [], st = tiState(o), e = tilesEdit(o.uuid), mine = TI.uuid === o.uuid ? TI : null;
  const title = (u) => { const x = S.view?.objects[u]; return x ? displayTitle(x) : u; };
  rows.push(row("API-linked", ti.linked.length ? ti.linked.map((t) => `<div class="conn">${linkTarget(t)} <span class="mo-kind">${esc(prettyType(t.ty))}</span></div>`).join("")
    : '<span class="none-v">no block: a command reaches nothing</span>', "", "The blocks whose OutputAPI is wired to the device's API connector, directly or through an OutputRef of it: the only blocks a tile's command reaches"));
  const other = tiOther(o);
  if (other && st) {
    const at = claimAt(other);
    if (e?.set) {
      rows.push(`<div class="sch-note warn">⚠ Claimed twice: ${esc(at)} and vislox's ${esc(S.edits?.file || "")} both claim its tiles, and lox compiles neither until one goes.</div>`,
        `<div class="ed-bar"><button type="button" class="ed-btn small" data-ti="drop"${mine?.busy ? " disabled" : ""} title="Take vislox's claim out and compile: ${esc(at)} keeps the device">Remove vislox's claim</button>` +
        (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "") + "</div>");
    } else rows.push(claimNote(other));
  } else if (e?.set && st) {
    rows.push(`<div class="ed-note">Written by vislox as <span class="mono">${esc(e.set)}</span> in ${esc(S.edits?.file || "")}${e.shared?.length ? `, shared with ${esc(e.shared.map(title).join(", "))}` : ""}.</div>`);
    rows.push(...staleRows((S.edits?.tile_sets || []).find((x) => x.name === e.set), "ti", mine));
  }
  if (e?.pending && st && !other) {
    const back = e.deployed ? "Back to the tiles deployed" : "Drop vislox's claim: the tiles keep the programs last written";
    rows.push(row("Tiles", `<span class="ed-v">edited here</span><span class="ed-tag" title="Saved into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      `<button type="button" class="ed-btn small" data-ti="revert" title="${esc(back)}">Revert</button>`, "pr-chg"));
  }
  let lintN = 0;
  if (!mine?.open || !st || other) {
    const old = marks ? o.tiles_old : null;
    for (const t of ti.tiles) {
      const was = old?.tiles.find((x) => x.index === t.index);
      const lint = tiLint(o.uuid, t.index);
      lintN += lint.length;
      rows.push(`<div class="mo-sub ti-h" data-tile="${t.index}">Tile ${t.index}${old && !was ? ' <span class="mo-tag">added</span>' : ""}</div>`);
      t.functions.forEach((f, j) => {
        const w = was?.functions[j];
        const changed = !!w && (w.api !== f.api || w.text !== f.text || w.to.map((x) => x.uuid.toLowerCase()).join() !== f.to.map((x) => x.uuid.toLowerCase()).join());
        rows.push(row(TI_FN[f.key], tiFnView(f, changed ? w : null), changed ? "pr-chg" : ""));
      });
      rows.push(...lint.map((f) => `<div class="ed-note lt-note">⚠ Lint (${esc(f.kind)}${f.file ? `, ${esc(checkAt(f))}` : ""}): ${esc(f.message)}</div>`));
    }
    for (const t of old?.tiles || []) if (!ti.tiles.some((x) => x.index === t.index)) rows.push(`<div class="mo-sub">Tile ${t.index} <span class="mo-tag">removed</span></div>`);
    if (!ti.tiles.length) rows.push('<div class="empty">No tiles: lay them out in Loxone Config.</div>');
    if (st && !other && ti.tiles.length) {
      const sets = (S.edits?.tile_sets || []).filter((x) => x.name !== e?.set && x.devices.length && !x.devices.includes(o.uuid));
      rows.push(`<div class="ed-bar"><button type="button" class="ed-btn small" data-ti="open">Edit tiles</button>` +
        (sets.length ? `<label class="se-same">Use the same tiles as <select data-ti="same"${mine?.busy ? " disabled" : ""}><option value="">…</option>${sets.map((x) => `<option value="${esc(x.name)}">${esc(x.devices.map(title).join(", "))}</option>`).join("")}</select></label>` : "") +
        (mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "") + "</div>");
    }
    return { rows, count: (e?.pending ? 1 : 0) + lintN || undefined };
  }
  // The editor: per tile, per function, the command, the text and the targets (the linked blocks, and any the
  // config names that are not linked, so they can be taken out).
  const dis = mine.busy ? " disabled" : "";
  const known = new Map(ti.linked.map((t) => [t.uuid.toLowerCase(), { t, linked: true }]));
  for (const t of ti.tiles) for (const f of t.functions) for (const x of f.to) if (!known.has(x.uuid.toLowerCase())) known.set(x.uuid.toLowerCase(), { t: x, linked: false });
  mine.rows.forEach((r, i) => {
    rows.push(`<div class="mo-sub ti-h">Tile ${r.index} <button type="button" class="ed-btn small" data-ti="clear" data-i="${i}"${dis} title="Clear every function: the tile does nothing">Clear</button></div>`);
    r.fns.forEach((f, j) => {
      const k = `${i}|${j}`;
      const opts = [...known].map(([u, { t, linked }]) => `<label class="ti-pick${linked ? "" : " bad"}"${linked ? "" : ' title="Not API-linked to this device: lox refuses it. Untick it, or link the block in Loxone Config."'}><input type="checkbox" data-ti-to="${esc(k)}" value="${esc(u)}"${f.to.includes(u) ? " checked" : ""}${dis}> ${esc(t.title)}${linked ? "" : " ⚠"}</label>`).join("");
      rows.push(`<div class="ti-fn"><div class="ti-k">${esc(TI_FN[f.key])}</div>` +
        `<input class="ti-api mono" data-ti-api="${esc(k)}" value="${esc(f.api)}" placeholder="command, e.g. SET(As;Tg;pulse)" aria-label="${esc(`Tile ${r.index} ${TI_FN[f.key]}: command`)}" autocomplete="off"${dis}>` +
        `<input class="ti-txt" data-ti-text="${esc(k)}" value="${esc(f.text)}" placeholder="status text, e.g. =ECHO(As)" aria-label="${esc(`Tile ${r.index} ${TI_FN[f.key]}: status text`)}" autocomplete="off"${dis}>` +
        (opts ? `<div class="ti-picks" role="group" aria-label="${esc(`Tile ${r.index} ${TI_FN[f.key]}: targets`)}">${opts}</div>` : "") + "</div>");
    });
  });
  rows.push('<div class="ed-note">No target: the command reaches every linked block of its type. A function without a command does nothing, and its text and targets go.</div>');
  if (e?.shared?.length) rows.push(`<label class="se-share"><input type="checkbox" data-ti="shareall"${TI.shareAll ? " checked" : ""}${dis}> Change ${esc(e.shared.map(title).join(", "))} too: ${e.shared.length === 1 ? "it shares" : "they share"} these tiles (${esc(e.set)}). Off: this device gets a set of its own.</label>`);
  rows.push(`<div class="ed-bar"><button type="button" class="primary small" data-ti="save"${dis}>${mine.busy ? "Compiling…" : "Save"}</button>` +
    `<button type="button" class="ed-btn small" data-ti="cancel"${dis}>Cancel</button>` +
    `<span class="ed-msg${mine.err ? " err" : ""}" role="status" aria-live="polite">${esc(mine.err || mine.msg || "Saving writes a tiles set into the module and compiles it; nothing is deployed. The layout stays with Loxone Config.")}</span></div>`);
  return { rows, count: e?.pending ? 1 : 0 };
}
/** On a block linked to a Touch Pure Flex's API connector: the device, and the tiles whose commands name it. */
function tiLinkedRows(o, row) {
  if (!o.api_linked?.length) return null;
  const u = o.uuid.toLowerCase();
  return o.api_linked.map((d) => {
    const used = (S.view?.objects[d.uuid]?.tiles?.tiles || []).flatMap((t) => t.functions.filter((f) => f.to.some((x) => x.uuid.toLowerCase() === u)).map((f) => `tile ${t.index} ${TI_FN[f.key].toLowerCase()}`));
    return row("API-linked to", `${linkTarget(d)}${used.length ? `<div class="none-v">Addressed by ${esc(used.join(", "))}</div>` : ""}`, "",
      "Its OutputAPI is wired to the device's API connector: the device's tiles can send it commands");
  });
}
/** The draft's tiles for the op: every tile vislox's set lists, and every tile the draft changes. */
function tiValue(o) {
  const e = tilesEdit(o.uuid), inSet = new Set((e?.tiles || []).map((t) => t.index));
  const fnOf = (f) => f.api.trim() ? { api: f.api, ...(f.text ? { text: f.text } : {}), to: f.to } : null;
  const key = (fns) => JSON.stringify(fns.map(fnOf));
  const tiles = [];
  for (const r of TI.rows) {
    const cfg = o.tiles.tiles.find((t) => t.index === r.index);
    if (!inSet.has(r.index) && cfg && key(r.fns) === key(tiDraftOf(cfg).fns)) continue;
    const t = { index: r.index };
    r.fns.forEach((f) => { const v = fnOf(f); if (v) t[f.key] = v; });
    tiles.push(t);
  }
  return tiles;
}
async function tiSend(o, base, op, label) {
  TI.uuid = o.uuid; TI.busy = true; TI.err = ""; TI.msg = ""; renderProps();
  try {
    const res = await editAct(base, [op], label);
    TI.busy = false;
    tiReset({ msg: res.unchanged ? "Nothing changed." : "Saved and compiled: pending, not deployed. Review it, then Apply or Discard." });
    await editShowNewest(res.blob);
  } catch (err) {
    TI.busy = false; TI.err = err.message;
  }
  renderProps();
}
async function tiSave() {
  const o = S.view?.objects[TI.uuid], st = o && tiState(o);
  if (!st || TI.busy || !TI.open) return;
  // Never save a draft over a newer config: the draft goes against the config it was made on.
  if (st.rev.blob !== TI.blob) { tiReset({ err: "The pending config changed while editing: the draft was dropped. Edit tiles again." }); renderProps(); return; }
  const tiles = tiValue(o);
  if (!tiles.length) { tiReset({ msg: "Nothing changed." }); renderProps(); return; }
  const e = tilesEdit(o.uuid);
  const value = { tiles, ...(e?.set && e.shared?.length && TI.shareAll ? { set: e.set } : {}) };
  await tiSend(o, TI.blob, { op: "tiles", uuid: o.uuid, value }, `Tiles of ${displayTitle(o)}`);
}
$("props").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-ti]");
  if (!b || b.tagName === "SELECT" || b.type === "checkbox") return;
  e.stopPropagation();
  const o = S.view?.objects[S.sel], what = b.dataset.ti;
  if (!o) return;
  if (what === "open") return tiOpen(S.sel);
  if (what === "cancel") { if (!TI.busy) { tiReset(); renderProps(); $("props").querySelector('[data-ti="open"]')?.focus(); } return; }
  if (what === "save") return tiSave();
  const st = tiState(o);
  if (what === "revert") {
    const ed = tilesEdit(o.uuid);
    if (!st || !ed) return;
    const r = setRevertValue("tiles", ed, S.edits?.tile_sets || [], o.uuid);
    if (r.why) { TI.uuid = o.uuid; TI.err = r.why; TI.msg = ""; renderProps(); return; }
    return tiSend(o, st.rev.blob, { op: "tiles", uuid: o.uuid, value: r.value }, `Revert the tiles of ${displayTitle(o)}`);
  }
  if (what === "drop") { if (st) tiSend(o, st.rev.blob, { op: "tiles", uuid: o.uuid, value: null }, `Remove vislox's claim on the tiles of ${displayTitle(o)}`); return; }
  if (what === "dropstale") { if (st) tiSend(o, st.rev.blob, { op: "tiles", uuid: b.dataset.u, value: null }, "Drop a stale claim on tiles"); return; }
  if (TI.busy || !TI.open) return;
  if (what === "clear") {
    const r = TI.rows[+b.dataset.i];
    if (r) for (const f of r.fns) Object.assign(f, { api: "", text: "", to: [] });
    TI.err = "";
    renderProps();
    $("props").querySelector(`[data-ti-api="${+b.dataset.i}|0"]`)?.focus();
  }
}), true);
$("props").addEventListener("change", whenReady((e) => {
  const t = e.target;
  if (t.dataset?.ti === "same") {
    const o = S.view?.objects[S.sel], st = o && tiState(o), x = (S.edits?.tile_sets || []).find((s) => s.name === t.value);
    if (st && x && !TI.busy) tiSend(o, st.rev.blob, { op: "tiles", uuid: o.uuid, value: { set: x.name } }, `Same tiles as ${x.devices.map((u) => { const y = S.view?.objects[u]; return y ? displayTitle(y) : u; }).join(", ")} for ${displayTitle(o)}`);
    return;
  }
  if (TI.busy || !TI.open) return;
  if (t.dataset?.ti === "shareall") { TI.shareAll = t.checked; return; }
  if (t.dataset?.tiTo != null) {
    const [i, j] = t.dataset.tiTo.split("|").map(Number), f = TI.rows[i]?.fns[j];
    if (!f) return;
    f.to = t.checked ? [...f.to.filter((u) => u !== t.value), t.value] : f.to.filter((u) => u !== t.value);
    TI.err = "";
  }
}));
$("props").addEventListener("input", whenReady((e) => {
  if (!TI.open || TI.busy) return;
  const f = e.target.closest?.("[data-ti-api], [data-ti-text]");
  if (!f) return;
  const [i, j] = (f.dataset.tiApi ?? f.dataset.tiText).split("|").map(Number), fn = TI.rows[i]?.fns[j];
  if (!fn) return;
  if (f.dataset.tiApi != null) fn.api = f.value; else fn.text = f.value;
}));
$("props").addEventListener("keydown", whenReady((e) => {
  if (!e.target.closest?.("[data-ti-api], [data-ti-text]")) return;
  if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); tiSave(); }
  else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); if (!TI.busy) { tiReset(); renderProps(); } }
}), true);

// ── adopting blocks (#72) ──────────────────────────────────────────────
// `lox logic adopt` puts a block Loxone Config drew under the module: its declaration goes into vislox's
// file of the page, it keeps its UUIDs, and the config stays exactly as it is (the server checks). It is
// an edit like any other: compiled, never deployed, undone through its reply's `undo`.
const AD = { uuid: null, open: false, slug: "", busy: false, msg: "", err: "" };
/** Whether adopting can be asked now: `{ ok, rev }`, or `{ why }`. */
/** The Changes list's Revert of each adopted block follows whether an edit can be sent now. It is drawn with the
 *  list, and an edit running then (or finished since) is no reason for it to stay off: this runs wherever that
 *  state changes (renderAddHud: an edit starts or ends, History's actions). */
function adoptRevertSync() {
  const st = adoptState(), box = $("changes");
  if (!box) return;
  for (const btn of box.querySelectorAll("[data-adopt-revert]")) {
    const b = editsBlock(btn.dataset.adoptRevert);
    btn.disabled = !!b?.hand_back || !st?.ok;
    btn.title = b?.hand_back || (st?.ok ? "Hand back to Loxone Config" : st?.why || "Hand back to Loxone Config");
  }
}
function adoptState() {
  const a = S.hist?.actions;
  if (!a?.shown || !a.edit?.available) return null;
  const r = editRev();
  if (!r || S.rev !== r.id) return null;
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy || AD.busy) return { why: "An edit is compiling." };
  return { ok: true, rev: r };
}
/** The Edit rows about Lox Logic: adopt an unmanaged block, or why not; hand an adopted one back. */
function adoptRows(o, row) {
  const st = adoptState(), nb = editsBlock(o.uuid);
  if (!st) return [];
  const mine = AD.uuid === o.uuid ? AD : null;
  const msg = mine?.msg || mine?.err ? `<span class="ed-msg${mine.err ? " err" : ""}" role="status">${esc(mine.err || mine.msg)}</span>` : "";
  if (nb?.adopted) {
    return [row("Lox Logic", `<span class="ed-v">adopted here</span><span class="ed-tag" title="Declared in ${esc(nb.file)}: the module manages it now, with the UUIDs Loxone Config gave it">managed</span>` +
      (st.ok ? `<button type="button" class="ed-btn small" data-ed="unadopt"${nb.hand_back ? " disabled" : ""} title="${esc(nb.hand_back || "Take its declaration out of vislox's file: Loxone Config owns it again, as the base has it")}">Hand back</button>` : "")),
      ...(st.ok && nb.hand_back ? [`<div class="ed-note">Hand back: ${esc(nb.hand_back)}</div>`] : []),
      ...(msg ? [`<div class="ed-bar">${msg}</div>`] : [])];
  }
  const ad = S.edits?.adopt;
  if (!ad || !(o.uuid in ad)) return [];
  if (ad[o.uuid]) return [`<div class="ed-note ad-note">Manage with Lox Logic: ${esc(ad[o.uuid])}</div>`];
  if (!mine?.open) {
    return [`<div class="ed-bar"><button type="button" class="ed-btn small" data-ed="adopt-open"${st.ok ? "" : ` disabled title="${esc(st.why)}"`} title="Put this block under the project's module (lox logic adopt). The config stays as it is.">Adopt into vislox</button>${msg}</div>`];
  }
  return [`<div class="pr ed-row"><label class="k" for="ad-slug">Slug</label><div class="v"><input id="ad-slug" value="${esc(AD.slug)}" placeholder="vx_… (from its name)" autocomplete="off" spellcheck="false"${AD.busy ? " disabled" : ""}></div></div>`,
    `<div class="ed-bar"><button type="button" class="primary small" data-ed="adopt"${AD.busy || !st.ok ? " disabled" : ""}>${AD.busy ? "Adopting…" : "Adopt"}</button>` +
    `<button type="button" class="ed-btn small" data-ed="adopt-cancel"${AD.busy ? " disabled" : ""}>Cancel</button>` +
    `<span class="ed-msg${AD.err ? " err" : ""}" role="status" aria-live="polite">${esc(AD.err || "lox declares it in vislox's files; it keeps its UUIDs, and nothing changes on the Miniserver.")}</span></div>`];
}
function adoptOpen(uuid) {
  Object.assign(AD, { uuid, open: true, slug: "", busy: false, msg: "", err: "" });
  S.closed.delete("Edit");
  renderProps();
  $("ad-slug")?.focus();
}
function adoptClose() {
  if (AD.busy) return;
  Object.assign(AD, { open: false, msg: "", err: "" });
  renderProps();
  $("props").querySelector('[data-ed="adopt-open"]')?.focus();
}
/** Send adopt ops (or unadopt); on success show the newest config. Returns the reply, or null. */
async function adoptSend(uuid, ops) {
  const st = adoptState();
  if (!st?.ok) return null;
  Object.assign(AD, { uuid, busy: true, err: "", msg: "" });
  renderProps(); renderAddHud();
  let res = null;
  try {
    res = await editAct(st.rev.blob, ops);
  } catch (err) {
    AD.err = err.message;
  }
  AD.busy = false;
  if (res) {
    AD.open = false;
    try { await editShowNewest(res.blob); } catch (err) { AD.err = `Done, but the page could not show it: ${err.message}`; }
  }
  // The config is the same revision, so no new view redraws the page: its badges, and the wire targets
  // a draw while the edit ran left out.
  if (S.view) renderPage();
  renderProps(); renderAddHud();
  return res;
}
async function adoptSave(uuid) {
  if (AD.busy || !AD.open || AD.uuid !== uuid) return;
  const slug = AD.slug.trim();
  if (slug && !/^vx_[a-z0-9_]+$/.test(slug)) { AD.err = "A slug is vx_ and then lowercase letters, digits and _."; renderProps(); $("ad-slug")?.focus(); return; }
  const res = await adoptSend(uuid, [{ op: "adopt", blocks: [slug ? { uuid, slug } : { uuid }] }]);
  if (res) AD.msg = `Adopted as ${res.adopted?.[0]?.slug || "a managed block"}: the config is unchanged. ${UNDO_KEYS[0]} hands it back.`;
  renderProps();
}
async function adoptHandBack(uuid) {
  if (editsBlock(uuid)?.hand_back) return;
  const res = await adoptSend(uuid, [{ op: "unadopt", uuids: [uuid] }]);
  if (res) { AD.msg = "Handed back to Loxone Config: the config is unchanged."; renderProps(); }
}
/** The blocks of page `pg` Properties would offer to adopt. */
function adoptOnPage(pg) {
  const ad = S.edits?.adopt || {};
  return Object.values(S.view?.objects || {}).filter((o) => o.page === pg.uuid && ad[o.uuid] === null);
}
/** "Adopt all": every block of the page lox can rebuild exactly; the reply names the rest. */
async function adoptPage() {
  // Not now (an older revision, an edit or undo compiling): say why, rather than send nothing silently.
  const now = adoptState(), st = addState();
  const why = !now ? "blocks are adopted on the newest config: show it first." : !now.ok ? now.why : !st?.ok ? st?.why : null;
  if (why) { deployMsg(`Cannot adopt now: ${why}`, true); return; }
  if (!adoptOnPage(st.page).length) return;
  const res = await adoptSend(null, [{ op: "adopt_page", page: st.page.uuid }]);
  if (!res) { deployMsg(AD.err || "Nothing was adopted.", true); return; }
  const n = res.adopted?.length || 0, sk = res.skipped || [];
  deployMsg(`Adopted ${plural(n, "block")} on ${st.page.title || "this page"}: the config is unchanged.` +
    (sk.length ? ` ${plural(sk.length, "block")} stay${sk.length === 1 ? "s" : ""} Loxone Config's: ${sk.map((x) => `${x.title} (${x.why})`).join("; ")}` : ""), false, !!sk.length);
}
$("btn-adopt-page").onclick = whenReady(adoptPage);
// ── deleting and renaming managed blocks (#80) ────────────────────────
// A block of vislox's files (added or adopted here) is renamed by its slug: lox keeps its UUIDs (`moved`), and
// the config does not change. A deployed or adopted one is deleted (`removed`): lox takes it out of the config
// with a tombstone, Apply deletes it on the Miniserver (asking for that list, #70), and Undo brings it back with
// its UUIDs until then. The rename draft belongs to one block and one pending config: it closes when either changes.
const RN = { uuid: null, blob: null, open: false, slug: "", err: "" };
/** The Edit rows of a block of vislox's files: its slug with Rename, and Delete when it is deployed or adopted. */
function lifecycleRows(o, row, st) {
  const nb = editsBlock(o.uuid);
  if (!nb || !st) return [];
  const blob = editRev()?.blob ?? null;
  if (RN.open && !(S.edit.busy && S.edit.uuid === o.uuid) && (RN.uuid !== o.uuid || RN.blob !== blob)) Object.assign(RN, { open: false, err: "" });
  const deletes = nb.deployed || nb.adopted;
  const busy = S.edit.busy && S.edit.uuid === o.uuid;
  if (RN.open && RN.uuid === o.uuid) {
    return [`<div class="pr ed-row"><label class="k" for="rn-slug">Slug</label><div class="v"><input id="rn-slug" value="${esc(RN.slug)}" autocomplete="off" spellcheck="false"${busy ? " disabled" : ""}></div></div>`,
      `<div class="ed-bar"><button type="button" class="primary small" data-ed="rename"${busy || !st.ok ? " disabled" : ""}>${busy ? "Renaming…" : "Rename"}</button>` +
      `<button type="button" class="ed-btn small" data-ed="rename-cancel"${busy ? " disabled" : ""}>Cancel</button>` +
      `<span class="ed-msg${RN.err ? " err" : ""}" role="status" aria-live="polite">${esc(RN.err || "lox keeps its UUIDs (moved); the config does not change.")}</span></div>`];
  }
  const del = deletes && st.ok
    ? `<button type="button" class="ed-btn small" data-ed="delete"${nb.delete ? " disabled" : ""} title="${esc(nb.delete || "Delete it from the config (removed): Apply deletes it on the Miniserver, Undo brings it back with its UUIDs")}">Delete</button>` : "";
  return [row("Slug", `<span class="ed-v mono">${esc(nb.slug)}</span>` +
      // The server says which can be renamed: not a Tree device created here (#85).
      (st.ok && nb.renames !== false ? `<button type="button" class="ed-btn small" data-ed="rename-open" title="Rename its slug in vislox's files; lox keeps its UUIDs">Rename</button>` : "") + del),
    ...(deletes && st.ok && nb.delete ? [`<div class="ed-note">Delete: ${esc(nb.delete)}</div>`] : [])];
}
function renameOpen(uuid) {
  const nb = editsBlock(uuid), st = S.view?.objects[uuid] && editState(S.view.objects[uuid]);
  if (!nb || !st?.ok) return;
  Object.assign(RN, { uuid, blob: st.rev.blob, open: true, slug: nb.slug, err: "" });
  S.closed.delete("Edit");
  renderProps();
  const f = $("rn-slug");
  if (f) { f.focus(); f.select(); }
}
function renameClose() {
  if (S.edit.busy) return;
  Object.assign(RN, { open: false, err: "" });
  renderProps();
  $("props").querySelector('[data-ed="rename-open"]')?.focus();
}
async function renameSave(uuid) {
  const o = S.view?.objects[uuid], st = o && editState(o), nb = editsBlock(uuid);
  if (!RN.open || RN.uuid !== uuid || !st?.ok || !nb || S.edit.busy) return;
  const slug = ($("rn-slug")?.value ?? RN.slug).trim();
  RN.slug = slug;
  if (slug === nb.slug) { renameClose(); return; }
  if (!/^vx_[a-z0-9_]+$/.test(slug)) { RN.err = "A slug is vx_ and then lowercase letters, digits and _."; renderProps(); $("rn-slug")?.focus(); return; }
  RN.err = "";
  const r = await editSend(uuid, st.rev, [{ op: "rename", uuid, slug }], `Rename ${displayTitle(o)} to ${slug}`);
  if (r === "failed") { RN.err = S.edit.err; S.edit.err = ""; renderProps(); $("rn-slug")?.focus(); return; }
  Object.assign(RN, { open: false, err: "" });
  S.edit.msg = `Renamed to ${slug}: lox keeps its UUIDs, and the config does not change.`;
  renderProps();
}
/** Delete a deployed or adopted block of vislox's files. */
async function editDelete(uuid) {
  const o = S.view?.objects[uuid], st = o && editState(o), nb = editsBlock(uuid);
  if (!st?.ok || !nb || nb.delete || !(nb.deployed || nb.adopted)) return;
  if (!window.confirm(`Delete ${displayTitle(o)}? lox takes it out of the config; Apply then deletes it on the Miniserver, after you confirm the list. Undo brings it back with its UUIDs until then.`)) return;
  await editSend(uuid, st.rev, [{ op: "remove_block", uuid }], `Delete ${displayTitle(o)}`);
}
/** Take out a block added here that is not deployed yet. */
async function editRemove(uuid) {
  const o = S.view?.objects[uuid], st = o && editState(o);
  if (!st?.ok || !editsAdded(uuid).length) return;
  if (!window.confirm(`Remove ${displayTitle(o)}? It was added here and is not deployed.`)) return;
  await editSend(uuid, st.rev, [{ op: "remove_block", uuid }], `Remove ${displayTitle(o)}`);
}
// ── moving blocks (#60) ────────────────────────────────────────────────
// A block the project's module manages is moved by dragging it (or Alt + an arrow key), snapped to
// Loxone Config's grid; its wires follow. lox keeps such a block's position in the lockfile, so the
// server writes it there and compiles (docs/editing.md): the move shows as the pending revision, marked
// "moved", with Revert. lox writes no position for a block Loxone Config drew: those stay put.
// "Arrange" lays out the page's managed blocks afresh (lox logic compile --relayout).
const GRID = 96; // Loxone Config snaps positions to multiples of 96 on save
const MV_KEYS = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
let MV = null; // the move being dragged or nudged: { uuid, orig, wires, labels, dx, dy, … }
S.mvBusy = false;
/** The blocks moved here and not deployed yet, or the one with UUID `uuid`; not those added here, which are new as a whole. */
function editsMoved(uuid) { return (S.edits?.moves || []).filter((m) => (!uuid || m.uuid === uuid) && !editsAdded(m.uuid).length); }
/** Whether the block `uuid` can be moved now: `{ rev }`, else null. */
function moveState(uuid) {
  const a = S.hist?.actions;
  if (!a?.shown || !a.edit?.available || a.running || MD.open || S.mvBusy || UNDO.busy) return null;
  if (!S.edits?.movable?.includes(uuid)) return null;
  const r = editRev();
  return r && S.rev === r.id ? { rev: r } : null;
}
/** The blocks of page `p` that lox can move. */
function mvOnPage(p) { return p.objects.filter((o) => !o.moved_to && S.edits?.movable?.includes(o.uuid)); }
const snapG = (v) => Math.round(v / GRID) * GRID;
/** Begin moving `o`: keep where it and its wires are, to draw them shifted. */
function mvBegin(o) {
  const p = curPage();
  const cp = (v) => JSON.parse(JSON.stringify(v));
  MV = {
    uuid: o.uuid, page: S.page, dx: 0, dy: 0,
    orig: { rect: [...o.rect], ports: cp(o.ports) },
    wires: p.wires.filter((w) => w.from[0] === o.uuid || w.to[0] === o.uuid).map((w) => [w, cp(w.points)]),
    labels: p.labels.filter((l) => l.obj === o.uuid).map((l) => [l, l.x, l.y]),
  };
}
/** Draw the block shifted by (dx, dy), its wires following: a wire leaves it straight, then turns halfway. */
function mvShift(dx, dy) {
  const o = curPage()?.objects.find((x) => x.uuid === MV.uuid && !x.moved_to);
  if (!o || S.page !== MV.page) return;
  // Not past the page's top left (a block already there goes no further).
  dx = Math.max(dx, Math.min(0, -MV.orig.rect[0])); dy = Math.max(dy, Math.min(0, -MV.orig.rect[1]));
  if (dx === MV.dx && dy === MV.dy && o.rect[0] === MV.orig.rect[0] + dx && o.rect[1] === MV.orig.rect[1] + dy) return;
  MV.dx = dx; MV.dy = dy;
  o.rect = [MV.orig.rect[0] + dx, MV.orig.rect[1] + dy, MV.orig.rect[2], MV.orig.rect[3]];
  o.ports.forEach((q, i) => { q.x = MV.orig.ports[i].x + dx; q.y = MV.orig.ports[i].y + dy; });
  for (const [w, pts] of MV.wires) {
    const a = [...pts[0]], b = [...pts[pts.length - 1]];
    if (w.from[0] === MV.uuid) { a[0] += dx; a[1] += dy; }
    if (w.to[0] === MV.uuid) { b[0] += dx; b[1] += dy; }
    const mx = snapG((a[0] + b[0]) / 2);
    w.points = dx || dy ? [a, [mx, a[1]], [mx, b[1]], b] : pts;
  }
  for (const [l, x, y] of MV.labels) { l.x = x + dx; l.y = y + dy; }
  renderPage();
}
function mvStart(e, o) {
  if (e.button !== 0) return;
  const st = moveState(o.uuid);
  if (!st) return;
  e.stopPropagation();
  e.preventDefault();
  if (MV) return;
  mvBegin(o);
  Object.assign(MV, { x0: e.clientX, y0: e.clientY, k: S.vp[S.page]?.k || 1, dragging: true, moved: false });
  svg.classList.add("moving");
  hideTip();
}
window.addEventListener("mousemove", whenReady((e) => {
  if (!MV?.dragging) return;
  const mx = e.clientX - MV.x0, my = e.clientY - MV.y0;
  if (!MV.moved && Math.abs(mx) + Math.abs(my) <= 3) return;
  MV.moved = true;
  const dx = snapG(MV.orig.rect[0] + mx / MV.k) - MV.orig.rect[0], dy = snapG(MV.orig.rect[1] + my / MV.k) - MV.orig.rect[1];
  mvShift(dx, dy);
}));
window.addEventListener("mouseup", whenReady(() => {
  if (!MV?.dragging) return;
  svg.classList.remove("moving");
  const m = MV;
  m.dragging = false;
  // A drag is not a click on the block.
  if (m.moved) drag = { moved: true };
  if (!m.moved) { MV = null; return; }
  // Selected, as in Loxone Config: Properties shows the move, or why lox refused it.
  if (S.sel !== m.uuid) select(m.uuid, { noReveal: true });
  mvSend(m);
}));
/** Alt + an arrow key: move the selected block a grid step (Shift: ten), sent once the keys rest. */
function mvNudge([sx, sy], n) {
  const o = S.sel && curPage()?.objects.find((x) => x.uuid === S.sel && !x.moved_to);
  if (!o || (MV && (MV.dragging || MV.uuid !== o.uuid)) || (!MV && !moveState(o.uuid))) return false;
  if (!MV) mvBegin(o);
  clearTimeout(MV.timer);
  // On the grid, also from a block Loxone Config left off it.
  mvShift(snapG(MV.orig.rect[0] + MV.dx + sx * n * GRID) - MV.orig.rect[0], snapG(MV.orig.rect[1] + MV.dy + sy * n * GRID) - MV.orig.rect[1]);
  MV.timer = setTimeout(() => mvSend(MV), 600);
  return true;
}
/** Put the block back where the page had it. */
function mvUndo(m) {
  if (MV === m) MV = null;
  const o = S.pageOf.get(m.page)?.objects.find((x) => x.uuid === m.uuid && !x.moved_to);
  if (!o) return;
  o.rect = m.orig.rect; o.ports.forEach((q, i) => { q.x = m.orig.ports[i].x; q.y = m.orig.ports[i].y; });
  for (const [w, pts] of m.wires) w.points = pts;
  for (const [l, x, y] of m.labels) { l.x = x; l.y = y; }
  if (S.page === m.page) renderPage();
}
/** Send the move; the block stays where it was dropped while lox compiles. */
async function mvSend(m) {
  if (MV !== m) return;
  const st = moveState(m.uuid);
  if (!st || (!m.dx && !m.dy)) { mvUndo(m); return; }
  MV = null;
  await mvEdit(m.uuid, st.rev, [{ op: "move", uuid: m.uuid, x: m.orig.rect[0] + m.dx, y: m.orig.rect[1] + m.dy }], () => mvUndo(m));
}
/** Send move or relayout ops: the reply's pending revision is shown, a refusal in Properties. The drawn
 *  move is undone only when the edit failed, not when it compiled and showing it failed. */
async function mvEdit(uuid, rev, ops, undo, label) {
  S.mvBusy = true; renderAddHud();
  let out = "ok";
  try {
    if (uuid) out = await editSend(uuid, rev, ops, label);
    else {
      const res = await editAct(rev.blob, ops, label);
      deployMsg("Laid out and compiled: pending, not deployed. Review it, then Apply or Discard.");
      await editShowNewest(res.blob);
    }
  } catch (err) {
    deployMsg(err.message, true);
  } finally {
    S.mvBusy = false;
    renderAddHud();
  }
  if (out === "failed") undo?.();
}
/** Back to where it is deployed. */
async function mvRevert(uuid) {
  const m = editsMoved(uuid)[0], st = m && moveState(uuid);
  if (!st) return;
  const o = S.view?.objects[uuid];
  await mvEdit(uuid, st.rev, [{ op: "move", uuid, x: m.deployed[0], y: m.deployed[1] }], null, `Revert the move of ${o ? displayTitle(o) : "a block"}`);
}
/** "Arrange": lay out the page's managed blocks afresh. */
async function mvRelayout() {
  const st = addState();
  if (!st?.ok || S.mvBusy || !mvOnPage(st.page).length || S.edits?.arrange?.[st.page.uuid] !== null) return;
  await mvEdit(null, st.rev, [{ op: "relayout", page: st.page.uuid }]);
}
$("btn-relayout").onclick = whenReady(mvRelayout);
/** The Position rows of Properties: a move not deployed, with Revert; how to move a block. */
function moveRows(o, row) {
  const out = { rows: [], count: 0 };
  if (!S.edits?.movable?.includes(o.uuid) || !S.hist?.actions?.edit?.available) return out;
  const st = moveState(o.uuid), m = editsMoved(o.uuid)[0];
  if (m) {
    out.count = 1;
    out.rows.push(row("Position", `<span class="ed-v">moved</span><span class="ed-tag" title="Saved into the module's lockfile and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>` +
      (st ? '<button type="button" class="ed-btn small" data-ed="unmove" title="Back to where it is deployed">Revert</button>' : ""), "pr-chg"));
  }
  if (st && o.page === S.page) out.rows.push('<div class="ed-note">Drag the block on the page to move it, or press Alt and an arrow key (with Shift: ten steps). It snaps to Loxone Config\'s grid.</div>');
  return out;
}

// ── wires (#61) ───────────────────────────────────────────────────────
// On the newest revision a wire is drawn by dragging from a port to a port on another block (an output
// to an input, either way round), reconnected by dragging an end of a wire vislox drew, and taken out
// with Delete. The server writes it as `to <- from` into vislox's file of the module (docs/editing.md)
// and compiles: it shows as the pending revision, like any edit. lox removes only the wires its source
// draws, so a wire Loxone Config or the rest of the module drew says so instead of offering Delete.
let WR = null; // the wire being drawn or reconnected: { fixed: { o, q }, re, moved, hot, path }
let WI = null; // the page shown, indexed for wiring: { objs: uuid → object, wired: "from>to" keys }
S.wrBusy = false;
S.wsel = null; // the wire whose card is open: { from, to, at }
S.wmsg = null; // a message in the card: { at }
/** Why wires cannot be drawn now, or null. */
function wireWhy() {
  const st = addState();
  if (!st) return "Editing is off.";
  if (!st.ok) return st.why;
  if (S.mvBusy || S.wrBusy) return "Another edit is compiling; wire again when it is done.";
  return null;
}
/** Whether wires can be drawn now: the add state (newest revision, editing on), else null. */
function wireState() { return wireWhy() ? null : addState(); }
const sameEnd = (a, b) => a[0] === b[0] && a[1] === b[1];
const wireKey = (f, t) => `${f[0]}|${f[1]}>${t[0]}|${t[1]}`;
/** What /api/edits says of the wire `w`: drawn by vislox's file now and/or when deployed; null: neither. */
function editsWire(w) { return (S.edits?.wires || []).find((x) => sameEnd(x.from, w.from) && sameEnd(x.to, w.to)) || null; }
/** The wires vislox drew or took out and has not deployed yet. */
function editsWires() { return (S.edits?.wires || []).filter((w) => w.now !== w.deployed); }
/** Who draws the wire `w`, by the server's rule (the one `unwire` refuses by): `vislox`, `adopted`,
 *  `module` or `loxone_config`. */
function wireOwner(w) {
  if (editsWire(w)?.now) return "vislox";
  const has = (list) => (list || []).some((x) => sameEnd(x.from, w.from) && sameEnd(x.to, w.to));
  // A wire a block adopted here brought along (#72): its declaration draws it.
  if (has(S.edits?.adopted_wires)) return "adopted";
  return has(S.edits?.module_wires) ? "module" : "loxone_config";
}
/** The ports of `o` a wire can start or end at, as the page draws them. */
function wrPorts(o) {
  if (o.moved_to || o.ty === "Text") return [];
  if (o.shape !== "tag") return o.ports;
  // A reference shows its wired end, else the one its type feeds.
  const wired = L.wired?.get(o.uuid);
  const feedsRight = o.ports.some((q) => q.side === "R" && wired?.has(q.key)) || (!o.ports.some((q) => q.side === "L" && wired?.has(q.key)) && o.ty === "InputRef");
  return ["L", "R"].map((side) => o.ports.find((q) => q.side === side && wired?.has(q.key)) || ((side === "R") === feedsRight ? o.ports.find((q) => q.side === side) : null)).filter(Boolean);
}
/** The centre of a port's stub. */
const stubAt = (q) => [q.side === "L" ? q.x - 62 : q.x + 62, q.y];
/** Why the wire `a` (an output) → `b` (an input) cannot be drawn here, or null. The server checks again. */
function wrWhy(a, b) {
  const lbl = (e) => `${displayTitle(e.o)}.${e.q.name}`;
  if (a.o.uuid === b.o.uuid) return "A wire goes from one block's output to another block's input.";
  if (a.q.side !== "R" || b.q.side !== "L") return "A wire goes from an output (on a block's right side) to an input (on its left side).";
  for (const e of [a, b]) if (S.edits?.managed?.includes(e.o.uuid)) return `The project's module manages ${displayTitle(e.o)}: wire it in its .loxl file.`;
  if (b.q.inv) return `${lbl(b)} is inverted in Loxone Config, which owns that input; lox does not wire it.`;
  for (const e of [a, b]) if (e.q.kind === "unknown") return `vislox cannot tell whether ${lbl(e)} carries text, a number or on/off (lox's port table does not list it); wire it in Loxone Config.`;
  if ((a.q.kind === "text") !== (b.q.kind === "text")) return `${lbl(a)} is a ${a.q.kind} output and ${lbl(b)} a ${b.q.kind} input: Loxone Config wires text only to text.`;
  if (WI?.wired.has(wireKey([a.o.uuid, a.q.key], [b.o.uuid, b.q.key]))) return `${lbl(a)} is already wired to ${lbl(b)}.`;
  return null;
}
/** Page coordinates of a mouse event. */
function pagePt(e) {
  const r = svg.getBoundingClientRect(), v = S.vp[S.page];
  return [(e.clientX - r.left - v.x) / v.k, (e.clientY - r.top - v.y) / v.k];
}
/** Draw the port targets (and the open card's end handles) over the page. Each is the size of its port's
 *  stub, so clicks beside it reach the block or wire below; a click on one selects its block. */
function wrDraw(p) {
  L.ports = el("g", { class: "wr-layer" }, vpG);
  WI = { objs: new Map(p.objects.filter((o) => !o.moved_to).map((o) => [o.uuid, o])), wired: new Set(p.wires.filter((w) => w.status !== "removed").map((w) => wireKey(w.from, w.to))) };
  // A card of a wire this page does not draw (another page, a new revision) closes.
  if (S.wsel && !p.wires.some((x) => sameEnd(x.from, S.wsel.from) && sameEnd(x.to, S.wsel.to))) { S.wsel = null; wcard.classList.add("hidden"); }
  if (wireState()) {
    for (const o of p.objects) {
      for (const q of wrPorts(o)) {
        const [cx, cy] = stubAt(q);
        const c = el("circle", { class: "port-hit", cx, cy, r: 50, "data-port": `${o.uuid}|${q.key}`, "data-side": q.side }, L.ports);
        c.addEventListener("mousedown", (e) => wrStart(e, { o, q }, null));
        hoverTip(c, () => `<b>${esc(displayTitle(o))}</b>.${esc(q.name)} <span class="tm">${q.side === "L" ? "input" : "output"} · ${q.kind}</span><br>Drag to ${q.side === "L" ? "an output" : "an input"} to wire it.`);
      }
    }
  }
  wrHandles();
  if (WR) wrMark();
  wcardPlace();
}
/** The open card's wire, if vislox drew it: its ends can be dragged elsewhere. Only these are redrawn
 *  when the card opens or closes. */
function wrHandles() {
  if (!L.ports) return;
  L.ports.querySelectorAll(".wire-end").forEach((n) => n.remove());
  const p = curPage();
  const w = S.wsel && wireState() && p?.wires.find((x) => sameEnd(x.from, S.wsel.from) && sameEnd(x.to, S.wsel.to) && x.status !== "removed");
  if (!w || wireOwner(w) !== "vislox") return;
  for (const [end, pt] of [["from", w.points[0]], ["to", w.points[w.points.length - 1]]]) {
    const other = end === "from" ? w.to : w.from, o = WI.objs.get(other[0]), q = o?.ports.find((x) => x.key === other[1]);
    if (!q) continue;
    const h = el("circle", { class: "wire-end", cx: pt[0], cy: pt[1], r: 60, "data-end": end }, L.ports);
    h.addEventListener("mousedown", (e) => wrStart(e, { o, q }, { w, moving: end }));
    hoverTip(h, "Drag this end to another port to reconnect the wire.");
  }
}
/** Begin drawing a wire from `fixed`; `re`: reconnecting the wire `re.w`, whose `re.moving` end follows. */
function wrStart(e, fixed, re) {
  if (e.button !== 0 || !wireState()) return;
  e.stopPropagation();
  e.preventDefault();
  hideTip();
  // A pan whose mouse-up has not cleared it yet does not pan along.
  drag = null;
  WR = { fixed, re, x0: e.clientX, y0: e.clientY, moved: false, hot: null };
  svg.classList.add("wiring");
  wrMark();
}
/** The draft wire, and the ports on the other side lit up: those it may go to, and those it may not. Again
 *  after the page is drawn anew mid-drag (live values, a new History). */
function wrMark() {
  WR.path = el("path", { class: "wire-draft" }, L.ports);
  WR.hot = null;
  const want = WR.fixed.q.side === "L" ? "R" : "L";
  for (const c of L.ports.querySelectorAll(`.port-hit[data-side="${want}"]`)) {
    const t = wrTarget(c);
    c.classList.add(t && !wrCheck(t) ? "cand" : "bad");
  }
}
/** The page object and port a target circle stands for. */
function wrTarget(c) {
  const [uuid, key] = c.dataset.port.split("|");
  const o = WI?.objs.get(uuid), q = o?.ports.find((x) => x.key === key);
  return q ? { o, q } : null;
}
/** Why the drag `d` may not end at `t`, or null: as an output → input wire, either way round. */
function wrCheck(t, d = WR) {
  const f = d.fixed;
  const [a, b] = f.q.side === "R" ? [f, t] : [t, f];
  if (d.re && sameEnd(d.re.w.from, [a.o.uuid, a.q.key]) && sameEnd(d.re.w.to, [b.o.uuid, b.q.key])) return "That is where the wire is now.";
  return wrWhy(a, b);
}
window.addEventListener("mousemove", whenReady((e) => {
  if (!WR) return;
  if (!WR.moved && Math.abs(e.clientX - WR.x0) + Math.abs(e.clientY - WR.y0) <= 3) return;
  WR.moved = true;
  const [x, y] = pagePt(e), k = S.vp[S.page].k;
  // The nearest port on the other side within 24 screen pixels.
  let best = null, bd = 24 / k;
  for (const c of L.ports.querySelectorAll(".port-hit.cand, .port-hit.bad")) {
    const d = Math.hypot(+c.getAttribute("cx") - x, +c.getAttribute("cy") - y);
    if (d < bd) { bd = d; best = c; }
  }
  if (WR.hot !== best) { WR.hot?.classList.remove("hot"); best?.classList.add("hot"); WR.hot = best; }
  const a = stubAt(WR.fixed.q), b = best ? [+best.getAttribute("cx"), +best.getAttribute("cy")] : [x, y];
  const mx = (a[0] + b[0]) / 2;
  WR.path.setAttribute("d", `M${a[0]},${a[1]} H${mx} V${b[1]} H${b[0]}`);
  WR.path.classList.toggle("bad", !!best?.classList.contains("bad"));
  if (best) showTip(e, best.classList.contains("bad") ? esc(wrCheck(wrTarget(best)) || "") : "Release to wire");
  else hideTip();
}));
window.addEventListener("mouseup", whenReady((e) => {
  if (!WR) return;
  const w = WR;
  WR = null;
  svg.classList.remove("wiring");
  hideTip();
  // A press on a port without a drag is a click on its block.
  if (!w.moved) { w.path?.remove(); if (!w.re) select(w.fixed.o.uuid); return; }
  // A drag is not a click on the page; a stale one would pan it on the next move.
  const d = (drag = { moved: true, x: e.clientX, y: e.clientY });
  setTimeout(() => { if (drag === d) drag = null; }, 0);
  const t = w.hot && wrTarget(w.hot);
  const why = t ? wrCheck(t, w) : null;
  if (!t || why) {
    renderPage();
    if (why) wcardMsg(e, why, true);
    return;
  }
  const f = w.fixed;
  const [a, b] = f.q.side === "R" ? [f, t] : [t, f];
  const ops = [];
  if (w.re) ops.push({ op: "unwire", from: w.re.w.from, to: w.re.w.to });
  ops.push({ op: "wire", from: [a.o.uuid, a.q.key], to: [b.o.uuid, b.q.key] });
  wrSend(ops, w.re ? "Reconnected" : "Wired", e, w.re ? "Reconnecting: compiling…" : "Wiring: compiling…");
}));
/** Send wire ops; the reply's pending revision is shown, a refusal (or why nothing can be sent now) in
 *  the card at the mouse (`at`). `busy`: the card's text while it compiles. */
async function wrSend(ops, done, at, busy) {
  const why = wireWhy();
  if (why) { renderPage(); wcardMsg(at, why, true); return; }
  const st = addState();
  wcardMsg(at, busy || "Compiling…");
  S.wrBusy = true; renderAddHud();
  try {
    const res = await editAct(st.rev.blob, ops);
    S.wsel = null;
    wcardMsg(null);
    deployMsg(`${done} and compiled: pending, not deployed. Review it, then Apply or Discard.`);
    S.wrBusy = false;
    await editShowNewest(res.blob);
  } catch (err) {
    S.wrBusy = false;
    renderPage();
    wcardMsg(at, err.message, true);
  } finally {
    S.wrBusy = false;
    renderAddHud();
  }
}
// The wire card: what a clicked wire is, with Delete or Revert, or why it cannot be changed here.
const wcard = document.createElement("div");
wcard.id = "wcard";
wcard.className = "wcard hidden";
wcard.setAttribute("role", "dialog");
wcard.setAttribute("aria-label", "Wire");
$("canvas-wrap").appendChild(wcard);
/** Where the card sits, relative to the canvas: at a mouse event, else where it was. */
function wcardAt(e) {
  const r = $("canvas-wrap").getBoundingClientRect();
  return e?.clientX != null ? [e.clientX - r.left, e.clientY - r.top] : (S.wsel || S.wmsg)?.at || [16, 16];
}
function wcardPlace() {
  const at = (S.wsel || S.wmsg)?.at;
  if (!at || wcard.classList.contains("hidden")) return;
  const W = $("canvas-wrap").clientWidth;
  wcard.style.left = Math.max(8, Math.min(W - wcard.offsetWidth - 8, at[0] + 12)) + "px";
  wcard.style.top = Math.max(8, at[1] + 12) + "px";
}
/** A message in the card at the mouse (`e`), or none. */
function wcardMsg(e, text, error) {
  const at = wcardAt(e);
  S.wsel = null;
  if (!text) { S.wmsg = null; wcard.classList.add("hidden"); wrHandles(); return; }
  S.wmsg = { at };
  wcard.innerHTML = `<div class="wc-msg${error ? " err" : ""}" role="status">${esc(text)}</div>` + (error ? '<div class="wc-bar"><button type="button" class="ed-btn small" data-wc="close">Close</button></div>' : "");
  wcard.classList.remove("hidden");
  wrHandles();
  wcardPlace();
}
/** Open the card of the wire `w` (as the page draws it), clicked at `e`. */
function wcardOpen(e, w) {
  S.wmsg = null;
  S.wsel = { from: w.from, to: w.to, at: wcardAt(e) };
  const v = wcardView(w);
  wcard.innerHTML = v.html;
  wcardKey = v.key;
  wcardSync(v);
  wcard.classList.remove("hidden");
  wrHandles();
  wcardPlace();
}
/** The open card follows what it says: /api/edits (who drew the wire) and whether an edit can be sent now
 *  both arrive after the click. It is drawn again only when the wire's own identity changes (who drew it, what
 *  it says); a change of the edit state (which every History refresh may bring) is patched in place, so that a
 *  click on its button never lands on a node that was replaced. A wire the page no longer draws closes it. */
let wcardKey = "";
function wcardRefresh() {
  if (!S.wsel || wcard.classList.contains("hidden")) return;
  const w = S.view && curPage()?.wires.find((x) => sameEnd(x.from, S.wsel.from) && sameEnd(x.to, S.wsel.to));
  if (!w) { wcardClose(); return; }
  const v = wcardView(w);
  if (v.key !== wcardKey) { wcard.innerHTML = v.html; wcardKey = v.key; wrHandles(); wcardPlace(); }
  wcardSync(v);
}
/** The card's parts that depend on whether an edit can be sent now, set in place. */
function wcardSync(v) {
  const set = (el, text) => { if (el && el.textContent !== text) el.textContent = text; };
  set(wcard.querySelector(".wc-drag"), v.st ? "Drag an end of the wire to another port to reconnect it." : "");
  set(wcard.querySelector(".wc-why"), v.why);
  const b = wcard.querySelector("[data-wc=unwire], [data-wc=rewire]");
  if (b && b.disabled !== !v.st) b.disabled = !v.st;
}
/** `{ key, html, st, why }`: the card of wire `w`; `key` names what `html` says (not the edit state). */
function wcardView(w) {
  const fo = WI?.objs.get(w.from[0]), to = WI?.objs.get(w.to[0]);
  const q = fo?.ports.find((x) => x.key === w.from[1]);
  const x = editsWire(w), st = wireState(), who = wireOwner(w);
  const removed = w.status === "removed";
  let what, btn = "";
  if (x?.now && !x.deployed) {
    what = '<span class="ed-v">drawn here</span><span class="ed-tag" title="Written into the module and compiled, not on the Miniserver yet: Apply deploys it, Discard drops it">not deployed</span>';
    btn = '<button type="button" class="ed-btn small" data-wc="unwire" title="Take the wire out again">Revert</button>';
  } else if (x?.now) {
    what = `<span class="ed-v">drawn in vislox</span> <span class="tm">(${esc(S.edits?.file || "")})</span>`;
    btn = '<button type="button" class="ed-btn small" data-wc="unwire" title="Take the wire out (Delete)">Delete</button>';
  } else if (x?.deployed && removed) {
    what = '<span class="ed-v">taken out here</span><span class="ed-tag" title="Taken out of the module and compiled; the Miniserver still has it until Apply">not deployed</span>';
    btn = '<button type="button" class="ed-btn small" data-wc="rewire" title="Draw the wire again, as deployed">Revert</button>';
  } else if (removed) {
    what = '<span class="tm">Not in this revision.</span>';
  } else {
    what = `<span class="tm">${esc(S.edits?.wire_why?.[who] || "")}</span>`;
  }
  const why = btn && S.hist?.actions?.shown ? wireWhy() || "" : "";
  const drag = who === "vislox" && !removed;
  const html = `<div class="wc-head"><b>${esc(fo ? displayTitle(fo) : "")}</b>.${esc(portName(fo, w.from[1]))} → <b>${esc(to ? displayTitle(to) : "")}</b>.${esc(portName(to, w.to[1]))}</div>` +
    `<div class="wc-sub tm">${esc(q?.kind || (w.analog ? "analog" : "digital"))}${w.status ? " · " + esc(w.status) : ""}</div>` +
    `<div class="wc-what">${what}</div>` +
    (drag ? '<div class="wc-note tm wc-drag"></div>' : "") +
    (btn ? '<div class="wc-note tm wc-why"></div>' : "") +
    `<div class="wc-bar">${btn}<button type="button" class="ed-btn small" data-wc="close">Close</button></div>`;
  return { key: html, html, st, why };
}
function wcardClose() {
  S.wsel = null; S.wmsg = null;
  if (wcard.classList.contains("hidden")) return false;
  wcard.classList.add("hidden");
  wrHandles();
  return true;
}
/** Delete (or Revert) the wire whose card is open. */
function wrDelete() {
  const w = S.wsel, x = w && editsWire(w);
  if (!x?.now || !wireState()) return false;
  wrSend([{ op: "unwire", from: w.from, to: w.to }], "Taken out", null);
  return true;
}
wcard.addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-wc]");
  if (!b) return;
  const w = S.wsel;
  if (b.dataset.wc === "close") wcardClose();
  else if (b.dataset.wc === "unwire") wrDelete();
  else if (b.dataset.wc === "rewire" && w) wrSend([{ op: "wire", from: w.from, to: w.to }], "Wired again", null);
}));
wcard.addEventListener("mousedown", whenReady((e) => e.stopPropagation()));

// ── adding blocks (#59) ────────────────────────────────────────────────
// “Add block” on the page picks a type lox can create (GET /api/block-types), then its name and values.
// The server declares it in a file of vislox's own for the page (<module>/vislox/<page>.loxl) and runs
// lox logic compile, which mints the block and places it on the page: it shows as the pending revision,
// and its Properties offer Remove until it is deployed. Adding never deploys: only Apply does.
S.types = null;
const AB = { open: false, ty: null, items: [], idx: 0, draft: {}, bad: {}, busy: false, err: "", parent: null };
// Objects of the object tree (#84) are added from the project tree: a folder (Virtual Inputs, Virtual Outputs,
// Netzwerkperipherie) or a device takes them. GET /api/edits' `add_under` names each folder and device of the config
// shown that takes new objects, with why `add_block` there would be refused (null: it would not); the palette's
// `periphery` says which types go where.
S.periph = null;
/** Whether an object can be added under the tree node `n`: `{ ok, rev }`, or `{ why }`; null: not offered at all. */
function periphAddState(n) {
  const a = S.hist?.actions, under = S.edits?.add_under;
  if (!a?.shown || !a.edit?.available || MD.open || !under || !(n.uuid in under)) return null;
  const r = editRev();
  if (!r) return { why: "The config on disk is not in the History yet." };
  if (S.rev !== r.id) return { why: `Objects are added on the newest config (${r.kind === "pending" ? "Pending" : "deployed"}).` };
  if (n.status === "removed") return { why: "This is not in the config any more." };
  if (under[n.uuid]) return { why: under[n.uuid] };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  return { ok: true, rev: r };
}
/** The palette's types that go under an object of type `ty`. */
function periphUnder(ty) { return (S.periph || []).filter((t) => t.folder === ty || t.devices?.includes(ty)); }
async function abLoadTypes() {
  if (S.types) return true;
  $("ab-body").innerHTML = '<div class="ab-sum">Loading the block types…</div>';
  let j;
  try { j = await api("/api/block-types"); } catch (err) { if (AB.open) $("ab-body").innerHTML = `<div class="ab-sum">${esc(err.message)}</div>`; return false; }
  S.types = j.types; S.periph = j.periphery || [];
  return true;
}
/** “Add” on a folder or device of the project tree: the dialog, for the types that go there. */
async function abOpenUnder(uuid) {
  const n = nodeById(uuid), st = n && periphAddState(n);
  if (!st?.ok) return;
  Object.assign(AB, { open: true, ty: null, draft: {}, bad: {}, busy: false, err: "", parent: { uuid, ty: n.ty, title: autoTitle(n.title) ? prettyType(n.ty) : n.title } });
  $("addblk").classList.remove("hidden");
  $("ab-h").textContent = "Add an object";
  $("ab-page").textContent = `in ${AB.parent.title}`;
  if (!(await abLoadTypes()) || !AB.open) return;
  // A calendar entry is added in the Operating modes view, where its mode and date are picked (#86).
  const under = periphUnder(n.ty);
  if (under.length && under.every((t) => t.view === "modes")) { abClose(); modesOpen(); mfOpen("entry"); return; }
  abPick("");
}
/** Whether a block can be added to the page shown: `{ ok, rev, page }`, or `{ why }`; null: not offered at all. */
function addState() {
  const a = S.hist?.actions, pg = S.view && curPage();
  if (!a?.shown || !pg || MD.open) return null;
  if (!a.edit?.available) return { why: `Editing is off: ${a.edit?.reason || "unknown"}.` };
  const r = editRev();
  if (!r) return { why: "The config on disk is not in the History yet." };
  if (S.rev !== r.id) return { why: `Blocks are added on the newest config (${r.kind === "pending" ? "Pending" : "deployed"}).` };
  if (pg.status === "removed") return { why: "This page is not in the config any more." };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  return { ok: true, rev: r, page: pg };
}
function renderAddHud() {
  renderNewPageBtn();
  adoptRevertSync();
  wcardRefresh();
  const st = addState(), b = $("btn-add-block"), r = $("btn-relayout");
  $("hud-add").classList.toggle("hidden", !st);
  if (!st) { undoListToggle(false); return; }
  // A page created here and not deployed (#88): Remove page, while the server says it can go.
  const cp = st.ok ? pageCreated(st.page.uuid) : null, rp = $("btn-remove-page");
  rp.classList.toggle("hidden", !cp || cp.deployed);
  rp.disabled = !cp || cp.remove != null || NP.busy;
  rp.title = !cp ? "" : cp.remove != null ? cp.remove : `Take out ${cp.title}, created here and not deployed: vislox deletes ${cp.file} (it is compiled, not deployed)`;
  renderUndo();
  b.disabled = !st.ok;
  b.title = st.ok ? `Add a block to ${st.page.title || "this page"} (it is compiled, not deployed)` : st.why;
  const n = st.ok ? mvOnPage(st.page).length : 0;
  // The server says per page whether lox can lay it out (null) or why not.
  const why = st.ok && n ? S.edits?.arrange?.[st.page.uuid] : undefined;
  r.disabled = !st.ok || !n || why !== null || S.mvBusy;
  const ap = $("btn-adopt-page"), na = st.ok ? adoptOnPage(st.page).length : 0;
  ap.disabled = !st.ok || !na || AD.busy;
  ap.title = !st.ok ? st.why
    : !na ? "No block of this page can be adopted: the module manages them, or Properties says why not"
    : `Put the ${plural(na, "block")} of this page the module does not manage under it (lox logic adopt); the config stays as it is`;
  r.title = !st.ok ? st.why
    : !n ? "lox lays out only the blocks the project's module manages, and this page has none"
    : why !== null ? (why || "lox cannot lay out this page")
    : `Lay out the ${plural(n, "block")} the module manages on this page afresh, as lox places new blocks (it is compiled, not deployed)`;
}
// ── new pages (#88) ───────────────────────────────────────────────────
// New page writes a file of vislox's headed `page "Title" (room: "…", category: "…")`; lox logic compile creates
// the page from it, and blocks added there go into the same file. The rooms and categories offered are the base's
// (lox resolves them there), served with GET /api/edits' `pages`, as is whether each created page can go again.
const NP = { open: false, draft: {}, bad: {}, busy: false, err: "" };
/** The page vislox's files create with UUID `uuid` (#88), as GET /api/edits lists it, or null. */
function pageCreated(uuid) { return uuid ? (S.edits?.pages?.created || []).find((p) => p.uuid && ciEq(p.uuid, uuid)) || null : null; }
/** The pages created here that are not deployed. */
function pagesCreated() { return (S.edits?.pages?.created || []).filter((p) => p.uuid && !p.deployed); }
/** Whether New page can be used now, or why not: the same rules as Add block, on no particular page. */
function newPageState() {
  const a = S.hist?.actions;
  if (!a?.shown || !S.view) return null;
  if (!a.edit?.available) return { why: `Editing is off: ${a.edit?.reason || "unknown"}.` };
  const r = editRev();
  if (!r) return { why: "The config on disk is not in the History yet." };
  if (S.rev !== r.id) return { why: `Pages are created on the newest config (${r.kind === "pending" ? "Pending" : "deployed"}).` };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  if (!S.edits?.pages) return { why: "vislox cannot read the project's base config." };
  if (!S.edits.pages.rooms.length) return { why: "The base config has no room a page can be in: make one in Loxone Config first." };
  return { ok: true, rev: r };
}
function renderNewPageBtn() {
  const st = newPageState(), b = $("btn-new-page");
  b.classList.toggle("hidden", !st);
  if (!st) return;
  b.disabled = !st.ok || NP.busy;
  b.title = st.ok ? "Create a logic page in a room (it is compiled, not deployed)" : st.why;
}
function npOpen() {
  if (!newPageState()?.ok) return;
  const rooms = S.edits.pages.rooms;
  Object.assign(NP, { open: true, draft: { title: "", room: rooms.length === 1 ? rooms[0] : "", category: "" }, bad: {}, busy: false, err: "" });
  $("newpage").classList.remove("hidden");
  npForm();
  $("np-title").focus();
}
function npClose() {
  if (NP.busy) return;
  NP.open = false;
  $("newpage").classList.add("hidden");
  $("btn-new-page").focus();
}
function npForm() {
  const p = S.edits?.pages || { rooms: [], categories: [] }, d = NP.draft, dis = NP.busy ? " disabled" : "";
  const bad = (f) => NP.bad[f] ? ` aria-invalid="true" aria-describedby="np-${f}-e"` : "";
  const err = (f) => NP.bad[f] ? `<div class="ed-bad" id="np-${f}-e">${esc(NP.bad[f])}</div>` : "";
  const opts = (list, v) => list.map((t) => `<option${t === v ? " selected" : ""}>${esc(t)}</option>`).join("");
  $("np-body").innerHTML = `<div class="ab-form">
    <div class="pr ed-row"><label class="k" for="np-title">Title</label><div class="v"><input id="np-title" data-np-field="title" value="${esc(d.title)}" placeholder="Küche Logik" autocomplete="off" spellcheck="false"${bad("title")}${dis}>${err("title")}</div></div>
    <div class="pr ed-row"><label class="k" for="np-room">Room</label><div class="v"><select id="np-room" data-np-field="room"${bad("room")}${dis}><option value="">Pick a room…</option>${opts(p.rooms, d.room)}</select>${err("room")}</div></div>
    <div class="pr ed-row"><label class="k" for="np-category">Category</label><div class="v"><select id="np-category" data-np-field="category"${dis}><option value="">Not assigned</option>${opts(p.categories, d.category)}</select></div></div>
    <div class="ed-bar"><button type="button" class="primary small" data-np="add"${dis}>${NP.busy ? "Compiling…" : "Create"}</button>
      <button type="button" class="ed-btn small" data-np="cancel"${dis}>Cancel</button>
      <span class="ed-msg${NP.err ? " err" : ""}" role="status" aria-live="polite">${esc(NP.err || "lox creates the page in the room when it compiles; add blocks to it next. Creating compiles the module; nothing is deployed.")}</span></div>
  </div>`;
}
$("np-body").addEventListener("input", whenReady((e) => {
  const f = e.target.closest("[data-np-field]");
  if (!f) return;
  NP.draft[f.dataset.npField] = f.value;
  delete NP.bad[f.dataset.npField];
}));
$("np-body").addEventListener("change", whenReady((e) => {
  const f = e.target.closest("select[data-np-field]");
  if (f) { NP.draft[f.dataset.npField] = f.value; delete NP.bad[f.dataset.npField]; }
}));
$("np-body").addEventListener("keydown", whenReady((e) => {
  if (e.key === "Escape") { e.preventDefault(); npClose(); }
  else if (e.key === "Enter" && e.target.tagName === "INPUT") { e.preventDefault(); npAdd(); }
}));
$("np-body").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-np]");
  if (b?.dataset.np === "add") npAdd();
  else if (b?.dataset.np === "cancel") npClose();
}));
$("newpage").addEventListener("mousedown", whenReady((e) => { if (e.target.id === "newpage") npClose(); }));
$("btn-new-page").onclick = whenReady(npOpen);
async function npAdd() {
  const st = newPageState();
  if (!st?.ok || NP.busy) return;
  const title = NP.draft.title.trim(), room = NP.draft.room, category = NP.draft.category;
  NP.bad = {}; NP.err = "";
  if (!title) NP.bad.title = "Give the page a title.";
  else if ((S.view?.pages || []).some((p) => p.title === title)) NP.bad.title = "A page has this title already: add blocks to it, or pick another title.";
  if (!room) NP.bad.room = "lox needs a room to create a page.";
  if (Object.keys(NP.bad).length) { NP.err = "Fix the marked values."; npForm(); $("np-body").querySelector('[aria-invalid="true"]')?.focus(); return; }
  NP.busy = true; npForm(); renderNewPageBtn();
  try {
    const res = await editAct(st.rev.blob, [{ op: "new_page", title, room, ...(category ? { category } : {}) }]);
    NP.busy = false;
    npClose();
    await editShowNewest(res.blob);
    const uuid = res.added?.[0]?.uuid, u = uuid && [...(S.pageOf?.keys() || [])].find((k) => ciEq(k, uuid));
    if (u) gotoPage(u);
  } catch (err) {
    NP.busy = false; NP.err = err.message;
    if (NP.open) npForm();
    renderNewPageBtn();
  }
}
/** Remove page: a page created here, not deployed, with nothing of vislox's on it. Undo creates it again. */
$("btn-remove-page").onclick = whenReady(async () => {
  const st = addState(), cp = st?.ok && pageCreated(st.page.uuid);
  if (!cp || cp.remove != null || NP.busy) return;
  NP.busy = true; renderAddHud();
  try {
    const res = await editAct(st.rev.blob, [{ op: "remove_page", page: cp.uuid }], `Remove the page ${cp.title}`);
    NP.busy = false;
    await editShowNewest(res.blob);
    if (!S.pageOf?.has(S.page)) { const first = visiblePages()[0]; if (first) gotoPage(first.uuid); }
  } catch (err) {
    NP.busy = false;
    showMsg(err.message);
  }
  renderAddHud();
});
async function abOpen() {
  const st = addState();
  if (!st?.ok) return;
  Object.assign(AB, { open: true, ty: null, draft: {}, bad: {}, busy: false, err: "", parent: null });
  $("addblk").classList.remove("hidden");
  $("ab-h").textContent = "Add a block";
  $("ab-page").textContent = `on ${st.page.title || "this page"}`;
  // Closed while the types loaded: leave the focus where it is.
  if (!(await abLoadTypes()) || !AB.open) return;
  abPick("");
}
function abClose() {
  if (AB.busy) return;
  AB.open = false;
  $("addblk").classList.add("hidden");
  if (AB.parent) $("tree").focus();
  else $("btn-add-block").focus();
}
/** Step one: the types, searched by name, type and purpose. */
function abPick(q) {
  AB.ty = null;
  $("ab-body").innerHTML = `<input id="ab-q" type="text" placeholder="Search block types… (And, Monoflop, timer)" autocomplete="off" spellcheck="false" role="combobox" aria-expanded="true" aria-controls="ab-list" aria-autocomplete="list" aria-label="Block type">
    <ul id="ab-list" role="listbox" aria-label="Block types"></ul>
    <div class="pal-foot"><kbd>↑</kbd><kbd>↓</kbd> choose · <kbd>Enter</kbd> pick · <kbd>Esc</kbd> close · ${AB.parent ? `only the types that go in ${esc(AB.parent.title)}` : "only the types lox can create on a page"}</div>`;
  const inp = $("ab-q");
  inp.value = q;
  inp.addEventListener("input", abList);
  inp.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { abMove(e.key === "ArrowDown" ? 1 : -1); e.preventDefault(); }
    else if (e.key === "Enter") { abChoose(AB.idx); e.preventDefault(); }
    else if (e.key === "Escape") { abClose(); e.preventDefault(); }
  });
  $("ab-list").addEventListener("click", (e) => { const li = e.target.closest("[data-i]"); if (li) abChoose(+li.dataset.i); });
  abList();
  inp.focus();
}
function abList() {
  const q = $("ab-q").value.trim().toLowerCase();
  // The module's templates and lox's library's (#83): an instance of one.
  const tpls = AB.parent ? [] : tplOffered().map((x) => ({ t: { type: x.name, summary: `Template · ${x.description || x.from}`, template: x }, name: x.name }));
  const all = AB.parent ? periphUnder(AB.parent.ty).map((t) => ({ t, name: prettyType(t.type) })) : [...S.types.map((t) => ({ t, name: prettyType(t.type) })), ...tpls];
  AB.items = !q ? all.sort((a, b) => a.name.localeCompare(b.name)) : all.map((it) => {
    const m = fuzzy(q, it.name) || fuzzy(q, it.t.type);
    const inSum = (it.t.summary || "").toLowerCase().includes(q);
    return m ? { ...it, score: m.score + 100, hits: fuzzy(q, it.name)?.hits } : inSum ? { ...it, score: 1 } : null;
  }).filter(Boolean).sort((a, b) => b.score - a.score);
  AB.idx = AB.items.length ? 0 : -1;
  $("ab-list").innerHTML = AB.items.length ? AB.items.map((it, i) => `<li role="option" id="ab-o${i}" data-i="${i}" aria-selected="${i === AB.idx}"><span class="at">${mark(it.name, it.hits)}</span>` +
    (it.name !== it.t.type ? `<span class="ac">${esc(it.t.type)}</span>` : "") + `<span class="as">${esc(it.t.summary || "")}</span></li>`).join("")
    : `<li class="sec" role="presentation">${AB.parent ? `Nothing that goes in ${esc(AB.parent.title)} matches.` : "No block type matches. Virtual inputs and devices are added in their folder of the project tree."}</li>`;
  $("ab-q").setAttribute("aria-activedescendant", AB.idx >= 0 ? "ab-o0" : "");
}
function abMove(d) {
  if (!AB.items.length) return;
  AB.idx = Math.max(0, Math.min(AB.items.length - 1, AB.idx + d));
  $("ab-list").querySelectorAll("[role=option]").forEach((li) => li.setAttribute("aria-selected", +li.dataset.i === AB.idx));
  $(`ab-o${AB.idx}`)?.scrollIntoView({ block: "nearest" });
  $("ab-q").setAttribute("aria-activedescendant", `ab-o${AB.idx}`);
}
function abChoose(i) {
  const it = AB.items[i];
  if (!it) return;
  Object.assign(AB, { ty: it.t, draft: it.t.template ? { slug: tplNewSlug(it.t.template) } : {}, bad: {}, err: "" });
  abForm();
  $("ab-body").querySelector("[data-ab-field]")?.focus();
}
/** The fields of a new block of type `t`: [field, label, kind, unit, default, group, choices]. */
function abFields(t) {
  // A push notification (#77) takes the notification's name, and its input is wired once it is on the page.
  if (t.mirrors) return t.settings.map((s) => [`s:${s.name}`, s.name === "mirrors" ? "Notification" : s.name === "MinimumTime" ? "Minimum time" : s.name, s.kind, null, "", "main", s.kind === "notification" ? abNotifications() : s.choices]);
  // An object of the object tree (#84): its name and settings, the ones lox needs first; a number may carry its unit.
  if (t.folder || t.devices) {
    const req = t.settings.filter((s) => s.required), rest = t.settings.filter((s) => !s.required);
    return [["title", "Name", "text", null, "", "main"],
      ...[...req, ...rest].map((s) => [`s:${s.name}`, s.name, s.kind === "number" ? "quantity" : s.kind, s.unit, s.required ? "required" : "", s.required ? "main" : "inputs", s.choices, s.required])];
  }
  const f = [["title", "Name", "text", null, "", "main"]];
  for (const s of t.settings) f.push([`s:${s.name}`, s.name, s.kind, null, "", "main", s.choices]);
  for (const p of t.ports.filter((q) => q.dir === "param")) f.push([`p:${p.key}`, p.label ? `${p.name} (${p.label})` : p.name, "quantity", p.unit, p.default, "main"]);
  for (const p of t.ports.filter((q) => q.dir === "input")) f.push([`p:${p.key}`, p.label ? `${p.name} (${p.label})` : p.name, "quantity", p.unit, p.default, "inputs"]);
  return f;
}
/** The push notifications of the config shown (Loxone Config's Messages), for a new notification block: [uuid, title]. */
function abNotifications() {
  return Object.values(S.view?.objects || {}).filter((x) => x.ty === "Notification" && x.status !== "removed")
    .map((x) => [x.uuid, displayTitle(x)]).sort((a, b) => a[1].localeCompare(b[1]));
}
/** A duration as Lox Logic writes one: 12h, 30min, 5s, 500ms, or seconds. */
const AB_DURATION = /^\d+(\.\d+)?(ms|s|min|h)?$/;
/** Step two: its name, settings and values. */
function abForm() {
  const t = AB.ty;
  if (t.template) { tplAbForm(t.template); return; }
  const field = ([field, label, kind, unit, def, , choices]) => {
    const v = AB.draft[field] ?? "", bad = AB.bad[field];
    const id = `ab-${field.replace(/[^\w-]/g, "_")}`;
    const attrs = `id="${id}" data-ab-field="${esc(field)}"${bad ? ` aria-invalid="true" aria-describedby="${id}-e"` : ""}${AB.busy ? " disabled" : ""}`;
    const input = kind === "notification"
      ? `<select ${attrs}><option value="">${choices.length ? "Pick a push notification…" : "The config has no push notification: make one in Loxone Config (Messages)"}</option>${choices.map(([u, n]) => `<option value="${esc(u)}"${u === v ? " selected" : ""}>${esc(n)}</option>`).join("")}</select>`
      : kind === "choice" || kind === "on_off"
      ? `<select ${attrs}><option value="">as Loxone Config places it</option>${(kind === "on_off" ? ["on", "off"] : choices).map((c) => `<option${c === v ? " selected" : ""}>${esc(c)}</option>`).join("")}</select>`
      : kind === "program"
      ? `<textarea ${attrs} class="prog" rows="10" autocomplete="off" spellcheck="false" placeholder="Empty: Loxone Config's program">\n${esc(v)}</textarea>`
      : `<input ${attrs} value="${esc(v)}"${kind === "number" || kind === "quantity" ? ` inputmode="decimal" class="num" placeholder="${esc(def)}"` : kind === "time" ? ' class="num" placeholder="HH:MM"' : kind === "duration" ? ' class="num" placeholder="5s"' : field === "title" ? ` placeholder="${esc(prettyType(t.type))}"` : def ? ` placeholder="${esc(def)}"` : ""} autocomplete="off" spellcheck="false">`;
    return `<div class="pr ed-row"><label class="k" for="${id}">${esc(label)}</label><div class="v">${input}${unit ? `<span class="ed-unit">${esc(unit)}</span>` : ""}${bad ? `<div class="ed-bad" id="${id}-e">${esc(bad)}</div>` : ""}</div></div>`;
  };
  const fs = abFields(t), main = fs.filter((f) => f[5] === "main"), inputs = fs.filter((f) => f[5] === "inputs");
  const inputsOpen = inputs.some((f) => AB.draft[f[0]] || AB.bad[f[0]]);
  $("ab-body").innerHTML = `<div class="ab-form">
    <div class="ab-sum"><b>${esc(prettyType(t.type))}</b> <span class="mono">${esc(t.type)}</span>${t.summary ? ` · ${esc(t.summary)}` : ""}${t.docs ? `<a href="${esc(t.docs)}" target="_blank" rel="noopener">Loxone documentation</a>` : ""}</div>
    ${main.map(field).join("")}
    ${inputs.length ? `<details${inputsOpen ? " open" : ""}><summary>${t.folder || t.devices ? `More settings (${inputs.length}): Loxone Config's defaults until set` : `Constants of the inputs (${inputs.length}): used until something is wired`}</summary>${inputs.map(field).join("")}</details>` : ""}
    <div class="ed-bar"><button type="button" class="primary small" data-ab="add"${AB.busy ? " disabled" : ""}>${AB.busy ? "Compiling…" : "Add"}</button>
      <button type="button" class="ed-btn small" data-ab="back"${AB.busy ? " disabled" : ""}>Other type</button>
      <span class="ed-msg${AB.err ? " err" : ""}" role="status" aria-live="polite">${esc(AB.err || (t.mirrors ? "No text means no message. Wire its input from a block once it is on the page. Adding compiles the module; nothing is deployed." : t.hardware ? "lox creates it unpaired, with the kind's inputs and outputs: pair it with a Tree search in Loxone Config. Adding compiles the module; nothing is deployed." : t.folder || t.devices ? "lox gives it its designation (VI, VHI, …). Empty settings get Loxone Config's defaults. Adding compiles the module; nothing is deployed, and nothing is sent to the device." : "Empty values get what Loxone Config places the block with. Adding compiles the module; nothing is deployed."))}</span></div>
  </div>`;
}
$("ab-body").addEventListener("input", whenReady((e) => {
  const f = e.target.closest("[data-ab-field]");
  if (!f) return;
  AB.draft[f.dataset.abField] = f.value;
  delete AB.bad[f.dataset.abField];
}));
$("ab-body").addEventListener("keydown", whenReady((e) => {
  const f = e.target.closest("[data-ab-field]");
  if (f && e.key === "Enter" && f.tagName === "INPUT") { e.preventDefault(); abAdd(); }
  else if (f && e.key === "Escape") { e.preventDefault(); abClose(); }
}));
$("ab-body").addEventListener("click", whenReady((e) => {
  const b = e.target.closest("[data-ab]");
  if (b?.dataset.ab === "add") abAdd();
  else if (b?.dataset.ab === "back") abPick("");
}));
$("addblk").addEventListener("mousedown", whenReady((e) => { if (e.target.id === "addblk") abClose(); }));
$("btn-add-block").onclick = whenReady(abOpen);
const AB_TIME = /^\d{1,2}:\d{2}(:\d{2})?$/;
async function abAdd() {
  const pn = AB.parent && nodeById(AB.parent.uuid);
  const st = AB.parent ? pn && periphAddState(pn) : addState(), t = AB.ty;
  if (!st?.ok || !t || AB.busy) return;
  if (t.template) { tplAbAdd(st, t.template); return; }
  const op = { op: "add_block", page: AB.parent ? AB.parent.uuid : st.page.uuid, type: t.type, params: {}, settings: {} }, bad = {};
  for (const [field, , kind, , , , choices, req] of abFields(t)) {
    const raw = AB.draft[field] ?? "", v = kind === "text" || kind === "program" ? raw : raw.trim();
    if (req && !v.trim()) { bad[field] = "lox makes none without it."; continue; }
    if (v === "" && !(kind === "text" && raw !== "")) continue;
    if (kind === "program" && !v.trim()) continue;
    if (kind === "number" && !EDIT_NUMBER.test(v)) { bad[field] = /,/.test(v) ? "Write a decimal point, not a comma." : "A number, like 12 or -2.5."; continue; }
    if (kind === "quantity" && !EDIT_QUANTITY.test(v)) { bad[field] = /,/.test(v) ? "Write a decimal point, not a comma." : "A number, like 12 or -2.5, or one with its unit, like 30min."; continue; }
    if (kind === "time" && !AB_TIME.test(v)) { bad[field] = "A time of day, like 19:00."; continue; }
    if (kind === "duration" && !AB_DURATION.test(v)) { bad[field] = "A duration, like 12h, 30min, 5s or 90 (seconds)."; continue; }
    if (kind === "choice" && !choices.includes(v)) continue;
    if (field === "title") { if (!v.trim()) bad[field] = "A name, or nothing."; else op.title = v; continue; }
    const [k, key] = field.split(/:(.*)/s);
    (k === "p" ? op.params : op.settings)[key] = v;
  }
  if (t.mirrors && !op.settings.mirrors) bad["s:mirrors"] = "Pick the push notification it sends.";
  AB.bad = bad; AB.err = "";
  if (Object.keys(bad).length) { AB.err = "Fix the marked values."; abForm(); $("ab-body").querySelector('[aria-invalid="true"]')?.focus(); return; }
  AB.busy = true; abForm();
  try {
    const res = await editAct(st.rev.blob, [op]);
    AB.busy = false;
    abClose();
    await editShowNewest(res.blob);
    const uuid = res.added?.[0]?.uuid;
    const u = uuid && Object.keys(S.view?.objects || {}).find((k) => ciEq(k, uuid));
    if (u) {
      S.edit = { uuid: u, open: false, draft: {}, bad: {}, busy: false, msg: "Added and compiled: pending, not deployed. Review it, then Apply or Discard.", err: "" };
      if (!AB.parent) gotoPage(st.page.uuid, { focus: u });
      select(u);
      if (AB.parent) revealInTree(u);
    }
  } catch (err) {
    AB.busy = false; AB.err = err.message;
    if (AB.open) abForm();
  }
}
/** Step two for a template (#83): the instance's name, its label and its arguments. */
function tplAbForm(x) {
  const field = (f, label, hint, ph, mono) => {
    const id = `ab-${f.replace(/[^\w-]/g, "_")}`, bad = AB.bad[f];
    return `<div class="pr ed-row"><label class="k" for="${id}">${esc(label)}</label><div class="v"><input id="${id}" data-ab-field="${esc(f)}" value="${esc(AB.draft[f] ?? "")}" placeholder="${esc(ph)}"${mono ? ' class="mono"' : ""} title="${esc(hint)}" autocomplete="off" spellcheck="false"${bad ? ` aria-invalid="true" aria-describedby="${id}-e"` : ""}${AB.busy ? " disabled" : ""}>${bad ? `<div class="ed-bad" id="${id}-e">${esc(bad)}</div>` : ""}</div></div>`;
  };
  const lp = x.params.find((p) => p.name === "label");
  $("ab-body").innerHTML = `<div class="ab-form">
    <div class="ab-sum"><b>${esc(x.name)}</b> <span class="mono">template</span> · ${esc(x.description || "")} <span class="tm">${esc(x.from)}</span>${x.outputs.length ? ` · outputs ${esc(x.outputs.map((o) => o.name).join(", "))}` : ""}</div>
    ${field("slug", "Instance name", "vx_ and lowercase letters, digits and _: its blocks are named after it", "vx_name", true)}
    ${lp ? field("label", "Label", "Binds the template's label parameter", lp.default ?? "required") : ""}
    ${x.params.filter((p) => p.name !== "label").map((p) => field(`a:${p.name}`, `${p.name} (${p.kind === "value" ? p.type : p.kind})`, tpParamHint(p), p.default ?? (p.required ? "required" : "open"), true)).join("")}
    <div class="ed-bar"><button type="button" class="primary small" data-ab="add"${AB.busy ? " disabled" : ""}>${AB.busy ? "Compiling…" : "Add"}</button>
      <button type="button" class="ed-btn small" data-ab="back"${AB.busy ? " disabled" : ""}>Other type</button>
      <span class="ed-msg${AB.err ? " err" : ""}" role="status" aria-live="polite">${esc(AB.err || `Names: an object Loxone Config drew as vx_ and its UUID (- as _), a block of vislox's by its slug. ${x.lib ? "vislox copies the template into its directory. " : ""}Adding compiles the module; nothing is deployed.`)}</span></div>
  </div>`;
}
async function tplAbAdd(st, x) {
  const slug = (AB.draft.slug || "").trim(), args = {};
  AB.bad = {};
  if (!/^vx_[a-z0-9_]+$/.test(slug)) AB.bad.slug = "vx_ and then lowercase letters, digits and _.";
  for (const p of x.params.filter((q) => q.name !== "label")) {
    const v = (AB.draft[`a:${p.name}`] || "").trim();
    if (v) args[p.name] = v;
    else if (p.required) AB.bad[`a:${p.name}`] = "Required.";
  }
  const label = (AB.draft.label || "").trim();
  const lp = x.params.find((p) => p.name === "label");
  if (lp && lp.required && !label) AB.bad.label = "Required.";
  AB.err = "";
  if (Object.keys(AB.bad).length) { AB.err = "Fix the marked values."; abForm(); $("ab-body").querySelector('[aria-invalid="true"]')?.focus(); return; }
  AB.busy = true; abForm();
  try {
    const res = await editAct(st.rev.blob, [{ op: "instance", slug, page: st.page.uuid, value: { template: x.name, ...(label ? { label } : {}), args } }]);
    AB.busy = false;
    abClose();
    await editShowNewest(res.blob);
    await S.editsLoad;
    const inst = tplByPath(slug), b = inst?.blocks.find((y) => !y.expression) || inst?.blocks[0];
    const u = b && Object.keys(S.view?.objects || {}).find((k) => ciEq(k, b.uuid));
    if (u) { gotoPage(st.page.uuid, { focus: u }); select(u); }
  } catch (err) {
    AB.busy = false; AB.err = err.message;
    if (AB.open) abForm();
  }
}

// ── the Miniserver's log ───────────────────────────────────────────────
// `lox log` on the server: the system log's last lines. What came after the last program change
// (an Apply, or a save from Loxone Config) is marked; lines that name a block's UUID link to it.
// `stale`: an Apply ran since the log was read, so it describes the program before. `staleSeq` counts
// Applies, so that only a read that started after the last one clears it.
S.log = { open: pref("logOpen", false), data: null, loading: false, again: false, stale: false, staleSeq: 0, drawn: "", onlySince: pref("logSince", false) };
function logInfo() { return S.hist?.log || { available: false }; }
/** The entries since the last program change; all of them when the lines hold none. */
function logRecent() {
  const d = S.log.data;
  const sys = d?.entries ? d.entries.slice(d.since ?? 0) : [];
  // Debug-Monitor lines since the last program change in the system log.
  const after = d?.entries && d.since != null ? d.entries[d.since].at : "";
  return [...sys, ...S.mon.lines.filter((e) => e.at >= after)];
}
/** UUID (a block's, or one of its ports') → the block it belongs to. */
function logTargets() {
  const v = S.view;
  if (!v) return new Map();
  if (logTargets.view === v) return logTargets.map;
  const m = new Map();
  for (const [u, o] of Object.entries(v.objects)) {
    m.set(u.toLowerCase(), u);
    for (const q of o.ports || []) if (q.uuid) m.set(String(q.uuid).toLowerCase(), u);
  }
  logTargets.view = v; logTargets.map = m;
  return m;
}
/** Blocks named in the log since the last program change, with the newest line about each. Only on the
 *  deployed revision: the log is about what runs. */
function logMarks() {
  const r = revById(S.rev);
  if ((!S.log.data && !S.mon.lines.length) || S.log.stale || !isDeployed(r)) return new Map();
  const t = logTargets(), m = new Map();
  for (const e of logRecent()) for (const u of e.uuids || []) { const b = t.get(u); if (b) m.set(b, e); }
  return m;
}
function renderLog() {
  const info = logInfo(), mon = monInfo(), pane = $("log-pane");
  pane.classList.toggle("hidden", !info.available && !mon.available);
  if (!info.available && !mon.available) return;
  if (mon.available) monConnect();
  renderMonBar();
  const open = S.log.open;
  pane.classList.toggle("open", open);
  $("log-toggle").setAttribute("aria-expanded", String(open));
  $("log-body").classList.toggle("hidden", !open);
  const b = $("btn-log");
  b.classList.toggle("hidden", !open || !info.available);
  b.disabled = S.log.loading;
  b.classList.toggle("busy", S.log.loading);
  b.querySelector(".lbl").textContent = S.log.loading ? "Reading…" : "Refresh";
  const d = S.log.data;
  const recent = d?.entries && d.since != null && !S.log.stale ? d.entries.length - d.since : null;
  $("log-count").textContent = recent ? `${recent} new` : "";
  $("log-count").title = recent ? `${plural(recent, "entry")} since the last program change` : "";
  const msg = $("log-msg");
  const text = !info.available ? `No system log: ${info.reason}.` : d?.error ? `Cannot read the log: ${d.error}` : !d ? (S.log.loading ? "Reading the log (lox log)…" : "")
    : S.log.stale ? (S.log.loading ? "Read before the last Apply; reading it again…" : "Read before the last Apply. Refresh to read it again.")
    : d.since == null ? `No program change in the last ${d.lines} lines.` : "";
  msg.textContent = text;
  msg.className = "log-msg" + (d?.error ? " error" : "") + (text ? "" : " hidden");
  $("log-only-w").classList.toggle("hidden", !d || d.since == null);
  $("chk-log-since").checked = S.log.onlySince;
  const list = $("log");
  const entries = d?.entries || [], since = d?.since ?? null;
  const from = S.log.onlySince && since != null ? since : 0;
  const rows = [];
  // The Debug-Monitor's lines, live, above the system log.
  for (let i = S.mon.lines.length - 1; i >= 0; i--) rows.push(monRow(S.mon.lines[i]));
  for (let i = entries.length - 1; i >= from; i--) {
    const e = entries[i];
    const links = logLinks(e);
    rows.push(`<li class="${esc(e.kind)}${since != null && i >= since ? " since" : ""}" data-i="${i}"><span class="tm">${esc(e.at)}</span><span class="tx">${esc(e.text)}</span>${links}</li>`);
    if (i === since) rows.push(`<li class="mark">▲ The program changed here (Apply, or a save from Loxone Config)</li>`);
  }
  list.innerHTML = rows.join("") || (d && !d.error ? '<li class="tx">The log is empty.</li>' : "");
  if (open && !d && !S.log.loading && info.available) loadLog();
}
function marksKey(m) { return [...m].map(([u, e]) => `${u} ${e.at} ${e.text}`).join("\n"); }
/** Redraw the page when the blocks' log marks are not what it shows. */
function redrawMarks() {
  if (S.view && marksKey(logMarks()) !== S.log.drawn) { renderPage(); applyFocus(); }
}
/** After an Apply: the log read before is about the old program. Hide its marks; read it again now if
 *  the panel is open, else when it opens. */
function logStale() {
  S.log.stale = true; S.log.staleSeq++;
  if (S.log.open) loadLog();
  renderLog(); redrawMarks();
}
/** Links to the blocks a log line names. */
function logLinks(e) {
  const t = logTargets();
  return (e.uuids || []).map((u) => t.get(u)).filter((u, j, a) => u && a.indexOf(u) === j).map((u) => {
    const o = S.view.objects[u];
    return `<button type="button" class="go" data-go="${esc(u)}" title="${esc(o.path || "")}">${esc(autoTitle(o.title) ? prettyType(o.ty) : o.title)}</button>`;
  }).join("");
}
/** A Debug-Monitor line; `_k` is its number in this browser, which stays when older lines drop off. */
function monRow(e) {
  return `<li class="${esc(e.kind)} monitor since" data-m="${e._k}"><span class="tm">${esc(e.at)}<span class="tag">monitor</span></span><span class="tx">${esc(e.text)}</span>${logLinks(e)}</li>`;
}
async function loadLog() {
  if (!logInfo().available) return;
  // One read at a time; a request during one (an Apply that finished) runs after it.
  if (S.log.loading) { S.log.again = true; return; }
  const seq = S.log.staleSeq;
  S.log.loading = true; renderLog();
  try {
    const r = await fetch("/api/log", { headers: { "X-Vislox": "log" } });
    const j = await r.json().catch(() => ({}));
    S.log.data = r.ok ? j : { ...(S.log.data || {}), error: j.error || r.statusText };
    if (!S.log.data.error && seq === S.log.staleSeq) S.log.stale = false;
  } catch (e) {
    S.log.data = { ...(S.log.data || {}), error: e.message };
  } finally {
    S.log.loading = false;
    renderLog();
    redrawMarks();
    if (S.log.again) { S.log.again = false; loadLog(); }
  }
}
function setLogOpen(open) {
  S.log.open = open; setPref("logOpen", open);
  renderLog();
  if (open && S.log.stale) loadLog();
}
/** From a block's log mark: open the log and show that line. */
function openLog(e) {
  if (!S.log.open) setLogOpen(true);
  // A Debug-Monitor line, or one of the system log's (whose read may have failed).
  const i = e._k ? -1 : (S.log.data?.entries || []).indexOf(e);
  const li = e._k ? $("log").querySelector(`li[data-m="${e._k}"]`) : i >= 0 ? $("log").querySelector(`li[data-i="${i}"]`) : null;
  if (!li) return;
  li.scrollIntoView({ block: "nearest" });
  li.classList.add("flash");
  setTimeout(() => li.classList.remove("flash"), 2000);
}
$("log-toggle").onclick = whenReady(() => setLogOpen(!S.log.open));
$("btn-log").onclick = whenReady(() => loadLog());
$("chk-log-since").onchange = whenReady((e) => { S.log.onlySince = e.target.checked; setPref("logSince", S.log.onlySince); renderLog(); });
$("log").addEventListener("click", whenReady((e) => {
  const g = e.target.closest("[data-go]");
  if (!g) return;
  const u = g.dataset.go, pg = S.view?.objects[u]?.page;
  if (pg) gotoPage(pg, { focus: u });
  select(u);
}));

// ── the Debug-Monitor ──────────────────────────────────────────────────
// With `--monitor`, vislox listens for the Miniserver's debug output (UDP). Start asks the Miniserver to
// send it (dev/sps/log/<vislox>), Stop ends it; the lines arrive on /api/monitor.
S.mon = { es: null, info: null, lines: [], busy: false, msg: "", fresh: true, seq: 0, marksT: null };
function monInfo() { return S.mon.info || S.hist?.monitor || { available: false }; }
function monConnect() {
  if (S.mon.es) return;
  const es = new EventSource("/api/monitor");
  S.mon.es = es;
  es.addEventListener("info", (e) => { S.mon.info = JSON.parse(e.data); renderMonBar(); renderMs(); });
  const add = (list, reset) => {
    for (const e of list) e._k = ++S.mon.seq;
    if (reset) {
      S.mon.lines = list;
      if (S.log.open) renderLog();
    } else {
      const all = [...S.mon.lines, ...list], gone = all.slice(0, Math.max(0, all.length - 500));
      S.mon.lines = all.slice(-500);
      // Only the new rows, on top; lines that dropped off go. A closed panel draws all when it opens.
      if (S.log.open) {
        const box = $("log");
        box.querySelector(":scope > li.tx")?.remove();
        box.insertAdjacentHTML("afterbegin", list.slice().reverse().map(monRow).join(""));
        for (const e of gone) box.querySelector(`li[data-m="${e._k}"]`)?.remove();
      }
    }
    // The blocks' marks, at most twice a second.
    if (!S.mon.marksT) S.mon.marksT = setTimeout(() => { S.mon.marksT = null; redrawMarks(); }, 500);
  };
  // Each (re)connect starts with every line kept: replace, then append.
  es.onopen = () => { S.mon.fresh = true; };
  es.addEventListener("lines", (e) => { add(JSON.parse(e.data), S.mon.fresh); S.mon.fresh = false; });
  es.addEventListener("reset", (e) => add(JSON.parse(e.data), true));
}
function renderMonBar() {
  const m = monInfo(), bar = $("mon-bar");
  bar.classList.toggle("hidden", !m.available);
  if (!m.available) return;
  const st = $("mon-state"), b = $("btn-mon");
  const on = !!m.until;
  const until = on ? new Date(m.until * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";
  const listen = m.listening && !m.listening.ok ? m.listening.error : "";
  st.textContent = S.mon.msg || listen || m.error || (on ? `Debug-Monitor on until ${until} · ${plural(m.packets, "packet")}`
    : `Debug-Monitor: listening on UDP ${m.port}` + (m.packets ? ` · ${plural(m.packets, "packet")}` : "") +
      (m.ignored ? ` · ${m.ignored} from other addresses (${m.ignoredFrom}) dropped` : ""));
  st.className = "mon-state" + (on ? " on" : "") + (listen || m.error ? " error" : "");
  b.disabled = S.mon.busy || !!listen;
  b.querySelector(".lbl").textContent = S.mon.busy ? "…" : on ? "Stop" : "Start";
  b.title = on ? "Ask the Miniserver to stop sending its debug output (dev/sps/log)"
    : `Ask the Miniserver to send its debug output to ${m.dest || "this host"}:${m.port} for ${m.minutes} min (dev/sps/log/<address>)`;
}
$("btn-mon").onclick = whenReady(async () => {
  const on = !!monInfo().until;
  if (!on && !window.confirm("Ask the Miniserver to send its debug output to vislox?\n\nThis is what Loxone Config's Debug Information does. " +
    "Nothing in the config changes and the program does not restart. vislox stops it again with Stop, when it exits, or after the time limit.")) return;
  S.mon.busy = true; S.mon.msg = ""; renderMonBar();
  try {
    const r = await fetch(on ? "/api/monitor/stop" : "/api/monitor/start", { method: "POST", headers: { "X-Vislox": "monitor" } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) S.mon.msg = j.error || r.statusText;
  } catch (e) { S.mon.msg = e.message; }
  finally { S.mon.busy = false; renderMonBar(); setTimeout(() => { S.mon.msg = ""; renderMonBar(); }, 10000); }
});

// ── live values ────────────────────────────────────────────────────────
// The Miniserver streams a value per state UUID; a wire's source connector
// has the UUID of the state it carries. Logic blocks the app does not see
// stream nothing, so their wires have no value.

// ── operate inputs ─────────────────────────────────────────────────────
// Opt-in (vislox --operate, with the admin token): Pulse, On/Off or a value for a virtual input,
// to test a change the way Loxone Config's Liveview does. Only virtual inputs of the deployed
// config, only with live values on, and every command is confirmed first.
const opRes = new Map(); // input uuid → { text, cls }
const opWait = new Map(); // input uuid → { head, states: Set, timer }: one per input waiting for its update
function opOn() { return !!S.hist?.operate?.available && liveHere(); }
/** The virtual input a block operates: itself, or the one an input reference mirrors. */
function opTarget(o) {
  if (!opOn()) return null;
  const t = o.ty === "VirtualIn" ? o : o.ty === "InputRef" && o.target?.ty === "VirtualIn" ? S.view.objects[o.target.uuid] : null;
  return t && t.status !== "removed" ? t : null;
}
function opAttr(t, k) { return t.attrs.find((a) => a.name === k)?.value; }
function opControls(t) {
  const analog = opAttr(t, "Analog") === "true";
  const secured = !!S.live.controls[t.uuid]?.secured;
  const u = esc(t.uuid);
  const b = (cmd, label, tip) => `<button type="button" class="ghost small" data-op="${cmd}" data-op-uuid="${u}" title="${esc(tip)}">${label}</button>`;
  let ctl;
  if (analog) {
    const checks = opAttr(t, "EnVal") === "true";
    const min = checks ? opAttr(t, "MinVal") : null, max = checks ? opAttr(t, "MaxVal") : null;
    ctl = `<input type="number" step="any" data-op-uuid="${u}" aria-label="Value for ${esc(t.title)}"${min != null ? ` min="${esc(min)}"` : ""}${max != null ? ` max="${esc(max)}"` : ""}>` +
      b("set", "Set", "Send this value to the virtual input");
  } else {
    ctl = b("pulse", "Pulse", "On for a moment, then off") + b("on", "On", "Switch the virtual input on") + b("off", "Off", "Switch the virtual input off");
  }
  const r = opRes.get(t.uuid);
  return `<div class="op">${secured ? '<span class="op-lock" title="This block has a visualization password: vislox asks for it once per session">🔒</span>' : ""}` +
    `<span class="op-what">${esc(t.title)} <span class="tm">${analog ? "analog" : "digital"} virtual input${t.uuid !== S.sel ? " (mirrored)" : ""}</span></span>` +
    `<div class="op-ctl">${ctl}</div>` +
    `<div class="op-res${r ? " " + r.cls : ""}" role="status" aria-live="polite">${r ? esc(r.text) : "Sends a command to the Miniserver."}</div></div>`;
}
function opShow(uuid, text, cls) {
  opRes.set(uuid, { text, cls });
  $("props").querySelectorAll(".op-res").forEach((n) => {
    if (n.closest(".op")?.querySelector(`[data-op-uuid="${CSS.escape(uuid)}"]`)) { n.textContent = text; n.className = "op-res " + cls; }
  });
}
function visuPassword(ask) {
  let p = "";
  try { p = sessionStorage.getItem("vislox.visu") || ""; } catch { /* private mode */ }
  if (!p && ask) {
    p = window.prompt("This block has a visualization password. Enter it (kept for this browser session only):") || "";
    if (p) try { sessionStorage.setItem("vislox.visu", p); } catch { /* private mode */ }
  }
  return p;
}
async function operate(uuid, cmd) {
  const t = S.view?.objects[uuid];
  if (!t || !opOn()) return;
  cmd = String(cmd ?? "").trim();
  if (!cmd) { opShow(uuid, "Enter a value first.", "err"); return; }
  const what = { pulse: "Pulse", on: "On", off: "Off" }[cmd] || `the value ${cmd}`;
  if (!window.confirm(`Send "${what}" to the virtual input "${t.title}" on the Miniserver?\n\nThis runs on the real installation: the logic it feeds reacts.`)) return;
  const secured = !!S.live.controls[uuid]?.secured;
  const body = { uuid, cmd };
  if (secured) {
    body.visu_password = visuPassword(true);
    if (!body.visu_password) { opShow(uuid, "Not sent: the block needs its visualization password.", "err"); return; }
  }
  opShow(uuid, "Sending…", "busy");
  let r;
  try { r = await act("/api/operate", body); }
  catch (e) {
    // Only a password the Miniserver refused is forgotten; a key that expired, a block that refused
    // the command or a network error leave it.
    if (e.data?.badPassword) try { sessionStorage.removeItem("vislox.visu"); } catch { /* private mode */ }
    opShow(uuid, e.message.startsWith("not sent") ? e.message[0].toUpperCase() + e.message.slice(1) : "Not sent: " + e.message, "err");
    return;
  }
  const head = r.code == null ? `Sent, ${r.note}` : `Code ${r.code} (${r.note})`;
  if (r.code !== 200) { opShow(uuid, head, "err"); return; }
  // The answer's value is not reliable; the state update that follows is (Communicating with the Miniserver, p. 14).
  const states = new Set(Object.values(S.live.controls[uuid]?.states || {}).filter((x) => typeof x === "string"));
  for (const q of t.ports) states.add(q.uuid);
  opExpect(uuid, head, states);
}
/** Wait for `uuid`'s state update (any of `states`); each input waits on its own. */
function opExpect(uuid, head, states, ms = 3000) {
  clearTimeout(opWait.get(uuid)?.timer);
  const timer = setTimeout(() => {
    if (opWait.get(uuid)?.timer !== timer) return;
    opWait.delete(uuid);
    opShow(uuid, `${head} · no state update came back (the input is not streamed to the app, or its value did not change)`, "warn");
  }, ms);
  opWait.set(uuid, { head, states, timer });
  opShow(uuid, `${head} · waiting for the state update…`, "busy");
}
/** A `values` event: did an input we just operated report back? */
function opSeen(d) {
  for (const [uuid, w] of [...opWait]) {
    const k = Object.keys(d).find((k) => w.states.has(k));
    if (!k) continue;
    clearTimeout(w.timer);
    opWait.delete(uuid);
    opShow(uuid, `${w.head} · state update: ${liveText(d[k])}`, "ok");
  }
}
function renderOpLog(box) {
  const all = S.hist?.operate?.log || [];
  const ent = (e) => `<div class="op-ent${e.sent && e.code === 200 ? "" : " err"}" title="${esc(`${new Date(e.time * 1000).toLocaleString()}\n${e.uuid}`)}">
      <span class="l1"><b>${esc(e.cmd)}</b> → ${esc(e.title)}${e.secured ? " 🔒" : ""}</span>
      <span class="dt">${esc(fmtWhen(e.time))} · ${e.sent ? (e.code == null ? esc(e.note) : `code ${e.code} ${esc(e.note)}`) : `not sent: ${esc(e.note)}`}</span></div>`;
  // Commands that went out, and, apart, requests that failed before theirs could.
  for (const [sent, title, key] of [[true, "Commands sent to inputs", "opLogOpen"], [false, "Not sent", "opNotOpen"]]) {
    const log = all.filter((e) => !!e.sent === sent);
    if (!log.length) continue;
    const d = document.createElement("details");
    d.className = "op-log";
    d.open = !!S[key];
    d.ontoggle = () => (S[key] = d.open);
    d.innerHTML = `<summary class="hist-sec">${title} <span class="count">${log.length}</span></summary>` + log.map(ent).join("");
    box.appendChild(d);
  }
}

/** Live values belong to what runs on the Miniserver: the deployed revision, never an uncommitted edit on disk. */
function liveHere() {
  const r = revById(S.rev);
  return S.live.on && !!S.live.es && isDeployed(r);
}
/** A connector's value: streamed by the Miniserver, passed on by a reference or a memory flag,
 *  a constant in the config, or computed from a simple gate's inputs (`view.live`).
 *  Entries are `[value, text, label, kind]`; kind is "const" or "computed" for values the
 *  Miniserver did not send, and "queried" for values vislox asked a block for (opt-in). Cached until the next change. */
let lvCache = new Map();
function liveVal(uuid) { return liveHere() ? lvResolve(uuid, 0) : undefined; }
function lvResolve(u, depth) {
  if (!u || depth > 60) return undefined;
  if (lvCache.has(u)) return lvCache.get(u);
  lvCache.set(u, undefined); // a loop resolves to nothing
  const r = lvFind(u, depth);
  lvCache.set(u, r);
  return r;
}
function lvFind(u, depth) {
  const e = S.live.values.get(u);
  if (e) return e;
  const g = S.view?.live || {};
  if (g.pass?.[u]) { const r = lvResolve(g.pass[u], depth + 1); if (r) return r; }
  const ref = g.refs?.[u];
  if (ref) {
    const st = S.live.controls[ref]?.states || {};
    const k = ["value", "active", "position"].find((k) => typeof st[k] === "string" && S.live.values.has(st[k]));
    if (k) return S.live.values.get(st[k]);
    const sv = sysVal(ref);
    if (sv) return sv;
  }
  const fed = lvFed(u);
  if (fed) return fed;
  const lc = g.moods?.[u];
  if (lc) {
    // Mood is the number of the active mood; with several active it is not known.
    const su = S.live.controls[lc]?.states?.activeMoods;
    let ids;
    try { ids = JSON.parse(S.live.values.get(su)?.[0]); } catch { ids = null; }
    if (Array.isArray(ids) && ids.length === 1) return [ids[0], null, "the lighting controller's active mood", "computed"];
  }
  if (g.weather && u in g.weather) {
    const w = weatherNow();
    const f = WEATHER_FIELDS[g.weather[u]];
    if (w && f && w.entry[f[0]] !== undefined) return [w.entry[f[0]], null, `the weather server's ${w.what} (${f[1]})`, "computed"];
  }
  if (g.consts && u in g.consts) return [g.consts[u], null, "constant in the config", "const"];
  const gate = g.gates?.[u];
  if (gate) {
    const v = gateEval(gate, depth);
    if (v !== undefined) return [v, null, `computed by vislox from the ${GATE_NAME[gate.op]} block's inputs`, "computed"];
  }
  return undefined;
}
/** Where an input the Miniserver streams is fed: wire source → the inputs it feeds. The value an
 *  input receives is its wire's value, and that of every reference or memory flag upstream that
 *  passes it on. Built once per view. */
function fedIndex() {
  const v = S.view;
  if (v._fed) return v._fed;
  const keyOf = new Map();
  for (const o of Object.values(v.objects)) for (const q of o.ports) keyOf.set(q.uuid, q.key);
  const pass = v.live?.pass || {};
  const idx = new Map();
  for (const o of Object.values(v.objects)) for (const q of o.ports) {
    if (q.inv) continue;
    const srcs = [...new Set(q.conns.filter((c) => c.dir === "in" && c.status !== "removed").map((c) => c.src))];
    if (!srcs.length) continue;
    const who = `${autoTitle(o.title) ? prettyType(o.ty) : o.title}.${q.name}`;
    const digital = srcs.every((s) => !analogKey(keyOf.get(s) || ""));
    for (const s0 of srcs) {
      const e = { input: q.uuid, who, multi: srcs.length > 1, digital, analog: analogKey(keyOf.get(s0) || "") };
      for (let s = s0, i = 0; s && i < 60; s = pass[s], i++) {
        if (!idx.has(s)) idx.set(s, []);
        idx.get(s).push(e);
      }
    }
  }
  return (v._fed = idx);
}
/** A wire's value from the input it feeds, when the Miniserver streams that input. */
function lvFed(u) {
  for (const e of fedIndex().get(u) || []) {
    const v = S.live.values.get(e.input);
    if (!v) continue;
    const n = lvNum(v);
    // Several wires into one digital input: it is on when any is, so only off tells them all.
    if (e.multi) {
      if (e.digital && n === 0) return [0, null, `${e.who} receives 0 from all its wires`, "computed"];
      continue;
    }
    // A digital input makes any analog value but 0 a 1.
    if (e.analog && n === 1) continue;
    return [v[0], v[1], `the value ${e.who} receives`];
  }
  return undefined;
}
/** Loxone's system variables that follow from the Miniserver's clock and today's sunrise and sunset. */
const SYS_VARS = {
  Year: (t) => t.y, Month: (t) => t.mo, Day: (t) => t.d, Hour: (t) => t.h, Minute: (t) => t.mi,
  Time: (t) => t.min,
  Daylight: (t) => t.sr === undefined || t.ss === undefined ? undefined : +(t.min >= t.sr && t.min < t.ss),
};
function sysVal(uuid) {
  const ty = S.view?.objects[uuid]?.ty;
  const f = SYS_VARS[ty];
  if (!f) return undefined;
  const gl = S.live.globals || {};
  const m = /^(\d+)-(\d+)-(\d+) (\d+):(\d+)/.exec(S.live.values.get(gl.miniserverTime)?.[0] || "");
  if (!m) return undefined;
  const [y, mo, d, h, mi] = m.slice(1).map(Number);
  const v = f({ y, mo, d, h, mi, min: h * 60 + mi, sr: lvNum(S.live.values.get(gl.sunrise)), ss: lvNum(S.live.values.get(gl.sunset)) });
  if (v === undefined) return undefined;
  return [v, null, `computed by vislox from the Miniserver's clock${ty === "Daylight" ? " and today's sunrise and sunset" : ""}`, "computed"];
}
/** Analog connectors, by key (as the server draws analog wires). */
function analogKey(k) { return /^A[QI]/.test(k) || ["Txt", "TQ", "V", "Pos", "Value", "Temp"].includes(k); }
const GATE_NAME = { and: "AND", or: "OR", xor: "XOR", not: "NOT", eq: "Equal", ne: "Not equal", gt: "Greater", ge: "Greater or equal",
  lt: "Less", le: "Less or equal", add: "Add", sub: "Subtract", mul: "Multiply", div: "Divide" };
function lvNum(e) {
  const v = e?.[0];
  const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : NaN;
  return Number.isFinite(n) ? n : undefined;
}
/** A stateless gate's output from its inputs; undefined when an input it needs is unknown. */
function gateEval(g, depth) {
  // Each input: a number, null (unwired: ignored), or undefined (wired, value unknown).
  const xs = g.ins.map((i) => {
    if (i.src) {
      const n = lvNum(lvResolve(i.src, depth + 1));
      return n === undefined ? undefined : i.inv ? +(n === 0) : n;
    }
    if (i.inv) return 1; // an unwired inverted input is a constant 1
    return i.def ?? null;
  });
  const wired = xs.filter((x) => x !== null);
  const known = wired.filter((x) => x !== undefined).map((x) => x !== 0);
  const unknown = wired.length - known.length;
  switch (g.op) {
    case "and": return known.includes(false) ? 0 : unknown || !wired.length ? undefined : 1;
    case "or": return known.includes(true) ? 1 : unknown || !wired.length ? undefined : 0;
    case "xor": return unknown || !wired.length ? undefined : known.filter(Boolean).length % 2;
    case "not": return unknown || !wired.length ? undefined : +!known[0];
  }
  if (xs[0] === undefined || xs[1] === undefined) return undefined;
  const a = xs[0] ?? 0, b = xs[1] ?? 0;
  switch (g.op) {
    case "eq": return +(a === b);
    case "ne": return +(a !== b);
    case "gt": return +(a > b);
    case "ge": return +(a >= b);
    case "lt": return +(a < b);
    case "le": return +(a <= b);
    case "add": return a + b;
    case "sub": return a - b;
    case "mul": return a * b;
    case "div": return b === 0 ? undefined : a / b;
  }
  return undefined;
}
/** `[value, text in the control's format, "Control · state"]` → what to show. */
function liveText(e) {
  const [v, t] = e;
  if (t) return t;
  // A Daytimer's or the weather server's event table: shown in its own section.
  if (v && typeof v === "object") return v.schedule ? `schedule (${v.entries.length} ${v.entries.length === 1 ? "entry" : "entries"})` : v.weather ? "weather (see Weather)" : JSON.stringify(v);
  if (typeof v === "number") return String(Number.isInteger(v) ? v : +v.toFixed(2));
  return String(v ?? "");
}
function liveOn(e) { const v = e[0]; return typeof v === "number" ? v !== 0 : !!v && v !== "0"; }
function liveTip(src) {
  if (!liveHere()) return "";
  const e = liveVal(src);
  return e ? `<br>value <b>${esc(liveText(e))}</b>${e[2] ? ` <span class="tm">${esc(e[2])}</span>` : ""}` : `<br><span class="tm">no live value</span>`;
}
function lvSpan(u) {
  const e = liveVal(u);
  return `<span class="lv${e ? "" : " none"}${e?.[3] ? " " + e[3] : ""}" data-lv="${esc(u)}"${e?.[2] ? ` title="${esc(e[2])}"` : ""}>${lvInner(e)}</span>`;
}
/** What a value span holds: the icon its state names (a Status block's), and the value. */
function lvInner(e) {
  return e ? lvIcon(e) + esc(clip(liveText(e), 40)) : "no value";
}

// ── schedules, weather and Status blocks ─────────────────────────────────
// A Daytimer's switching times (and an Intelligent Room Controller's) as the config stores them, next to
// the event table the Miniserver streams for it: what runs now, edits made in the app included.

/** Live values come in from the Miniserver (on any revision: a schedule is compared with each). */
function liveStreaming() { return S.live.on && !!S.live.es; }
/** The Miniserver's schedule for `si` (`{default, entries}`), when it streams one. */
function liveSchedule(si) {
  const v = S.live.values.get(si.state)?.[0];
  return v && typeof v === "object" && v.schedule ? v : undefined;
}
function hhmm(m) { return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`; }
/** A number as live values write it. */
const numText = (v) => liveText([+v]);
const byTime = (a, b) => a.from - b.from || a.to - b.to;
/** Entries for comparing. A digital timer's value does not count: the Miniserver sends 0 where the file has 1. */
const schedKey = (es, analog) => es.map((e) => `${e.from}-${e.to}-${analog ? +e.value : ""}-${!!e.activate}`).join("|");
function schedLine(es, analog) {
  return es.length ? es.map((e) => `${hhmm(e.from)}–${hhmm(e.to)}${analog ? " " + numText(e.value) : ""}${e.activate ? " (activate)" : ""}`).join(", ") : "no entries";
}
/** One row per column: the config's columns, then the compared revision's, then any column only entries name,
 *  then the Miniserver's modes no column has. A column's entries are those that name it (by UUID, so columns
 *  without a mode stay apart); the Miniserver's are matched by the column's mode number. */
function schedRows(si, live) {
  const rows = [], seen = new Set();
  const add = (col, c) => { if (!seen.has(col)) { seen.add(col); rows.push({ col, title: c?.title ?? col, target: c?.target, number: c?.number ?? null }); } };
  for (const c of si.cols) add(c.col, c);
  for (const c of si.old_cols || []) add(c.col, c);
  for (const e of [...si.entries, ...(si.old_entries || [])]) add(e.col, { title: `${e.col} (no operating mode in this config)` });
  const numbers = new Set(rows.map((r) => r.number).filter((n) => n != null));
  for (const e of live?.entries || []) if (!numbers.has(e.mode)) { numbers.add(e.mode); rows.push({ col: null, title: `Mode ${e.mode}`, number: e.mode, liveOnly: true }); }
  for (const r of rows) {
    r.mine = r.col == null ? [] : si.entries.filter((e) => e.col === r.col).sort(byTime);
    r.old = si.old_entries && r.col != null ? si.old_entries.filter((e) => e.col === r.col).sort(byTime) : null;
    r.theirs = live && r.number != null ? live.entries.filter((e) => e.mode === r.number).sort(byTime) : null;
    r.differs = !!r.theirs && schedKey(r.mine, si.analog) !== schedKey(r.theirs, si.analog);
    r.changed = !!r.old && schedKey(r.old, si.analog) !== schedKey(r.mine, si.analog);
  }
  return rows;
}
/** How the config's schedule and the Miniserver's differ: the rows, and the default value. */
function schedCompare(si, live) {
  const rows = schedRows(si, live);
  const defDiff = si.analog && si.default != null && live.default != null && +si.default !== +live.default;
  const what = rows.filter((r) => r.differs).map((r) => r.title);
  if (defDiff) what.push("default value");
  return { rows, defDiff, what: what.join(", "), same: !what.length };
}
/** One day as a bar: each entry a segment over the hours, its value in the tooltip. */
function schedBar(es, si, cls) {
  const segs = es.map((e) => {
    const l = (e.from / 1440) * 100, w = Math.max(((e.to - e.from) / 1440) * 100, 0.4);
    const tip = `${hhmm(e.from)}–${hhmm(e.to)}${si.analog ? " · " + numText(e.value) : ""}${e.activate ? " · needs activation" : ""}`;
    return `<span class="sch-seg${e.activate ? " act" : ""}" style="left:${l.toFixed(2)}%;width:${w.toFixed(2)}%" title="${esc(tip)}">${si.analog && w > 9 ? esc(numText(e.value)) : ""}</span>`;
  }).join("");
  return `<div class="sch-bar ${cls}" title="${esc(schedLine(es, si.analog))}">${segs}</div>`;
}
function scheduleRows(o, row, marks) {
  const si = o.schedule;
  const live = liveStreaming() ? liveSchedule(si) : undefined;
  const cmp = live ? schedCompare(si, live) : null;
  const rows = schedRows(si, live);
  const r = revById(S.rev);
  const out = setClaims(o.uuid).map(claimNote);
  // What the comparison means on this revision.
  let note;
  if (!S.live.on) note = `<div class="sch-note">Turn on live values to compare with the switching times the Miniserver runs.</div>`;
  else if (!live) note = `<div class="sch-note">The Miniserver sends no schedule for this block${liveStreaming() ? " (only blocks shown in the app have one)" : ""}.</div>`;
  else if (cmp.same) note = `<div class="sch-note ok">The Miniserver runs the same switching times.</div>`;
  else if (r?.kind === "pending") note = `<div class="sch-note warn">⚠ The Miniserver runs other switching times (${esc(cmp.what)}). Apply would replace them with this config's.</div>`;
  else if (isDeployed(r)) note = `<div class="sch-note warn">⚠ Changed in the app since this config (${esc(cmp.what)}). An edit in the app is a custom change until Loxone Config saves the program: Apply refuses while one is pending, and replaces it with the config's switching times when you allow that.</div>`;
  else note = `<div class="sch-note warn">The Miniserver runs other switching times than this revision (${esc(cmp.what)}).</div>`;
  out.push(note);
  if (si.analog && (si.default != null || live)) {
    const differs = cmp?.defDiff;
    out.push(row("Default value", `${si.default != null ? esc(numText(si.default)) : '<span class="none-v">—</span>'}${live ? ` <span class="sch-live${differs ? " diff" : ""}" title="The Miniserver's">${esc(numText(live.default))}</span>` : ""}`, differs ? "pr-chg" : ""));
  }
  out.push(`<div class="sch-grid"><div class="sch-axis"><span></span><span class="sch-hours"><i>0</i><i>6</i><i>12</i><i>18</i><i>24 h</i></span></div>` + rows.map((x) => {
    const name = x.target ? `<button type="button" class="lnk" data-go="${esc(x.target.uuid)}" data-page="${esc(x.target.page || "")}" title="${esc(x.title)}">${esc(x.title)}</button>` : `<span title="${esc(x.title)}">${esc(x.title)}</span>`;
    const chg = marks && x.changed;
    const was = chg ? `<div class="sch-was">was ${esc(schedLine(x.old, si.analog))}</div>` : "";
    return `<div class="sch-row${x.differs ? " diff" : ""}${chg ? " chg" : ""}"><div class="sch-mode">${name}</div><div class="sch-bars">${schedBar(x.mine, si, "cfg")}${x.theirs ? schedBar(x.theirs, si, "live") : ""}</div>${was}${x.differs ? `<div class="sch-was">Miniserver: ${esc(schedLine(x.theirs, si.analog))}</div>` : ""}</div>`;
  }).join("") + `</div>`);
  if (live) out.push(`<div class="sch-legend"><span class="sch-key cfg"></span>this config <span class="sch-key live"></span>the Miniserver</div>`);
  return out;
}
/** The schedules (`/api/schedules` of a revision) whose live entries differ: what Apply would replace. */
function scheduleConflicts(blocks) {
  const out = [];
  for (const b of blocks) {
    const live = liveSchedule(b.schedule);
    if (!live) continue;
    const cmp = schedCompare(b.schedule, live);
    const by = claimsOf(b.uuid).map(claimAt);
    if (!cmp.same) out.push(`${autoTitle(b.title) ? prettyType(b.ty) : b.title}: ${cmp.what}${by.length ? ` (claimed by ${by.join(", ")})` : ""}`);
  }
  return out;
}

/** The Miniserver's clock as Loxone time (seconds since 2009-01-01 in its local time), from its
 *  `miniserverTime` state; undefined until that came. */
function msLoxTime() {
  const m = /^(\d+)-(\d+)-(\d+) (\d+):(\d+)(?::(\d+))?/.exec(S.live.values.get(S.live.globals?.miniserverTime)?.[0] || "");
  if (!m) return undefined;
  const [y, mo, d, h, mi, sec] = m.slice(1).map((x) => Number(x || 0));
  return (Date.UTC(y, mo - 1, d, h, mi, sec) - Date.UTC(2009, 0, 1)) / 1000;
}
/** The weather server's current weather: its `actual` state, else the forecast's hour that has begun on the
 *  Miniserver's clock (its timestamps are in the Miniserver's local time, not the browser's). */
function weatherNow() {
  const gl = S.live.globals || {};
  const tab = (k) => { const v = S.live.values.get(gl[k])?.[0]; return v && v.weather && v.entries?.length ? v : null; };
  const a = tab("weather.actual");
  if (a) return { entry: a.entries[0], what: "current weather" };
  const f = tab("weather.forecast");
  const now = msLoxTime();
  if (!f || now === undefined) return null;
  const past = f.entries.filter((e) => e.time <= now);
  return past.length ? { entry: past[past.length - 1], what: "forecast for this hour" } : null;
}
/** A weather data block's `WType=` → the field of a weather entry, and what it is. */
const WEATHER_FIELDS = {
  1: ["temperature", "temperature"], 2: ["dewPoint", "dew point"], 3: ["humidity", "relative humidity"],
  4: ["windSpeed", "wind speed"], 5: ["windDirection", "wind direction"], 7: ["solarRadiation", "solar radiation"],
  9: ["precipitation", "precipitation"], 10: ["type", "weather type"], 11: ["pressure", "air pressure"],
  26: ["perceived", "perceived temperature"],
};
function weatherRows(row) {
  if (!liveHere()) return null;
  const w = weatherNow();
  if (!w) return { rows: [`<div class="empty">The Miniserver sends no weather (it needs a weather subscription).</div>`], count: 0 };
  const rows = [row("From", esc(w.what))];
  for (const [k, label] of Object.values(WEATHER_FIELDS)) if (w.entry[k] !== undefined) rows.push(row(label, `<span class="lv computed">${esc(numText(w.entry[k]))}</span>`));
  return { rows, count: rows.length - 1 };
}

/** The icon names the structure file lists: only those are shown (and `/api/app/icon` serves no other). */
function appIconNames() {
  const a = S.app;
  if (!a) return new Set();
  if (appIconNames.app === a) return appIconNames.set;
  const set = new Set();
  for (const k of ["rooms", "cats", "controls"]) for (const x of Object.values(a[k] || {})) if (x.image) set.add(x.image);
  appIconNames.app = a; appIconNames.set = set;
  return set;
}
/** A live value's icon (e[4]), when the structure file knows it. */
function lvIcon(e) { return e?.[4] && appIconNames().has(e[4]) ? appIcon(e[4]) : ""; }
/** A Status block (the app's TextState): its live text with the icon and colour the Miniserver gives. The icon
 *  shows when the structure file lists it; else its colour. */
function statusRows(o) {
  if (!liveHere()) return null;
  const st = S.live.controls[o.uuid]?.states || {};
  if (!st.textAndIcon) return null;
  const text = S.live.values.get(st.textAndIcon);
  let ic = null, color = null;
  try { const j = JSON.parse(S.live.values.get(st.iconAndColor)?.[0] || "null"); ic = j?.icon || null; color = j?.color || null; } catch { /* not JSON */ }
  const icon = ic && appIconNames().has(ic) ? appIcon(ic) : "";
  const sw = color && /^#[0-9a-f]{3,8}$/i.test(color) ? `<span class="swatch" style="background:${color}" title="${esc(color)}"></span>` : "";
  return [`<div class="status-live">${icon}${sw}<span>${text ? esc(liveText(text)) : '<span class="none-v">no value</span>'}</span></div>`];
}

/** The Properties sections built from live values other than a connector's: they are rebuilt when one of the
 *  states they read (`deps`) changes. */
function liveSections(o, row, marks) {
  const out = [];
  const st = S.live.controls[o.uuid]?.states || {};
  const stat = statusRows(o);
  if (stat) out.push({ name: "Status", rows: stat, deps: [st.textAndIcon, st.iconAndColor] });
  if (o.schedule) out.push({ name: "Schedule", rows: scheduleRows(o, row, marks), count: o.schedule.cols.length, deps: [o.schedule.state] });
  if (o.ty === "WeatherServer") {
    const w = weatherRows(row), gl = S.live.globals || {};
    if (w) out.push({ name: "Weather", rows: w.rows, count: w.count, deps: [gl["weather.actual"], gl["weather.forecast"], gl.miniserverTime] });
  }
  return out;
}
function liveSectionsRefresh(ch) {
  const o = S.sel && S.view?.objects[S.sel];
  if (!o || !ch) return;
  const marks = S.marks && S.view.compared;
  for (const sec of liveSections(o, propRow, marks)) {
    if (!sec.deps.some((d) => d && ch.has(d))) continue;
    const body = document.querySelector(`#props .ps-b[data-body="${CSS.escape(sec.name)}"]`);
    if (body) body.innerHTML = sec.rows.join("");
  }
}

function renderLiveSwitch() {
  const info = S.hist?.live || { available: false, reason: "" };
  const sw = $("sw-live"), chk = $("chk-live");
  const on = info.available && S.live.on;
  chk.disabled = !info.available;
  chk.checked = on;
  sw.classList.toggle("disabled", !info.available);
  sw.classList.toggle("on", on);
  sw.title = info.available ? `Show the current values from ${info.source} on the wires (L)` : `No live values: ${info.reason}`;
  const here = liveHere();
  const st = S.live.status || { state: "connecting" };
  const ls = $("live-state");
  // A known reason (close or response code) names itself: "blocked", "firmware update", …
  ls.textContent = !here ? "other revision" : st.label || ({ live: "", connecting: "connecting…", idle: "connecting…", offline: "offline", stopped: "stopped", ended: "input ended", off: "off" }[st.state] ?? st.state);
  ls.className = "live-state st-" + (here ? st.state : "elsewhere");
  ls.title = [st.detail, info.source && `from ${info.source}`].filter(Boolean).join("\n");
  $("legend").classList.toggle("live", on);
  // With a known reason, its plain sentence (the detail's first line) says what to do.
  $("live-note").textContent = on && !here ? "Live values show on the deployed revision only (what the Miniserver runs, not an edit on disk)"
    : on && st.reason && st.state !== "live" ? (st.detail || "").split("\n")[0] : "";
  svg.classList.toggle("live", here);
  svg.classList.toggle("live-dim", here && S.live.dim);
  $("lg-query").classList.toggle("hidden", !info.query);
  liveQuerySync(here && info.query && S.live.query);
  modesLive();
}

/** Opt-in block queries (`/api/live/query`): the server queries the blocks behind the wires of the
 *  page on screen while this stream is open; their values come on `/api/live`, marked "queried". */
function liveQuerySync(want) {
  const key = want && S.page ? `${S.rev}|${S.page}` : "";
  if (key === S.live.qkey) { renderQueryNote(); return; }
  S.live.qes?.close();
  S.live.qes = null;
  S.live.qkey = key;
  S.live.qstat = null;
  if (key) liveQueryOpen(key);
  renderQueryNote();
}
/** Ask for the page's queries (a POST with X-Vislox, so no other site can start them), then hold
 *  them with the stream for the token it returns. A stream the server no longer knows (it restarted)
 *  asks again. */
function liveQueryOpen(key) {
  const [rev, page] = [S.rev, S.page];
  fetch("/api/live/query", { method: "POST", headers: { "X-Vislox": "query", "Content-Type": "application/json" }, body: JSON.stringify({ rev, page }) })
    .then((r) => r.json().then((j) => (r.ok ? j : Promise.reject(new Error(j.error || r.statusText)))))
    .then((j) => {
      if (S.live.qkey !== key) return;
      S.live.qstat = { blocks: j.blocks, planned: true };
      const es = new EventSource(`/api/live/query?token=${encodeURIComponent(j.token)}`);
      S.live.qes = es;
      es.addEventListener("query", (e) => { if (S.live.qes === es) { S.live.qstat = JSON.parse(e.data); renderQueryNote(); } });
      es.addEventListener("refused", () => {
        es.close();
        if (S.live.qes !== es) return;
        S.live.qes = null;
        setTimeout(() => { if (S.live.qkey === key && !S.live.qes) liveQueryOpen(key); }, 2000);
      });
      renderQueryNote();
    })
    .catch((e) => { if (S.live.qkey === key) { S.live.qstat = { error: e.message }; renderQueryNote(); } });
}
function renderQueryNote() {
  const n = $("query-note"), q = S.live.qstat;
  const failed = q?.failed || [];
  const text = !S.live.qkey || !q ? ""
    : q.error ? `not queried: ${q.error}`
    : q.planned ? (q.blocks ? `querying ${q.blocks} block${q.blocks === 1 ? "" : "s"}…` : "no blocks to query here")
    : (q.blocks ? `${q.answered} of ${q.blocks} blocks answered` : "nothing to query: the Miniserver streams the rest")
      + (failed.length ? ` · ${failed.length} cannot be queried` : "");
  const title = failed.join("\n");
  // Called on every repaint: touch the DOM only when something changed.
  if (n.textContent !== text) n.textContent = text;
  if (n.title !== title) n.title = title;
}

function liveStart() {
  if (S.live.es) return;
  const es = new EventSource("/api/live");
  S.live.es = es;
  S.live.status = { state: "connecting" };
  es.addEventListener("status", (e) => {
    S.live.status = JSON.parse(e.data);
    renderLiveSwitch();
    // A climate block's Properties say why the periods are not there yet.
    if (!S.live.periods && mdClimate(S.view?.objects[S.sel])) renderProps();
  });
  es.addEventListener("snapshot", (e) => {
    const d = JSON.parse(e.data);
    S.live.values = new Map(Object.entries(d.values));
    S.live.controls = d.controls || {};
    S.live.globals = d.globals || {};
    S.live.periods = d.periods || null;
    lvCache = new Map();
    liveQueue(null);
  });
  // The heating and cooling periods, as each Miniserver connection reads them (null until asked).
  es.addEventListener("periods", (e) => {
    S.live.periods = JSON.parse(e.data);
    if (mdClimate(S.view?.objects[S.sel])) renderProps();
    modesLive();
  });
  es.addEventListener("values", (e) => {
    const d = JSON.parse(e.data);
    for (const k in d) if (d[k] === null) S.live.values.delete(k); else S.live.values.set(k, d[k]);
    opSeen(Object.fromEntries(Object.entries(d).filter(([, v]) => v !== null)));
    lvCache = new Map();
    liveQueue(Object.keys(d));
  });
  es.onerror = () => {
    if (es.readyState === EventSource.OPEN) return;
    S.live.status = { state: "offline", detail: "vislox is not reachable; retrying" };
    renderLiveSwitch();
  };
}
function liveStop() {
  S.live.es?.close();
  S.live.es = null;
  S.live.status = null;
  S.live.values = new Map();
  S.live.controls = {};
  S.live.globals = {};
  S.live.periods = null;
  liveQueue(null);
}
// Turned on by the user (the switch, L): let the server try again after it stopped (blocked,
// login refused). A reload or a reconnecting stream never does, so it cannot lengthen a block.
function setLive(on, retry = true) {
  S.live.on = on;
  setPref("live", on);
  if (on && !retry) liveStart();
  else if (on) {
    fetch("/api/live/retry", { method: "POST", headers: { "X-Vislox": "live" } })
      .catch(() => {})
      .finally(() => { if (S.live.on) liveStart(); });
  } else liveStop();
  renderLiveSwitch();
}

/** Repaint changed values once per frame; `null`: everything. */
let livePending = new Set(), liveAll = false, liveRaf = 0;
function liveQueue(keys) {
  if (keys) keys.forEach((k) => livePending.add(k)); else liveAll = true;
  if (!liveRaf) liveRaf = requestAnimationFrame(() => {
    liveRaf = 0;
    const ch = liveAll ? null : livePending;
    liveAll = false; livePending = new Set();
    liveApply(ch);
    if (!ch && S.sel) renderProps();
  });
}

/** Index the page by source connector: its wires, and where its value is written. */
function liveBuild(p) {
  const idx = new Map();
  const get = (src) => {
    let it = idx.get(src);
    if (!it) idx.set(src, (it = { lines: [], pills: [], analog: false, seen: new Set() }));
    return it;
  };
  vpG.querySelectorAll("[data-src]").forEach((n) => {
    const it = get(n.dataset.src);
    it.lines.push(n);
    if (n.classList.contains("analog")) it.analog = true;
  });
  const pill = (src, x, y, end, owner) => {
    const it = get(src), key = `${Math.round(x / 50)}|${Math.round(y / 50)}`;
    if (it.seen.has(key)) return;
    it.seen.add(key);
    const g = el("g", { class: "live-pill", "data-f": owner }, L.pills);
    g.style.display = "none";
    const r = el("rect", { y: y - 190, height: 150, rx: 75 }, g);
    const t = el("text", { y: y - 79 }, g);
    it.pills.push({ g, r, t, x: end ? x - 150 : x + 150, end });
  };
  for (const w of p.wires) pill(w.src, w.points[0][0], w.points[0][1], false, w.from[0]);
  for (const l of p.labels) pill(l.src, l.x, l.y, l.side === "L", l.obj);
  // Pins with no wire to carry their value, and inputs fed by several wires, show it at the pin
  // (as Loxone Config's Liveview does): a pill for analog values, the pin's colour for digital ones.
  for (const o of p.objects) {
    const info = o.shape !== "tag" && !o.moved_to && S.view.objects[o.uuid];
    if (!info) continue;
    const wired = L.wired.get(o.uuid);
    for (const q of o.ports) {
      const pi = info.ports.find((r) => r.key === q.key);
      if (!pi) continue;
      const left = q.side === "L";
      const ins = pi.conns.filter((c) => c.dir === "in").length;
      if (left ? ins === 1 || (!wired?.has(q.key) && q.def != null) : wired?.has(q.key)) continue;
      const it = get(pi.uuid);
      it.pin = true;
      const n = el("rect", { class: `pin-lv${analogKey(q.key) ? " analog" : ""}`, x: left ? q.x - 112 : q.x + 12, y: q.y - 40, width: 100, height: 80, rx: 14 }, L.pills);
      it.lines.push(n);
      if (analogKey(q.key)) it.analog = true;
      pill(pi.uuid, left ? q.x - 112 : q.x + 112, q.y, left, o.uuid);
    }
  }
  return idx;
}

/** Repaint every value on the page: derived values change with their inputs, so `ch` (the
 *  streamed values that changed, or null for a full repaint) only decides what flashes. */
function liveApply(ch) {
  renderLiveSwitch();
  if (!ch) lvCache = new Map();
  for (const [src, it] of L.live || []) {
    const e = liveVal(src);
    for (const n of it.lines) {
      n.classList.toggle("lv-has", !!e);
      n.classList.toggle("lv-on", !!e && !n.classList.contains("analog") && liveOn(e));
    }
    const n = it.pin && lvNum(e);
    const show = !!e && (it.analog || typeof e[0] === "string" || (it.pin && n !== undefined && n !== 0 && n !== 1));
    const txt = show ? clip(liveText(e), 18) : "";
    for (const pl of it.pills) {
      pl.g.style.display = show ? "" : "none";
      pl.g.classList.toggle("computed", e?.[3] === "computed");
      pl.g.classList.toggle("queried", e?.[3] === "queried");
      if (!show) continue;
      if (pl.t.textContent !== txt) {
        const was = pl.t.textContent;
        pl.t.textContent = txt;
        const w = txt.length * 58 + 100, x = pl.end ? pl.x - w : pl.x;
        pl.r.setAttribute("x", x); pl.r.setAttribute("width", w);
        pl.t.setAttribute("x", x + 50);
        if (ch && was) flash(pl.g);
      }
    }
  }
  document.querySelectorAll("#props [data-lv], #deploy-new [data-lv]").forEach((n) => {
    const e = liveVal(n.dataset.lv);
    const html = lvInner(e);
    const changed = n.innerHTML !== html;
    if (changed) n.innerHTML = html;
    n.className = "lv" + (e ? "" : " none") + (e?.[3] ? " " + e[3] : "");
    n.title = e?.[2] || "";
    if (ch && e && changed) flash(n);
  });
  liveSectionsRefresh(ch);
  modesLive();
}
function flash(n) {
  n.classList.remove("tick");
  void n.getBoundingClientRect();
  n.classList.add("tick");
  clearTimeout(n._tick);
  n._tick = setTimeout(() => n.classList.remove("tick"), 700);
}

// ── live updates ───────────────────────────────────────────────────────
function connect() {
  const es = new EventSource("/api/events");
  // Changes while the stream was down (or before it opened) are not replayed: catch up.
  es.onopen = () => { if (S.hist) setStatus("live", "live"); loadHistory(); };
  // A new deployed revision may have taken over the app's calendar edits.
  es.addEventListener("history", () => { loadHistory(true); loadCal(); });
  // `changed`: re-read it. `read` again unchanged: only an Apply waiting for a read cares.
  es.addEventListener("app", (e) => { if (e.data !== "read" || (S.deployed && !awaitedAppRead())) loadApp(); });
  es.addEventListener("actions", (e) => {
    if (!S.hist) return;
    const was = S.hist.actions;
    S.hist.actions = JSON.parse(e.data);
    renderActions();
    editsChanged();
    // The Edit section shows whether editing is on and what runs.
    const a = S.hist.actions;
    if ((was?.running !== a.running || was?.edit?.available !== a.edit?.available || was?.edit?.reason !== a.edit?.reason) && S.sel && S.view) renderProps();
  });
  es.addEventListener("calendar", () => loadCal());
  es.addEventListener("operate", (e) => { if (S.hist) { S.hist.operate = JSON.parse(e.data); renderHistory(); } });
  es.onerror = () => {
    setStatus("disconnected — retrying", "error");
    if (es.readyState === EventSource.CLOSED && streamRetryTimer == null) {
      streamRetryTimer = setTimeout(() => { streamRetryTimer = null; connect(); }, 1000);
    }
  };
}
