//! Operating modes and calendar entries from vislox (#86). The design is in
//! `docs/editing.md`.
//!
//! Both are objects of the object tree ([`crate::periphery`]): a `Mode`
//! goes into the *Betriebsmodi* folder (`ModeCaption`), and lox mints its
//! `Mode=` number (one below the lowest in use); a `CalendarEntry` goes into
//! the *Betriebszeiten* folder (`CalendarCaption`). vislox declares them in
//! its periphery file:
//!
//! ```text
//! vx_mode_1 = Mode("Party")
//! vx_calendarentry_1 = CalendarEntry("Silvester", switches: vx_mode_1, every: 12-31)
//! vx_calendarentry_2 = CalendarEntry("Ferien", switches: vacation, from: 2026-07-04, to: 2026-09-05)
//! ```
//!
//! An entry's mode and date are two values of the block, set by the
//! `entry_mode` and `entry_date` ops (and as `switches` and `date` of its
//! `add_block`):
//!
//! - the mode is its UUID, in every op and every `previous`; it is written
//!   the one way lox takes it ([`crate::sched_edit::Modes::spell`]: a
//!   built-in mode by its day word, a mode the module names by its slug, any
//!   other by its quoted title);
//! - the date is written as the source writes it, one of four kinds:
//!   `every: 12-31`, `easter: 1`, `from: 2026-07-04, to: 2026-09-05` or
//!   `from: 10-01, to: 04-30` ([`DateArgs`]). The same text is read back from
//!   the compiled config ([`date_of`]), so whether an edit is deployed
//!   compares the date, not its wording.
//!
//! Entries Loxone Config made, and the module's own, are not edited here.

use crate::sched_edit::Modes;
use loxone_config::LoxoneDoc;
use loxone_config::logic::Value;
use loxone_config::xml::Element;

/// The type of an operating mode, and of a calendar entry.
pub const MODE: &str = "Mode";
pub const ENTRY: &str = crate::calendar::ENTRY;

/// The argument that names the mode an entry switches on, and the setting
/// of an `add_block` that carries the mode's UUID.
pub const SWITCHES: &str = "switches";
/// The setting of an `add_block` that carries the date as the source writes it.
pub const DATE: &str = "date";

const EVERY: &str = "every";
const EASTER: &str = "easter";
const FROM: &str = "from";
const TO: &str = "to";
/// The arguments that state an entry's date, in the order they are written.
const DATE_KEYS: &[&str] = &[EVERY, EASTER, FROM, TO];

/// The first day a period may start on: Loxone counts periods from it.
const FIRST_YEAR: i32 = 2009;
/// The last year a period may reach (the year view's last).
const LAST_YEAR: i32 = 9999;
/// The most days an entry may lie before or after Easter Sunday.
const MAX_EASTER: i64 = 366;

/// How an entry states its date, for messages.
const USAGE: &str = "every: 12-31 (a day every year), easter: 1 (days after Easter Sunday), \
                     from: 2026-07-04, to: 2026-09-05 (one period) or from: 10-01, to: 04-30 \
                     (a range every year)";

/// An entry's date, checked: one of the four kinds the language writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateArgs {
    Every {
        month: u32,
        day: u32,
    },
    Easter(i64),
    Period {
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    },
    Yearly {
        from: (u32, u32),
        to: (u32, u32),
    },
}

/// `MM-DD`, exactly, a day of some year (29 February is).
fn month_day(s: &str) -> Option<(u32, u32)> {
    let (m, d) = s.split_once('-')?;
    let two = |p: &str| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit());
    if !two(m) || !two(d) {
        return None;
    }
    let (m, d): (u32, u32) = (m.parse().ok()?, d.parse().ok()?);
    chrono::NaiveDate::from_ymd_opt(2000, m, d).map(|_| (m, d))
}

/// `YYYY-MM-DD`, exactly, a day of the calendar.
fn full_date(s: &str) -> Option<chrono::NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !s
        .bytes()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

