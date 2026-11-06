"use strict";
// vislox — Operating modes: a year calendar of when each mode is on.
// The dates come decoded from the server (`calendar.date` of each entry, see src/calendar.rs);
// here they are only laid out on the days of a year. Loaded after app.js, whose helpers it uses.
// With live values on, the Miniserver's heating and cooling periods (`periods` on /api/live, see
// src/calendar.rs `Season`) are marked on the days too, and climate blocks' Properties list them.

const MD = { open: false, year: new Date().getFullYear(), hidden: new Set(), liveKey: "", cache: null, pcache: [], plist: null };
/** The years the view steps through: calendar periods count from 2009 (Loxone's day 0). */
const MD_FIRST_YEAR = 2009, MD_LAST_YEAR = 9999;
/** One colour per mode, in the order of the config's modes; white text reads on each. */
const MD_COLORS = ["#c8691c", "#2f6fb8", "#8a4fc0", "#1f8a5c", "#c23a5a", "#8c6d12", "#0d7f9c", "#5d655d", "#b0428f", "#4d6b1f"];
const MD_MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MD_WEEKDAYS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

// ── dates: whole days since 1970-01-01 (UTC, so no daylight-saving shifts) ──
// setUTCFullYear, not Date.UTC, which would read years 0–99 as 1900–1999.
function mdDay(y, m, d) { const t = new Date(0); t.setUTCFullYear(y, m - 1, d); return Math.floor(t.getTime() / 864e5); }
function mdYmd(n) { const t = new Date(n * 864e5); return [t.getUTCFullYear(), t.getUTCMonth() + 1, t.getUTCDate()]; }
/** `month`/`day` in year `y`, or null when the year has no such day (29 Feb). */
function mdDate(y, m, d) { const n = mdDay(y, m, d); return mdYmd(n)[1] === m ? n : null; }
function mdParse(s) { const m = /^(-?\d+)-(\d+)-(\d+)$/.exec(s || ""); return m ? mdDay(+m[1], +m[2], +m[3]) : null; }
/** Easter Sunday of the Gregorian calendar (Meeus/Jones/Butcher): [month, day]. */
function easterSunday(y) {
  const a = y % 19, b = Math.floor(y / 100), c = y % 100, d = Math.floor(b / 4), e = b % 4;
  const f = Math.floor((b + 8) / 25), g = Math.floor((b - f + 1) / 3);
  const h = (19 * a + b - d - g + 15) % 30, i = Math.floor(c / 4), k = c % 4;
  const l = (32 + 2 * e + 2 * i - h - k) % 7, m = Math.floor((a + 11 * h + 22 * l) / 451);
  return [Math.floor((h + l - 7 * m + 114) / 31), ((h + l - 7 * m + 114) % 31) + 1];
}
/** The days of year `y` a decoded date (`calendar.date`) covers, in order. A yearly range whose
 *  end comes before its start runs across New Year: in one year it covers both ends. */
function entryDays(date, y) {
  if (!date) return [];
  const first = mdDay(y, 1, 1), last = mdDay(y, 12, 31);
  const span = (a, b) => { const out = []; for (let n = Math.max(a, first); n <= Math.min(b, last); n++) out.push(n); return out; };
  switch (date.kind) {
    case "fixed": { const n = mdDate(y, date.month, date.day); return n === null ? [] : [n]; }
    case "easter": {
      // An offset can reach into the year before or after its Easter.
      const out = [];
      for (const yy of [y - 1, y, y + 1]) { const [m, d] = easterSunday(yy); const n = mdDay(yy, m, d) + date.offset; if (n >= first && n <= last) out.push(n); }
      return out;
    }
    case "period": { const a = mdParse(date.from), b = mdParse(date.to); return a === null || b === null ? [] : span(a, b); }
    case "yearly": {
      // 29 Feb in another year: a range starts on 1 Mar, and ends on 28 Feb.
      const start = mdDate(y, date.month, date.day) ?? mdDay(y, 3, 1);
      const end = mdDate(y, date.month2, date.day2) ?? mdDay(y, 2, 28);
      if (date.month * 100 + date.day <= date.month2 * 100 + date.day2) return span(start, end);
      return [...span(first, end), ...span(start, last)];
    }
  }
  return [];
}

