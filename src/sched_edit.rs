//! Editing a DayTimer's switching times (#73): the `schedule` op of
//! `POST /api/edit`, and the `schedule … end` sets and `.schedule` claims it
//! writes into vislox's file of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic writes a DayTimer's switching times with a named set and a
//! claim (lox's `language.md`, *schedule*):
//!
//! ```text
//! schedule vx_uhr_schedule
//!     weekdays 06:00-08:00 = 21.5, 17:00-22:00 = 21.5
//!     weekend 08:00-23:00 = 21.5
//!     holiday vacation off
//! end
//!
//! vx_10000020_0000_0200_ffff504f94000001.schedule = vx_uhr_schedule
//! ```
//!
//! - The claim replaces the timer's whole schedule. A weekday the set does not
//!   name is off all day, so vislox always writes all seven (`off` where the
//!   timer is off); another mode the set names gets a column, one it does not
//!   name gets none.
//! - Intervals are whole minutes, `24:00` ends a day, they do not overlap and
//!   do not wrap past midnight. An analog timer needs `= <value>` on each; a
//!   digital timer takes none.
//! - Several timers may claim one set; one timer claims one set.
//! - Dropping the claim leaves the timer as the base has it: before Apply, the
//!   switching times Loxone Config gave it; after, the last ones written.
//!
//! An op names each column by its operating mode's UUID (the `Modes=` UUIDs
//! the file and Properties use); vislox writes it the one way lox takes it: a
//! built-in mode by its day word, a mode the module names by its slug, any
//! other by its quoted title. Columns with the same intervals share a line,
//! with `daily`, `weekdays` and `weekend` where they fit.

use crate::edit::{self, Applied, Claims, Ctx, Files, Lock};
use crate::set_claim;
use loxone_config::LoxoneDoc;
use loxone_config::logic::{Day, Interval, ScheduleLine};
use loxone_config::xml::Element;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The most intervals one column takes (Loxone Config's editor shows far
/// fewer; this only bounds a request).
const MAX_SPANS: usize = 48;

/// One interval, as an op carries it: minutes of the day (`1440` is
/// `24:00`), the value of an analog timer (a decimal number, as text), and
/// whether it needs activation (`activate`, the timer's `Act` input).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: u32,
    pub to: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub activate: bool,
}

/// What a `schedule` op gives a timer.
///
/// - `modes` alone: these switching times. They go into the set the timer
///   claims when no other timer claims it, else into a new set.
/// - `set` and `modes`: write the set `set` (a new one, or vislox's set of
///   that name, for every timer that claims it) and claim it.
/// - `set` alone: claim vislox's set `set` as it is (*Use the same times
///   as …*).
/// - neither: drop the timer's claim, after `restore`.
///
/// `restore` writes other sets of vislox's back first: what an op's
/// `previous` carries when the op rewrote a set other timers share, so that
/// undoing it gives them their switching times back too.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct Sched {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Operating-mode UUID → the intervals of its column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<BTreeMap<String, Vec<Span>>>,
    /// Set name → its switching times, written back before the rest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub restore: BTreeMap<String, BTreeMap<String, Vec<Span>>>,
}

/// The built-in operating modes: their number (the `N` of their fixed UUID)
/// and day word, in the order vislox writes the lines.
const BUILTIN: &[(u8, &str)] = &[
    (4, "mon"),
    (5, "tue"),
    (6, "wed"),
    (7, "thu"),
    (8, "fri"),
    (9, "sat"),
    (10, "sun"),
    (1, "holiday"),
    (2, "vacation"),
    (3, "dayoff"),
    (11, "heating"),
    (12, "cooling"),
];

const WEEKDAYS: [u8; 5] = [4, 5, 6, 7, 8];
const WEEKEND: [u8; 2] = [9, 10];

/// The fixed UUID of built-in mode `n`.
pub fn builtin_uuid(n: u8) -> String {
    format!("00000000-0000-{n:04x}-1500000000000000")
}

fn builtin_of(uuid: &str) -> Option<(u8, &'static str)> {
    BUILTIN
        .iter()
        .find(|(n, _)| builtin_uuid(*n).eq_ignore_ascii_case(uuid))
        .copied()
}

fn is_day_word(w: &str) -> bool {
    BUILTIN.iter().any(|(_, d)| *d == w) || ["weekdays", "weekend", "daily"].contains(&w)
}