impl DateArgs {
    /// Read a date as the source writes it (`every: 12-31`, `easter: -2`,
    /// `from: …, to: …`), checked as lox checks it, and a period also to
    /// end on or after its start.
    pub fn parse(text: &str) -> Result<DateArgs, String> {
        let mut parts: Vec<(&str, &str)> = Vec::new();
        for p in text.split(',') {
            let (k, v) = p
                .split_once(':')
                .ok_or_else(|| format!("the date is written {USAGE}"))?;
            parts.push((k.trim(), v.trim()));
        }
        match parts.as_slice() {
            [(EVERY, v)] => {
                if full_date(v).is_some() {
                    return Err(format!(
                        "every: is a day every year, without the year. For that one day, write \
                         from: {v}, to: {v}"
                    ));
                }
                let (month, day) = month_day(v).ok_or_else(|| {
                    format!("every: {v} is no day of the year (MM-DD, like 12-31)")
                })?;
                Ok(DateArgs::Every { month, day })
            }
            [(EASTER, v)] => {
                let n: i64 = v.parse().map_err(|_| {
                    format!("easter: {v} is no whole number of days after Easter Sunday")
                })?;
                if !(-MAX_EASTER..=MAX_EASTER).contains(&n) {
                    return Err(format!(
                        "easter: {n} is more than {MAX_EASTER} days from Easter Sunday"
                    ));
                }
                Ok(DateArgs::Easter(n))
            }
            [(FROM, f), (TO, t)] => {
                match (month_day(f), month_day(t), full_date(f), full_date(t)) {
                    (Some(from), Some(to), _, _) => Ok(DateArgs::Yearly { from, to }),
                    (_, _, Some(from), Some(to)) => {
                        use chrono::Datelike;
                        if from.year() < FIRST_YEAR {
                            return Err(format!(
                                "from: {f}: a period starts on {FIRST_YEAR}-01-01 or later"
                            ));
                        }
                        if to.year() > LAST_YEAR {
                            return Err(format!(
                                "to: {t}: a period ends in {LAST_YEAR} at the latest"
                            ));
                        }
                        if to < from {
                            return Err(format!("to: {t} is before from: {f}"));
                        }
                        Ok(DateArgs::Period { from, to })
                    }
                    _ => {
                        // A day written as one but none of the calendar (02-30).
                        let shaped = |s: &str| {
                            s.len() == 5 && s.as_bytes()[2] == b'-'
                                || s.len() == 10 && s.as_bytes()[4] == b'-'
                        };
                        for (k, v) in [(FROM, f), (TO, t)] {
                            if shaped(v) && month_day(v).is_none() && full_date(v).is_none() {
                                return Err(format!("{k}: {v} is no day of the year"));
                            }
                        }
                        Err(format!(
                            "from: {f}, to: {t}: both are dates (one period: from: 2026-07-04, to: \
                         2026-09-05) or both days of the year (every year: from: 10-01, to: 04-30)"
                        ))
                    }
                }
            }
            _ => Err(format!("the date is written {USAGE}")),
        }
    }

    /// The arguments it is written with.
    pub fn args(&self) -> Vec<(String, Value)> {
        let md = |(m, d): (u32, u32)| Value::Date(format!("{m:02}-{d:02}"));
        let ymd = |d: chrono::NaiveDate| Value::Date(d.format("%Y-%m-%d").to_string());
        match *self {
            DateArgs::Every { month, day } => vec![(EVERY.into(), md((month, day)))],
            DateArgs::Easter(n) => vec![(EASTER.into(), Value::Number(n.to_string()))],
            DateArgs::Period { from, to } => {
                vec![(FROM.into(), ymd(from)), (TO.into(), ymd(to))]
            }
            DateArgs::Yearly { from, to } => vec![(FROM.into(), md(from)), (TO.into(), md(to))],
        }
    }