// ── the config's modes and entries ──────────────────────────────────────
function mdKey(m) { return m.target ? m.target.uuid : `n${m.number ?? m.title}`; }
/** Every mode an entry switches on (or did, in the compared revision), with its colour. */
function mdModes() {
  const out = new Map();
  const add = (key, title, uuid) => { if (!out.has(key)) out.set(key, { key, title, uuid, color: MD_COLORS[out.size % MD_COLORS.length] }); };
  for (const m of S.view?.modes || []) add(m.uuid, m.title, m.uuid);
  for (const e of mdEntries()) {
    add(mdKey(e.cal.mode), e.cal.mode.title, e.cal.mode.target?.uuid);
    if (e.cal.old_mode) add(mdKey(e.cal.old_mode), e.cal.old_mode.title, e.cal.old_mode.target?.uuid);
  }
  return out;
}
function mdEntries() {
  return Object.values(S.view?.objects || {}).filter((o) => o.calendar).map((o) => ({ uuid: o.uuid, title: displayTitle(o), status: o.status, cal: o.calendar }));
}
function mdEntriesN(n, what) { return `${n} ${what}${n === 1 ? "entry" : "entries"}`; }
function mdMarks() { return !!(S.marks && S.view?.compared); }

/** Year `y` day by day: which entries switch a mode on, and, in change review, which
 *  (day, mode) pairs the viewed revision adds or removes. */
function mdYear(y) {
  const days = new Map();
  const at = (n) => { let d = days.get(n); if (!d) days.set(n, (d = { on: [], add: [], rem: [] })); return d; };
  const marks = mdMarks();
  const entries = mdEntries().map((e) => ({ ...e, key: mdKey(e.cal.mode), days: entryDays(e.cal.date, y) }));
  for (const e of entries) {
    if (e.status !== "removed") for (const n of e.days) at(n).on.push(e);
    if (!marks) continue;
    if (e.status === "removed") { for (const n of e.days) at(n).rem.push(e); continue; }
    if (e.status === "added") { for (const n of e.days) at(n).add.push(e); continue; }
    if (!e.cal.old_date && !e.cal.old_mode) continue;
    const oldDays = e.cal.old_date ? entryDays(e.cal.old_date, y) : e.days;
    const oldKey = e.cal.old_mode ? mdKey(e.cal.old_mode) : e.key;
    const was = new Set(oldDays), is = new Set(e.days), sameMode = oldKey === e.key;
    for (const n of e.days) if (!sameMode || !was.has(n)) at(n).add.push(e);
    for (const n of oldDays) if (!sameMode || !is.has(n)) at(n).rem.push({ ...e, key: oldKey });
  }
  return { days, entries };
}

/** The modes that are on now, from the live value of each Mode's `Q`; null without live values. */
function mdLiveOn() {
  if (!liveHere()) return null;
  const on = new Set();
  for (const m of S.view?.modes || []) {
    const q = S.view.objects[m.uuid]?.ports.find((p) => p.key === "Q");
    const e = q && liveVal(q.uuid);
    if (e && liveOn(e)) on.add(m.uuid);
  }
  return on;
}
function mdLiveKey(on) {
  const st = S.live.status || {};
  return [on ? [...on].sort().join(" ") : "-", JSON.stringify(mdPeriods()), st.state, st.label].join("|");
}
/** Live values changed: repaint only when the set of modes that are on, the periods or the live state changed. */
function modesLive() {
  if (MD.open && mdLiveKey(mdLiveOn()) !== MD.liveKey) renderModes();
}
function mdToday() { const t = new Date(); return mdDay(t.getFullYear(), t.getMonth() + 1, t.getDate()); }

// ── the heating and cooling periods (live) ─────────────────────────────
const MD_PERIODS = [{ key: "heat", title: "Heating period", word: "heating" }, { key: "cool", title: "Cooling period", word: "cooling" }];
/** Whether this Miniserver connection reads the periods (not with --live-from). */
function mdPeriodsAvailable() { return !!S.hist?.live?.periods; }
/** The periods the live session read (`{heat, cool}`, each `{date?, text?, why?, value, stale?,
 *  failed?}` or null); null without live values on this revision, or until they are asked for.
 *  `stale`: read on an earlier connection, not again on this one (yet: `failed` says why not). */