/// The operating modes a schedule names, of one config and lockfile.
pub struct Modes {
    /// Mode UUID (lowercase) → its title and `Mode=` number.
    by_uuid: BTreeMap<String, (String, i64)>,
    /// Mode UUID (lowercase) → the slug the module names it by.
    slugs: BTreeMap<String, String>,
}

impl Modes {
    pub fn new(doc: &LoxoneDoc, lock: &Lock) -> Modes {
        fn walk(el: &Element, out: &mut BTreeMap<String, (String, i64)>) {
            if el.name == "C"
                && el.attr("Type") == Some("Mode")
                && let Some(u) = el.attr("U")
            {
                let number = el
                    .attr("Mode")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                let title = el.attr_decoded("Title").unwrap_or_default().into_owned();
                out.insert(u.to_ascii_lowercase(), (title, number));
            }
            for c in el.child_elements() {
                walk(c, out);
            }
        }
        let mut by_uuid = BTreeMap::new();
        walk(&doc.xml.root, &mut by_uuid);
        let mut slugs = BTreeMap::new();
        for (s, u) in lock.slugs.iter().chain(&lock.externals) {
            let u = u.to_ascii_lowercase();
            if by_uuid.contains_key(&u) && !is_day_word(s) && builtin_of(&u).is_none() {
                slugs.insert(u, s.clone());
            }
        }
        Modes { by_uuid, slugs }
    }

    /// How a line names the mode `uuid`.
    fn day(&self, uuid: &str) -> Result<Day, String> {
        let u = uuid.to_ascii_lowercase();
        if let Some((_, w)) = builtin_of(&u) {
            return Ok(Day::Keyword(w.into()));
        }
        let (title, _) = self
            .by_uuid
            .get(&u)
            .ok_or_else(|| format!("the config has no operating mode {uuid}"))?;
        if let Some(s) = self.slugs.get(&u) {
            return Ok(Day::Slug(s.clone()));
        }
        let same = self.by_uuid.values().filter(|(t, _)| t == title).count();
        if same > 1 {
            return Err(format!(
                "{same} operating modes are titled \"{title}\", and lox names a mode by its title: \
                 rename one in Loxone Config"
            ));
        }
        Ok(Day::Title(title.clone()))
    }

    /// The modes a line's day names, by UUID (lowercase).
    fn uuids(&self, day: &Day) -> Result<Vec<String>, String> {
        let builtin = |ns: &[u8]| ns.iter().map(|n| builtin_uuid(*n)).collect();
        Ok(match day {
            Day::Keyword(k) => match k.as_str() {
                "weekdays" => builtin(&WEEKDAYS),
                "weekend" => builtin(&WEEKEND),
                "daily" => builtin(&[4, 5, 6, 7, 8, 9, 10]),
                w => match BUILTIN.iter().find(|(_, d)| *d == w) {
                    Some((n, _)) => vec![builtin_uuid(*n)],
                    None => return Err(format!("unknown day `{w}`")),
                },
            },
            Day::Slug(s) => match self.slugs.iter().find(|(_, x)| *x == s) {
                Some((u, _)) => vec![u.clone()],
                None => return Err(format!("`{s}` names no operating mode the lockfile knows")),
            },
            Day::Title(t) => {
                let found: Vec<&String> = self
                    .by_uuid
                    .iter()
                    .filter(|(_, (x, _))| x == t)
                    .map(|(u, _)| u)
                    .collect();
                match found.as_slice() {
                    [u] => vec![(*u).clone()],
                    [] => return Err(format!("the config has no operating mode \"{t}\"")),
                    _ => return Err(format!("several operating modes are titled \"{t}\"")),
                }
            }
        })
    }

    /// Where the mode `uuid`'s line goes: the weekdays, the other built-in
    /// modes, then the config's own modes by number.
    fn order(&self, uuid: &str) -> (u8, i64, String) {
        match builtin_of(uuid) {
            Some((n, _)) => (
                0,
                BUILTIN.iter().position(|(m, _)| *m == n).unwrap_or(0) as i64,
                String::new(),
            ),
            None => {
                let (t, n) = self.by_uuid.get(uuid).cloned().unwrap_or_default();
                (1, n, t)
            }
        }
    }
}

