/* exported IS_MAC, UNDO, UNDO_KEYS, adoptedUncommitted, constEditState, constsPending, editAct, editGoNewest, editKey, editRev, editShowNewest, editState, editsAdded, editsBlock, editsChanged, editsPending, renderUndo, showingWhy, undoListToggle, undoStep, undoSync */
"use strict";
// Shared editing state, gates and the edit/undo send choke point (classic script).
// Reads: S.hist/rev/view/viewRev/sel/pageOf/edit/edits/editsKey/editsSeq/editsLoad,
// S.sentFrom/checkKey/mvBusy/wrBusy, MD.open,
// AB.busy, MV and the app's managed-source/default helpers; sessionStorage keeps this tab's undo.
// Writes: S.edits/edit/editsKey/editsSeq/editsLoad/sentFrom/rev/base/follow/sel/checkKey,
// UNDO (blob/lastAt/deployed/done/undone/prev/since/note/inflight/busy/chain/sending).
// Refreshes: managedRefresh, renderPage, renderChanges, renderRights, renderActions,
// renderTree, renderProps, wcardRefresh, renderModes, mdRenderEdit, renderAddHud,
// renderUndo/renderUndoList, renderHistory, renderSelects, loadView, select;
// loadHistory/loadCheck refresh server state; act sends through the original /api/edit path.
// Per-kind editor drafts/forms/handlers remain in app.js. Request sequence guards and
// undo ordering/storage are moved unchanged.

// ── editing values (Properties) ────────────────────────────────────────
// A block's name, its unwired inputs' constants and the settings of a physical input or output are
// edited here. The server writes them into vislox's own file of the project's module and runs
// lox logic compile (docs/editing.md); the compile shows as the pending revision, so it is reviewed,
// applied and discarded like any other. Saving never deploys: only Apply does.
S.edits = null; // GET /api/edits: what vislox's file claims, and which of it is not deployed
S.edit = { uuid: null, open: false, draft: {}, bad: {}, busy: false, msg: "", err: "" };
S.editsKey = null; // the server's key of what /api/edits says (vislox's file, deployed and pending config)
S.editsSeq = 0;
async function loadEdits() {
  // Loads overlap (each change of the key starts one): an older answer arriving after a newer one is dropped, as
  // `S.editsKey` already names the newer and nothing would load it again.
  const seq = ++S.editsSeq;
  const hwWas = JSON.stringify((S.edits?.blocks || []).filter((b) => b.hardware).map((b) => [b.uuid, b.paired]));
  const mdWas = mdEditKey();
  const adoptionsWas = JSON.stringify(adoptedUncommitted());
  const was = managedSet(), under = JSON.stringify(S.edits?.add_under || null), consts = JSON.stringify(S.edits?.constants || null), exprs = JSON.stringify([S.edits?.expressions || null, S.edits?.templates?.instances || null, S.edits?.templates?.templates || null]);
  if (!S.hist?.actions?.shown) { S.edits = null; managedRefresh(was); return; }
  let got = null, failed = false;
  try { got = await api("/api/edits"); } catch { failed = true; }
  // The caller that waits for its load (`await S.editsLoad`) waits for the newest, whose data is applied.
  if (seq !== S.editsSeq) return S.editsLoad;
  if (failed) { S.edits = null; S.editsKey = null; } else S.edits = got;
  // The managed blocks' badges and the legend follow the lockfile and the module (#64); the expressions'
  // groups follow the module too (#82).
  const exprsNow = JSON.stringify([S.edits?.expressions || null, S.edits?.templates?.instances || null, S.edits?.templates?.templates || null]);
  if (S.view && managedSet() === was && exprsNow !== exprs) renderPage();
  managedRefresh(was);
  if (S.view && JSON.stringify(adoptedUncommitted()) !== adoptionsWas) renderChanges();
  // The claims' λ file:line in Rights and schedules too (#68); the changes a constant causes, grouped (#81).
  if (S.view) S.view.compared && (JSON.stringify(S.edits?.constants || null) !== consts || exprsNow !== exprs) ? renderChanges() : renderRights();
  renderActions();
  // Where the tree offers Add (#84) follows the server's `add_under`.
  if (S.view && (JSON.stringify(S.edits?.add_under || null) !== under ||
    JSON.stringify((S.edits?.blocks || []).filter((b) => b.hardware).map((b) => [b.uuid, b.paired])) !== hwWas)) renderTree();
  if (S.sel && S.view) renderProps();
  wcardRefresh();
  // Add mode, Add entry and each entry's Edit follow the server (#86): the year is drawn again only when what
  // they say changed, the header's buttons always.
  if (MD.open) mdEditKey() !== mdWas ? renderModes() : mdRenderEdit();
}
/** Fetch the edits again only when the server says they may have changed. */
function editsChanged() {
  const key = S.hist?.actions?.edit?.key ?? null;
  if (key !== S.editsKey) { S.editsKey = key; S.editsLoad = loadEdits(); }
  // The server checks the module when it changes and counts the checks it keeps.
  const ck = S.hist?.actions?.edit?.check ?? null;
  if (ck !== S.checkKey) { S.checkKey = ck; loadCheck(); }
}