function mdPeriods() { return liveHere() && S.live.periods ? S.live.periods : null; }
/** Whether a block is a climate block (Intelligent Room Controller, Climate Controller). */
function mdClimate(o) { return /IRoomController|^HVACController$/.test(o?.ty || ""); }
/** Each period, with whether today is in it (null: it cannot be read); the same list until the
 *  periods or the day change. */
function mdPeriodList() {
  const p = mdPeriods();
  if (!p) return null;
  const today = mdToday();
  if (MD.plist?.src === p && MD.plist.today === today) return MD.plist.list;
  const ty = mdYmd(today)[0];
  const list = MD_PERIODS.map((d) => {
    const s = p[d.key] || { why: "not read yet", value: "" };
    return { ...d, season: s, today: s.date ? entryDays(s.date, ty).includes(today) : null };
  });
  MD.plist = { src: p, today, list };
  return list;
}
/** Why there are no periods yet: they are being read, or the live connection is not up. */
function mdPeriodsPending() {
  const st = S.live.status || { state: "connecting" };
  if (st.state === "live") return "Reading the heating and cooling periods from the Miniserver…";
  const now = st.label || { connecting: "connecting", idle: "connecting", offline: "offline", stopped: "stopped", ended: "the input ended", off: "off" }[st.state] || st.state;
  return `The heating and cooling periods are read when live values connect (live values: ${now}).`;
}
/** A good read of an earlier connection: what to add to its text. */
function mdPeriodStale(p) {
  if (!p.season.date || !p.season.stale) return "";
  return p.season.failed ? `from the previous connection; reading it again failed: ${p.season.failed}` : "from the previous connection; reading it again…";
}
/** Whether today is in either period, as a sentence. */
function mdPeriodsToday(list) {
  const known = list.filter((p) => p.today !== null), inn = known.filter((p) => p.today);
  if (!known.length) return "";
  if (inn.length === 2) return "Today is in both the heating and the cooling period.";
  if (inn.length === 1) return `Today is in the ${inn[0].word} period.`;
  return known.length === 2 ? "Today is in neither period." : `Today is not in the ${known[0].word} period.`;
}
/** A period as it reads: its days, or why it cannot be read. */
function mdPeriodText(p) {
  return p.season.text || `cannot read: ${p.season.why || "unknown"}${p.season.value ? ` (“${p.season.value}”)` : ""}`;
}
/** Properties of a climate block (Intelligent Room Controller, Climate Controller): the periods
 *  it heats or cools in. `row(k, v)` makes a row; null when there is nothing to show. */
function mdPeriodRows(o, row) {
  if (!mdClimate(o) || !liveHere() || !mdPeriodsAvailable()) return null;
  const list = mdPeriodList();
  if (!list) return [`<div class="empty">${esc(mdPeriodsPending())}</div>`];
  const rows = list.map((p) => {
    const stale = mdPeriodStale(p);
    return row(p.title, `<span class="md-psw ${p.key}"></span>${p.season.text ? esc(p.season.text) : `<span class="none-v">${esc(mdPeriodText(p))}</span>`}${p.today ? ' <span class="md-now">today</span>' : ""}${stale ? ` <span class="md-pstale">${esc(stale)}</span>` : ""}`);
  });
  const today = mdPeriodsToday(list);
  if (today) rows.push(row("Today", esc(today)));
  rows.push(`<div class="pr-note">Set in the app's operating-mode settings, not in the config. <button type="button" class="lnk" data-modes-open>Show on the year</button></div>`);
  return rows;
}
function mdFmt(n) { const [y, m, d] = mdYmd(n); return `${d} ${MD_MONTHS[m - 1].slice(0, 3)} ${y}`; }

