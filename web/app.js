"use strict";
// vislox — read-only Loxone Config style viewer.
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
  follow: true, marks: true, vp: {}, open: new Set(["__pages"]), chgOff: new Set(),
  objPage: new Map(), pageOf: new Map(), loadSeq: 0, onlyChanged: false, chgIdx: -1,
  closed: new Set(["Advanced"]), dim: true, treeFocus: null, hash: null, applyingHash: false,
  live: { on: pref("live", false), dim: pref("liveDim", true), es: null, status: null, values: new Map(), controls: {}, globals: {} },
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
function allRevs() { return S.hist ? [...S.hist.pending, ...S.hist.history] : []; }
function revById(id) { return allRevs().find((r) => r.id === id); }
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
  else if (r.kind === "live") title = r.label.replace(/ \(same as an older commit\)/, "");
  else if (m) title = `Backup v${m[1]}`;
  else title = title.replace(/^\[[^\]]+\]\s*/, "");
  const sub = r.kind === "git" ? `${fmtWhen(r.time, true)} · ${r.detail}` : fmtWhen(r.time, true);
  return { title, sub, short: `${title} · ${fmtWhen(r.time)}` };
}

// ── URL state: #page=…&sel=…&rev=…&base=… ─────────────────────────────
function readHash() {
  const h = new URLSearchParams(location.hash.slice(1));
  return { page: h.get("page"), sel: h.get("sel"), rev: h.get("rev"), base: h.get("base") };
}
function writeHash(push) {
  if (S.applyingHash) return;
  const h = new URLSearchParams();
  if (S.page) h.set("page", S.page);
  if (S.sel) h.set("sel", S.sel);
  if (!S.follow && S.rev) h.set("rev", S.rev);
  if (!S.follow && S.base) h.set("base", S.base);
  const s = "#" + h.toString();
  if (s === location.hash) return;
  if (push) history.pushState(null, "", s); else history.replaceState(null, "", s);
}
window.addEventListener("popstate", async () => {
  const h = readHash();
  S.applyingHash = true;
  try {
    if (h.rev && (h.rev !== S.rev || (h.base || null) !== S.base) && revById(h.rev)) {
      S.rev = h.rev; S.base = h.base || null; S.follow = false; $("chk-follow").checked = false;
      S.hash = h; renderHistory(); renderSelects(); await loadView();
    } else {
      if (h.page && h.page !== S.page) gotoPage(h.page);
      select(h.sel || null, { noReveal: false });
    }
  } finally { S.applyingHash = false; }
});

