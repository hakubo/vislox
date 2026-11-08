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
const MD_DATE_FORMAT = new Intl.DateTimeFormat("en", { timeZone: "UTC", weekday: "long", year: "numeric", month: "long", day: "numeric" });

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

function renderModes(focusDay) {
  if (!MD.open || !S.view) return;
  const gridHadDayFocus = $("md-grid").contains(document.activeElement) && document.activeElement.matches("[data-day]");
  if (focusDay != null) MD.focusDay = focusDay;
  else if (gridHadDayFocus) MD.focusDay = +document.activeElement.dataset.day;
  mfSync();
  mdRenderEdit();
  MD.year = Math.max(MD_FIRST_YEAR, Math.min(MD_LAST_YEAR, MD.year));
  const y = MD.year;
  if (MD.focusDay == null || mdYmd(MD.focusDay)[0] !== y) MD.focusDay = mdYmd(mdToday())[0] === y ? mdToday() : mdDay(y, 1, 1);
  const modes = mdModes(), { days, entries } = mdYear(y), marks = mdMarks();
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
  }).join("") + (liveOnNow ? "" : `<span class="md-hint">${S.live.on ? "Live values show on the deployed revision only (what the Miniserver runs, not an edit on disk)." : `Turn on live values to see which modes are on now${mdPeriodsAvailable() ? " and the heating and cooling periods" : ""}.`}</span>`)
    + mdPeriodLegend(periods);

  // The year: 12 months, weeks from Monday. The days of the entry being drafted are outlined (#86).
  const shown = (e) => !MD.hidden.has(e.key), draft = mfDays(y);
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
      const dayPeriods = pdays.filter((p) => p.days.has(n));
      const cls = ["md-d", cols.length ? "md-on" : "", (lead + i) % 7 >= 5 ? "md-we" : "", n === today ? "md-today" : "",
        marks && add && rem ? "md-chg" : marks && add ? "md-add" : marks && rem ? "md-rem" : "",
        S.sel && d && [...d.on, ...d.add, ...d.rem].some((e) => e.uuid === S.sel) ? "md-hl" : "",
        draft.has(n) ? "md-draft" : "", MF.open && MF.what === "entry" ? "md-pickable" : "",
        ...dayPeriods.map((p) => "md-" + p.key)].filter(Boolean).join(" ");
      const onEntries = (d?.on || []).filter(shown), removed = marks ? (d?.rem || []).filter(shown) : [];
      const added = new Set(marks ? (d?.add || []).map((e) => e.uuid) : []);
      const describe = (e, tag = "") => `${e.title} (${modes.get(e.key)?.title || e.cal.mode.title}${tag ? `, ${tag}` : ""})`;
      const details = [...onEntries.map((e) => describe(e, added.has(e.uuid) ? "added" : "")), ...removed.map((e) => describe(e, "removed")), ...dayPeriods.map((p) => p.title)];
      const date = MD_DATE_FORMAT.format(new Date(n * 864e5));
      const aria = `${date}. ${details.length ? `Modes and periods: ${details.join(", ")}.` : "No operating mode or period."}`;
      const tab = MD.focusDay === n ? 0 : -1;
      cells += `<button type="button" class="${cls}" data-day="${n}" tabindex="${tab}" aria-label="${esc(aria)}"${bg ? ` style="${bg}"` : ""}>${i + 1}</button>`;
    }
    html += `<div class="md-month"><div class="md-mh">${MD_MONTHS[m - 1]}</div><div class="md-wk">${MD_WEEKDAYS.map((w) => `<span>${w}</span>`).join("")}</div><div class="md-days">${cells}</div></div>`;
  }
  $("md-grid").innerHTML = html;
  $("md-grid").setAttribute("aria-label", `Operating modes in ${y}`);
  if (gridHadDayFocus) {
    hideTip();
    MD.restoringFocus = true;
    try { $("md-grid").querySelector(`[data-day="${MD.focusDay}"]`)?.focus({ preventScroll: true }); }
    finally { MD.restoringFocus = false; }
  }

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
    // Edit (#86): offered on each entry the server names; disabled with why on those not edited here.
    const est = e.status !== "removed" ? mdEntryState(e.uuid) : null;
    const edit = est ? `<button type="button" class="ed-btn small md-edit" data-md-edit="${esc(e.uuid)}"${est.ok && !MF.busy ? "" : " disabled"} title="${esc(est.ok ? `Edit ${e.title}: its name, mode and date` : est.why)}" aria-label="Edit ${esc(e.title)}">Edit</button>` : "";
    return `<li class="md-li">${edit}<button type="button" class="md-e${e.uuid === S.sel ? " sel" : ""}${e.days.length ? "" : " none"}${marks && e.status ? " " + e.status : ""}" data-go="${esc(e.uuid)}" aria-current="${e.uuid === S.sel}">` +
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
$("md-grid").addEventListener("focusin", (e) => {
  const c = e.target.closest("[data-day]");
  if (!c || !MD.cache || MD.restoringFocus) return;
  MD.focusDay = +c.dataset.day;
  const r = c.getBoundingClientRect();
  showTip({ target: c, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }, mdDayTip(MD.focusDay));
});
$("md-grid").addEventListener("focusout", (e) => { if (!e.relatedTarget?.closest?.("[data-day]")) hideTip(); });
$("md-grid").addEventListener("keydown", (e) => {
  const c = e.target.closest("[data-day]");
  if (!c) return;
  const day = +c.dataset.day, weekday = (new Date(day * 864e5).getUTCDay() + 6) % 7;
  let next = null;
  switch (e.key) {
    case "ArrowLeft": next = day - 1; break;
    case "ArrowRight": next = day + 1; break;
    case "ArrowUp": next = day - 7; break;
    case "ArrowDown": next = day + 7; break;
    case "Home": next = day - weekday; break;
    case "End": next = day + 6 - weekday; break;
    case "PageUp": case "PageDown": {
      const [y, m, d] = mdYmd(day), ny = y + (e.key === "PageUp" ? -1 : 1);
      if (ny >= MD_FIRST_YEAR && ny <= MD_LAST_YEAR) next = mdDate(ny, m, d) ?? mdDay(ny, m + 1, 1) - 1;
      break;
    }
    case "Escape": e.preventDefault(); e.stopPropagation(); modesClose(); svg.focus({ preventScroll: true }); return;
    case "Enter": case " ":
      if (MF.open && MF.what === "entry") {
        e.preventDefault(); e.stopPropagation();
        if (e.shiftKey && MF.keyboardAnchor != null) mfPick(MF.keyboardAnchor, day);
        else { mfPick(day, day); MF.keyboardAnchor = day; }
        return;
      }
      return;
    case "+": case "=": case "-": case "_": case "0": case "1": e.preventDefault(); e.stopPropagation(); return;
    default: return;
  }
  e.preventDefault(); e.stopPropagation();
  if (next == null) return;
  const [year] = mdYmd(next);
  if (year < MD_FIRST_YEAR || year > MD_LAST_YEAR) return;
  MD.year = year;
  renderModes(next);
});
$("md-grid").addEventListener("click", (e) => {
  // A day selects its first entry (one it switches on, else one removed from it).
  // While an entry is drafted, a day is picked for it instead (#86).
  const c = e.target.closest("[data-day]");
  if (MF.open && MF.what === "entry") {
    if (c && e.detail === 0) {
      if (e.shiftKey && MF.keyboardAnchor != null) mfPick(MF.keyboardAnchor, +c.dataset.day);
      else { mfPick(+c.dataset.day, +c.dataset.day); MF.keyboardAnchor = +c.dataset.day; }
    }
    return;
  }
  const d = c && MD.cache?.days.get(+c.dataset.day);
  const first = d && [...d.on, ...d.rem].find((x) => !MD.hidden.has(x.key));
  if (first) select(first.uuid);
});
/** The window's keys while the view is open (app.js asks first): ← / → step the year outside the date grid,
 *  Esc closes, and the keys that pan or zoom the hidden page do nothing. True when handled. */
