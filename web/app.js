"use strict";
// vislox — read-only Loxone Config style viewer.
// Canvas coordinates are the config's own units: one port row is 192.

const ROW = 192, LABEL_W = 2112, SVGNS = "http://www.w3.org/2000/svg";
const $ = (id) => document.getElementById(id);
const S = {
  hist: null, view: null, rev: null, base: null, page: null, sel: null,
  follow: true, marks: true, vp: {}, open: new Set(), chgOff: new Set(),
  objPage: new Map(), loadSeq: 0,
};

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
function clip(s, n) { s = String(s ?? ""); return s.length > n ? s.slice(0, Math.max(1, n - 1)) + "…" : s; }
function prettyType(t) { return String(t || "").replace(/([a-z])([A-Z0-9])/g, "$1 $2"); }
function fmtTime(t) {
  if (!t) return "";
  const d = new Date(t * 1000), now = Date.now() / 1000, ago = now - t;
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  if (ago < 86400 && new Date().getDate() === d.getDate()) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" }) + " " + d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
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
}
function allRevs() { return S.hist ? [...S.hist.pending, ...S.hist.history] : []; }
function revById(id) { return allRevs().find((r) => r.id === id); }
function textColor(rgb) {
  const m = /^(\d+),(\d+),(\d+)$/.exec(rgb || "");
  if (!m) return false;
  const [r, g, b] = m.slice(1).map(Number);
  return 0.299 * r + 0.587 * g + 0.114 * b > 186;
}

// ── history ────────────────────────────────────────────────────────────
async function loadHistory(fromEvent) {
  let h;
  try { h = await api("/api/history"); } catch (e) { setStatus("server unreachable", "error"); return; }
  const prevNewest = S.hist ? (S.hist.pending[0] || S.hist.history[0])?.id : null;
  S.hist = h;
  $("doc-sub").textContent = h.label + (h.error ? " · " + h.error : "");
  const newest = h.pending[0] || h.history[0];
  let changed = false;
  if (!S.rev || !revById(S.rev) || (S.follow && newest && newest.id !== S.rev)) {
    // Stay in the section the user is looking at.
    const inPending = S.rev && S.rev.startsWith("pending:");
    const pick = (inPending ? h.pending[0] : null) || newest;
    if (pick && pick.id !== S.rev) { S.rev = pick.id; S.base = pick.base; changed = true; }
  }
  if (S.base && !revById(S.base)) { S.base = revById(S.rev)?.base || null; changed = true; }
  renderHistory();
  renderSelects();
  if (fromEvent) {
    setStatus("updated " + new Date().toLocaleTimeString(), "live flash");
    setTimeout(() => setStatus("live · watching for changes", "live"), 1500);
  }
  if (changed || !S.view) await loadView();
  else if (fromEvent && newest && newest.id !== prevNewest && !S.follow) {
    setStatus("new revision available", "live flash");
  }
}

function renderHistory() {
  const box = $("hist");
  box.innerHTML = "";
  const sec = (title, list, empty) => {
    const h = document.createElement("div");
    h.className = "hist-sec";
    h.textContent = title;
    box.appendChild(h);
    if (!list.length) {
      const n = document.createElement("div");
      n.className = "none";
      n.textContent = empty;
      box.appendChild(n);
    }
    for (const r of list) {
      const d = document.createElement("div");
      d.className = `hrev k-${r.kind}` + (r.id === S.rev ? " sel" : "") + (r.id === S.base ? " base" : "") + (r.current ? " current" : "");
      d.title = `${r.label}\n${r.detail}\n${new Date(r.time * 1000).toLocaleString()}\n\nClick: view this revision (compared with the one before)\nShift+click: compare the viewed revision with this one`;
      const pills = [];
      if (r.kind === "pending" && r.current) pills.push('<span class="pill pend">would be applied</span>');
      if (r.kind !== "pending" && r.current) pills.push('<span class="pill cur">on disk</span>');
      if (r.kind === "live") pills.push('<span class="pill new">seen live</span>');
      if (r.id === S.base) pills.push('<span class="pill">compared with</span>');
      d.innerHTML = `<span class="node"></span><div class="l1"><span class="lbl">${esc(r.label)}</span><span class="tm">${esc(fmtTime(r.time))}</span></div>
        <div class="dt">${esc(r.detail)}</div>${pills.length ? `<div class="tags">${pills.join("")}</div>` : ""}`;
      d.onclick = (ev) => {
        if (ev.shiftKey) { if (r.id !== S.rev) S.base = r.id; }
        else { S.rev = r.id; S.base = r.base; }
        const newest = list[0];
        S.follow = r.id === newest.id && !ev.shiftKey ? S.follow : false;
        $("chk-follow").checked = S.follow;
        renderHistory(); renderSelects(); loadView();
      };
      box.appendChild(d);
    }
  };
  if (S.hist.pending.length) sec("Pending — compiled, not deployed", S.hist.pending, "");
  sec("History", S.hist.history, "No revisions yet.");
}

