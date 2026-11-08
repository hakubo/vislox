/* exported allRevs, followLatest, loadHistory, readHash, revById, writeHash */
"use strict";
// Revision navigation (classic script, loaded before app.js).
// Reads: S.hist/rev/base/follow/view/viewRev/hash/page/sel/live/rollbackRev/removals,
// MD.open, running.loaded, rollbackPreview() and removalsPreview().
// Writes: S.hist/rev/base/follow/hash/applyingHash and local request/retry counters.
// Refreshes: renderHistory, renderSelects, loadView, renderLiveSwitch, renderPull,
// renderActions, renderLog, editsChanged, undoSync, showingWhy, loadRunning,
// liveStart, gotoPage, select, modesOpen, modesClose, setStatus.
// Async history and popstate requests retain their sequence guards.

function allRevs() { return S.hist ? [...(rollbackPreview() ? [S.rollbackRev] : []), ...(removalsPreview() ? [S.removals.revision] : []), ...S.hist.pending, ...S.hist.history, ...(S.hist.backups || [])] : []; }
function newestRev(hist = S.hist) { return hist ? hist.pending[0] || hist.history[0] || null : null; }
function revById(id) { return allRevs().find((r) => r.id === id); }

// ── URL state: #page=…&sel=…&rev=…&base=… ─────────────────────────────
function readHash() {
  const h = new URLSearchParams(location.hash.slice(1));
  return { page: h.get("page"), sel: h.get("sel"), rev: h.get("rev"), base: h.get("base"), view: h.get("view") };
}
function writeHash(push) {
  if (S.applyingHash) return;
  const h = new URLSearchParams();
  if (S.page) h.set("page", S.page);
  if (S.sel) h.set("sel", S.sel);
  if (!S.follow && S.rev) h.set("rev", S.rev);
  if (!S.follow && S.base) h.set("base", S.base);
  if (MD.open) h.set("view", "modes");
  const s = "#" + h.toString();
  if (s === location.hash) return;
  if (push) history.pushState(null, "", s); else history.replaceState(null, "", s);
}
let popstateSeq = 0;
let historyLoadSeq = 0;
let historyRetryTimer = null;
let historyRetryDelay = 1000;
window.addEventListener("popstate", async () => {
  const seq = ++popstateSeq;
  const h = readHash();
  S.applyingHash = true;
  S.hash = h;
  try {
    let light = false;
    if (!h.rev) {
      const latest = newestRev();
      if (S.hist && S.follow && S.rev === latest?.id && S.base === (latest?.base || null)) light = true;
      else await followLatest(seq);
    } else if (revById(h.rev)) {
      if (h.rev !== S.rev || (h.base || null) !== S.base || S.follow) {
        S.rev = h.rev; S.base = h.base || null; S.follow = false; $("chk-follow").checked = false;
        renderHistory(); renderSelects(); await loadView();
        if (seq !== popstateSeq) return;
      } else light = true;
    } else {
      // Keep the current revision for a stale or unavailable history URL.
      light = true;
    }
    if (light && S.viewRev === S.rev) {
      S.hash = null;
      if (h.page && h.page !== S.page) gotoPage(h.page);
      select(h.sel || null, { noReveal: false });
      if (h.view === "modes") modesOpen({ quiet: true }); else modesClose(true);
    }
  } finally { if (seq === popstateSeq) S.applyingHash = false; }
});

// ── history ────────────────────────────────────────────────────────────
function followLatest(navigation) {
  S.follow = true; S.rev = null; $("chk-follow").checked = true;
  return loadHistory(false, navigation);
}
async function loadHistory(fromEvent, navigation) {
  const seq = ++historyLoadSeq;
  let h;
  try { h = await api("/api/history"); } catch (e) {
    if (seq !== historyLoadSeq || (navigation != null && navigation !== popstateSeq)) return;
    setStatus("server unreachable", "error");
    if (!S.hist && historyRetryTimer == null) {
      const delay = historyRetryDelay;
      historyRetryDelay = Math.min(historyRetryDelay * 2, 30000);
      historyRetryTimer = setTimeout(() => { historyRetryTimer = null; loadHistory(); }, delay);
    }
    return;
  }
  if (seq !== historyLoadSeq || (navigation != null && navigation !== popstateSeq)) return;
  clearTimeout(historyRetryTimer);
  historyRetryTimer = null;
  historyRetryDelay = 1000;
  if ($("status").classList.contains("error")) setStatus("live", "live");
  const prevNewest = newestRev(S.hist)?.id;
  S.hist = h;
  // The History showing another config than an edit was made on ends the wait for it, for good (a Discard may
  // bring the first one back).
  showingWhy();
  $("doc-sub").textContent = h.label + (h.error ? " · " + h.error : "");
  if (h.vislox) {
    $("ver").textContent = "v" + h.vislox.version;
    $("ver").title = `vislox ${h.vislox.version}` + (h.vislox.commit ? ` (commit ${h.vislox.commit})` : "");
  }
  if (h.running && !running.loaded) loadRunning(false);
  const newest = newestRev(h);
  let changed = false;
  if (S.hash?.rev && revById(S.hash.rev) && !S.rev) {
    S.rev = S.hash.rev; S.base = S.hash.base || null; S.follow = false; $("chk-follow").checked = false; changed = true;
  }
  if (!S.rev || !revById(S.rev) || (S.follow && newest && newest.id !== S.rev)) {
    const inPending = S.rev && S.rev.startsWith("pending:");
    const pick = (inPending ? h.pending[0] : null) || newestRev(h);
    if (pick && pick.id !== S.rev) { S.rev = pick.id; S.base = pick.base; changed = true; }
  }
  if (S.base && !revById(S.base)) { S.base = revById(S.rev)?.base || null; changed = true; }
  renderHistory();
  renderSelects();
  if (S.live.on && h.live?.available) liveStart();
  renderLiveSwitch();
  renderPull();
  undoSync();
  renderActions();
  editsChanged();
  renderLog();
  if (fromEvent) {
    setStatus("updated " + new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }), "live flash");
    setTimeout(() => setStatus("live", "live"), 2500);
  }
  if (changed || !S.view) await loadView();
  else if (fromEvent && newest && newest.id !== prevNewest && !S.follow) setStatus("new revision in History", "live flash");
}