function modesKey(e) {
  if (!MD.open || e.target.closest?.("[data-day]")) return false;
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

// ── adding modes, and adding or editing calendar entries (#86) ─────────
// "Add mode" makes a Mode in the Operating Modes folder (lox mints its Mode= number); "Add entry" and an entry's
// "Edit" make or change a CalendarEntry: its name, the mode it switches on and its date, one of four kinds. Days
// are typed, or picked on the year: click a day, or drag across days for a range. The draft's days are outlined
// on the year. Saving sends `add_block` (with `switches`, the mode's UUID, and `date`, as the source writes it),
// or `title`, `entry_mode` and `entry_date` ops: compiled, never deployed. Which folders take new objects and
// which entries are edited here, and why not, come from the server (`add_under`, `entries` of GET /api/edits).
const MF = { open: false, what: null, uuid: null, base: null, draft: {}, orig: null, bad: {}, busy: false, err: "", msg: "", drag: null };
const MF_KINDS = [["every", "Every year on a day"], ["easter", "Relative to Easter"], ["period", "One period"], ["yearly", "A range every year"]];
const MF_MD = /^\d{2}-\d{2}$/, MF_YMD = /^\d{4}-\d{2}-\d{2}$/;
function mdModeTitle(u) { const m = (S.view?.modes || []).find((x) => ciEq(x.uuid, u)); return m ? m.title : u; }
function mdPad(n) { return String(n).padStart(2, "0"); }
function mdMdText(n) { const [, m, d] = mdYmd(n); return `${mdPad(m)}-${mdPad(d)}`; }
function mdYmdText(n) { const [y, m, d] = mdYmd(n); return `${String(y).padStart(4, "0")}-${mdPad(m)}-${mdPad(d)}`; }
/** The folder of type `ty` that takes new objects here (GET /api/edits' `add_under`), by UUID. */
function mdFolder(ty) { return Object.keys(S.edits?.add_under || {}).find((u) => nodeById(u)?.ty === ty) || null; }
/** Whether modes and entries can be edited now: `{ ok, rev }`, or `{ why }`; null: editing is not offered. */
function mdEditState() {
  const a = S.hist?.actions;
  if (!a?.shown || !S.edits || !S.view) return null;
  if (!a.edit?.available) return { why: `Editing is off: ${a.edit?.reason || "unknown"}.` };
  const r = editRev();
  if (!r) return { why: "The config on disk is not in the History yet." };
  if (S.rev !== r.id) return { why: `Modes and calendar entries are edited on the newest config (${r.kind === "pending" ? "Pending" : "deployed"}).` };
  if (a.running) return { why: `${a.running} is running.` };
  if (UNDO.busy) return { why: "Undo or redo is compiling." };
  return { ok: true, rev: r };
}
/** Whether a mode (`ModeCaption`) or an entry (`CalendarCaption`) can be added: `{ ok, rev, folder }` or `{ why }`. */
function mdAddState(ty) {
  const st = mdEditState();
  if (!st) return null;
  if (!st.ok) return st;
  const f = mdFolder(ty);
  if (!f) return { why: `This config has no ${ty === "ModeCaption" ? "Operating Modes" : "calendar"} folder vislox can add to.` };
  if (S.edits.add_under[f]) return { why: S.edits.add_under[f] };
  return { ...st, folder: f };
}
/** Whether the entry `uuid` can be edited: `{ ok, rev }` or `{ why }`; null: the server says nothing of it. */
function mdEntryState(uuid) {
  const st = mdEditState();
  if (!st) return null;
  const k = Object.keys(S.edits.entries || {}).find((u) => ciEq(u, uuid));
  if (k === undefined) return null;
  if (S.edits.entries[k]) return { why: S.edits.entries[k] };
  return st;
}
function mfState() { return MF.what === "mode" ? mdAddState("ModeCaption") : MF.uuid ? mdEntryState(MF.uuid) : mdAddState("CalendarCaption"); }

/** A date as the source writes it → the form's fields. */
function mfDateDraft(text) {
  const t = text || "";
  let m;
  if ((m = /^every: (\d{2}-\d{2})$/.exec(t))) return { kind: "every", every: m[1] };
  if ((m = /^easter: (-?\d+)$/.exec(t))) return { kind: "easter", easter: m[1] };
  if ((m = /^from: (\d{4}-\d{2}-\d{2}), to: (\d{4}-\d{2}-\d{2})$/.exec(t))) return { kind: "period", from: m[1], to: m[2] };
  if ((m = /^from: (\d{2}-\d{2}), to: (\d{2}-\d{2})$/.exec(t))) return { kind: "yearly", yfrom: m[1], yto: m[2] };
  const today = mdToday();
  return { kind: "every", every: mdMdText(today) };
}
/** The form's date as the source writes it (`text`), or what is wrong with a field (`bad`). The server checks it again. */
function mfDateText(d) {
  const v = (k) => (d[k] || "").trim();
  switch (d.kind) {
    case "every": return MF_MD.test(v("every")) ? { text: `every: ${v("every")}` } : { bad: { every: "A day of the year, MM-DD, like 12-31." } };
    case "easter": return /^[-+]?\d+$/.test(v("easter")) ? { text: `easter: ${+v("easter")}` } : { bad: { easter: "Whole days after Easter Sunday, like 1, or before it, like -2." } };
    case "period": {
      const bad = {};
      if (!MF_YMD.test(v("from"))) bad.from = "A date, YYYY-MM-DD.";
      if (!MF_YMD.test(v("to"))) bad.to = "A date, YYYY-MM-DD.";
      return Object.keys(bad).length ? { bad } : { text: `from: ${v("from")}, to: ${v("to")}` };
    }
    case "yearly": {
      const bad = {};
      if (!MF_MD.test(v("yfrom"))) bad.yfrom = "A day of the year, MM-DD.";
      if (!MF_MD.test(v("yto"))) bad.yto = "A day of the year, MM-DD.";
      return Object.keys(bad).length ? { bad } : { text: `from: ${v("yfrom")}, to: ${v("yto")}` };
    }
  }
  return { bad: { kind: "Pick the kind of date." } };
}
/** The form's date as the year lays it out (`calendar.date`), or null while it does not read. */
function mfDate(d) {
  const r = mfDateText(d);
  if (!r.text) return null;
  const md = (s) => s.split("-").map(Number);
  switch (d.kind) {
    case "every": { const [month, day] = md(d.every.trim()); return { kind: "fixed", month, day }; }
    case "easter": return { kind: "easter", offset: +d.easter.trim() };
    case "period": return { kind: "period", from: d.from.trim(), to: d.to.trim() };
    case "yearly": { const [month, day] = md(d.yfrom.trim()), [month2, day2] = md(d.yto.trim()); return { kind: "yearly", month, day, month2, day2 }; }
  }
  return null;
}

/** Open the form: a new mode (`what` "mode"), a new entry, or the entry `uuid`. */
function mfOpen(what, uuid) {
  MF.what = what; MF.uuid = uuid || null;
  const st = mfState();
  if (!st?.ok) { MF.what = null; MF.uuid = null; return; }
  let draft = { title: "" };
  if (what === "entry" && uuid) {
    const o = S.view.objects[uuid], e = editsBlock(uuid)?.entry || {};
    // A mode that no longer resolves stays as written (`switches`), shown as unknown.
    draft = { title: o?.title || "", mode: e.mode || e.switches || "", ...mfDateDraft(e.date) };
  } else if (what === "entry") {
    const modes = S.view.modes || [];
    const holiday = modes.find((m) => ciEq(m.uuid, seUuid(1)));
    draft = { title: "", mode: (holiday || modes[0])?.uuid || "", ...mfDateDraft("") };
  }
  Object.assign(MF, { open: true, base: st.rev.blob, draft, orig: { ...draft, date: mfDateText(draft).text }, bad: {}, busy: false, err: "", msg: "", drag: null, keyboardAnchor: null });
  mfRender();
  renderModes();
  $("md-form").querySelector("[data-mf]")?.focus();
}
function mfClose(msg) {
  Object.assign(MF, { open: false, what: null, uuid: null, base: null, draft: {}, orig: null, bad: {}, busy: false, err: "", drag: null, msg: msg || "" });
  mfRender();
  if (MD.open) renderModes();
}
/** A draft belongs to the pending config it was opened on, and an entry's to that entry while it is selected. */
function mfSync() {
  if (!MF.open || MF.busy) return;
  const r = editRev();
  if (!r || r.blob !== MF.base) mfClose("The pending config changed: the form was closed. Open it again to edit the newest.");
  else if (MF.uuid && !ciEq(S.sel, MF.uuid)) mfClose();
}

function mfRender() {
  const box = $("md-form");
  if (!MF.open) {
    box.classList.toggle("hidden", !MF.msg);
    box.innerHTML = MF.msg ? `<div class="ed-msg" role="status">${esc(MF.msg)}</div>` : "";
    return;
  }
  box.classList.remove("hidden");
  const d = MF.draft, dis = MF.busy ? " disabled" : "";
  const field = (k, label, ph, hint) => {
    const id = `mf-${k}`, bad = MF.bad[k];
    return `<div class="pr ed-row"><label class="k" for="${id}">${esc(label)}</label><div class="v"><input id="${id}" data-mf="${k}" value="${esc(d[k] ?? "")}" placeholder="${esc(ph)}" title="${esc(hint)}" autocomplete="off" spellcheck="false"${bad ? ` aria-invalid="true" aria-describedby="${id}-e"` : ""}${dis}>${bad ? `<div class="ed-bad" id="${id}-e">${esc(bad)}</div>` : ""}</div></div>`;
  };
  let html;
  if (MF.what === "mode") {
    html = `<h4 id="md-form-h">Add an operating mode</h4>${field("title", "Name", "Party", "The mode's name; lox numbers it")}`;
  } else {
    const modes = S.view?.modes || [];
    // A mode the config does not have (or none) is shown as such, never as the first mode.
    const known = modes.some((m) => ciEq(m.uuid, d.mode));
    const opts = (known ? "" : `<option value="${esc(d.mode || "")}" selected>${d.mode ? `(unknown: ${esc(d.mode)})` : "(none)"}</option>`) +
      modes.map((m) => `<option value="${esc(m.uuid)}"${ciEq(m.uuid, d.mode) ? " selected" : ""}>${esc(m.title)}</option>`).join("");
    const kinds = MF_KINDS.map(([k, t]) => `<option value="${k}"${d.kind === k ? " selected" : ""}>${esc(t)}</option>`).join("");
    const dates = {
      every: field("every", "Day", "12-31", "Month and day, MM-DD: every year on that day"),
      easter: field("easter", "Days after Easter", "1", "Days after Easter Sunday (1: Easter Monday), or before it (-2: Good Friday)"),
      period: field("from", "From", "2026-07-04", "The first day, YYYY-MM-DD (2009 or later)") + field("to", "To", "2026-09-05", "The last day, YYYY-MM-DD"),
      yearly: field("yfrom", "From", "10-01", "The first day every year, MM-DD") + field("yto", "To", "04-30", "The last day every year, MM-DD; earlier than From runs across New Year"),
    }[d.kind] || "";
    html = `<h4 id="md-form-h">${MF.uuid ? `Edit ${esc(MF.orig.title || "the entry")}` : "Add a calendar entry"}</h4>${field("title", "Name", "Silvester", "The entry's name")}
      <div class="pr ed-row"><label class="k" for="mf-mode">Switches on</label><div class="v"><select id="mf-mode" data-mf="mode"${dis}${MF.bad.mode ? ' aria-invalid="true"' : ""}>${opts}</select>${MF.bad.mode ? `<div class="ed-bad">${esc(MF.bad.mode)}</div>` : ""}</div></div>
      <div class="pr ed-row"><label class="k" for="mf-kind">Date</label><div class="v"><select id="mf-kind" data-mf="kind"${dis}>${kinds}</select></div></div>${dates}`;
  }
  const note = MF.what === "mode" ? "Adding compiles the module; nothing is deployed." : "Click a day on the year, or drag across days for a range. Saving compiles the module; nothing is deployed.";
  html += `<div class="ed-bar"><button type="button" class="primary small" data-mf-act="save"${dis}>${MF.busy ? "Compiling…" : MF.uuid ? "Save" : "Add"}</button><button type="button" class="ed-btn small" data-mf-act="cancel"${dis}>Cancel</button>
    <span class="ed-msg${MF.err ? " err" : ""}" role="status" aria-live="polite">${esc(MF.err || note)}</span></div>`;
  box.innerHTML = html;
}

async function mfSave() {
  const st = mfState();
  if (!st?.ok || MF.busy || !MF.open) return;
  const d = MF.draft, bad = {}, title = (d.title || "").trim();
  if (!title) bad.title = "A name.";
  let ops = [], label;
  if (MF.what === "mode") {
    ops = [{ op: "add_block", page: st.folder, type: "Mode", title, params: {}, settings: {} }];
    label = `Add the mode ${title}`;
  } else {
    const date = mfDateText(d);
    Object.assign(bad, date.bad || {});
    if (!d.mode) bad.mode = "Pick the mode it switches on.";
    if (!MF.uuid) {
      ops = [{ op: "add_block", page: st.folder, type: "CalendarEntry", title, params: {}, settings: { switches: d.mode, date: date.text } }];
      label = `Add the calendar entry ${title}`;
    } else {
      if (title !== (MF.orig.title || "").trim()) ops.push({ op: "title", uuid: MF.uuid, value: title });
      if (!ciEq(d.mode, MF.orig.mode)) ops.push({ op: "entry_mode", uuid: MF.uuid, value: d.mode });
      if (date.text && date.text !== MF.orig.date) ops.push({ op: "entry_date", uuid: MF.uuid, value: date.text });
      label = `Calendar entry ${MF.orig.title || title}`;
    }
  }
  MF.bad = bad; MF.err = "";
  if (Object.keys(bad).length) { MF.err = "Fix the marked values."; mfRender(); $("md-form").querySelector('[aria-invalid="true"]')?.focus(); return; }
  if (!ops.length) { MF.err = "Nothing changed."; mfRender(); return; }
  MF.busy = true; mfRender();
  try {
    const res = await editAct(MF.base, ops, label);
    const added = res.added?.[0]?.uuid, keep = MF.uuid;
    MF.busy = false;
    mfClose(`${MF.what === "mode" ? "Added" : keep ? "Saved" : "Added"} and compiled: pending, not deployed. Review it, then Apply or Discard.`);
    await editShowNewest(res.blob);
    const u = added && Object.keys(S.view?.objects || {}).find((k) => ciEq(k, added));
    if (u && S.view.objects[u].calendar) select(u, { noReveal: true });
    if (MD.open) renderModes();
  } catch (err) {
    MF.busy = false; MF.err = err.message;
    mfRender();
  }
}

/** The days the draft covers in year `y`, and those a drag is picking. */
function mfDays(y) { return MF.open && MF.what === "entry" ? new Set(entryDays(mfDate(MF.draft), y)) : new Set(); }
function mfPaint() {
  if (!MD.open) return;
  const draft = mfDays(MD.year), dr = MF.drag, lo = dr ? Math.min(dr.a, dr.b) : 0, hi = dr ? Math.max(dr.a, dr.b) : -1;
  for (const c of $("md-grid").querySelectorAll("[data-day]")) {
    const n = +c.dataset.day;
    c.classList.toggle("md-draft", draft.has(n));
    c.classList.toggle("md-pick", n >= lo && n <= hi);
  }
}
/** Days `a` to `b` picked on the year: one day, or a range (one period, or every year for a yearly range). */
function mfPick(a, b) {
  const lo = Math.min(a, b), hi = Math.max(a, b), d = MF.draft, [y] = mdYmd(lo);
  if (lo === hi) {
    if (d.kind === "easter") { const [m, dd] = easterSunday(y); d.easter = String(lo - mdDay(y, m, dd)); }
    else if (d.kind === "period") { d.from = d.to = mdYmdText(lo); }
    else if (d.kind === "yearly") { d.yfrom = d.yto = mdMdText(lo); }
    else { d.kind = "every"; d.every = mdMdText(lo); }
  } else if (d.kind === "yearly") { d.yfrom = mdMdText(lo); d.yto = mdMdText(hi); }
  else { d.kind = "period"; d.from = mdYmdText(lo); d.to = mdYmdText(hi); }
  MF.bad = {}; MF.err = "";
  mfRender();
  mfPaint();
}

/** What the year and the entries' Edit buttons follow of GET /api/edits: the entries, where to add, each entry's values. */
function mdEditKey() {
  const e = S.edits;
  return e ? JSON.stringify([e.entries || null, e.add_under || null, (e.blocks || []).filter((b) => b.entry).map((b) => [b.uuid, b.entry])]) : "";
}
function mdRenderEdit() {
  for (const [id, ty, what] of [["md-add-mode", "ModeCaption", "a mode"], ["md-add-entry", "CalendarCaption", "a calendar entry"]]) {
    const st = mdAddState(ty), b = $(id);
    b.classList.toggle("hidden", !st);
    b.disabled = !st?.ok || MF.busy;
    b.title = !st ? "" : st.ok ? `Add ${what} (it is compiled, not deployed)` : st.why;
  }
}

$("md-add-mode").onclick = () => mfOpen("mode");
$("md-add-entry").onclick = () => mfOpen("entry");
$("md-form").addEventListener("input", (e) => {
  const f = e.target.closest("[data-mf]");
  if (!f || !MF.open) return;
  MF.draft[f.dataset.mf] = f.value;
  if (f.dataset.mf === "kind") {
    // The fields of the other kind, from the date picked so far where they can.
    const n = mdToday(), d = MF.draft;
    if (d.kind === "every") d.every ||= mdMdText(n);
    if (d.kind === "easter") d.easter ||= "1";
    if (d.kind === "period") { d.from ||= mdYmdText(n); d.to ||= d.from; }
    if (d.kind === "yearly") { d.yfrom ||= mdMdText(n); d.yto ||= d.yfrom; }
    mfRender();
  }
  mfPaint();
});
$("md-form").addEventListener("click", (e) => {
  const b = e.target.closest("[data-mf-act]");
  if (b?.dataset.mfAct === "save") mfSave();
  else if (b?.dataset.mfAct === "cancel") mfClose();
});
$("md-form").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.tagName === "INPUT") { e.preventDefault(); mfSave(); }
  else if (e.key === "Escape" && !MF.busy) { e.preventDefault(); e.stopPropagation(); mfClose(); }
});
$("md-list").addEventListener("click", (e) => {
  const b = e.target.closest("[data-md-edit]");
  if (!b) return;
  e.stopPropagation();
  select(b.dataset.mdEdit, { noReveal: true });
  mfOpen("entry", b.dataset.mdEdit);
});
$("md-grid").addEventListener("mousedown", (e) => {
  const c = e.target.closest("[data-day]");
  if (!c || e.button !== 0 || !MF.open || MF.what !== "entry" || MF.busy) return;
  e.preventDefault();
  MF.keyboardAnchor = null;
  MF.drag = { a: +c.dataset.day, b: +c.dataset.day };
  mfPaint();
});
$("md-grid").addEventListener("mouseover", (e) => {
  const c = MF.drag && e.target.closest("[data-day]");
  if (c && +c.dataset.day !== MF.drag.b) { MF.drag.b = +c.dataset.day; mfPaint(); }
});
window.addEventListener("mouseup", () => {
  if (!MF.drag) return;
  const { a, b } = MF.drag;
  MF.drag = null;
  mfPick(a, b);
});
