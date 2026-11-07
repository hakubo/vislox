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

use crate::edit::{self, Applied, Ctx, Files, Lock};
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

/// The sets and claims of vislox's file of claims.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Scheds {
    /// Set name → its lines, as lox reads them.
    pub sets: BTreeMap<String, Vec<ScheduleLine>>,
    /// The slug of a timer (an extern of the file, or a block of vislox's
    /// files) → the set it claims.
    pub claims: BTreeMap<String, String>,
}

impl Scheds {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.claims.is_empty()
    }

    /// The timers that claim `set`.
    pub fn claimants(&self, set: &str) -> Vec<&str> {
        self.claims
            .iter()
            .filter(|(_, s)| s.as_str() == set)
            .map(|(t, _)| t.as_str())
            .collect()
    }

    /// Drop the sets no timer claims.
    pub fn prune(&mut self) {
        let used: BTreeSet<String> = self.claims.values().cloned().collect();
        self.sets.retain(|s, _| used.contains(s));
    }

    /// The sets, then the claims, as the file has them.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.is_empty() {
            return out;
        }
        out.push_str(
            "\n# Switching times edited in vislox. A claim replaces the timer's whole schedule;\n\
             # deleting it leaves the timer as the deployed config has it.\n",
        );
        for (name, lines) in &self.sets {
            out.push_str(&format!("schedule {name}\n"));
            for l in lines {
                out.push_str(&format!("\t{l}\n"));
            }
            out.push_str("end\n");
        }
        out.push('\n');
        for (t, s) in &self.claims {
            out.push_str(&format!("{t}.schedule = {s}\n"));
        }
        out
    }
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
    files
        .claims
        .0
        .keys()
        .find(|u| edit::slug(u) == target)
        .cloned()
        .or_else(|| lock.uuid_of(target).map(String::from))
}

/// What `files` give the timer `target`: its set and the set's switching
/// times (`modes` none when they cannot be read against `modes`).
pub fn current(files: &Files, target: &str, modes: &Modes) -> Option<Sched> {
    let s = &files.claims.2;
    let set = s.claims.get(target)?;
    Some(Sched {
        set: Some(set.clone()),
        modes: s.sets.get(set).and_then(|l| modes_of(l, modes).ok()),
        restore: BTreeMap::new(),
    })
}