/// Every column of `lines`, by mode UUID (lowercase), with all seven
/// weekdays (one the set does not name is off).
pub fn modes_of(
    lines: &[ScheduleLine],
    modes: &Modes,
) -> Result<BTreeMap<String, Vec<Span>>, String> {
    let mut out: BTreeMap<String, Vec<Span>> = BTreeMap::new();
    for l in lines {
        for d in &l.days {
            for u in modes.uuids(d)? {
                out.entry(u)
                    .or_default()
                    .extend(l.intervals.iter().map(|iv| Span {
                        from: iv.from.into(),
                        to: iv.to.into(),
                        value: iv.value.clone(),
                        activate: iv.activate,
                    }));
            }
        }
    }
    for n in 4..=10 {
        out.entry(builtin_uuid(n)).or_default();
    }
    for v in out.values_mut() {
        v.sort_by_key(|s| (s.from, s.to));
    }
    Ok(out)
}

use crate::schedule::clock as hm;

/// Check the switching times an op gives a timer (`analog`: it takes a value
/// on each interval), and put them in order: by mode UUID (lowercase), each
/// column by time, all seven weekdays.
pub fn checked(
    given: &BTreeMap<String, Vec<Span>>,
    analog: bool,
    modes: &Modes,
) -> Result<BTreeMap<String, Vec<Span>>, String> {
    let mut out: BTreeMap<String, Vec<Span>> = BTreeMap::new();
    for (uuid, spans) in given {
        let u = uuid.trim().to_ascii_lowercase();
        let day = modes.day(&u)?;
        if out.contains_key(&u) {
            return Err(format!("{day} is given twice"));
        }
        if spans.len() > MAX_SPANS {
            return Err(format!("{day}: at most {MAX_SPANS} intervals"));
        }
        let mut v = Vec::new();
        for s in spans {
            let at = format!("{day} {}-{}", hm(s.from), hm(s.to));
            if s.to > 1440 {
                return Err(format!("{at}: a day ends at 24:00"));
            }
            if s.from >= s.to {
                return Err(format!(
                    "{at}: it ends before it starts. Split an interval across midnight into two \
                     (22:00-24:00, and 00:00-06:00 on the next day)"
                ));
            }
            let value = match (&s.value, analog) {
                (Some(x), true) => Some(edit::number(x).map_err(|e| format!("{at}: {e}"))?),
                (None, true) => {
                    return Err(format!(
                        "{at}: the timer is analog, so each interval needs a value"
                    ));
                }
                (Some(x), false) if x.trim() != "1" => {
                    return Err(format!(
                        "{at}: the timer is digital, so an interval is only on and takes no value"
                    ));
                }
                (_, false) => None,
            };
            v.push(Span {
                from: s.from,
                to: s.to,
                value,
                activate: s.activate,
            });
        }
        v.sort_by_key(|s| (s.from, s.to));
        for w in v.windows(2) {
            if w[1].from < w[0].to {
                return Err(format!(
                    "{day}: {}-{} and {}-{} overlap",
                    hm(w[0].from),
                    hm(w[0].to),
                    hm(w[1].from),
                    hm(w[1].to)
                ));
            }
        }
        out.insert(u, v);
    }
    for n in 4..=10 {
        out.entry(builtin_uuid(n)).or_default();
    }
    Ok(out)
}