/** One claimed value, whatever the order of the list: its op, object and port or setting. */
function editKey(c) { return [c.op, c.uuid, c.port ?? c.name ?? ""].join("|"); }
/** The revision edits are made against: the newest compile, else the deployed config (what the server's
 *  `edit_revisions` takes as the base, so both pick the same). */
function editRev() {
  const p = S.hist?.pending?.[0];
  return p?.current ? p : deployedRev();
}
/** The block added here (#59) with UUID `uuid`, deployed or not. */
function editsBlock(uuid) { return (S.edits?.blocks || []).find((b) => b.uuid && b.uuid === uuid) || null; }
/** The blocks adopted here (#72) that the deployed revision's commit does not have yet. */
function adoptedUncommitted() { return (S.edits?.blocks || []).filter((b) => b.adopted && !b.committed); }
/** The blocks added here (#59) that are not deployed yet, or the one with UUID `uuid`. */
function editsAdded(uuid) { return (S.edits?.blocks || []).filter((b) => b.uuid && !b.deployed && (!uuid || b.uuid === uuid)); }
/** The values vislox edited and has not deployed yet, of one object or of all; not those of a new block, which is new as a whole. */
function editsPending(uuid) { return (S.edits?.claims || []).filter((c) => c.pending && (c.op === "let" ? !uuid : (!uuid || c.uuid === uuid) && !editsAdded(c.uuid).length)); }
/** vislox's constants (#81) whose value is not deployed, that a port of `o` takes. */
function constsPending(o) {
  const names = new Set(o.ports.map((q) => constUse(o.uuid, q.key)).filter((u) => u?.ours).map(constLet).filter(Boolean));
  return (S.edits?.claims || []).filter((c) => c.op === "let" && c.pending && names.has(c.name));
}
/** Whether `o` can be edited now: `{ ok }`, or `{ why }` and maybe `{ newest }` to go to. */
function editState(o) {
  const a = S.hist?.actions;
  if (!a?.shown || (!o.edit && !editsBlock(o.uuid))) return null;
  if (!a.edit?.available) return { why: `Editing is off: ${a.edit?.reason || "unknown"}.` };
  if (S.edits?.managed?.includes(o.uuid)) return { why: `The project's module manages this block: change its values in ${srcPath(managedSrc(o.uuid)) || "its .loxl file"}.` };
  const r = editRev();
  if (!r) return { why: "The config on disk is not in the History yet." };
  if (S.rev !== r.id) return { why: `Edits are made on the newest config (${r.kind === "pending" ? "Pending" : "deployed"}).`, newest: r };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  return { ok: true, rev: r };
}
/** Whether edits can be sent now, on the newest config: `{ ok, rev }` or `{ why }`. */
function constEditState() {
  const a = S.hist?.actions, r = editRev();
  if (!a?.shown || !a.edit?.available) return { why: `Editing is off${a?.edit?.reason ? `: ${a.edit.reason}` : ""}.` };
  if (!r || S.rev !== r.id) return { why: "Edits are made on the newest config." };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  return { ok: true, rev: r };
}
/** An edit is sent against the config the page shows, and the server refuses it (409) when the pending config
 *  has changed since. The reply of an edit comes before the History shows its compile: until it does, the page
 *  still shows the config the edit was made on, and another edit made on it would be refused. `undoSend`, the
 *  one way an edit is sent, refuses it here with the reason instead. Self-clearing: the History showing any other
 *  config than `base` ends it (ours, or one a compile elsewhere made), and so does time (the server decides). */