// ── rendering ──────────────────────────────────────────────────────────
function modesOpen(opts) {
  if (!S.view) return;
  MD.open = true;
  $("modes").classList.remove("hidden");
  renderModes();
  if (!opts?.quiet) { writeHash(true); $("md-title").focus({ preventScroll: true }); }
}
function modesClose(quiet) {
  if (!MD.open) return;
  MD.open = false;
  $("modes").classList.add("hidden");
  hideTip();
  if (!quiet) writeHash(true);
}
function modesStep(d) { MD.year = Math.max(MD_FIRST_YEAR, Math.min(MD_LAST_YEAR, MD.year + d)); renderModes(); }

function renderModes() {
  if (!MD.open || !S.view) return;
  MD.year = Math.max(MD_FIRST_YEAR, Math.min(MD_LAST_YEAR, MD.year));
  const y = MD.year, modes = mdModes(), { days, entries } = mdYear(y), marks = mdMarks();
  MD.cache = { year: y, days, modes };
  const liveOnNow = mdLiveOn(), today = mdToday();
  // The periods' days this year, unless their chip hides them.
  const periods = mdPeriodList();
  const pdays = (periods || []).filter((p) => p.season.date && !MD.hidden.has("p:" + p.key)).map((p) => ({ key: p.key, title: p.title, days: new Set(entryDays(p.season.date, y)) }));
  MD.pcache = pdays;
  MD.liveKey = mdLiveKey(liveOnNow);
  // Today's modes come from today's year, whichever year is on screen.
  const ty = mdYmd(today)[0];
  const todayKeys = new Set(((ty === y ? days : mdYear(ty).days).get(today)?.on || []).map((e) => e.key));
  $("md-year").textContent = y;
  $("md-today").disabled = y === ty;
  $("md-prev").disabled = y <= MD_FIRST_YEAR;
  $("md-next").disabled = y >= MD_LAST_YEAR;

  // Legend: one chip per mode; click hides or shows its days.
  $("md-legend").innerHTML = [...modes.values()].map((m) => {
    const n = entries.filter((e) => e.key === m.key && e.status !== "removed").length;
    const live = liveOnNow && m.uuid ? liveOnNow.has(m.uuid) : false;
    const why = !liveOnNow ? "" : live ? (todayKeys.has(m.key) ? "On now; the calendar has it today." : "On now, but no calendar entry has it today: logic or the app switched it on.")
      : todayKeys.has(m.key) ? "Off now, although a calendar entry has it today." : "Off now.";
    return `<button type="button" class="md-chip${MD.hidden.has(m.key) ? " off" : ""}${live ? " live" : ""}" data-mode="${esc(m.key)}" aria-pressed="${!MD.hidden.has(m.key)}" title="${esc(`${m.title}: ${mdEntriesN(n, "calendar ")}. Click to ${MD.hidden.has(m.key) ? "show" : "hide"} its days.${why ? "\n" + why : ""}`)}">` +
      `<span class="md-sw" style="background:${m.color}"></span>${esc(m.title)}${live ? '<span class="md-now">on now</span>' : ""}</button>`;
  }).join("") + (liveOnNow ? "" : `<span class="md-hint">${S.live.on ? "Live values show on the current, deployed revision only." : `Turn on live values to see which modes are on now${mdPeriodsAvailable() ? " and the heating and cooling periods" : ""}.`}</span>`)
    + mdPeriodLegend(periods);

  // The year: 12 months, weeks from Monday.
  const shown = (e) => !MD.hidden.has(e.key);
  let html = "";
  for (let m = 1; m <= 12; m++) {
    const first = mdDay(y, m, 1), len = mdDay(m === 12 ? y + 1 : y, m === 12 ? 1 : m + 1, 1) - first;
    const lead = (new Date(first * 864e5).getUTCDay() + 6) % 7;
    let cells = "";
    for (let i = 0; i < lead; i++) cells += '<span class="md-d md-pad"></span>';
    for (let i = 0; i < len; i++) {
      const n = first + i, d = days.get(n), on = (d?.on || []).filter(shown);
      const add = (d?.add || []).filter(shown).length > 0, rem = (d?.rem || []).filter(shown).length > 0;
      const cols = [...new Set(on.map((e) => e.key))].map((k) => modes.get(k)?.color || "#888");
      const bg = cols.length === 1 ? `background:${cols[0]}` : cols.length > 1 ? `background:linear-gradient(to bottom,${cols.map((c, j) => `${c} ${(j * 100) / cols.length}% ${((j + 1) * 100) / cols.length}%`).join(",")})` : "";
      const cls = ["md-d", cols.length ? "md-on" : "", (lead + i) % 7 >= 5 ? "md-we" : "", n === today ? "md-today" : "",
        marks && add && rem ? "md-chg" : marks && add ? "md-add" : marks && rem ? "md-rem" : "",
        S.sel && d && [...d.on, ...d.add, ...d.rem].some((e) => e.uuid === S.sel) ? "md-hl" : "",
        ...pdays.filter((p) => p.days.has(n)).map((p) => "md-" + p.key)].filter(Boolean).join(" ");
      cells += `<span class="${cls}" data-day="${n}"${bg ? ` style="${bg}"` : ""}>${i + 1}</span>`;
    }
    html += `<div class="md-month"><div class="md-mh">${MD_MONTHS[m - 1]}</div><div class="md-wk">${MD_WEEKDAYS.map((w) => `<span>${w}</span>`).join("")}</div><div class="md-days">${cells}</div></div>`;
  }
  $("md-grid").innerHTML = html;
  $("md-grid").setAttribute("aria-label", `Operating modes in ${y}`);

  // The entries: those with days this year first, by their first day.
  const sorted = [...entries].filter((e) => e.status !== "removed" || marks)
    .sort((a, b) => (a.days.length ? a.days[0] : Infinity) - (b.days.length ? b.days[0] : Infinity) || a.title.localeCompare(b.title));
  $("md-count").textContent = entries.length ? sorted.length : "";
  $("md-list").innerHTML = sorted.length ? sorted.map((e) => {
    const mode = modes.get(e.key), c = e.cal;
    const st = marks && e.status ? ` <span class="st ${e.status}">${e.status}</span>` : "";
    const when = c.when == null ? `<span class="none-v">Cannot read: ${esc(c.why || "unknown")}</span>` : marks && c.old_when != null ? `<span class="old">${esc(c.old_when)}</span> <span class="new">${esc(c.when)}</span>` : esc(c.when);
    const modeT = marks && c.old_mode ? `<span class="old">${esc(c.old_mode.title)}</span> <span class="new">${esc(c.mode.title)}</span>` : esc(c.mode.title);
    const inYear = e.days.length ? (e.days.length === 1 ? mdFmt(e.days[0]) : `${plural(e.days.length, "day")} in ${y}`) : c.date ? `not in ${y}` : "";
    return `<li><button type="button" class="md-e${e.uuid === S.sel ? " sel" : ""}${e.days.length ? "" : " none"}${marks && e.status ? " " + e.status : ""}" data-go="${esc(e.uuid)}" aria-current="${e.uuid === S.sel}">` +
      `<span class="md-sw" style="background:${mode?.color || "#888"}"></span><span class="md-et">${esc(e.title)}${st}</span>` +
      `<span class="md-ew">${when}</span><span class="md-em">${modeT}${inYear ? ` · <span class="md-ey">${esc(inYear)}</span>` : ""}</span></button></li>`;
  }).join("") : '<li class="empty">This config has no calendar entries.</li>';
}