/// The lines of a set: the columns with the same intervals on one line, in
/// the order of [`BUILTIN`] and then the config's modes; seven weekdays
/// alike are `daily`, Monday to Friday `weekdays`, Saturday and Sunday
/// `weekend`.
pub fn lines_of(
    cols: &BTreeMap<String, Vec<Span>>,
    modes: &Modes,
) -> Result<Vec<ScheduleLine>, String> {
    let mut order: Vec<&String> = cols.keys().collect();
    order.sort_by_key(|u| modes.order(u));
    // The groups of columns with the same intervals, in the order of their
    // first column.
    let mut groups: Vec<(&Vec<Span>, Vec<&String>)> = Vec::new();
    for u in order {
        let spans = &cols[u];
        match groups.iter_mut().find(|(s, _)| *s == spans) {
            Some((_, us)) => us.push(u),
            None => groups.push((spans, vec![u])),
        }
    }
    let mut lines = Vec::new();
    for (spans, us) in groups {
        let numbers: Vec<u8> = us
            .iter()
            .filter_map(|u| builtin_of(u))
            .map(|b| b.0)
            .collect();
        let has = |ns: &[u8]| ns.iter().all(|n| numbers.contains(n));
        // A group's word stands where its first day would.
        let daily = has(&[4, 5, 6, 7, 8, 9, 10]);
        let (weekdays, weekend) = (!daily && has(&WEEKDAYS), !daily && has(&WEEKEND));
        let mut days: Vec<Day> = Vec::new();
        let mut push = |d: Day| {
            if !days.contains(&d) {
                days.push(d);
            }
        };
        for u in us {
            match builtin_of(u) {
                Some((n, _)) if daily && (4..=10).contains(&n) => {
                    push(Day::Keyword("daily".into()))
                }
                Some((n, _)) if weekdays && WEEKDAYS.contains(&n) => {
                    push(Day::Keyword("weekdays".into()))
                }
                Some((n, _)) if weekend && WEEKEND.contains(&n) => {
                    push(Day::Keyword("weekend".into()))
                }
                Some((_, w)) => push(Day::Keyword(w.into())),
                None => push(modes.day(u)?),
            }
        }
        lines.push(ScheduleLine {
            days,
            intervals: spans
                .iter()
                .map(|s| Interval {
                    from: s.from as u16,
                    to: s.to as u16,
                    value: s.value.clone(),
                    activate: s.activate,
                })
                .collect(),
            comment: None,
        });
    }
    Ok(lines)
}

/// The timer a slug of vislox's files names: an extern of the file of
/// claims (`vx_<uuid>`), or a block vislox added or adopted.
pub fn timer_of(files: &Files, lock: &Lock, target: &str) -> Option<String> {
    set_claim::member_of(files, lock, target)
}

/// The switching times vislox's file writes: its `schedule` sets and claims.
pub type Scheds = set_claim::SetStore<SchedKind>;

/// Switching times as a kind of set claim ([`set_claim`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SchedKind;

impl set_claim::SetKind for SchedKind {
    type Line = ScheduleLine;
    type Content = BTreeMap<String, Vec<Span>>;
    type Val = Sched;
    const KEYWORD: &'static str = "schedule";
    const BLOCK: &'static str = "DayTimer";
    const WHAT: &'static str = "switching times";
    const STEM: &'static str = "schedule";
    const HEADER: &'static str = "# Switching times edited in vislox. A claim replaces the timer's whole schedule;\n\
         # deleting it leaves the timer as the deployed config has it.\n";
    const CONTENT_KEY: &'static str = "modes";
    const MEMBERS_KEY: &'static str = "timers";

    fn store(c: &Claims) -> &Scheds {
        &c.2
    }
    fn store_mut(c: &mut Claims) -> &mut Scheds {
        &mut c.2
    }
    fn claimed_elsewhere<'a>(ctx: &'a Ctx) -> &'a BTreeMap<String, String> {
        ctx.sched_claimed
    }
    fn wrong_type(name: &str) -> String {
        format!(
            "{name}: only a DayTimer's switching times are written in Lox Logic; change these in \
             Loxone Config"
        )
    }
    fn parts(v: &Sched) -> set_claim::Parts<'_, Self> {
        (v.set.as_ref(), v.modes.as_ref(), &v.restore)
    }
    fn make(
        set: Option<String>,
        modes: Option<Self::Content>,
        restore: BTreeMap<String, Self::Content>,
    ) -> Sched {
        Sched {
            set,
            modes,
            restore,
        }
    }
    fn content_of(lines: &[ScheduleLine], doc: &LoxoneDoc, lock: &Lock) -> Option<Self::Content> {
        modes_of(lines, &Modes::new(doc, lock)).ok()
    }
    /// The timer about to claim it first: its kind (analog or digital)
    /// decides, and names the errors.
    fn lines_of(
        cols: &Self::Content,
        ctx: &Ctx,
        timers: &[(String, &Element)],
    ) -> Result<Vec<ScheduleLine>, String> {
        let modes = Modes::new(ctx.doc, ctx.lock);
        let a = timers.first().is_some_and(|(_, e)| analog(e));
        let first = timers.first().map(|(_, e)| title(e)).unwrap_or_default();
        let cols = checked(cols, a, &modes).map_err(|e| format!("{first}: {e}"))?;
        lines_of(&cols, &modes).map_err(|e| format!("{first}: {e}"))
    }
    /// An analog timer needs a value on each interval, a digital one takes
    /// none: every timer on a set is of one kind.
    fn check(
        lines: &[ScheduleLine],
        ctx: &Ctx,
        set: &str,
        timers: &[(String, &Element)],
    ) -> Result<(), String> {
        let spans: Vec<Span> = modes_of(lines, &Modes::new(ctx.doc, ctx.lock))
            .map_err(|e| format!("`{set}`: {e}"))?
            .into_values()
            .flatten()
            .collect();
        for (_, e) in timers {
            if analog(e) && spans.iter().any(|s| s.value.is_none()) {
                return Err(format!(
                    "{} is analog and shares `{set}`, whose intervals have no value",
                    title(e)
                ));
            }
            if !analog(e) && spans.iter().any(|s| s.value.is_some()) {
                return Err(format!(
                    "{} is digital and shares `{set}`, whose intervals have values",
                    title(e)
                ));
            }
        }
        Ok(())
    }
}