S.sentFrom = null; // { base, at }: an edit made on the config `base` has been answered
const SHOWING_WHY = "The last edit is still being shown. Try again in a moment.";
function showingWhy() {
  const s = S.sentFrom;
  if (!s) return "";
  if (editRev()?.blob !== s.base || Date.now() - s.at > 10000) { S.sentFrom = null; return ""; }
  return SHOWING_WHY;
}
/** Show the newest config, once the History has the compile `blob` (its file watcher may lag behind the reply). */
async function editShowNewest(blob) {
  await loadHistory();
  for (let i = 0; blob && editRev()?.blob !== blob && i < 40; i++) {
    await new Promise((ok) => setTimeout(ok, 250));
    await loadHistory();
  }
  const r = editRev();
  if (r && S.rev !== r.id) { S.rev = r.id; S.base = r.base || null; renderHistory(); renderSelects(); await loadView(); }
  // Following, the History's own refresh may have switched to it and still be loading it.
  else if (r && S.viewRev !== r.id) await loadView();
}
function editGoNewest() { S.follow = true; $("chk-follow").checked = true; editShowNewest(); }

// ── undo and redo (#62) ────────────────────────────────────────────────
// Every edit's reply carries `undo`: the ops that undo it, built from each op's `previous`
// (docs/editing.md). Undo sends them as an edit of its own, so it is checked and compiled like any
// other (and refused like any other: never deployed, never over uncompiled module changes). Its reply's
// `undo` is the redo. The stack is this tab's, and belongs to the pending config it was built on
// (`blob`): when that changes outside this tab (another tab's edit, a compile, Discard, Apply, Sync, a
// Pull), the stack no longer describes it and is cleared, saying why. It is kept in sessionStorage
// under that blob, so a reload of this tab picks it up, and only on that very config.
const UNDO_MAX = 50; // edits kept
const UNDO_LAG = 20000; // ms the History may take to show this tab's own compile
const UNDO_QUEUE = 3; // undo and redo steps waiting their turn: a held key does not pile up more
const UNDO_KEY = "vislox.undo.";
// done: undoable edits, oldest first; undone: redoable ones, the next redo last. `blob`, `lastAt` and
// `deployed`: the pending config, the server's last Apply/Sync/Discard and the deployed revision the
// stack was built on; `prev`/`since`: the blob of the request this tab sent last, and when it ended.
// `inflight`: requests on the way; `busy`: undo and redo steps queued or running (`chain`), each until
// the page shows what it compiled; `sending`: the edit being sent.
const UNDO = { blob: null, lastAt: null, deployed: null, done: [], undone: [], prev: null, since: 0, note: "",
  inflight: 0, busy: 0, chain: Promise.resolve(), sending: null };