/** The legend's periods: a chip each (hides or shows its days), and whether today is in them. */
function mdPeriodLegend(list) {
  if (!mdPeriodsAvailable() || !liveHere()) return "";
  if (!list) return `<span class="md-psep"></span><span class="md-hint">${esc(mdPeriodsPending())}</span>`;
  const chips = list.map((p) => {
    // Hidden and unreadable are separate: a hidden chip always shows again on a click.
    const key = "p:" + p.key, off = MD.hidden.has(key), ok = !!p.season.date, stale = mdPeriodStale(p);
    const tip = ok ? `${p.title}: ${p.season.text} every year, from the Miniserver (set in the app's operating-mode settings).${stale ? ` Read ${stale}.` : ""}${p.today ? " Today is in it." : ""} Click to ${off ? "show" : "hide"} its days.`
      : `${p.title}: ${mdPeriodText(p)}${off ? ". Click to show it again." : ""}`;
    return `<button type="button" class="md-chip md-pchip${off ? " off" : ""}${ok ? "" : " none"}${stale ? " stale" : ""}" data-mode="${key}" aria-pressed="${!off}"${ok || off ? "" : " disabled"} title="${esc(tip)}">` +
      `<span class="md-psw ${p.key}"></span>${esc(p.title)}: ${ok ? esc(p.season.text) : '<span class="none-v">cannot read</span>'}${p.today ? '<span class="md-now">today</span>' : ""}</button>`;
  }).join("");
  const today = mdPeriodsToday(list);
  return `<span class="md-psep"></span>${chips}${today ? `<span class="md-ptoday">${esc(today)}</span>` : ""}`;
}