// ── history ────────────────────────────────────────────────────────────
async function loadHistory(fromEvent) {
  let h;
  try { h = await api("/api/history"); } catch (e) { setStatus("server unreachable", "error"); return; }
  const prevNewest = S.hist ? (S.hist.pending[0] || S.hist.history[0])?.id : null;
  S.hist = h;
  $("doc-sub").textContent = h.label + (h.error ? " · " + h.error : "");
  const newest = h.pending[0] || h.history[0];
  let changed = false;
  if (S.hash?.rev && revById(S.hash.rev) && !S.rev) {
    S.rev = S.hash.rev; S.base = S.hash.base || null; S.follow = false; $("chk-follow").checked = false; changed = true;
  }
  if (!S.rev || !revById(S.rev) || (S.follow && newest && newest.id !== S.rev)) {
    const inPending = S.rev && S.rev.startsWith("pending:");
    const pick = (inPending ? h.pending[0] : null) || newest;
    if (pick && pick.id !== S.rev) { S.rev = pick.id; S.base = pick.base; changed = true; }
  }
  if (S.base && !revById(S.base)) { S.base = revById(S.rev)?.base || null; changed = true; }
  renderHistory();
  renderSelects();
  if (S.live.on && h.live?.available) liveStart();
  renderLiveSwitch();
  renderPull();
  renderActions();
  if (fromEvent) {
    setStatus("updated " + new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }), "live flash");
    setTimeout(() => setStatus("live", "live"), 2500);
  }
  if (changed || !S.view) await loadView();
  else if (fromEvent && newest && newest.id !== prevNewest && !S.follow) setStatus("new revision in History", "live flash");
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
      const info = revInfo(r);
      const b = document.createElement("button");
      b.type = "button";
      b.className = `hrev k-${r.kind}` + (r.id === S.rev ? " sel" : "") + (r.id === S.base ? " base" : "") + (r.current ? " current" : "");
      b.setAttribute("aria-current", r.id === S.rev ? "true" : "false");
      b.title = `${r.label}\n${r.detail}\n${new Date(r.time * 1000).toLocaleString()}\n\nClick: view it, compared with the revision before\nShift+click: compare the viewed revision with this one`;
      const pills = [];
      if (r.kind === "pending" && r.current) pills.push('<span class="pill pend">would be deployed</span>');
      if (r.kind !== "pending" && r.current) pills.push('<span class="pill cur">on disk</span>');
      if (r.kind === "live") pills.push('<span class="pill new">not in git</span>');
      if (r.id === S.base) pills.push('<span class="pill">compared with</span>');
      b.innerHTML = `<span class="node"></span><span class="l1"><span class="lbl">${esc(info.title)}</span></span>
        <span class="dt">${esc(info.sub)}</span>${pills.length ? `<span class="tags">${pills.join("")}</span>` : ""}`;
      b.onclick = (ev) => {
        if (ev.shiftKey) { if (r.id !== S.rev) S.base = r.id; }
        else { S.rev = r.id; S.base = r.base; }
        S.follow = !ev.shiftKey && r.id === list[0].id ? S.follow : false;
        $("chk-follow").checked = S.follow;
        renderHistory(); renderSelects(); loadView(); writeHash(true);
      };
      box.appendChild(b);
    }
  };
  if (S.hist.pending.length) { sec("Pending deployment", S.hist.pending); sec("Deployed history", S.hist.history); }
  else sec(null, S.hist.history);
  if (!S.hist.history.length) box.insertAdjacentHTML("beforeend", '<div class="empty">No revisions yet.</div>');
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
  const seq = ++S.loadSeq;
  showMsg("Loading…");
  let v;
  try {
    v = await api(`/api/view?rev=${encodeURIComponent(S.rev)}&base=${encodeURIComponent(S.base || "")}`);
  } catch (e) {
    if (seq === S.loadSeq) showMsg("Cannot show this revision: " + e.message);
    return;
  }
  if (seq !== S.loadSeq) return;
  S.view = v;
  S.objPage = new Map();
  S.pageOf = new Map(v.pages.map((p) => [p.uuid, p]));
  for (const p of v.pages) for (const o of p.objects) if (!o.moved_to && (!S.objPage.has(o.uuid) || o.status !== "removed")) S.objPage.set(o.uuid, p.uuid);
  $("doc-title").textContent = v.title || "Loxone config";
  $("doc-title").title = v.title || "";
  document.title = `${v.title || "Loxone config"} — vislox`;
  const h = S.hash; S.hash = null;
  if (h?.page && S.pageOf.has(h.page)) S.page = h.page;
  if (!S.pageOf.has(S.page)) S.page = (v.pages.find((p) => p.changes > 0) || v.pages[0])?.uuid || null;
  if (h?.sel && v.objects[h.sel]) S.sel = h.sel;
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
  if (S.sel) applyFocus();
  writeHash(false);
}
function renderLegend() {
  $("legend").classList.toggle("diff", !!(S.view?.compared && S.marks));
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
$("tab-list").addEventListener("keydown", (e) => {
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home" && e.key !== "End") return;
  e.preventDefault();
  const tabs = [...$("tab-list").querySelectorAll(".tab")];
  let i = tabs.indexOf(document.activeElement);
  i = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : Math.max(0, Math.min(tabs.length - 1, i + (e.key === "ArrowRight" ? 1 : -1)));
  gotoPage(tabs[i].dataset.uuid);
  $("tab-list").querySelector(`[data-uuid="${CSS.escape(tabs[i].dataset.uuid)}"]`)?.focus();
});
function gotoPage(uuid, opts) {
  if (!uuid || !S.pageOf.has(uuid)) return;
  if (S.page !== uuid) {
    S.page = uuid;
    renderTabs(); renderPage();
    if (S.sel) applyFocus();
    writeHash(true);
  }
  if (opts && opts.focus) focusObj(opts.focus, opts.zoom);
}
function stepPage(d) {
  const pages = visiblePages();
  const i = pages.findIndex((p) => p.uuid === S.page);
  const j = Math.max(0, Math.min(pages.length - 1, i + d));
  if (pages[j]) gotoPage(pages[j].uuid);
}
document.querySelectorAll(".tab-nav button").forEach((b) => (b.onclick = () => stepPage(b.dataset.nav === "next" ? 1 : -1)));
$("chk-only-changed").onchange = (e) => { S.onlyChanged = e.target.checked; renderTabs(); };
$("btn-pages").onclick = () => openPalette("");

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
svg.addEventListener("wheel", (e) => {
  e.preventDefault();
  const r = svg.getBoundingClientRect();
  if (e.ctrlKey || !e.shiftKey) zoomAt(Math.pow(1.0018, -e.deltaY), e.clientX - r.left, e.clientY - r.top);
  else pan(-e.deltaY, 0);
}, { passive: false });
let drag = null;
svg.addEventListener("mousedown", (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  drag = { x: e.clientX, y: e.clientY, moved: false };
  svg.classList.add("panning");
});
window.addEventListener("mousemove", (e) => {
  if (!drag) return;
  const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
  if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
  if (drag.moved) { pan(dx, dy); drag.x = e.clientX; drag.y = e.clientY; hideTip(); }
});
window.addEventListener("mouseup", () => { svg.classList.remove("panning"); setTimeout(() => (drag = null), 0); });
svg.addEventListener("click", (e) => {
  if (drag && drag.moved) return;
  if (e.target === svg || e.target.classList.contains("sheet") || e.target.classList.contains("sheet-dots")) select(null);
});
$("btn-fit").onclick = fit;
$("btn-zoom-in").onclick = () => zoomCenter(1.25);
$("btn-zoom-out").onclick = () => zoomCenter(0.8);
$("btn-zoom-pct").onclick = () => { const v = S.vp[S.page]; if (v) zoomCenter(K100 / v.k); };
window.addEventListener("resize", () => { if (S.page && !S.vp[S.page]) fit(); updateMinimap(); });

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
  vpG.innerHTML = "";
  hideTip();
  const p = curPage();
  const marks = S.marks && !!S.view?.compared;
  svg.classList.toggle("marks-on", marks);
  svg.classList.toggle("marks-off", !marks);
  svg.classList.toggle("dim-unchanged", marks && S.dim && !!p && p.changes > 0);
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

  for (const w of p.wires) {
    const pts = w.points.map((q) => q.join(",")).join(" ");
    const line = el("polyline", { class: `wire${w.analog ? " analog" : ""}${w.status ? " d-" + w.status : ""}`, points: pts, "data-f": w.from[0], "data-t": w.to[0], "data-src": w.src }, L.wires);
    const hit = el("polyline", { class: "wire-hit", points: pts }, L.wires);
    const fo = objByUuid.get(w.from[0]), to = objByUuid.get(w.to[0]);
    hoverTip(hit, () => `<b>${esc(fo ? displayTitle(fo) : "")}</b>.${esc(portName(fo, w.from[1]))} → <b>${esc(to ? displayTitle(to) : "")}</b>.${esc(portName(to, w.to[1]))}<br><span class="tm">${w.analog ? "analog" : "digital"}${w.status ? " · " + w.status : ""}</span>${liveTip(w.src)}`);
    hit.addEventListener("mouseenter", () => line.classList.add("hl"));
    hit.addEventListener("mouseleave", () => line.classList.remove("hl"));
    hit.addEventListener("click", (e) => { e.stopPropagation(); if (!drag?.moved) select(w.to[0]); });
  }

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
    const addr = t.iname ? ` · ${t.iname}` : "";
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
  const g = el("g", { class: `${cls}${o.status ? " d-" + o.status : ""}${o.moved_to ? " ghosted" : ""}${o.uuid === S.sel ? " sel" : ""}`, "data-uuid": o.moved_to ? "" : o.uuid }, parent);
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
function objTip(o) {
  const info = S.view.objects[o.uuid] || {};
  const parts = [`<b>${esc(displayTitle(o))}</b> <span class="tm">${esc(prettyType(o.ty))}</span>`];
  if (autoTitle(o.title) && o.title) parts.push(`<span class="tm">${esc(o.title)}</span>`);
  if (info.old_title) parts.push(`renamed from <i>${esc(info.old_title)}</i>`);
  if (o.target) parts.push(`mirrors <b>${esc(o.target.title)}</b>${o.target.page_title ? ` on page “${esc(o.target.page_title)}”` : o.target.path ? ` in ${esc(o.target.path)}` : ""}${o.target.page && o.target.page !== S.page ? "<br><span class='tm'>double-click to go there</span>" : ""}`);
  if (info.room || info.category) parts.push(esc([info.room, info.category].filter(Boolean).join(" · ")));
  if (o.ty === "Text") parts.push(esc(clip(noteText(o), 300)).replace(/\n/g, "<br>"));
  if (o.status) parts.push(`<i>${o.status}</i>`);
  return parts.join("<br>");
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

function drawTag(parent, o, page) {
  const [x, y, w, h] = o.rect;
  const g = wrapObj(parent, o, "tag");
  const wired = L.wired?.get(o.uuid);
  const sideWired = (side) => o.ports.some((q) => q.side === side && wired?.has(q.key));
  // The side the wire leaves from: where it is wired, else what the type says.
  const feedsRight = sideWired("R") || (!sideWired("L") && o.ty === "InputRef");
  // Analog if the wires on its feeding side are; an unwired reference goes by its address (AI, AQ, VI…).
  const fed = o.ports.filter((q) => (q.side === "R") === feedsRight && L.kind.has(o.uuid + "|" + q.key));
  const analog = fed.length ? fed.some((q) => L.kind.get(o.uuid + "|" + q.key)) : /^(A[QI]|VI|VA)/.test(o.target?.iname || "");
  const rx = h / 2, cy = y + h / 2;
  el("rect", { class: "halo", x: x - 50, y: y - 50, width: w + 100, height: h + 100, rx: rx + 50 }, g);
  el("rect", { class: "bar", x, y, width: w, height: h, rx, ry: rx }, g);
  const t = o.target || {};
  const addr = t.iname || (o.ty === "Memory" ? "M" : o.ty === "OutputRefLM" ? "✉" : "");
  const addrW = addr ? Math.max(400, addr.length * 76 + 180) : 0;
  // The address sits in its own pill at the wired end, with the kind chip beside it.
  const ax = feedsRight ? x + w - addrW : x;
  if (addr) {
    el("rect", { class: "addr", x: ax, y, width: addrW, height: h, rx, ry: rx }, g);
    el("text", { class: "addr-t", x: ax + addrW / 2, y: cy + 38, "text-anchor": "middle" }, g, clip(addr, 7));
  }
  const kx = feedsRight ? ax - 100 : ax + addrW + 100;
  drawKind(g, kx, cy, analog);
  const icX = feedsRight ? x + 110 : x + w - 110;
  el("text", { class: "ic-t", x: icX, y: cy + 36, "text-anchor": "middle" }, g, t.page ? "⚙" : "i");
  const offPage = t.page && t.page !== page.uuid;
  const nameW = w - addrW - 220 - 200 - 60;
  const name = clip(o.title + (offPage ? " ↗" : ""), Math.floor(nameW / 60));
  if (feedsRight) el("text", { class: "cell-t", x: x + 210, y: cy + 36 }, g, name);
  else el("text", { class: "cell-t", x: x + w - 210, y: cy + 36, "text-anchor": "end" }, g, name);
  for (const side of ["L", "R"]) {
    const q = o.ports.find((p) => p.side === side && wired?.has(p.key)) || ((side === "R") === feedsRight ? o.ports.find((p) => p.side === side) : null);
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
  el("polyline", { class: `xref-wire${analog ? " analog" : ""}${l.status ? " d-" + l.status : ""}`, "data-f": l.obj, "data-src": l.src, points: `${l.x},${l.y} ${(l.x + wx) / 2},${l.y} ${(l.x + wx) / 2},${y} ${wx},${y}` }, g);
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
$("minimap").addEventListener("mousedown", (e) => { mmDrag = true; minimapGo(e); e.preventDefault(); });
window.addEventListener("mousemove", (e) => { if (mmDrag) minimapGo(e); });
window.addEventListener("mouseup", () => (mmDrag = false));

// ── selection, focus mode & properties ────────────────────────────────
function select(uuid, opts) {
  S.sel = uuid || null;
  vpG.querySelectorAll(".sel").forEach((n) => n.classList.remove("sel"));
  if (uuid) vpG.querySelectorAll(`[data-uuid="${CSS.escape(uuid)}"]`).forEach((n) => n.classList.add("sel"));
  applyFocus();
  renderProps();
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

function renderProps() {
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
  const row = (k, v, cls) => `<div class="pr ${cls || ""}"><div class="k">${esc(k)}</div><div class="v">${v}</div></div>`;

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
  const note = o.ty === "Text" ? o.attrs.find((a) => a.name === "Text") : null;
  if (note) section("Note", [`<div class="note-p${marks && note.changed ? " pr-chg" : ""}">${marks && note.changed ? chg(note.old, note.value, true) : esc(note.value || "")}</div>`]);
  const params = o.ports.filter((q) => q.def != null || q.def_changed || q.inv)
    .map((q) => row(q.name + (q.inv ? " (inverted)" : ""), chg(q.old_def, q.def, q.def_changed), marks && q.def_changed ? "pr-chg" : ""));
  section("Parameters", params, params.length);

  const connRows = (dir) => o.ports.filter((q) => q.conns.some((c) => c.dir === dir)).map((q) => {
    const cs = q.conns.filter((c) => c.dir === dir);
    const add = marks && cs.some((c) => c.status === "added"), rem = marks && cs.some((c) => c.status === "removed");
    return row(q.name, cs.map((c) => `<div class="conn ${marks ? c.status : ""}"><span class="arr">${dir === "in" ? "←" : "→"}</span>${linkTarget(c.target)}</div>`).join(""),
      add && !rem ? "pr-add" : rem && !add ? "pr-rem" : add && rem ? "pr-chg" : "");
  });
  const ins = connRows("in"), outs = connRows("out");
  section("Inputs", ins, ins.length);
  section("Outputs", outs, outs.length);

  if (o.mirrored_by.length) {
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
  details.push(row("UUID", `<span class="mono">${esc(o.uuid)}</span><button type="button" class="copy" data-copy="${esc(o.uuid)}">Copy</button>`));
  section("Details", details);

  const raw = o.attrs.filter((a) => !["Title", "Type", "IName", ...(note ? ["Text"] : [])].includes(a.name) || (a.changed && a !== note)).map((a) => {
    let v;
    if (a.name === "Cl" && a.value && /^\d+,\d+,\d+$/.test(a.value) && !(marks && a.changed)) v = `<span class="swatch" style="background:rgb(${esc(a.value)})"></span> ${esc(a.value)}`;
    else if (marks && a.changed) v = chg(a.old, a.value, true);
    else if (a.value && a.value.length > 160) v = `<span class="long">${esc(a.value)}</span><button type="button" class="more">Show all</button>`;
    else v = val(a.value);
    return row(a.name, v, marks && a.changed ? (a.value == null ? "pr-rem" : "pr-chg") : "");
  });
  const rawChanged = marks && o.attrs.some((a) => a.changed);
  section("Advanced", raw, raw.length);

  const color = o.attrs.find((a) => a.name === "Cl")?.value;
  const facts = [];
  if (o.page) facts.push(`<button type="button" class="chip" data-page="${esc(o.page)}" data-go="${esc(o.uuid)}" title="Show on its page">▦ ${esc(S.pageOf.get(o.page)?.title || "page")}</button>`);
  if (o.room) facts.push(`<span class="chip" title="Room">⌂ ${esc(o.room)}</span>`);
  if (o.category) facts.push(`<span class="chip" title="Category">◆ ${esc(o.category)}</span>`);
  if (o.target) facts.push(`<span class="chip" title="Mirrors">↪ ${linkTarget(o.target)}</span>`);
  const title = autoTitle(o.title) ? prettyType(o.ty) : o.title;
  if (note) S.closed.delete("Note");
  const st = marks && o.status ? ` <span class="st ${o.status}">${o.status}</span>` : "";
  const renamed = marks && o.old_title ? `<div class="ph-s"><span class="old">${esc(o.old_title)}</span>→ <span class="new">${esc(o.title)}</span></div>` : "";
  box.innerHTML = `<div class="ph"><div class="ph-top">${color && /^\d+,\d+,\d+$/.test(color) && color !== "0,0,0" ? `<span class="swatch" style="background:rgb(${esc(color)})"></span>` : ""}<span class="ph-t">${esc(title)}</span>${st}</div>
    <div class="ph-s">${esc(prettyType(o.ty))}</div>${renamed}${facts.length ? `<div class="ph-facts">${facts.join("")}</div>` : ""}</div>` +
    sections.map((s) => {
      const open = !S.closed.has(s.name) || (s.name === "Advanced" && rawChanged);
      return `<section><button type="button" class="ps-h" aria-expanded="${open}" data-sec="${esc(s.name)}">${esc(s.name)}${s.count ? `<span class="count">${s.count}</span>` : ""}</button><div class="ps-b"${open ? "" : " hidden"}>${s.rows.join("")}</div></section>`;
    }).join("");
}
$("props").addEventListener("click", (e) => {
  const h = e.target.closest(".ps-h");
  if (h) {
    const open = h.getAttribute("aria-expanded") !== "true";
    h.setAttribute("aria-expanded", open);
    h.nextElementSibling.hidden = !open;
    open ? S.closed.delete(h.dataset.sec) : S.closed.add(h.dataset.sec);
    return;
  }
  const c = e.target.closest(".copy");
  if (c) { copyText(c.dataset.copy).then(() => { c.textContent = "Copied"; setTimeout(() => (c.textContent = "Copy"), 1200); }); return; }
  const m = e.target.closest(".more");
  if (m) { m.previousElementSibling.classList.add("open"); m.remove(); return; }
  const a = e.target.closest("[data-go]");
  if (!a) return;
  if (a.dataset.page) gotoPage(a.dataset.page, { focus: a.dataset.go });
  select(a.dataset.go);
});

// ── tree ───────────────────────────────────────────────────────────────
const ICON = { Pages: ["#4f7fc4", "▦"], Page: ["#4f7fc4", "P"], Refs: ["#8a908a", "⇄"], Place: ["#8b6cc4", "R"], Category: ["#b5891f", "C"],
  VirtualIn: ["#0f8fb0", "V"], VirtualOut: ["#0f8fb0", "V"], InputRef: ["#8a908a", "→"], OutputRef: ["#8a908a", "←"], Memory: ["#6b726b", "M"], User: ["#6b726b", "U"] };
function icon(n) {
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
    const num = n.ty === "Page" || n.ty === "Refs" ? `<span class="tn-n">${n.ty === "Page" ? treeChildren(n).reduce((a, c) => a + (c.synthetic ? 0 : 1), 0) : n.children.length}</span>` : "";
    return `<div class="tn-row ${marks ? n.status : ""}${n.uuid === S.sel ? " sel" : ""}${n.uuid === S.treeFocus ? " kfocus" : ""}" role="treeitem" aria-level="${depth + 1}"${kids ? ` aria-expanded="${open}"` : ""} aria-selected="${n.uuid === S.sel}" data-uuid="${esc(n.uuid)}" style="padding-left:${4 + depth * 14}px">` +
      `<span class="tn-tw${open ? " open" : ""}" aria-hidden="true">${kids ? "▶" : ""}</span>${icon(n)}<span class="tn-t">${esc(name)}</span>${ty ? `<span class="tn-ty">${esc(ty)}</span>` : ""}${badge}${num}</div>`;
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
$("tree").addEventListener("click", (e) => {
  const row = e.target.closest(".tn-row");
  if (!row) return;
  const uuid = row.dataset.uuid;
  S.treeFocus = uuid;
  if (e.target.closest(".tn-tw") && row.hasAttribute("aria-expanded")) { toggleNode(uuid); return; }
  activateNode(uuid);
});
$("tree").addEventListener("dblclick", (e) => {
  const row = e.target.closest(".tn-row");
  if (row && row.hasAttribute("aria-expanded")) toggleNode(row.dataset.uuid);
});
$("tree").addEventListener("keydown", (e) => {
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
});
$("tree").addEventListener("focus", () => {
  if (!S.treeFocus) { const r = $("tree").querySelector(".tn-row"); if (r) { S.treeFocus = r.dataset.uuid; r.classList.add("kfocus"); } }
});
$("tree-filter").addEventListener("input", renderTree);

// ── changes ────────────────────────────────────────────────────────────
const KIND = { added: ["+", "added"], removed: ["−", "removed"], renamed: ["~", "renamed"], param: ["~", "parameters"], attr: ["~", "settings"], "wire+": ["+", "wires added"], "wire-": ["−", "wires removed"], moved: ["↔", "moved"] };
function shownChanges() { return (S.view?.changes || []).filter((c) => !S.chgOff.has(c.kind)); }
function renderChanges() {
  const v = S.view, box = $("changes"), filt = $("chg-filter");
  box.innerHTML = ""; filt.innerHTML = "";
  const compact = !v.compared || !v.changes.length;
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
    return;
  }
  $("chg-count").textContent = v.changes.length;
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
  if (!v.changes.length) { box.innerHTML = '<div class="empty">No differences between these revisions.</div>'; return; }
  const list = shownChanges().slice(0, 2000);
  list.forEach((c, i) => {
    const d = document.createElement("button");
    d.type = "button";
    d.className = "chg" + (i === S.chgIdx ? " cur" : "");
    d.setAttribute("role", "listitem");
    const where = c.page ? S.pageOf.get(c.page)?.title : c.obj ? v.objects[c.obj]?.path : "";
    d.innerHTML = `<span class="k ${esc(c.kind)}" aria-label="${esc(KIND[c.kind]?.[1] || c.kind)}">${KIND[c.kind]?.[0] || "~"}</span><span class="x"><span class="t">${esc(c.text)}</span>${where ? `<span class="p">${esc(where)}</span>` : ""}</span>`;
    d.onclick = () => gotoChange(i);
    box.appendChild(d);
  });
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
$("chg-prev").onclick = () => gotoChange(S.chgIdx < 0 ? -1 : S.chgIdx - 1);
$("chg-next").onclick = () => gotoChange(S.chgIdx + 1);

// ── command palette ────────────────────────────────────────────────────
const pal = { items: [], idx: 0 };
function openPalette(q) {
  if (!S.view) return;
  $("palette").classList.remove("hidden");
  const inp = $("pal-input");
  inp.value = q || "";
  renderPalette();
  inp.focus();
}
function closePalette() { $("palette").classList.add("hidden"); }
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
    blocks.push({ kind: "block", title: displayTitle(o), meta: `${prettyType(o.ty)} · ${p.title}`, page: p.uuid, uuid: o.uuid });
  }
  let items;
  if (!q) items = [{ sec: "Pages" }, ...pages];
  else {
    const score = (it) => { const m = fuzzy(q, it.title); return m ? { ...it, score: m.score + (it.kind === "page" ? 50 : 0), hits: m.hits } : null; };
    const ps = pages.map(score).filter(Boolean).sort((a, b) => b.score - a.score).slice(0, 12);
    const bs = blocks.map(score).filter(Boolean).sort((a, b) => b.score - a.score).slice(0, 40);
    items = [...(ps.length ? [{ sec: "Pages" }, ...ps] : []), ...(bs.length ? [{ sec: "Blocks" }, ...bs] : [])];
  }
  pal.items = items;
  pal.idx = items.findIndex((x) => !x.sec);
  const ul = $("pal-list");
  ul.innerHTML = items.length ? items.map((it, i) => it.sec
    ? `<li class="sec" role="presentation">${esc(it.sec)}</li>`
    : `<li role="option" id="pal-${i}" data-i="${i}" aria-selected="${i === pal.idx}"><span class="pk">${it.kind}</span><span class="pt">${mark(it.title, it.hits)}</span><span class="pm">${esc(it.meta)}</span></li>`).join("")
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
  closePalette();
  if (it.kind === "page") { gotoPage(it.page); select(null, { noReveal: true }); }
  else { gotoPage(it.page, { focus: it.uuid }); select(it.uuid); }
  svg.focus({ preventScroll: true });
}
$("pal-input").addEventListener("input", renderPalette);
$("pal-input").addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown") { palMove(1); e.preventDefault(); }
  else if (e.key === "ArrowUp") { palMove(-1); e.preventDefault(); }
  else if (e.key === "Enter") { palOpen(pal.idx); e.preventDefault(); }
  else if (e.key === "Escape") { closePalette(); e.preventDefault(); }
});
$("pal-list").addEventListener("click", (e) => { const li = e.target.closest("[data-i]"); if (li) palOpen(+li.dataset.i); });
$("palette").addEventListener("mousedown", (e) => { if (e.target.id === "palette") closePalette(); });
$("btn-search").onclick = () => openPalette("");

// ── help ───────────────────────────────────────────────────────────────
function openHelp() { $("help").classList.remove("hidden"); $("help-close").focus(); }
function closeHelp() { $("help").classList.add("hidden"); }
$("btn-help").onclick = openHelp;
$("help-close").onclick = closeHelp;
$("help").addEventListener("mousedown", (e) => { if (e.target.id === "help") closeHelp(); });

// ── controls ───────────────────────────────────────────────────────────
$("sel-rev").onchange = (e) => {
  S.rev = e.target.value;
  S.base = revById(S.rev)?.base || null;
  S.follow = false; $("chk-follow").checked = false;
  renderHistory(); renderSelects(); loadView(); writeHash(true);
};
$("sel-base").onchange = (e) => { S.base = e.target.value || null; S.follow = false; $("chk-follow").checked = false; renderHistory(); loadView(); writeHash(true); };
$("chk-changes").onchange = (e) => {
  S.marks = e.target.checked;
  if (!S.view) return;
  renderLegend(); renderTabs(); renderTree(); renderPage(); renderProps(); applyFocus();
};
$("chk-live").onchange = (e) => setLive(e.target.checked);
$("chk-live-dim").checked = S.live.dim;
$("chk-live-dim").onchange = (e) => { S.live.dim = e.target.checked; setPref("liveDim", S.live.dim); renderLiveSwitch(); };
$("chk-dim").onchange = (e) => { S.dim = e.target.checked; renderPage(); applyFocus(); };
$("chk-follow").onchange = (e) => { S.follow = e.target.checked; if (S.follow) { S.rev = null; loadHistory(); } writeHash(false); };

window.addEventListener("keydown", (e) => {
  if (!$("palette").classList.contains("hidden") || !$("help").classList.contains("hidden")) {
    if (e.key === "Escape") { closePalette(); closeHelp(); }
    return;
  }
  if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) { e.preventDefault(); openPalette(""); return; }
  if ((e.ctrlKey || e.metaKey) && e.key === "f") { e.preventDefault(); $("tree-filter").focus(); $("tree-filter").select(); return; }
  const tag = e.target.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") { if (e.key === "Escape") e.target.blur(); return; }
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const inTree = e.target.id === "tree", inTabs = e.target.closest?.("#tab-list");
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
});

for (const [sid, a, b] of [["split-left", "tree-pane", "props-pane"], ["split-right", "hist-pane", "chg-pane"]]) {
  $(sid).addEventListener("mousedown", (e) => {
    e.preventDefault();
    const A = $(a), B = $(b), total = A.offsetHeight + B.offsetHeight, y0 = e.clientY, h0 = A.offsetHeight;
    const mv = (ev) => {
      const h = Math.min(total - 72, Math.max(72, h0 + ev.clientY - y0));
      A.style.flex = `0 0 ${h}px`; B.style.flex = `1 1 ${total - h}px`;
    };
    const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", mv);
    window.addEventListener("mouseup", up);
  });
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
    if (j.committed) {
      pullMsg("Pulled: the Miniserver's config is committed as a new backup.");
      await loadHistory(true);
    } else pullMsg("Up to date: the Miniserver runs the config of the last backup.", "info");
  } catch (e) {
    pullMsg(e.message, "error");
  } finally {
    pulling = false; renderPull(); loadMs();
  }
}
$("btn-pull").onclick = pull;
$("pull-msg").onclick = () => pullMsg("");

// ── the Miniserver: status chip and preflight ──────────────────────────
// The server checks the Miniserver lox talks to (its web port, FTP, the unauthenticated
// jdev/cfg/apiKey, lox's token) and refuses Pull, Apply and Sync when they could not work.
S.ms = null;
// What the last check found against an action. It only warns: the server checks again when the
// button is clicked, so a cause that has cleared does not keep the button off.
function msBlocked(what) { return S.ms?.available ? S.ms.blocked?.[what] || "" : ""; }
function msWarn(what) { const b = msBlocked(what); return b ? `\n\n⚠ The last check found: ${b}. It is checked again when you click.` : ""; }
async function loadMs(fresh) {
  try { S.ms = await api("/api/miniserver" + (fresh ? "?fresh=1" : "")); } catch { return; }
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
    : m.reachable ? [m.serial, m.firmware].filter(Boolean).join(" · ") || "Miniserver" : "Miniserver unreachable";
  chip.title = why.length ? `Miniserver: ${why.join("\n")}`
    : `Miniserver ${m.serial || ""} at ${m.host}` + (m.warnings.length ? `\n${m.warnings.join("\n")}` : "");
  const tls = { on: "available", none: "none", expired: "certificate expired", unknown: "unknown" }[m.tls] || "unknown";
  const tok = m.tokenValidUntil ? new Date(m.tokenValidUntil * 1000).toLocaleString([], { ...MONTH, year: "numeric", hour: "2-digit", minute: "2-digit" }) + (m.tokenValidUntil * 1000 < Date.now() ? " (expired)" : "") : "no token yet";
  const row = (k, v) => v == null || v === "" ? "" : `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`;
  $("ms-body").innerHTML = `<h3>Miniserver</h3><dl>
      ${row("Address", m.host)}${row("Serial", m.serial)}
      ${m.expected ? row("Config belongs to", `${m.expected.serial} (${m.expected.from})` + ({ confirmed: " ✓", wrong: " ✗ not this one" }[m.identity] || "")) : ""}
      ${row("Firmware", m.firmware)}
      ${row("Deployed config", m.config?.deployed && `Loxone Config ${m.config.deployed}`)}
      ${row("Pending config", m.config?.pending && `Loxone Config ${m.config.pending}`)}
      ${m.reachable ? row("Connection", m.local === false ? "remote" : m.local ? "local" : "unknown") + row("TLS", tls)
        + row("Live-update slot", m.eventSlots == null ? "" : m.eventSlots ? "free" : "none free") : ""}
      ${row("lox's token valid until", tok)}</dl>
    ${m.checks.map((c) => `<div class="chk ${c.ok ? "ok" : "bad"}"><span class="dot" aria-hidden="true"></span><span><b>${esc(c.name)}</b> <span class="d">${esc(c.ok ? (c.detail === "answers" ? "answers" : "answers: " + c.detail) : c.detail)}</span></span></div>`).join("")}
    ${m.discovered?.length ? `<div class="chk"><span><b>On the local network</b> <span class="d">${m.discovered.map((d) => esc(`${d.serial} at ${d.ip} (${d.firmware})`)).join(", ")}</span></span></div>` : ""}
    ${m.discoveryNote ? `<div class="chk bad"><span class="dot" aria-hidden="true"></span><span><b>Discovery</b> <span class="d">${esc(m.discoveryNote)}</span></span></div>` : ""}
    ${why.map((y) => `<div class="blocked">${esc(blocked.filter((w) => m.blocked[w] === y).map((w) => w[0].toUpperCase() + w.slice(1)).join(", "))} would fail: ${esc(y)}</div>`).join("")}
    ${m.warnings.length ? `<ul class="warns">${m.warnings.map((w) => `<li>${esc(w)}</li>`).join("")}</ul>` : ""}`;
  $("ms-when").textContent = "Checked " + new Date(m.checked * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
function closeMsPop() { $("ms-pop").classList.add("hidden"); $("ms-chip").setAttribute("aria-expanded", "false"); }
$("ms-chip").onclick = () => {
  const open = $("ms-pop").classList.toggle("hidden") === false;
  $("ms-chip").setAttribute("aria-expanded", String(open));
  if (open) loadMs();
};
$("ms-recheck").onclick = async () => {
  const b = $("ms-recheck");
  b.disabled = true; b.textContent = "Checking…";
  await loadMs(true);
  b.disabled = false; b.textContent = "Check again";
};
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("ms-pop").classList.contains("hidden")) { closeMsPop(); $("ms-chip").focus(); e.stopPropagation(); }
}, true);
document.addEventListener("click", (e) => { if (!e.target.closest(".ms-wrap")) closeMsPop(); });

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
    headers: { "Content-Type": "application/json", "X-Vislox-Token": t },
    body: JSON.stringify(body || {}),
  });
  const j = await r.json().catch(() => ({}));
  if (r.status === 401) try { localStorage.removeItem("vislox.token"); } catch { /* private mode */ }
  if (j.preflight || /apply|sync/.test(path)) loadMs();
  if (!r.ok) { const e = new Error(j.error || r.statusText); e.data = j; throw e; }
  return j;
}
function deployMsg(text, error) {
  const m = $("deploy-msg");
  m.textContent = text || "";
  m.className = "deploy-msg" + (error ? " error" : "") + (text ? "" : " hidden");
}
/** The newest compile, if it differs from what is deployed. */
function pendingChange() {
  const p = S.hist?.pending?.[0];
  const base = p && revById(p.base);
  return p && base && p.blob !== base.blob ? p : null;
}
function renderActions() {
  const a = S.hist?.actions;
  $("deploy-pane").classList.toggle("hidden", !a?.shown);
  if (!a?.shown) return;
  const busy = !!a.running;
  const pend = pendingChange();
  $("deploy-pend").classList.toggle("idle", !pend && !a.running);
  $("deploy-pend").textContent = a.running === "Apply" ? "Deploying to the Miniserver… (download, check, upload, restart, verify)"
    : a.running ? `${a.running}…` : pend ? `Compiled changes are pending (${fmtWhen(pend.time)}). Review them, then apply.`
    : !a.apply.available ? `Apply is off: ${a.apply.reason}.`
    : "Nothing pending. Edit the project's module and compile it (lox logic …); the result shows here for review.";
  $("deploy-row").classList.toggle("hidden", !pend && !a.running);
  $("btn-apply").disabled = !a.apply.available || busy || !pend;
  const sync = $("btn-sync");
  sync.disabled = !a.apply.available || busy || !!pend;
  sync.classList.toggle("busy", a.running === "Sync");
  sync.querySelector(".lbl").textContent = a.running === "Sync" ? "Syncing…" : "Sync from Miniserver";
  sync.title = !a.apply.available ? `Cannot sync: ${a.apply.reason}`
    : pend ? "Apply or discard the pending changes first"
    : "Make the Miniserver's running config the project's base (lox config merge), then commit and push" + msWarn("sync");
  $("btn-discard").disabled = !a.apply.available || busy || !pend;
  $("btn-apply").title = !a.apply.available ? `Cannot apply: ${a.apply.reason}`
    : "Deploy to the Miniserver with lox config push, then commit and push to git" + msWarn("apply");
  renderSteps(a);
  const last = a.last;
  if (last && last.at !== renderActions.shownAt) {
    renderActions.shownAt = last.at;
    const head = { apply: ["Applied.\n", "Apply failed.\n"], sync: ["", "Sync failed.\n"], discard: ["", "Discard failed.\n"] }[last.kind] || ["", ""];
    deployMsg(head[last.ok ? 0 : 1] + last.message, !last.ok);
  }
}
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
  if (a.running === "Apply") renderSteps.tick = setTimeout(() => renderSteps(S.hist?.actions || a), 1000);
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
  renderDeployed();
}
/** The changed blocks of the deployed revision, each with a link and its first live value. */
function renderDeployed() {
  const box = $("deploy-new"), v = S.view;
  const seen = new Set(), rows = [];
  for (const c of v?.compared ? v.changes : []) {
    if (!c.obj || seen.has(c.obj) || c.kind === "removed" || !v.objects[c.obj]) continue;
    seen.add(c.obj);
    rows.push(c);
  }
  box.classList.toggle("hidden", !rows.length);
  if (!rows.length) { box.innerHTML = ""; return; }
  const shown = rows.slice(0, 12).map((c) => {
    const o = v.objects[c.obj];
    const q = o.ports.find((p) => p.conns.some((x) => x.dir === "out")) || o.ports[0];
    const name = autoTitle(o.title) ? prettyType(o.ty) : o.title;
    return `<div class="pr"><span class="k ${esc(c.kind)}">${esc(KIND[c.kind]?.[1] || c.kind)}</span>` +
      `<button type="button" class="go" data-go="${esc(c.obj)}" title="${esc(o.path || "")}">${esc(name)}</button>${q ? lvSpan(q.uuid) : ""}</div>`;
  }).join("");
  box.innerHTML = `<div class="h">Changed by this Apply<button type="button" class="x" aria-label="Close">×</button></div>${shown}` +
    (rows.length > 12 ? `<div class="note">and ${rows.length - 12} more in Changes</div>` : "") +
    '<div class="note">Values fill in as the Miniserver sends them; the canvas marks every change.</div>';
}
$("deploy-new").addEventListener("click", (e) => {
  if (e.target.closest(".x")) { $("deploy-new").classList.add("hidden"); return; }
  const g = e.target.closest("[data-go]");
  if (!g) return;
  const u = g.dataset.go, pg = S.view?.objects[u]?.page;
  if (pg) gotoPage(pg, { focus: u });
  select(u);
});
$("btn-apply").onclick = async () => {
  const pend = pendingChange();
  if (!pend) return;
  if (!window.confirm("Deploy the pending changes to the Miniserver?\n\nlox config push uploads the config and restarts the Miniserver's program (a few seconds), " +
    "checks that it runs exactly the compiled config, then vislox commits and pushes to git.")) return;
  deployMsg("");
  $("deploy-new").classList.add("hidden");
  // lox refuses some pushes until the user allows them; ask, then retry with that allowed.
  const opts = {};
  for (let tries = 0; tries < 3; tries++) {
    try { await act("/api/apply", opts); await showDeployed(); return; } catch (err) {
      const h = err.data?.hints || {};
      if (h.accept_version && !opts.accept_version && window.confirm(
        `This config was saved by a newer Loxone Config (version ${h.accept_version}) than the project last compiled with.\n\n` +
        "Accept that version and apply? Nothing else changes; the project's lockfile records the new version.")) {
        opts.accept_version = h.accept_version; continue;
      }
      if (h.custom_changes && !opts.discard_custom_changes && window.confirm(
        `The Miniserver holds ${h.custom_changes.length} change(s) that are not in its program file ` +
        "(made in the app, the web UI or by Loxone Config without saving):\n\n  " + h.custom_changes.join("\n  ") +
        "\n\nApplying DELETES them. To keep them instead, load the project from the Miniserver in Loxone Config and save it, then apply again." +
        "\n\nDelete them and apply?")) {
        opts.discard_custom_changes = true; continue;
      }
      deployMsg(err.message, true);
      return;
    }
  }
};
$("btn-sync").onclick = async () => {
  deployMsg("");
  try { const j = await act("/api/sync"); deployMsg(j.message); } catch (err) { deployMsg(err.message, true); }
};
$("btn-discard").onclick = async () => {
  if (!window.confirm("Discard the pending changes? The module, lock and compiled config go back to the last commit.")) return;
  deployMsg("");
  try { const j = await act("/api/discard"); deployMsg(j.message); } catch (err) { deployMsg(err.message, true); }
};