const IS_MAC = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
const UNDO_KEYS = IS_MAC ? ["⌘Z", "⇧⌘Z"] : ["Ctrl+Z", "Shift+Ctrl+Z"];
/** A short name for an edit, from its ops, as the page names things now (before the edit shows). */
function undoLabel(ops) {
  const obj = (u) => { const o = S.view?.objects[u]; return o ? displayTitle(o) : "a block"; };
  const end = (e) => { const o = S.view?.objects[e[0]]; return `${obj(e[0])} ${o ? portName(o, e[1]) : e[1]}`; };
  const wire = (w) => `${end(w.from)} → ${end(w.to)}`;
  const kinds = new Set(ops.map((o) => o.op));
  const op = ops[0];
  // A calendar entry (#86): its name, mode and date, saved together.
  if (ops.some((o) => o.op === "entry_mode" || o.op === "entry_date") && new Set(ops.map((o) => o.uuid)).size === 1) return `Calendar entry ${obj(op.uuid)}`;
  if (ops.every((o) => EDIT_ATTR[o.op] || o.op === "param" || o.op === "setting")) {
    const uuids = new Set(ops.map((o) => o.uuid));
    if (ops.length === 1) {
      const o = S.view?.objects[op.uuid];
      const what = EDIT_ATTR[op.op] || (op.op === "param" ? (o ? portName(o, op.port) : op.port) : op.name);
      return `${what} of ${obj(op.uuid)}: ${op.value == null ? "Loxone Config's value" : op.value}`;
    }
    return uuids.size === 1 ? `${ops.length} values of ${obj(op.uuid)}` : `${ops.length} values`;
  }
  if (ops.length === 1 && op.op === "add_block" && op.type === "Mode") return `Add the mode ${op.title}`;
  if (ops.length === 1 && op.op === "add_block") return `Add ${op.title || op.type}`;
  // Constants (#81): changed, made (and bound), dropped.
  if (op.op === "let" && ops.length === 1) return op.value == null ? `Drop the constant ${op.name}` : `Constant ${op.name}: ${op.value}`;
  if (op.op === "let" && ops.slice(1).every((o) => o.op === "param" && o.value === op.name)) return `Make the constant ${op.name} = ${op.value}`;
  if (ops.length === 1 && op.op === "defaults") return op.value == null ? `Drop the default ${op.type} ${op.port}` : `Default ${op.type} ${op.port}: ${op.value}`;
  // Made when the op is made, and kept on its history entry: a deleted block is gone from the view afterwards.
  if (ops.length === 1 && op.op === "remove_block") { const b = editsBlock(op.uuid); return `${b?.deployed || b?.adopted ? "Delete" : "Remove"} ${obj(op.uuid)}`; }
  if (ops.length === 1 && op.op === "restore_block") {
    const m = (op.source || "").match(/^(\w+) = \w+\((?:"((?:[^"\\]|\\.)*)")?/);
    return `Bring back ${S.view?.objects[op.uuid] ? obj(op.uuid) : m?.[2] || m?.[1] || "a block"}`;
  }
  if (ops.length === 1 && op.op === "rename") return `Rename ${obj(op.uuid)} to ${op.slug}`;
  // Expressions and named signals (#82).
  if (ops.length === 1 && op.op === "expr") return op.value == null ? `Drop the expression on ${end([op.uuid, op.port])}` : `${end([op.uuid, op.port])} ← ${op.value}`;
  if (ops.length === 1 && op.op === "signal") return op.value == null ? `Drop the signal ${op.slug}` : `Signal ${op.slug} = ${op.value}${op.title ? ` (${op.title})` : ""}`;
  if (ops.length === 2 && op.op === "signal" && ops[1].op === "expr" && op.value != null) return `Name the expression ${op.slug}`;
  if (ops.length === 2 && op.op === "expr" && ops[1].op === "signal" && ops[1].value == null) return `Inline the signal ${ops[1].slug}`;
  // Template instances (#83).
  if (ops.length === 1 && op.op === "instance") return op.value == null ? `Remove the instance ${op.slug}` : `Instance ${op.slug}: ${op.value.template}`;
  if (ops.length === 1 && op.op === "tidy") return "Remove obsolete statements";
  if (kinds.size === 1 && op.op === "move") return ops.length === 1 ? `Move ${obj(op.uuid)}` : `Move ${plural(ops.length, "block")}`;
  if (ops.length === 1 && op.op === "relayout") return `Arrange ${S.pageOf?.get(op.page)?.title || "the page"}`;
  if (ops.length === 1 && op.op === "adopt") return op.blocks.length === 1 ? `Adopt ${obj(op.blocks[0].uuid)}` : `Adopt ${plural(op.blocks.length, "block")}`;
  if (ops.length === 1 && op.op === "adopt_page") return `Adopt the blocks of ${S.pageOf?.get(op.page)?.title || "the page"}`;
  if (ops.length === 1 && op.op === "unadopt") return op.uuids.length === 1 ? `Hand ${obj(op.uuids[0])} back to Loxone Config` : `Hand ${plural(op.uuids.length, "block")} back to Loxone Config`;
  if (ops.length === 1 && op.op === "access") return `Access to ${obj(op.uuid)}${op.value == null ? ": as last written" : op.value.grants ? "" : ": shared set"}`;
  if (ops.length === 1 && op.op === "new_rule") return `New rule ${op.title}`;
  // New pages (#88).
  if (ops.length === 1 && op.op === "new_page") return `New page ${op.title}`;
  if (ops.length === 1 && op.op === "remove_page") return `Remove the page ${pageCreated(op.page)?.title || S.pageOf?.get(op.page)?.title || ""}`.trim();
  if (op.op === "rule" || op.op === "restore_rule") return `Rule ${obj(op.uuid)}${op.value == null ? ": as last written" : ""}`;
  if (ops.length === 1 && op.op === "members") return `Members of ${obj(op.uuid)}${op.value == null ? ": as last written" : ""}`;
  if (ops.length === 1 && op.op === "moods") return `Moods of ${obj(op.uuid)}${op.value == null ? ": Loxone Config's" : op.value.moods ? "" : ": shared set"}`;
  if (ops.length === 1 && op.op === "schedule") return `Switching times of ${obj(op.uuid)}${op.value == null ? ": Loxone Config's" : op.value.modes ? "" : ": shared set"}`;
  if (ops.length === 1 && op.op === "wire") return `Wire ${wire(op)}`;
  if (ops.length === 1 && op.op === "unwire") return `Delete wire ${wire(op)}`;
  if (ops.length === 2 && op.op === "unwire" && ops[1].op === "wire") return `Reconnect ${wire(op)} to ${end(sameEnd(op.from, ops[1].from) ? ops[1].to : ops[1].from)}`;
  return plural(ops.length, "edit");
}
/** What the server says now: the pending config, its last Apply/Sync/Discard, the deployed revision. */
function undoNow() {
  const r = editRev();
  return r && { blob: r.blob, lastAt: S.hist?.actions?.last?.at ?? null, deployed: deployedRev()?.id ?? null };
}
function undoLoad(blob) {
  try { return JSON.parse(sessionStorage.getItem(UNDO_KEY + blob) || "null"); } catch { return null; }
}
/** Keep the stack under its blob, and drop the ones kept for other blobs: they describe no config any more. */
function undoSave() {
  try {
    for (let i = sessionStorage.length - 1; i >= 0; i--) {
      const k = sessionStorage.key(i);
      if (k?.startsWith(UNDO_KEY) && k !== UNDO_KEY + UNDO.blob) sessionStorage.removeItem(k);
    }
    if (!UNDO.blob) return;
    if (UNDO.done.length || UNDO.undone.length) {
      sessionStorage.setItem(UNDO_KEY + UNDO.blob, JSON.stringify({ lastAt: UNDO.lastAt, deployed: UNDO.deployed, done: UNDO.done, undone: UNDO.undone }));
    } else sessionStorage.removeItem(UNDO_KEY + UNDO.blob);
  } catch { /* storage off: the stack lives as long as the page */ }
}
/** Whether a stack built on `was` still describes the config `now`. The server's last outcome is unknown
 *  (null) after it restarted: then the blob and the deployed revision decide. */