/** For a tooltip: the day, and every entry of a shown mode on it. */
function mdDayTip(n) {
  const { modes } = MD.cache, marks = mdMarks(), shown = (e) => !MD.hidden.has(e.key);
  const all = MD.cache.days.get(n);
  const d = all && { on: all.on.filter(shown), add: all.add.filter(shown), rem: all.rem.filter(shown) };
  const line = (e, tag) => `<br><span class="md-tsw" style="background:${modes.get(e.key)?.color || "#888"}"></span>${esc(e.title)} · ${esc(modes.get(e.key)?.title || e.cal.mode.title)}${tag ? ` <span class="tm">${tag}</span>` : ""}`;
  let html = `<b>${esc(mdFmt(n))}</b>${n === mdToday() ? ' <span class="tm">today</span>' : ""}`;
  for (const p of MD.pcache || []) if (p.days.has(n)) html += `<br><span class="md-psw ${p.key}"></span>${esc(p.title)}`;
  if (!d || (!d.on.length && !d.rem.length)) return html + '<br><span class="tm">No calendar entry</span>';
  const added = new Set(marks ? d.add.map((e) => e.uuid) : []);
  for (const e of d.on) html += line(e, added.has(e.uuid) ? "added" : "");
  if (marks) for (const e of d.rem) html += line(e, "removed");
  return html;
}

// ── events ─────────────────────────────────────────────────────────────
$("md-prev").onclick = () => modesStep(-1);
$("md-next").onclick = () => modesStep(1);
$("md-today").onclick = () => { MD.year = mdYmd(mdToday())[0]; renderModes(); };
$("md-close").onclick = () => { modesClose(); svg.focus({ preventScroll: true }); };
$("md-legend").addEventListener("click", (e) => {
  const c = e.target.closest("[data-mode]");
  if (!c) return;
  MD.hidden.has(c.dataset.mode) ? MD.hidden.delete(c.dataset.mode) : MD.hidden.add(c.dataset.mode);
  renderModes();
});
// Properties of a climate block: "Show on the year".
$("props").addEventListener("click", (e) => { if (e.target.closest("[data-modes-open]")) modesOpen(); });
$("md-list").addEventListener("click", (e) => {
  const b = e.target.closest("[data-go]");
  if (b) select(b.dataset.go);
});
$("md-grid").addEventListener("mousemove", (e) => {
  const c = e.target.closest("[data-day]");
  if (c && MD.cache) showTip(e, mdDayTip(+c.dataset.day)); else hideTip();
});
$("md-grid").addEventListener("mouseleave", hideTip);
$("md-grid").addEventListener("click", (e) => {
  // A day selects its first entry (one it switches on, else one removed from it).
  const c = e.target.closest("[data-day]");
  const d = c && MD.cache?.days.get(+c.dataset.day);
  const first = d && [...d.on, ...d.rem].find((x) => !MD.hidden.has(x.key));
  if (first) select(first.uuid);
});
/** The window's keys while the view is open (app.js asks first): ← / → step the year, Esc
 *  closes, and the keys that pan or zoom the hidden page do nothing. True when handled. */
function modesKey(e) {
  if (!MD.open) return false;
  switch (e.key) {
    case "ArrowLeft": modesStep(-1); break;
    case "ArrowRight": modesStep(1); break;
    case "Escape": modesClose(); svg.focus({ preventScroll: true }); break;
    case "ArrowUp": case "ArrowDown": case "+": case "=": case "-": case "_": case "0": case "1": break;
    default: return false;
  }
  e.preventDefault();
  return true;
}