    /// As the source writes it: `from: 10-01, to: 04-30`.
    pub fn text(&self) -> String {
        self.args()
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The same date as the config stores it, decoded by [`crate::calendar`].
    fn of_stored(d: &crate::calendar::EntryDate) -> DateArgs {
        use crate::calendar::EntryDate as E;
        match *d {
            E::Fixed { month, day } => DateArgs::Every { month, day },
            E::Easter { offset } => DateArgs::Easter(offset),
            E::Period { from, to } => DateArgs::Period { from, to },
            E::Yearly {
                month,
                day,
                month2,
                day2,
            } => DateArgs::Yearly {
                from: (month, day),
                to: (month2, day2),
            },
        }
    }
}

/// The date the arguments of a block of vislox's files state, as the
/// source writes it; `None` when they state none (or one vislox does not
/// read).
pub fn date_text(args: &[(String, Value)]) -> Option<String> {
    let text = args
        .iter()
        .filter(|(k, _)| DATE_KEYS.contains(&k.as_str()))
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    (!text.is_empty())
        .then(|| DateArgs::parse(&text).ok().map(|d| d.text()))
        .flatten()
}

/// Whether `key` is an argument of an entry's mode or date.
pub fn is_entry_arg(key: &str) -> bool {
    key == SWITCHES || DATE_KEYS.contains(&key)
}

/// Replace the date of an entry's arguments with `date`, after its
/// `switches:` (or first, without one).
pub fn set_date(args: &mut Vec<(String, Value)>, date: Option<&DateArgs>) {
    args.retain(|(k, _)| !DATE_KEYS.contains(&k.as_str()));
    if let Some(d) = date {
        let at = args
            .iter()
            .position(|(k, _)| k == SWITCHES)
            .map_or(0, |i| i + 1);
        for (i, a) in d.args().into_iter().enumerate() {
            args.insert(at + i, a);
        }
    }
}

/// Set `switches:` of an entry's arguments (first), or drop it.
pub fn set_mode(args: &mut Vec<(String, Value)>, v: Option<Value>) {
    args.retain(|(k, _)| k != SWITCHES);
    if let Some(v) = v {
        args.insert(0, (SWITCHES.to_string(), v));
    }
}

/// The mode `switches:` names, by UUID (lowercase), in `modes`.
pub fn mode_uuid(modes: &Modes, args: &[(String, Value)]) -> Option<String> {
    let v = args.iter().find(|(k, _)| k == SWITCHES).map(|(_, v)| v)?;
    modes.uuid_of(v)
}

/// The UUID of the operating mode numbered `n` in `doc` (the first in
/// document order), else of the built-in mode with that number.
fn mode_numbered(doc: &LoxoneDoc, n: i64) -> Option<String> {
    fn walk(el: &Element, n: i64) -> Option<String> {
        if el.name == "C" && el.attr("Type") == Some(MODE) {
            let m = el
                .attr("Mode")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0);
            if m == n {
                return el.attr("U").map(str::to_ascii_lowercase);
            }
        }
        el.child_elements().find_map(|c| walk(c, n))
    }
    walk(&doc.xml.root, n).or_else(|| {
        u8::try_from(n)
            .ok()
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= 12)
            .map(crate::sched_edit::builtin_uuid)
    })
}

/// The mode the entry `el` of `doc` switches on, by UUID (lowercase), as
/// its `CalType=` numbers it (absent: 0, the holiday mode).
pub fn mode_of(doc: &LoxoneDoc, el: &Element) -> Option<String> {
    let n = crate::calendar::mode_number(el.attr(crate::calendar::MODE_ATTR))?;
    mode_numbered(doc, n)
}

/// The date the entry `el` stores, as the source writes it; `None` for a
/// date the language cannot write (a weekday in a month).
pub fn date_of(el: &Element) -> Option<String> {
    let d = crate::calendar::EntryDate::from_attrs(|a| el.attr(a)).ok()?;
    Some(DateArgs::of_stored(&d).text())
}

/// The value of `switches:` for the mode `uuid` of the pending config.
pub fn spell(modes: &Modes, uuid: &str) -> Result<Value, String> {
    modes.spell(uuid.trim())
}