const undoSame = (was, now) => was.blob === now.blob && was.deployed === now.deployed && (now.lastAt == null || was.lastAt === now.lastAt);
/** Empty the stack; `note` says why, when there was something to lose (or `always`). */
function undoClear(note, always) {
  if (always || UNDO.done.length || UNDO.undone.length) UNDO.note = note;
  UNDO.done = []; UNDO.undone = [];
}
/** Check the stack against what the server says, and clear it (saying why) when the config changed
 *  outside this tab. Called on every History update. */
function undoSync() {
  const now = undoNow();
  if (!now || UNDO.inflight) return;
  if (UNDO.blob && undoSame(UNDO, now)) return;
  // This tab's own compile, which the History shows a moment later.
  if (UNDO.blob && now.blob === UNDO.prev && UNDO.deployed === now.deployed && (now.lastAt == null || UNDO.lastAt === now.lastAt) && Date.now() - UNDO.since < UNDO_LAG) return;
  const first = !UNDO.blob;
  // Kept for this very config by this tab (a reload): picked up.
  const kept = first && undoLoad(now.blob);
  if (kept && undoSame({ ...kept, blob: now.blob }, now) && (kept.done?.length || kept.undone?.length)) {
    Object.assign(UNDO, now, { done: kept.done || [], undone: kept.undone || [], prev: null, note: "" });
    return;
  }
  const why = now.lastAt != null && now.lastAt !== UNDO.lastAt ? ({ apply: "Apply deployed", discard: "Discard dropped", sync: "Sync replaced" }[S.hist?.actions?.last?.kind] || "an Apply, Sync or Discard changed") + " the pending config"
    : now.deployed !== UNDO.deployed ? "the deployed config changed (an Apply, Sync or Pull)"
    : "the pending config changed outside this tab (an edit in another tab, or a compile)";
  if (!first) undoClear(`Undo history cleared: ${why}.`);
  Object.assign(UNDO, now, { prev: null });
  undoSave();
}
/** lox mints new UUIDs for a block added again (an undo of Remove, a redo of Add): the reply names the
 *  UUID it `restores`, and every edit kept here that names the old one names the new one from now on. */