// ── live values ────────────────────────────────────────────────────────
// The Miniserver streams a value per state UUID; a wire's source connector
// has the UUID of the state it carries. Logic blocks the app does not see
// stream nothing, so their wires have no value.

/** Live values belong to what runs on the Miniserver: the current, deployed revision. */
function liveHere() {
  const r = revById(S.rev);
  return S.live.on && !!S.live.es && !!r && !!r.current && r.kind !== "pending";
}
/** A connector's value: streamed by the Miniserver, passed on by a reference or a memory flag,
 *  a constant in the config, or computed from a simple gate's inputs (`view.live`).
 *  Entries are `[value, text, label, kind]`; kind is "const" or "computed" for values the
 *  Miniserver did not send. Cached until the next change. */
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
  return `<span class="lv${e ? "" : " none"}${e?.[3] ? " " + e[3] : ""}" data-lv="${esc(u)}"${e?.[2] ? ` title="${esc(e[2])}"` : ""}>${e ? esc(clip(liveText(e), 40)) : "no value"}</span>`;
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
  $("live-note").textContent = on && !here ? "Live values show on the current, deployed revision only"
    : on && st.reason && st.state !== "live" ? (st.detail || "").split("\n")[0] : "";
  svg.classList.toggle("live", here);
  svg.classList.toggle("live-dim", here && S.live.dim);
}