/// The `entry_mode` or `entry_date` op on the entry `b` of vislox's files,
/// named `name`: its `previous` (the mode's UUID, the date's text).
pub fn edit(
    ctx: &crate::edit::Ctx,
    name: &str,
    b: &mut crate::blocks::NewBlock,
    op: &crate::edit::Op,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    use crate::edit::Op;
    if b.ty != ENTRY {
        return Err(format!(
            "{name}: only a calendar entry switches a mode on a date"
        ));
    }
    let modes = Modes::new(ctx.doc, ctx.lock);
    match op {
        Op::EntryMode { .. } => {
            let previous = mode_uuid(&modes, &b.args);
            let v = value
                .map(|u| spell(&modes, u).map_err(|e| format!("{name}: {e}")))
                .transpose()?;
            set_mode(&mut b.args, v);
            Ok(previous)
        }
        Op::EntryDate { .. } => {
            let previous = date_text(&b.args);
            let d = value
                .map(|t| DateArgs::parse(t).map_err(|e| format!("{name}: {e}")))
                .transpose()?;
            set_date(&mut b.args, d.as_ref());
            Ok(previous)
        }
        _ => unreachable!("an entry's mode or date"),
    }
}

/// The arguments of a new entry from its `add_block`'s settings: `switches`
/// (the mode's UUID) and `date` (as the source writes it), both needed.
pub fn new_args(
    ctx: &crate::edit::Ctx,
    settings: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(k) = settings.keys().find(|k| *k != SWITCHES && *k != DATE) {
        return Err(format!(
            "a calendar entry takes its mode ({SWITCHES}) and its date ({DATE}), not {k}"
        ));
    }
    let mode = settings
        .get(SWITCHES)
        .filter(|m| !m.trim().is_empty())
        .ok_or("a new calendar entry needs the operating mode it switches on")?;
    let date = settings
        .get(DATE)
        .filter(|d| !d.trim().is_empty())
        .ok_or("a new calendar entry needs its date")?;
    let modes = Modes::new(ctx.doc, ctx.lock);
    let mut args = Vec::new();
    set_mode(&mut args, Some(spell(&modes, mode)?));
    set_date(&mut args, Some(&DateArgs::parse(date)?));
    Ok(args)
}

/// An `add_block`'s settings that add the entry `b` again: its mode by
/// UUID, its date as written. `None` where it names a mode no more.
pub fn again_settings(
    ctx: &crate::edit::Ctx,
    b: &crate::blocks::NewBlock,
) -> std::collections::BTreeMap<String, String> {
    let modes = Modes::new(ctx.doc, ctx.lock);
    let mut out = std::collections::BTreeMap::new();
    if let Some(u) = mode_uuid(&modes, &b.args) {
        out.insert(SWITCHES.to_string(), u);
    }
    if let Some(d) = date_text(&b.args) {
        out.insert(DATE.to_string(), d);
    }
    out
}

/// Why the entry `uuid` is not edited here: the module declares it, or
/// Loxone Config made it, or it is no calendar entry.
pub fn not_ours(files: &crate::edit::Files, ctx: &crate::edit::Ctx, uuid: &str) -> String {
    let Some(el) = crate::edit::find_ci(&ctx.doc.xml.root, uuid) else {
        return format!("no object {uuid} in the pending config");
    };
    let name = el
        .attr_decoded("Title")
        .map(|t| t.into_owned())
        .unwrap_or_default();
    if el.attr("Type") != Some(ENTRY) {
        return format!("{name}: only a calendar entry switches a mode on a date");
    }
    match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_none() => {
            format!("{name}: the module declares this entry (`{s}`); change it in its .loxl file")
        }
        _ => format!(
            "{name}: Loxone Config made this entry; vislox edits the entries added here (change \
             it in Loxone Config, or take it over with `lox logic adopt --uuid {uuid}`)"
        ),
    }
}

/// For each calendar entry of `doc`: whether its mode and date are edited
/// here (`null`), or why not (`GET /api/edits`' `entries`).
pub fn edit_refusals(
    files: &crate::edit::Files,
    ctx: &crate::edit::Ctx,
) -> serde_json::Map<String, serde_json::Value> {
    fn walk<'a>(el: &'a Element, out: &mut Vec<&'a Element>) {
        if el.name == "C" && el.attr("Type") == Some(ENTRY) {
            out.push(el);
        }
        for c in el.child_elements() {
            walk(c, out);
        }
    }
    let mut all = Vec::new();
    walk(&ctx.doc.xml.root, &mut all);
    all.into_iter()
        .filter_map(|el| {
            let u = el.attr("U")?;
            let ours = ctx
                .lock
                .slug_of(u)
                .is_some_and(|s| files.block(s).is_some());
            let why = if !ours {
                Some(not_ours(files, ctx, u))
            } else if date_of(el).is_none() {
                Some("Its date cannot be read; undo the edit that wrote it".to_string())
            } else {
                None
            };
            Some((u.to_string(), serde_json::json!(why)))
        })
        .collect()
}