function undoRemap(added) {
  for (const a of added || []) {
    if (!a.restores || !a.uuid || a.restores === a.uuid) continue;
    const swap = (v) => v === a.restores ? a.uuid : Array.isArray(v) ? v.map(swap)
      : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, swap(x)])) : v;
    for (const e of [...UNDO.done, ...UNDO.undone]) { e.undo = swap(e.undo); e.redo = swap(e.redo); }
    if (S.sel === a.restores) S.sel = a.uuid;
  }
}
/** Send `ops` against `base`, and keep what the reply says: a new edit named `label`, or the step that
 *  undoes (`redo` false) or redoes `entry`. The one way an edit, an undo or a redo is sent. Throws as
 *  act() does, and then keeps nothing. */
async function undoSend(base, ops, { label, entry, redo } = {}) {
  // An undo or redo waits for the History to show the step before it (undoRun); an edit is refused meanwhile.
  const why = entry ? "" : showingWhy();
  if (why) throw new Error(why);
  UNDO.inflight++;
  renderAddHud();
  try {
    const res = await act("/api/edit", { base, ops });
    // Made on another config than the stack's: the stack no longer describes it.
    if (UNDO.blob !== base) undoClear("Undo history cleared: the edit was made on a config the history was not built on.");
    const name = entry ? entry.label : label;
    if (res.unchanged) {
      // Nothing to compile: the config already is what the step leads to. The step is done, its ops kept.
      if (entry && (redo ? UNDO.undone : UNDO.done).at(-1) === entry) {
        (redo ? UNDO.undone : UNDO.done).pop();
        (redo ? UNDO.done : UNDO.undone).push(entry);
      }
    } else {
      undoRemap(res.added);
      const inv = Array.isArray(res.undo) && res.undo.length ? res.undo : null;
      if (!inv) {
        // What came before no longer leads here: none of it can be undone safely.
        undoClear(`“${name}” cannot be ${entry && !redo ? "redone" : "undone"}: ${res.undo_error || "the server sent no way back"}. The undo history was cleared.`, true);
      } else if (!entry) {
        UNDO.done.push({ label: name, undo: inv, redo: ops, at: Date.now() });
        if (UNDO.done.length > UNDO_MAX) UNDO.done.shift();
        UNDO.undone = [];
        UNDO.note = "";
      } else if (redo) {
        UNDO.undone.pop(); entry.undo = inv; UNDO.done.push(entry);
      } else {
        UNDO.done.pop(); entry.redo = inv; UNDO.undone.push(entry);
      }
    }
    Object.assign(UNDO, undoNow() || {}, { blob: res.blob, prev: base });
    undoSave();
    if (!res.unchanged && res.blob && res.blob !== base) S.sentFrom = { base, at: Date.now() };
    return res;
  } finally {
    UNDO.inflight--;
    UNDO.since = Date.now();
    renderAddHud();
  }
}
/** POST /api/edit for an edit of the page: kept for undo (`label`: how the list names it; by default
 *  made from the ops). Throws as act() does. */