function renderSelects() {
  const label = (r) => `${r.kind === "pending" ? "⏳ " : r.kind === "live" ? "● " : ""}${clip(r.label, 46)} · ${fmtTime(r.time)}`;
  const rs = $("sel-rev"), bs = $("sel-base");
  rs.innerHTML = allRevs().map((r) => `<option value="${esc(r.id)}"${r.id === S.rev ? " selected" : ""}>${esc(label(r))}</option>`).join("");
  bs.innerHTML = `<option value="">nothing (plain view)</option>` +
    allRevs().filter((r) => r.id !== S.rev).map((r) => `<option value="${esc(r.id)}"${r.id === S.base ? " selected" : ""}>${esc(label(r))}</option>`).join("");
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
  for (const p of v.pages) for (const o of p.objects) if (!S.objPage.has(o.uuid) || o.status !== "removed") S.objPage.set(o.uuid, p.uuid);
  $("doc-title").textContent = v.title || "Loxone config";
  document.title = `vislox — ${v.title || ""}`;
  if (!v.pages.find((p) => p.uuid === S.page)) {
    const firstChanged = v.pages.find((p) => p.changes > 0);
    S.page = (firstChanged || v.pages[0])?.uuid || null;
  }
  if (S.sel && !v.objects[S.sel]) S.sel = null;
  $("legend").classList.toggle("hidden", !v.compared || !S.marks);
  renderTabs();
  renderTree();
  renderChanges();
  renderPage();
  renderProps();
}

function showMsg(t) {
  const m = $("canvas-msg");
  m.textContent = t || "";
  m.classList.toggle("show", !!t);
}

// ── tabs ───────────────────────────────────────────────────────────────
function renderTabs() {
  const list = $("tab-list");
  list.innerHTML = "";
  for (const p of S.view.pages) {
    const t = document.createElement("div");
    t.className = "tab" + (p.uuid === S.page ? " active" : "") + (p.status === "added" || p.status === "removed" ? " " + p.status : "");
    t.innerHTML = esc(p.title || "(untitled page)") + (S.view.compared && S.marks && p.changes ? ` <span class="badge ${p.status === "removed" ? "removed" : p.status === "added" ? "added" : "changed"}">${p.changes}</span>` : "");
    t.title = p.old_title ? `${p.old_title} → ${p.title}` : p.title;
    t.onclick = () => gotoPage(p.uuid);
    list.appendChild(t);
    if (p.uuid === S.page) setTimeout(() => t.scrollIntoView({ block: "nearest", inline: "nearest" }), 0);
  }
}
function gotoPage(uuid, opts) {
  if (!uuid) return;
  if (S.page !== uuid) { S.page = uuid; renderTabs(); renderPage(); }
  if (opts && opts.focus) focusObj(opts.focus);
}
document.querySelectorAll(".tab-nav button").forEach((b) => {
  b.onclick = () => {
    const pages = S.view?.pages || [];
    if (!pages.length) return;
    const i = pages.findIndex((p) => p.uuid === S.page);
    const j = { first: 0, prev: Math.max(0, i - 1), next: Math.min(pages.length - 1, i + 1), last: pages.length - 1 }[b.dataset.nav];
    gotoPage(pages[j].uuid);
  };
});

// ── canvas ─────────────────────────────────────────────────────────────
const svg = $("canvas"), vpG = $("viewport");
function curPage() { return S.view?.pages.find((p) => p.uuid === S.page); }

function pageExtent(p) {
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity;
  for (const o of p.objects) {
    const [x, y, w, h] = o.rect;
    x1 = Math.min(x1, x); y1 = Math.min(y1, y); x2 = Math.max(x2, x + w); y2 = Math.max(y2, y + h);
  }
  for (const l of p.labels) {
    x1 = Math.min(x1, l.x - LABEL_W - 200); x2 = Math.max(x2, l.x + LABEL_W + 200);
  }
  if (!isFinite(x1)) return p.sheet ? p.sheet : [0, 0, 24000, 16000];
  return [x1, y1, x2 - x1, y2 - y1];
}

function applyVp() {
  const v = S.vp[S.page];
  if (v) vpG.setAttribute("transform", `translate(${v.x},${v.y}) scale(${v.k})`);
}
function fit() {
  const p = curPage();
  if (!p) return;
  const [x, y, w, h] = pageExtent(p);
  const W = svg.clientWidth, H = svg.clientHeight, pad = 40;
  const k = Math.min((W - 2 * pad) / Math.max(w, 1), (H - 2 * pad) / Math.max(h, 1), 0.12);
  S.vp[S.page] = { k, x: (W - w * k) / 2 - x * k, y: Math.max(pad, (H - h * k) / 2) - y * k };
  applyVp();
}
function zoomAt(f, cx, cy) {
  const v = S.vp[S.page];
  if (!v) return;
  const k = Math.min(0.6, Math.max(0.008, v.k * f));
  v.x = cx - (cx - v.x) * (k / v.k);
  v.y = cy - (cy - v.y) * (k / v.k);
  v.k = k;
  applyVp();
}
svg.addEventListener("wheel", (e) => {
  e.preventDefault();
  const r = svg.getBoundingClientRect();
  zoomAt(Math.pow(1.0015, -e.deltaY), e.clientX - r.left, e.clientY - r.top);
}, { passive: false });
let drag = null;
svg.addEventListener("mousedown", (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  drag = { x: e.clientX, y: e.clientY, moved: false };
  svg.classList.add("panning");
});
window.addEventListener("mousemove", (e) => {
  if (!drag) return;
  const v = S.vp[S.page];
  const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
  if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
  if (v && drag.moved) { v.x += dx; v.y += dy; drag.x = e.clientX; drag.y = e.clientY; applyVp(); }
});
window.addEventListener("mouseup", () => { svg.classList.remove("panning"); setTimeout(() => (drag = null), 0); });
svg.addEventListener("click", (e) => {
  if (drag && drag.moved) return;
  if (e.target === svg || e.target.classList.contains("sheet") || e.target.classList.contains("sheet-dots")) select(null);
});
$("btn-fit").onclick = fit;
$("btn-zoom-in").onclick = () => zoomAt(1.25, svg.clientWidth / 2, svg.clientHeight / 2);
$("btn-zoom-out").onclick = () => zoomAt(0.8, svg.clientWidth / 2, svg.clientHeight / 2);
window.addEventListener("resize", () => { if (S.page && !S.vp[S.page]) fit(); });