/// What an entry of vislox's files states, for `GET /api/edits`' block:
/// its mode by UUID and its date as written (`null` where none).
pub fn listed(modes: &Modes, b: &crate::blocks::NewBlock) -> Option<serde_json::Value> {
    (b.ty == ENTRY).then(|| {
        serde_json::json!({
            "mode": mode_uuid(modes, &b.args),
            "date": date_text(&b.args),
        })
    })
}

/// The entries of vislox's files that switch the mode `slug` on.
pub fn switched_by(files: &crate::edit::Files, slug: &str) -> Vec<String> {
    files
        .pages
        .values()
        .flat_map(|p| p.blocks.iter())
        .filter(|b| {
            b.ty == ENTRY
                && matches!(b.args.iter().find(|(k, _)| k == SWITCHES), Some((_, Value::Ref(s))) if s == slug)
        })
        .map(|b| b.title.clone().unwrap_or_else(|| b.slug.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_lox_reads_them() {
        let t = |s: &str| DateArgs::parse(s).map(|d| d.text());
        assert_eq!(t("every: 12-31").unwrap(), "every: 12-31");
        assert_eq!(t(" every :02-29").unwrap(), "every: 02-29");
        assert_eq!(t("easter: -2").unwrap(), "easter: -2");
        assert_eq!(t("easter: +1").unwrap(), "easter: 1");
        assert_eq!(
            t("from: 2026-07-04, to: 2026-09-05").unwrap(),
            "from: 2026-07-04, to: 2026-09-05"
        );
        assert_eq!(
            t("from: 10-01, to: 04-30").unwrap(),
            "from: 10-01, to: 04-30"
        );
        // A one-day period.
        assert!(t("from: 2026-12-24, to: 2026-12-24").is_ok());
        let e = t("every: 2026-12-24").unwrap_err();
        assert!(e.contains("from: 2026-12-24, to: 2026-12-24"), "{e}");
        assert!(t("every: 2-3").is_err());
        assert!(t("every: 02-30").is_err());
        assert!(t("every: 13-01").is_err());
        let e = t("from: 2008-12-31, to: 2009-01-05").unwrap_err();
        assert!(e.contains("2009-01-01"), "{e}");
        assert!(t("from: 2026-09-05, to: 2026-07-04").is_err());
        assert!(t("from: 2026-07-04, to: 09-05").is_err());
        assert!(
            t("from: 02-30, to: 03-01")
                .unwrap_err()
                .contains("02-30 is no day of the year")
        );
        assert!(
            t("from: 2026-02-30, to: 2026-03-01")
                .unwrap_err()
                .contains("no day")
        );
        assert!(t("from: 10-01").is_err());
        assert!(t("easter: 1.5").is_err());
        assert!(t("easter: 400").is_err());
        assert!(t("easter: 99999999999999999999").is_err());
        assert!(t("every: 12-31, easter: 1").is_err());
        assert!(t("").is_err());
        assert!(t("on: 12-31").is_err());
    }

    #[test]
    fn the_date_goes_after_the_mode() {
        let mut args = vec![
            (SWITCHES.to_string(), Value::Ref("holiday".into())),
            (EVERY.to_string(), Value::Date("12-31".into())),
        ];
        set_date(
            &mut args,
            Some(&DateArgs::parse("from: 10-01, to: 04-30").unwrap()),
        );
        let keys: Vec<&str> = args.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, [SWITCHES, FROM, TO]);
        assert_eq!(date_text(&args).unwrap(), "from: 10-01, to: 04-30");
        set_mode(&mut args, None);
        set_mode(&mut args, Some(Value::Str("Urlaub".into())));
        assert_eq!(args[0].0, SWITCHES);
        set_date(&mut args, None);
        assert_eq!(date_text(&args), None);
    }
}