async function editAct(base, ops, label) {
  undoSync();
  const p = undoSend(base, ops, { label: label || undoLabel(ops) });
  UNDO.sending = p;
  try { return await p; } finally { if (UNDO.sending === p) UNDO.sending = null; }
}
/** Why undo or redo cannot run now, or null. `queue`: for a step that waits its turn behind the undo
 *  and redo steps and the edit being sent. Edits are made on the newest config, shown: the same rule as
 *  every other edit. */
function undoWhy(queue) {
  const a = S.hist?.actions;
  if (!a?.shown) return "Editing is off.";
  if (!a.edit?.available) return `Editing is off: ${a.edit?.reason || "unknown"}.`;
  const r = editRev();
  if (!r) return "The config on disk is not in the History yet.";
  if (S.rev !== r.id) return `Undo and redo change the newest config (${r.kind === "pending" ? "Pending" : "deployed"}): show it first.`;
  // An edit compiling (this tab's own save, or the step before) is waited for, not refused: the server
  // reports it running before the page's own request comes back.
  if (a.running && !(queue && a.running === "Edit")) return `${a.running} is running.`;
  if (queue) return null;
  if (UNDO.busy || UNDO.inflight || S.mvBusy || S.wrBusy || AB.busy || S.edit.busy || MV) return "An edit is compiling.";
  return null;
}
/** Undo the last edit, or (`redo`) redo the last one undone. A step waits its turn behind the steps
 *  before it and the edit being sent, so a quick Ctrl+Z after a save undoes that save. */