const tip = $("tooltip");
function showTip(e, html) {
  tip.innerHTML = html;
  tip.classList.remove("hidden");
  tip.style.left = e.clientX + 14 + "px";
  tip.style.top = e.clientY + 14 + "px";
}
function hideTip() { tip.classList.add("hidden"); }
function hoverTip(node, html) {
  node.addEventListener("mousemove", (e) => showTip(e, typeof html === "function" ? html() : html));
  node.addEventListener("mouseleave", hideTip);
}

function renderPage() {
  vpG.innerHTML = "";
  const p = curPage();
  svg.classList.toggle("marks-on", S.marks && !!S.view?.compared);
  svg.classList.toggle("marks-off", !(S.marks && S.view?.compared));
  if (!p) { showMsg(S.view && !S.view.pages.length ? "This config has no logic pages." : ""); return; }
  showMsg(p.objects.length ? "" : "This page is empty.");

  // Sheet with the dot grid.
  const ext = pageExtent(p);
  const sh = p.sheet || [0, 0, 23808, 16032];
  const sx1 = Math.min(sh[0], ext[0] - 480), sy1 = Math.min(sh[1], ext[1] - 480);
  const sx2 = Math.max(sh[0] + sh[2], ext[0] + ext[2] + 480), sy2 = Math.max(sh[1] + sh[3], ext[1] + ext[3] + 480);
  el("rect", { class: "sheet", x: sx1, y: sy1, width: sx2 - sx1, height: sy2 - sy1 }, vpG);
  el("rect", { class: "sheet-dots", x: sx1, y: sy1, width: sx2 - sx1, height: sy2 - sy1 }, vpG);

  const gGhost = el("g", {}, vpG), gWires = el("g", {}, vpG), gRefs = el("g", {}, vpG), gObjs = el("g", {}, vpG), gBadges = el("g", {}, vpG);
  const objByUuid = new Map(p.objects.map((o) => [o.uuid, o]));

  // Wires
  for (const w of p.wires) {
    const pts = w.points.map((q) => q.join(",")).join(" ");
    const cls = `wire${w.analog ? " analog" : ""}${w.status ? " d-" + w.status : ""}`;
    const line = el("polyline", { class: cls, points: pts }, gWires);
    const hit = el("polyline", { class: "wire-hit", points: pts }, gWires);
    const fo = objByUuid.get(w.from[0]), to = objByUuid.get(w.to[0]);
    hoverTip(hit, () => `<b>${esc(fo?.title)}</b>.${esc(portName(fo, w.from[1]))} → <b>${esc(to?.title)}</b>.${esc(portName(to, w.to[1]))}${w.status ? `<br><i>${w.status}</i>` : ""}`);
    hit.addEventListener("mouseenter", () => line.classList.add("hl"));
    hit.addEventListener("mouseleave", () => line.classList.remove("hl"));
    hit.addEventListener("click", (e) => { e.stopPropagation(); if (!drag?.moved) select(w.to[0]); });
  }

  // Off-page connections, stacked per port.
  const stack = new Map();
  for (const l of p.labels) {
    const key = l.obj + "|" + l.port;
    const n = stack.get(key) || 0;
    stack.set(key, n + 1);
    drawXref(gRefs, l, n);
  }

  for (const o of p.objects) {
    if (o.shape === "tag") drawTag(gObjs, o, p);
    else drawBlock(gObjs, o);
    if (o.status && S.view.compared) drawBadge(gBadges, o);
    if (o.moved_to) {
      const g = el("g", { class: "xref ghost-note" }, gBadges);
      const t = el("text", { class: "pg", x: o.rect[0], y: o.rect[1] + o.rect[3] + 150 }, g, `→ moved to page “${o.moved_to.page_title}”`);
      t.style.fontSize = "120px";
      g.addEventListener("click", (e) => { e.stopPropagation(); followTarget(o.moved_to); });
    }
    if (o.old_rect && S.view.compared) {
      const [x, y, w, h] = o.old_rect;
      el("rect", { class: "ghost", x, y, width: w, height: h, rx: 40 }, gGhost);
      el("line", { class: "ghost-line ghost", x1: x + w / 2, y1: y + h / 2, x2: o.rect[0] + o.rect[2] / 2, y2: o.rect[1] + o.rect[3] / 2 }, gGhost);
    }
  }
  if (!S.vp[S.page]) fit(); else applyVp();
}