fn title(e: &Element) -> String {
    e.attr_decoded("Title").unwrap_or_default().into_owned()
}

fn analog(e: &Element) -> bool {
    e.attr("Analog") == Some("true")
}

/// Apply a `schedule` op: give the DayTimer `uuid` the switching times
/// `value`, or drop vislox's claim on them (`None`). Its `previous` undoes it
/// whole: the timer's claim as it was, and the switching times of every
/// other timer whose set the op rewrote (`restore`).
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&Sched>,
) -> Result<Applied, String> {
    set_claim::apply::<SchedKind>(files, ctx, uuid, value)
}

/// A schedule as the compiled config has it, comparable with a set's:
/// column (lowercase UUID) → (from, to, value, activate), the value only on
/// an analog timer.
type Compiled = BTreeMap<String, Vec<(u32, u32, Option<u64>, bool)>>;

fn compiled_of(el: &Element, uuid: &str) -> Option<(bool, Compiled)> {
    let t = crate::schedule::read(el, uuid, |_| None)?;
    let cols: BTreeSet<&str> = t
        .modes
        .iter()
        .map(String::as_str)
        .chain(t.entries.iter().map(|e| e.col.as_str()))
        .collect();
    let out = cols
        .into_iter()
        .map(|c| {
            (
                c.to_ascii_lowercase(),
                crate::schedule::col_key(&t.entries, c, t.analog),
            )
        })
        .collect();
    Some((t.analog, out))
}

fn expected_of(cols: &BTreeMap<String, Vec<Span>>, analog: bool) -> Compiled {
    cols.iter()
        .map(|(u, spans)| {
            let mut v: Vec<_> = spans
                .iter()
                .map(|s| {
                    let value = s.value.as_deref().and_then(|x| x.parse::<f64>().ok());
                    (
                        s.from,
                        s.to,
                        analog.then(|| value.unwrap_or(1.0).to_bits()),
                        s.activate,
                    )
                })
                .collect();
            v.sort();
            (u.clone(), v)
        })
        .collect()
}

/// After the compile: the timers among `uuids`, those sharing a set with
/// them, and those on the sets `restored` (an undo wrote them back), whose
/// switching times in `out` are not their set's. lox writes a claim exactly;
/// this catches a claim it did not write at all.
pub fn missed(
    files: &Files,
    out: &LoxoneDoc,
    lock: &Lock,
    uuids: &[String],
    restored: &[String],
) -> Vec<String> {
    let modes = Modes::new(out, lock);
    let s = &files.claims.2;
    let sets = set_claim::sets_written::<SchedKind>(files, lock, uuids, restored);
    let mut bad = Vec::new();
    for set in sets {
        let Some(cols) = s.sets.get(set).and_then(|l| modes_of(l, &modes).ok()) else {
            continue;
        };
        for t in s.claimants(set) {
            let Some(u) = timer_of(files, lock, t) else {
                continue;
            };
            let Some(el) = edit::find(&out.xml.root, &u) else {
                continue;
            };
            let title = el.attr_decoded("Title").unwrap_or_default().into_owned();
            match compiled_of(el, &u) {
                Some((analog, got)) if got == expected_of(&cols, analog) => {}
                _ => bad.push(title),
            }
        }
    }
    bad
}