function liveStart() {
  if (S.live.es) return;
  const es = new EventSource("/api/live");
  S.live.es = es;
  S.live.status = { state: "connecting" };
  es.addEventListener("status", (e) => { S.live.status = JSON.parse(e.data); renderLiveSwitch(); });
  es.addEventListener("snapshot", (e) => {
    const d = JSON.parse(e.data);
    S.live.values = new Map(Object.entries(d.values));
    S.live.controls = d.controls || {};
    S.live.globals = d.globals || {};
    lvCache = new Map();
    liveQueue(null);
  });
  es.addEventListener("values", (e) => {
    const d = JSON.parse(e.data);
    for (const k in d) S.live.values.set(k, d[k]);
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
    const txt = e ? clip(liveText(e), 40) : "no value";
    const changed = n.textContent !== txt;
    n.textContent = txt;
    n.className = "lv" + (e ? "" : " none") + (e?.[3] ? " " + e[3] : "");
    n.title = e?.[2] || "";
    if (ch && e && changed) flash(n);
  });
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
  es.onopen = () => { setStatus("live", "live"); if (S.hist) loadHistory(); };
  es.addEventListener("history", () => loadHistory(true));
  es.addEventListener("actions", (e) => { if (S.hist) { S.hist.actions = JSON.parse(e.data); renderActions(); } });
  es.onerror = () => setStatus("disconnected — retrying", "error");
}
S.hash = readHash();
loadHistory().then(connect);
loadMs();
// The chip stays roughly current while the page is open.
setInterval(() => { if (!document.hidden) loadMs(); }, 5 * 60 * 1000);