/// Write `cols` into vislox's set `set`, checked for the timers that claim it
/// (and `also`, about to claim it): an analog one needs a value on each
/// interval, a digital one takes none; all of one kind.
fn write_set(
    files: &mut Files,
    ctx: &Ctx,
    modes: &Modes,
    set: &str,
    cols: Option<&BTreeMap<String, Vec<Span>>>,
    also: Option<(&str, &Element)>,
) -> Result<(), String> {
    // The timer about to claim it first: its kind decides, and names errors.
    let mut timers: Vec<(String, &Element)> =
        also.map(|(t, e)| (t.to_string(), e)).into_iter().collect();
    for t in files.claims.2.claimants(set) {
        if also.is_some_and(|(a, _)| a == t) {
            continue;
        }
        if let Some(u) = timer_of(files, ctx.lock, t)
            && let Some(e) = edit::find(&ctx.doc.xml.root, &u)
        {
            timers.push((t.to_string(), e));
        }
    }
    let title = |e: &Element| e.attr_decoded("Title").unwrap_or_default().into_owned();
    let analog = |e: &Element| e.attr("Analog") == Some("true");
    if let Some(cols) = cols {
        let a = timers.first().is_some_and(|(_, e)| analog(e));
        let first = timers.first().map(|(_, e)| title(e)).unwrap_or_default();
        let cols = checked(cols, a, modes).map_err(|e| format!("{first}: {e}"))?;
        let lines = lines_of(&cols, modes).map_err(|e| format!("{first}: {e}"))?;
        files.claims.2.sets.insert(set.to_string(), lines);
    }
    let lines = files
        .claims
        .2
        .sets
        .get(set)
        .ok_or_else(|| format!("vislox has no set `{set}`; give the switching times"))?;
    let spans: Vec<Span> = modes_of(lines, modes)
        .map_err(|e| format!("`{set}`: {e}"))?
        .into_values()
        .flatten()
        .collect();
    for (_, e) in &timers {
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
    let el = edit::find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    if el.attr("Type") != Some("DayTimer") {
        return Err(format!(
            "{name}: only a DayTimer's switching times are written in Lox Logic; change these in \
             Loxone Config"
        ));
    }
    let target = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some() => s.to_string(),
        Some(_) => {
            return Err(format!(
                "{name}: the module manages this block; change its switching times in its .loxl \
                 file"
            ));
        }
        None => edit::slug(uuid),
    };
    let modes = Modes::new(ctx.doc, ctx.lock);
    let before = files.claims.2.clone();
    let mut previous = current(files, &target, &modes);
    // Sets of vislox's written back first (an undo's).
    if let Some(v) = value {
        for (set, cols) in &v.restore {
            if !files.claims.2.sets.contains_key(set) {
                return Err(format!(
                    "{name}: vislox's set `{set}` is gone; its switching times cannot be put back"
                ));
            }
            write_set(files, ctx, &modes, set, Some(cols), None)?;
        }
    }
    match value {
        Some(v) if v.set.is_some() || v.modes.is_some() => {
            // A timer takes one schedule. Dropping vislox's claim is always
            // let through: it is how a timer the module claims too is mended.
            if let Some(at) = ctx.sched_claimed.get(&uuid.to_ascii_lowercase()) {
                return Err(format!(
                    "{name}: {at} claims its switching times; change them there (a timer takes \
                     one schedule)"
                ));
            }
            let s = &files.claims.2;
            let set = match &v.set {
                Some(n) => n.trim().to_string(),
                None => match s.claims.get(&target) {
                    Some(own) if s.claimants(own) == [target.as_str()] => own.clone(),
                    _ => {
                        let stem = format!("{name} schedule");
                        crate::adopt::default_slug(&stem, "DayTimer", &|x| taken(files, ctx, x))
                    }
                },
            };
            if !s.sets.contains_key(&set) {
                crate::adopt::check_slug(&set).map_err(|e| format!("The set's name: {e}"))?;
                if taken(files, ctx, &set) {
                    return Err(format!(
                        "The set's name: `{set}` is taken in the module; pick another"
                    ));
                }
                if v.modes.is_none() {
                    return Err(format!(
                        "{name}: vislox has no set `{set}`; give the switching times"
                    ));
                }
            }
            write_set(
                files,
                ctx,
                &modes,
                &set,
                v.modes.as_ref(),
                Some((&target, el)),
            )
            .map_err(|e| {
                if e.starts_with(&name) {
                    e
                } else {
                    format!("{name}: {e}")
                }
            })?;
            files.claims.2.claims.insert(target.clone(), set.clone());
            // An extern of the file names a timer Loxone Config drew.
            if target == edit::slug(uuid) {
                let c = files.claims.0.entry(uuid.to_string()).or_default();
                c.ty = "DayTimer".into();
            }
        }
        _ => {
            files.claims.2.claims.remove(&target);
        }
    }
    files.claims.2.prune();
    files.claims.prune();
    // The sets the op rewrote that other timers still claim: their old
    // switching times go into the undo, unless the undo's own set has them.
    let mut restore = BTreeMap::new();
    for (set, lines) in &before.sets {
        let now = files.claims.2.sets.get(set);
        let others = files.claims.2.claimants(set).iter().any(|t| *t != target);
        let own = previous.as_ref().and_then(|p| p.set.as_ref()) == Some(set);
        if now.is_some_and(|n| n != lines) && others && !own {
            let cols = modes_of(lines, &modes).map_err(|e| format!("`{set}`: {e}"))?;
            restore.insert(set.clone(), cols);
        }
    }
    if !restore.is_empty() {
        previous.get_or_insert_with(Sched::default).restore = restore;
    }
    Ok(Applied {
        previous: serde_json::json!(previous),
        added: None,
    })
}

/// Whether a new set may not be named `s`: a name of the module, of the
/// lockfile (its tombstones too) or of vislox's files (its mood sets, #74,
/// among them: the sets share one namespace).
pub(crate) fn taken(files: &Files, ctx: &Ctx, s: &str) -> bool {
    ctx.names.contains(s)
        || ctx.lock.slugs.contains_key(s)
        || ctx.lock.tombstones.contains(s)
        || ctx.lock.externals.contains_key(s)
        || files.names(s)
        || files.claims.2.sets.contains_key(s)
        || files.claims.3.sets.contains_key(s)
        || files.claims.0.keys().any(|u| edit::slug(u) == s)
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
    let mut sets: BTreeSet<&str> = restored.iter().map(String::as_str).collect();
    for u in uuids {
        let target = lock
            .slug_of(u)
            .filter(|t| files.block(t).is_some())
            .map(String::from)
            .unwrap_or_else(|| edit::slug(u));
        if let Some(set) = s.claims.get(&target) {
            sets.insert(set);
        }
    }
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