function portName(o, key) {
  return o?.ports.find((q) => q.key === key)?.name || key;
}

function wrapObj(parent, o, cls) {
  const g = el("g", { class: `${cls}${o.status ? " d-" + o.status : ""}${o.moved_to ? " ghosted" : ""}${o.uuid === S.sel ? " sel" : ""}`, "data-uuid": o.moved_to ? "" : o.uuid }, parent);
  g.addEventListener("click", (e) => { e.stopPropagation(); if (!drag?.moved) select(o.uuid); });
  g.addEventListener("dblclick", (e) => {
    e.stopPropagation();
    if (o.target?.page && o.target.page !== S.page) gotoPage(o.target.page, { focus: o.target.uuid });
  });
  hoverTip(g, () => objTip(o));
  return g;
}
function objTip(o) {
  const info = S.view.objects[o.uuid] || {};
  const parts = [`<b>${esc(o.title)}</b> · ${esc(prettyType(o.ty))}`];
  if (info.old_title) parts.push(`renamed from <i>${esc(info.old_title)}</i>`);
  if (o.target) parts.push(`mirrors <b>${esc(o.target.title)}</b>${o.target.page_title ? ` on page “${esc(o.target.page_title)}”` : o.target.path ? ` in ${esc(o.target.path)}` : ""}${o.target.page && o.target.page !== S.page ? "<br>double-click to go there" : ""}`);
  if (info.room || info.category) parts.push(esc([info.room, info.category].filter(Boolean).join(" · ")));
  if (o.status) parts.push(`<i>${o.status}</i>`);
  return parts.join("<br>");
}

function drawPins(g, o, inner) {
  for (const q of o.ports) {
    const analog = /^A[QI]/.test(q.key);
    if (q.side === "L") {
      el("polygon", { class: `pin-in${analog ? " analog" : ""}${q.inv ? " inv" : ""}`, points: `${q.x - 130},${q.y - 60} ${q.x},${q.y - 60} ${q.x},${q.y + 60} ${q.x - 130},${q.y + 60} ${q.x - 70},${q.y}` }, g);
    } else {
      el("polygon", { class: "pin-out", points: `${q.x},${q.y - 62} ${q.x + 120},${q.y} ${q.x},${q.y + 62}` }, g);
    }
    if (q.status) el("circle", { class: "pin-chg", cx: q.side === "L" ? q.x + 40 : q.x - 40, cy: q.y, r: 45 }, g);
    if (inner) {
      const twin = o.ports.some((r) => r !== q && r.side !== q.side && Math.abs(r.y - q.y) < 10);
      const n = twin ? Math.floor(inner * 0.55) : inner;
      const t = el("text", { class: "port-t", x: q.side === "L" ? q.x + 80 : q.x - 80, y: q.y + 40, "text-anchor": q.side === "L" ? "start" : "end" }, g, clip(q.name, n));
      const tt = el("title", {}, t, `${q.name} (${q.key})${q.def != null ? ` = ${q.def}` : ""}`);
      void tt;
    }
  }
}

function drawBlock(parent, o) {
  const [x, y, w, h] = o.rect;
  const g = wrapObj(parent, o, "blk");
  const color = o.color ? `rgb(${o.color})` : "#69c350";
  const large = o.shape === "large";
  const hh = large ? 240 : 288;
  el("rect", { class: "halo", x: x - 50, y: y - 50, width: w + 100, height: h + 100, rx: 60 }, g);
  el("rect", { class: "body", x, y, width: w, height: h, stroke: color }, g);
  el("rect", { x, y, width: w, height: hh, fill: color }, g);
  const dark = textColor(o.color);
  if (large) {
    el("text", { class: "hdr-t" + (dark ? " dark" : ""), x: x + 70, y: y + 170 }, g, clip(o.title, Math.floor((w - 420) / 72)));
    // gear and info buttons
    for (const [i, s] of [[0, "i"], [1, "⚙"]]) {
      const cx = x + w - 120 - i * 190;
      el("circle", { class: "icon", cx, cy: y + 120, r: 78 }, g);
      el("text", { class: "icon-t", x: cx, y: y + 158, "text-anchor": "middle" }, g, s);
    }
    el("rect", { class: "sub", x: x + 1, y: y + hh, width: w - 2, height: 240 }, g);
    el("text", { class: "sub-t", x: x + w / 2, y: y + hh + 170, "text-anchor": "middle" }, g, clip(prettyType(o.ty), Math.floor(w / 62)));
    const info = S.view.objects[o.uuid] || {};
    const meta = [info.room, info.category].filter(Boolean);
    meta.forEach((m, i) => {
      if (480 + ROW * (i + 1) <= h) el("text", { class: "meta-t", x: x + w / 2, y: y + 480 + ROW * i + 136, "text-anchor": "middle" }, g, clip(m, 16));
    });
  } else {
    const label = o.title && o.title !== o.ty ? o.title : prettyType(o.ty);
    el("text", { class: "hdr-t" + (dark ? " dark" : ""), x: x + w / 2, y: y + 196, "text-anchor": "middle", style: "font-size:112px" }, g, clip(label, Math.floor(w / 66)));
  }
  drawPins(g, o, large ? 20 : 8);
}