/// Whether the timer `uuid` has other switching times in `a` than in `b`
/// (or is in only one of them).
pub fn differs(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    let of = |d: &LoxoneDoc| edit::find(&d.xml.root, uuid).and_then(|e| compiled_of(e, uuid));
    of(a) != of(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(from: u32, to: u32, value: Option<&str>) -> Span {
        Span {
            from,
            to,
            value: value.map(Into::into),
            activate: false,
        }
    }

    fn no_modes() -> Modes {
        Modes {
            by_uuid: BTreeMap::new(),
            slugs: BTreeMap::new(),
        }
    }

    fn text(lines: &[ScheduleLine]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn columns_alike_share_a_line_with_the_languages_day_groups() {
        let m = no_modes();
        let mut cols = BTreeMap::new();
        for n in 4..=8 {
            cols.insert(
                builtin_uuid(n),
                vec![span(360, 480, None), span(1020, 1350, None)],
            );
        }
        for n in [9, 10] {
            cols.insert(builtin_uuid(n), vec![span(480, 1380, None)]);
        }
        cols.insert(builtin_uuid(1), vec![]);
        cols.insert(builtin_uuid(2), vec![]);
        let lines = lines_of(&checked(&cols, false, &m).unwrap(), &m).unwrap();
        assert_eq!(
            text(&lines),
            [
                "weekdays 06:00-08:00, 17:00-22:30",
                "weekend 08:00-23:00",
                "holiday vacation off"
            ]
        );
        // And back: the same columns.
        assert_eq!(
            modes_of(&lines, &m).unwrap(),
            checked(&cols, false, &m).unwrap()
        );
    }

    #[test]
    fn all_seven_weekdays_alike_are_daily_and_a_weekday_left_out_is_off() {
        let m = no_modes();
        let all: BTreeMap<String, Vec<Span>> = (4..=10)
            .map(|n| (builtin_uuid(n), vec![span(0, 1440, Some("21.5"))]))
            .collect();
        let lines = lines_of(&checked(&all, true, &m).unwrap(), &m).unwrap();
        assert_eq!(text(&lines), ["daily 00:00-24:00 = 21.5"]);
        // Monday only: the other six days are written as off.
        let mon = BTreeMap::from([(builtin_uuid(4), vec![span(60, 120, Some("1"))])]);
        let mut c = checked(&mon, true, &m).unwrap();
        c.get_mut(&builtin_uuid(4)).unwrap()[0].activate = true;
        assert_eq!(
            text(&lines_of(&c, &m).unwrap()),
            [
                "mon 01:00-02:00 = 1 activate",
                "tue wed thu fri weekend off"
            ]
        );
    }

    #[test]
    fn the_language_rules_are_checked_before_lox_sees_them() {
        let m = no_modes();
        let one = |s: Span| BTreeMap::from([(builtin_uuid(4), vec![s])]);
        let err = |c: &BTreeMap<String, Vec<Span>>, analog| checked(c, analog, &m).unwrap_err();
        assert!(err(&one(span(1320, 360, None)), false).contains("Split an interval"));
        assert!(err(&one(span(0, 1441, None)), false).contains("24:00"));
        assert!(err(&one(span(0, 60, None)), true).contains("needs a value"));
        assert!(err(&one(span(0, 60, Some("2"))), false).contains("digital"));
        assert!(err(&one(span(0, 60, Some("2,5"))), true).contains("decimal point"));
        // A digital timer's "1" is on: written without a value.
        let c = checked(&one(span(0, 60, Some("1"))), false, &m).unwrap();
        assert_eq!(c[&builtin_uuid(4)][0].value, None);
        let two = BTreeMap::from([(
            builtin_uuid(4),
            vec![span(0, 120, None), span(60, 180, None)],
        )]);
        assert!(err(&two, false).contains("overlap"));
        // Touching is fine.
        let touch = BTreeMap::from([(
            builtin_uuid(4),
            vec![span(60, 180, None), span(0, 60, None)],
        )]);
        assert_eq!(
            checked(&touch, false, &m).unwrap()[&builtin_uuid(4)][0].from,
            0
        );
        // A mode the config does not have.
        let party = BTreeMap::from([("10000011-0000-0110-ffff504f94000001".to_string(), vec![])]);
        assert!(err(&party, false).contains("no operating mode"));
    }
}