function undoStep(redo) {
  const why = undoWhy(true);
  if (why) { deployMsg(`Cannot ${redo ? "redo" : "undo"} now: ${why}`, true); return; }
  if (UNDO.busy >= UNDO_QUEUE) return;
  UNDO.busy++;
  renderAddHud();
  // Done: the page drew blocks as not movable, and Properties said it was compiling, while it ran.
  UNDO.chain = UNDO.chain.then(() => undoRun(redo)).catch(() => {}).finally(() => {
    UNDO.busy--;
    renderAddHud();
    if (!UNDO.busy && S.view) { renderPage(); if (S.sel) renderProps(); }
  });
}
/** Resolves once `ok()` holds, or after `ms`. */
async function until(ok, ms) {
  for (const end = Date.now() + ms; !ok() && Date.now() < end;) await new Promise((r) => setTimeout(r, 50));
}
async function undoRun(redo) {
  const word = redo ? "Redo" : "Undo";
  // The edit before it first: sent, compiled, and shown.
  if (UNDO.sending) await UNDO.sending.catch(() => {});
  await until(() => !S.mvBusy && !S.wrBusy && !AB.busy && !S.edit.busy && !MV, 10000);
  if (UNDO.blob && editRev()?.blob !== UNDO.blob) await editShowNewest(UNDO.blob).catch(() => {});
  // The server done with it too: the History says so a moment after the reply.
  await until(() => S.hist?.actions?.running !== "Edit", 30000);
  undoSync();
  const e = (redo ? UNDO.undone : UNDO.done).at(-1);
  if (!e) return;
  const why = undoWhy(true) || (editRev()?.blob !== UNDO.blob ? "the History does not show the last edit yet" : null);
  if (why) { deployMsg(`Cannot ${word.toLowerCase()} now: ${why}`, true); return; }
  deployMsg(`${word}: ${e.label}… (compiling; nothing is deployed)`);
  let res;
  try {
    // Against the stack's own config, which the History shows (checked above).
    res = await undoSend(UNDO.blob, redo ? e.redo : e.undo, { entry: e, redo });
  } catch (err) {
    deployMsg(`${word} of “${e.label}” refused: ${err.message}`, true);
    return;
  }
  const cleared = !UNDO.done.length && !UNDO.undone.length && UNDO.note;
  // What the step left out (#80: a wire whose other end is gone when a deleted block comes back).
  const left = res.warnings?.length ? ` Left out: ${res.warnings.join("; ")}.` : "";
  deployMsg(cleared ? UNDO.note : `${redo ? "Redone" : "Undone"}: ${e.label}. ${res.unchanged ? "Nothing changed: the config already was that way." : "Compiled: pending, not deployed. Review it, then Apply or Discard."}${left}`, false, !!cleared || !!left);
  try {
    await editShowNewest(res.blob);
    if (S.sel && !S.view?.objects[S.sel]) select(null, { noReveal: true });
    else if (S.sel) renderProps();
  } catch (err) {
    deployMsg(`${redo ? "Redone" : "Undone"} and compiled, but the page could not show it: ${err.message}`, true);
  }
}
function renderUndo() {
  const u = $("btn-undo"), r = $("btn-redo"), l = $("btn-undo-list");
  if (!u) return;
  const why = undoWhy(), last = UNDO.done.at(-1), next = UNDO.undone.at(-1);
  u.disabled = !last || !!why;
  r.disabled = !next || !!why;
  u.title = last ? `Undo: ${last.label} (${UNDO_KEYS[0]})${why ? `\n${why}` : ""}` : `Nothing to undo${UNDO.note ? `\n${UNDO.note}` : ""}`;
  r.title = next ? `Redo: ${next.label} (${UNDO_KEYS[1]})${why ? `\n${why}` : ""}` : "Nothing to redo";
  u.setAttribute("aria-label", last ? `Undo ${last.label}` : "Undo");
  r.setAttribute("aria-label", next ? `Redo ${next.label}` : "Redo");
  l.classList.toggle("note", !!UNDO.note);
  l.title = UNDO.note ? `Recent edits\n${UNDO.note}` : "Recent edits";
  if (!$("undo-pop").classList.contains("hidden")) renderUndoList();
}
/** The recent edits: those undone (to redo) above the line, the done ones below, newest first. */
function renderUndoList() {
  const t = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const li = (e, cls) => `<li class="${cls}"><span class="u-l">${esc(e.label)}</span><span class="u-t">${esc(t(e.at))}</span></li>`;
  const undone = UNDO.undone.slice(-5), done = UNDO.done.slice().reverse().slice(0, 10);
  $("undo-pop").innerHTML = `<h3>Recent edits in this tab</h3>` +
    (UNDO.note ? `<p class="u-note" role="status">${esc(UNDO.note)}</p>` : "") +
    (undone.length || done.length ? `<ol>${undone.map((e) => li(e, "undone")).join("")}${done.map((e, i) => li(e, i ? "" : "top")).join("")}</ol>` : '<p class="u-foot">No edits to undo.</p>') +
    `<p class="u-foot">${UNDO_KEYS[0]} undoes the top one, ${UNDO_KEYS[1]} redoes. Each is compiled like an edit; nothing is deployed.</p>`;
}
function undoListToggle(open) {
  const p = $("undo-pop"), b = $("btn-undo-list");
  open ??= p.classList.contains("hidden");
  p.classList.toggle("hidden", !open);
  b.setAttribute("aria-expanded", String(open));
  if (open) renderUndoList();
}
$("btn-undo").onclick = () => undoStep(false);
$("btn-redo").onclick = () => undoStep(true);
$("btn-undo-list").onclick = () => undoListToggle();
document.addEventListener("click", (e) => { if (!e.target.closest("#undo-pop, #btn-undo-list")) undoListToggle(false); });