function drawTag(parent, o, page) {
  const [x, y, w, h] = o.rect;
  const g = wrapObj(parent, o, "tag");
  const feedsRight = o.ports.some((q) => q.side === "R");
  const rx = h / 2;
  el("rect", { class: "halo", x: x - 50, y: y - 50, width: w + 100, height: h + 100, rx: rx + 50 }, g);
  el("rect", { class: "bar", x, y, width: w, height: h, rx, ry: rx }, g);
  const t = o.target || {};
  const addr = t.iname || (o.ty === "Memory" ? "M" : o.ty === "OutputRefLM" ? "✉" : "");
  const addrW = addr ? Math.max(380, addr.length * 80 + 120) : 0;
  const icX = feedsRight ? x + 110 : x + w - 110;
  // address cell next to the block side
  if (addr) {
    const ax = feedsRight ? x + w - addrW : x;
    el("line", { x1: feedsRight ? ax : ax + addrW, y1: y, x2: feedsRight ? ax : ax + addrW, y2: y + h, stroke: "#555", "stroke-width": 1, "vector-effect": "non-scaling-stroke" }, g);
    el("text", { class: "addr-t", x: ax + addrW / 2, y: y + 136, "text-anchor": "middle" }, g, clip(addr, 7));
  }
  el("circle", { class: "ic", cx: icX, cy: y + h / 2, r: 62 }, g);
  el("text", { class: "ic-t", x: icX, y: y + h / 2 + 34, "text-anchor": "middle" }, g, t.page ? "⚙" : "i");
  const nameX = feedsRight ? x + 230 : x + addrW + 60;
  const nameW = w - addrW - 290;
  const offPage = t.page && t.page !== page.uuid;
  el("text", { class: "cell-t", x: nameX, y: y + 136 }, g, clip(o.title + (offPage ? " ↗" : ""), Math.floor(nameW / 66)));
  drawPins(g, o, 0);
}

function drawXref(parent, l, n) {
  const left = l.side === "L";
  const y = l.y + n * (ROW + 24);
  const bx = left ? l.x - 150 - LABEL_W : l.x + 150;
  const t = l.target;
  const g = el("g", { class: `xref${l.status ? " d-" + l.status : ""}` }, parent);
  const analog = /^A[QI]/.test(l.port) || /^A[QI]/.test(t.port || "");
  const wx = left ? bx + LABEL_W : bx;
  el("polyline", { class: `xref-wire${analog ? " analog" : ""}${l.status ? " d-" + l.status : ""}`, points: `${l.x},${l.y} ${(l.x + wx) / 2},${l.y} ${(l.x + wx) / 2},${y} ${wx},${y}` }, g);
  el("rect", { class: "bar", x: bx, y: y - ROW / 2, width: LABEL_W, height: ROW, rx: 30 }, g);
  const icon = l.incoming ? "⇥" : "↦";
  el("text", { class: "ic-t", x: bx + 90, y: y + 38, "text-anchor": "middle" }, g, icon);
  const name = `${t.port ? t.port + ": " : ""}${t.title}`;
  el("text", { class: "t", x: bx + 190, y: y + 36 }, g, clip(name, 22));
  el("text", { class: "pg", x: bx + LABEL_W - 60, y: y + 34, "text-anchor": "end" }, g, clip(t.page_title ? "↗ " + t.page_title : t.path ? "⌂ " + (t.path.split(" / ").pop() || "") : "", 12));
  g.addEventListener("click", (e) => { e.stopPropagation(); if (!drag?.moved) followTarget(t); });
  hoverTip(g, () => `${l.incoming ? "Comes from" : "Goes to"} <b>${esc(t.title)}</b>${t.port ? "." + esc(t.port) : ""} <span style="color:#777">(${esc(prettyType(t.ty))})</span><br>${t.page_title ? `on page “${esc(t.page_title)}” — click to go there` : t.path ? `in ${esc(t.path)} — click to select` : ""}${l.status ? `<br><i>${l.status}</i>` : ""}`);
}

function drawBadge(parent, o) {
  const sym = { added: "+", removed: "−", changed: "~", moved: "↔" }[o.status];
  const col = { added: "#1a9f3c", removed: "#d62f2f", changed: "#e08a00", moved: "#2d7fd3" }[o.status];
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

function focusObj(uuid) {
  const p = curPage();
  const o = p?.objects.find((x) => x.uuid === uuid);
  if (!o) return;
  if (!S.vp[S.page]) fit();
  const v = S.vp[S.page];
  v.k = Math.max(v.k, 0.06);
  const [x, y, w, h] = o.rect;
  v.x = svg.clientWidth / 2 - (x + w / 2) * v.k;
  v.y = svg.clientHeight / 2 - (y + h / 2) * v.k;
  applyVp();
  const g = vpG.querySelector(`[data-uuid="${CSS.escape(uuid)}"]`);
  if (g) { g.classList.remove("flash"); void g.getBBox(); g.classList.add("flash"); setTimeout(() => g.classList.remove("flash"), 2000); }
}

// ── selection & properties ─────────────────────────────────────────────
function select(uuid) {
  S.sel = uuid;
  vpG.querySelectorAll(".sel").forEach((n) => n.classList.remove("sel"));
  if (uuid) vpG.querySelectorAll(`[data-uuid="${CSS.escape(uuid)}"]`).forEach((n) => n.classList.add("sel"));
  renderProps();
  revealInTree(uuid);
}

function linkTarget(t, extra) {
  const where = t.page_title ? ` <span style="color:#888">(${esc(t.page_title)})</span>` : t.path ? ` <span style="color:#888">(${esc(t.path)})</span>` : "";
  return `<span class="lnk" data-go="${esc(t.uuid)}" data-page="${esc(t.page || "")}">${esc(t.title)}${t.port ? "." + esc(t.port) : ""}</span>${where}${extra || ""}`;
}

function renderProps() {
  const box = $("props");
  const o = S.sel && S.view?.objects[S.sel];
  if (!o) {
    $("props-title").textContent = "Properties";
    box.innerHTML = '<div class="empty">Select a block to see its configuration.</div>';
    return;
  }
  $("props-title").textContent = `Properties (${prettyType(o.ty)})`;
  const marks = S.marks && S.view.compared;
  const st = marks && o.status ? `<span class="st ${o.status}">${o.status}</span>` : "";
  const rows = [];
  const cat = (name) => rows.push(`<tr class="cat" data-cat="${esc(name)}"><td colspan="2">${esc(name)}</td></tr>`);
  const row = (k, v, cls, catName) => rows.push(`<tr class="${cls || ""}" data-in="${esc(catName)}"><td title="${esc(k)}">${esc(k)}</td><td>${v}</td></tr>`);
  const val = (v) => (v == null || v === "" ? '<span style="color:#aaa">—</span>' : `<span class="pgv">${esc(clip(v, 600))}</span>`);
  const chgVal = (oldV, newV, changed) => (marks && changed ? `<span class="old">${esc(clip(oldV ?? "—", 200))}</span><span class="new">${esc(clip(newV ?? "—", 400))}</span>` : val(newV));

  cat("General");
  row("Name", marks && o.old_title ? chgVal(o.old_title, o.title, true) : val(o.title), marks && o.old_title ? "chg" : "", "General");
  row("Type", esc(o.ty), "", "General");
  const iname = o.attrs.find((a) => a.name === "IName");
  if (iname) row("Short name", val(iname.value), "", "General");
  row("UUID", `<span class="mono">${esc(o.uuid)}</span>`, "", "General");
  if (o.page) {
    const pg = S.view.pages.find((p) => p.uuid === o.page);
    row("Page", `<span class="lnk" data-page="${esc(o.page)}" data-go="${esc(o.uuid)}">${esc(pg?.title || o.page)}</span>`, "", "General");
  } else if (o.path) row("Location", esc(o.path), "", "General");
  if (o.room) row("Room", val(o.room), "", "General");
  if (o.category) row("Category", val(o.category), "", "General");
  if (o.target) row("Mirrors", linkTarget(o.target), "", "General");
  if (o.mirrored_by.length) {
    row("Used on pages", o.mirrored_by.map(([pg, ref]) => {
      const p = S.view.pages.find((x) => x.uuid === pg);
      return `<span class="conn"><span class="lnk" data-page="${esc(pg)}" data-go="${esc(ref)}">${esc(p?.title || pg)}</span></span>`;
    }).join(""), "", "General");
  }

  const conns = o.ports.filter((q) => q.conns.length);
  if (conns.length) {
    cat("Connections");
    for (const q of conns) {
      const anyAdd = q.conns.some((c) => c.status === "added"), anyRem = q.conns.some((c) => c.status === "removed");
      row(q.name, q.conns.map((c) => `<span class="conn ${marks ? c.status : ""}">${c.dir === "in" ? "← " : "→ "}${linkTarget(c.target)}</span>`).join(""),
        marks && (anyAdd || anyRem) ? (anyAdd && !anyRem ? "chg-add" : !anyAdd && anyRem ? "chg-rem" : "chg") : "", "Connections");
    }
  }
  const params = o.ports.filter((q) => q.def != null || q.def_changed || q.inv);
  if (params.length) {
    cat("Parameters");
    for (const q of params) {
      row(q.name + (q.inv ? " (inverted)" : ""), chgVal(q.old_def, q.def, q.def_changed), marks && q.def_changed ? "chg" : "", "Parameters");
    }
  }
  const attrs = o.attrs.filter((a) => !["Title", "Type", "IName"].includes(a.name) || a.changed);
  if (attrs.length) {
    cat("Attributes");
    for (const a of attrs) row(a.name, chgVal(a.old, a.value, a.changed), marks && a.changed ? (a.value == null ? "chg-rem" : "chg") : "", "Attributes");
  }
  box.innerHTML = `<div class="pg-head"><div class="t">${esc(o.title)}</div><div class="s">${esc(prettyType(o.ty))}</div>${st}</div><table class="pg">${rows.join("")}</table>`;
  box.querySelectorAll("tr.cat").forEach((tr) => {
    tr.onclick = () => {
      tr.classList.toggle("closed");
      box.querySelectorAll(`tr[data-in="${CSS.escape(tr.dataset.cat)}"]`).forEach((r) => (r.style.display = tr.classList.contains("closed") ? "none" : ""));
    };
  });
}
$("props").addEventListener("click", (e) => {
  const a = e.target.closest(".lnk");
  if (!a) return;
  if (a.dataset.page) gotoPage(a.dataset.page, { focus: a.dataset.go });
  if (a.dataset.go) select(a.dataset.go);
});

// ── tree ───────────────────────────────────────────────────────────────
const ICON = { Pages: ["#5b8fd6", "▦"], Page: ["#5b8fd6", "P"], Place: ["#9c7bd1", "R"], Category: ["#d1a23f", "C"], VirtualIn: ["#00a2c7", "V"], VirtualOut: ["#00a2c7", "V"], Memory: ["#888", "M"], User: ["#777", "U"] };
function icon(n) {
  const [c, t] = ICON[n.ty] || (n.children.length ? ["#b5b5b5", "▤"] : ["#69c350", "•"]);
  return `<span class="tn-ic" style="background:${c}">${t}</span>`;
}
function subtreeChanges(n, acc = { n: 0 }) {
  if (n.status) acc.n++;
  for (const c of n.children) subtreeChanges(c, acc);
  return acc.n;
}
function renderTree() {
  const box = $("tree");
  const q = $("tree-filter").value.trim().toLowerCase();
  box.innerHTML = "";
  if (!S.view) return;
  const marks = S.marks && S.view.compared;
  if (q) {
    const out = [];
    const walk = (n, path) => {
      if (out.length > 400) return;
      if (n.title.toLowerCase().includes(q) || n.ty.toLowerCase().includes(q)) out.push([n, path]);
      for (const c of n.children) walk(c, path ? path + " / " + n.title : n.title);
    };
    S.view.tree.forEach((n) => walk(n, ""));
    for (const [n, path] of out) {
      const d = document.createElement("div");
      d.className = "tn";
      d.innerHTML = `<div class="tn-row ${marks ? n.status : ""}${n.uuid === S.sel ? " sel" : ""}" data-uuid="${esc(n.uuid)}"><span class="tn-tw leaf"></span>${icon(n)}<span class="tn-t">${esc(n.title)}</span><span class="tn-ty">${esc(path)}</span></div>`;
      box.appendChild(d);
    }
    if (!out.length) box.innerHTML = '<div class="empty" style="padding:10px;color:#888">Nothing matches.</div>';
    return;
  }
  const build = (n, parent) => {
    const d = document.createElement("div");
    d.className = "tn";
    const kids = n.children.length;
    const open = S.open.has(n.uuid);
    const cnt = marks ? subtreeChanges(n) : 0;
    const badge = marks && kids && cnt && !n.status ? ` <span class="badge changed">${cnt}</span>` : "";
    const showTy = n.ty !== "Page" && !n.children.length && n.title !== n.ty ? `<span class="tn-ty">${esc(prettyType(n.ty))}</span>` : "";
    d.innerHTML = `<div class="tn-row ${marks ? n.status : ""}${n.uuid === S.sel ? " sel" : ""}" data-uuid="${esc(n.uuid)}"><span class="tn-tw${kids ? "" : " leaf"}">${kids ? (open ? "−" : "+") : ""}</span>${icon(n)}<span class="tn-t">${esc(n.title)}</span>${showTy}${badge}</div>`;
    parent.appendChild(d);
    if (kids && open) {
      const k = document.createElement("div");
      k.className = "tn-kids";
      d.appendChild(k);
      for (const c of n.children) build(c, k);
    }
  };
  const pages = [];
  const collect = (n) => { if (n.page) pages.push(n); else n.children.forEach(collect); };
  S.view.tree.forEach(collect);
  const nested = pages.some((pg) => !S.view.tree.includes(pg));
  const roots = nested ? [{ uuid: "__pages", title: "Pages", ty: "Pages", status: "", page: false, children: pages }, ...S.view.tree] : S.view.tree;
  if (!S.open.size) S.open.add("__pages");
  for (const n of roots) build(n, box);
}
function findPath(uuid) {
  const path = [];
  const walk = (n) => {
    if (n.uuid === uuid) return true;
    for (const c of n.children) if (walk(c)) { path.push(n.uuid); return true; }
    return false;
  };
  for (const n of S.view?.tree || []) if (walk(n)) break;
  return path;
}
function revealInTree(uuid) {
  if (uuid) for (const p of findPath(uuid)) S.open.add(p);
  renderTree();
  const row = uuid && $("tree").querySelector(`.tn-row[data-uuid="${CSS.escape(uuid)}"]`);
  if (row) row.scrollIntoView({ block: "nearest" });
}
$("tree").addEventListener("click", (e) => {
  const row = e.target.closest(".tn-row");
  if (!row) return;
  const uuid = row.dataset.uuid;
  if (e.target.classList.contains("tn-tw") && !e.target.classList.contains("leaf")) {
    S.open.has(uuid) ? S.open.delete(uuid) : S.open.add(uuid);
    renderTree();
    return;
  }
  if (uuid === "__pages") { S.open.has(uuid) ? S.open.delete(uuid) : S.open.add(uuid); renderTree(); return; }
  const isPage = S.view.pages.some((p) => p.uuid === uuid);
  if (isPage) { gotoPage(uuid); S.open.add(uuid); }
  const pg = S.objPage.get(uuid);
  if (pg) gotoPage(pg, { focus: uuid });
  select(uuid);
});
$("tree").addEventListener("dblclick", (e) => {
  const row = e.target.closest(".tn-row");
  if (!row) return;
  const u = row.dataset.uuid;
  S.open.has(u) ? S.open.delete(u) : S.open.add(u);
  renderTree();
});
$("tree-filter").addEventListener("input", renderTree);

// ── changes ────────────────────────────────────────────────────────────
const KIND = { added: ["+", "added"], removed: ["−", "removed"], renamed: ["~", "renamed"], param: ["~", "parameters"], attr: ["~", "settings"], "wire+": ["+", "wires added"], "wire-": ["−", "wires removed"], moved: ["↔", "moved"] };
function renderChanges() {
  const v = S.view, box = $("changes"), filt = $("chg-filter");
  box.innerHTML = ""; filt.innerHTML = "";
  if (!v.compared) {
    $("chg-title").innerHTML = "Changes";
    box.innerHTML = '<div class="none">Pick a revision to compare with to see what changed.</div>';
    return;
  }
  $("chg-title").innerHTML = `Changes <span class="count">${v.changes.length}</span>`;
  const counts = {};
  for (const c of v.changes) counts[c.kind] = (counts[c.kind] || 0) + 1;
  for (const k of Object.keys(KIND)) {
    if (!counts[k]) continue;
    const p = document.createElement("span");
    p.className = "pill" + (S.chgOff.has(k) ? " off" : "");
    p.textContent = `${KIND[k][1]} ${counts[k]}`;
    p.onclick = () => { S.chgOff.has(k) ? S.chgOff.delete(k) : S.chgOff.add(k); renderChanges(); };
    filt.appendChild(p);
  }
  if (!v.changes.length) { box.innerHTML = '<div class="none">No differences.</div>'; return; }
  const pageTitle = (u) => v.pages.find((p) => p.uuid === u)?.title;
  let shown = 0;
  for (const c of v.changes) {
    if (S.chgOff.has(c.kind)) continue;
    if (++shown > 2000) break;
    const d = document.createElement("div");
    d.className = "chg";
    const where = c.page ? pageTitle(c.page) : c.obj ? v.objects[c.obj]?.path : "";
    d.innerHTML = `<span class="k ${esc(c.kind)}">${KIND[c.kind]?.[0] || "~"}</span><div class="x"><div class="t">${esc(c.text)}</div>${where ? `<div class="p">${esc(where)}</div>` : ""}</div>`;
    d.onclick = () => {
      if (c.page) gotoPage(c.page, { focus: c.obj });
      if (c.obj) select(c.obj);
    };
    box.appendChild(d);
  }
}

// ── controls ───────────────────────────────────────────────────────────
$("sel-rev").onchange = (e) => {
  S.rev = e.target.value;
  S.base = revById(S.rev)?.base || null;
  S.follow = false; $("chk-follow").checked = false;
  renderHistory(); renderSelects(); loadView();
};
$("sel-base").onchange = (e) => { S.base = e.target.value || null; renderHistory(); loadView(); };
$("chk-changes").onchange = (e) => {
  S.marks = e.target.checked;
  if (!S.view) return;
  $("legend").classList.toggle("hidden", !S.view.compared || !S.marks);
  renderTabs(); renderTree(); renderPage(); renderProps();
};
$("chk-follow").onchange = (e) => { S.follow = e.target.checked; if (S.follow) { S.rev = null; loadHistory(); } };

window.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === "f") { e.preventDefault(); $("tree-filter").focus(); return; }
  if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
  if (e.key === "+" || e.key === "=") zoomAt(1.25, svg.clientWidth / 2, svg.clientHeight / 2);
  else if (e.key === "-") zoomAt(0.8, svg.clientWidth / 2, svg.clientHeight / 2);
  else if (e.key === "0") fit();
  else if (e.key === "PageDown" || e.key === "PageUp") {
    const pages = S.view?.pages || [];
    const i = pages.findIndex((p) => p.uuid === S.page);
    const j = e.key === "PageDown" ? Math.min(pages.length - 1, i + 1) : Math.max(0, i - 1);
    if (pages[j]) gotoPage(pages[j].uuid);
  } else if (e.key === "Escape") select(null);
});

// Splitters between the stacked panes.
for (const [sid, a, b] of [["split-left", "tree-pane", "props-pane"], ["split-right", "hist-pane", "chg-pane"]]) {
  $(sid).addEventListener("mousedown", (e) => {
    e.preventDefault();
    const A = $(a), B = $(b), total = A.offsetHeight + B.offsetHeight, y0 = e.clientY, h0 = A.offsetHeight;
    const mv = (ev) => {
      const h = Math.min(total - 80, Math.max(80, h0 + ev.clientY - y0));
      A.style.flex = `0 0 ${h}px`; B.style.flex = `1 1 ${total - h}px`;
    };
    const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", mv);
    window.addEventListener("mouseup", up);
  });
}

// ── live updates ───────────────────────────────────────────────────────
function connect() {
  const es = new EventSource("/api/events");
  es.onopen = () => setStatus("live · watching for changes", "live");
  es.addEventListener("history", () => loadHistory(true));
  es.onerror = () => setStatus("disconnected — retrying…", "error");
}
loadHistory().then(connect);
