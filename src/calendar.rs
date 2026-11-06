//! Operating-mode calendar entries (`CalendarEntry`, under the Calendar
//! folder): the date an entry stores, decoded from the `.Loxone` file's
//! attributes, and how it reads (*Every year on 24 Dec*, *Easter Monday
//! (Easter +1)*, *4 Jul 2026 – 5 Sep 2026*, *Every year 1 Oct – 30 Apr*).
//!
//! The file stores an entry differently from the web-service API that
//! Loxone's Operating Mode Schedule datasheet describes. lox's
//! `loxone-format.md` ("Calendar entries") documents the file, and is
//! followed here:
//!
//! - `Mode=` is the kind of date: absent a fixed date every year
//!   (`Month`/`Day`), `1` relative to Easter Sunday (`EasterOffset` days,
//!   absent 0), `3` one period (`From`/`To`, days since 2009-01-01), `4` a
//!   range every year (`Month`/`Day` to `Month2`/`Day2`). No other value
//!   occurs in lox's corpus; anything else is left undecoded.
//! - `CalType=` is the `Mode=` number of the operating mode the entry
//!   switches on (absent: 0, the holiday mode).
//!
//! The decoded date is serialized for the browser as well, so that views
//! of the year (which days a mode is on) can work from it without
//! decoding the attributes again.
//!
//! The Miniserver's heating and cooling periods are not in the file: they
//! are set in the app's operating-mode settings, and read with
//! `jdev/sps/calendargetheatperiod` / `calendargetcoolperiod` (the
//! datasheet's "Commands"), which answer `MM-DD/MM-DD`, e.g. `10-15/04-15`.
//! [`Season`] decodes such an answer into a range every year.

use chrono::{Days, NaiveDate};
use loxone_config::xml::Element;
use serde::Serialize;
use std::collections::HashMap;

/// The block type of a calendar entry.
pub const ENTRY: &str = "CalendarEntry";

/// The attribute that names the operating mode by its `Mode=` number.
pub const MODE_ATTR: &str = "CalType";

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days of Easter with a name of their own, by their offset from Easter
/// Sunday.
const EASTER_DAYS: &[(i64, &str)] = &[
    (-47, "Shrove Tuesday"),
    (-46, "Ash Wednesday"),
    (-7, "Palm Sunday"),
    (-3, "Maundy Thursday"),
    (-2, "Good Friday"),
    (-1, "Holy Saturday"),
    (0, "Easter Sunday"),
    (1, "Easter Monday"),
    (39, "Ascension Day"),
    (49, "Whit Sunday"),
    (50, "Whit Monday"),
    (60, "Corpus Christi"),
];

/// The date of a calendar entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EntryDate {
    /// One day every year.
    Fixed { month: u32, day: u32 },
    /// Days after Easter Sunday (before it when negative).
    Easter { offset: i64 },
    /// One period, both days included. `to` may lie before `from`: lox's
    /// corpus has two such entries.
    Period { from: NaiveDate, to: NaiveDate },
    /// A range every year, from `month`/`day` to `month2`/`day2`; it runs
    /// across New Year when the end comes before the start.
    Yearly {
        month: u32,
        day: u32,
        month2: u32,
        day2: u32,
    },
}

/// Day 0 of a period's `From=`/`To=`.
fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(2009, 1, 1).expect("valid date")
}

/// Whether `month`/`day` is a day of some year (29 Feb is).
fn is_day_of_year(month: i64, day: i64) -> bool {
    let (Ok(m), Ok(d)) = (u32::try_from(month), u32::try_from(day)) else {
        return false;
    };
    NaiveDate::from_ymd_opt(2000, m, d).is_some()
}

fn day_month(month: u32, day: u32) -> String {
    format!("{day} {}", MONTHS[month as usize - 1])
}

fn full(d: NaiveDate) -> String {
    d.format("%-d %b %Y").to_string()
}

/// Reads the whole numbers a date is stored in, by field name. The file's
/// attributes and the API's fields share it, so that both validate alike:
/// a difference between them would read as a change made in the app.
struct Fields<G: Fn(&str) -> Option<String>>(G);

impl<G: Fn(&str) -> Option<String>> Fields<G> {
    /// `absent`: the value when the field is missing; `None` makes that an error.
    fn int(&self, name: &str, absent: Option<i64>) -> Result<i64, String> {
        match (self.0)(name) {
            Some(v) => v
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("{name} is “{v}”, not a whole number")),
            None => absent.ok_or_else(|| format!("it has no {name}")),
        }
    }

    fn day_of_year(&self, m: &str, d: &str) -> Result<(u32, u32), String> {
        let (month, day) = (self.int(m, None)?, self.int(d, None)?);
        if is_day_of_year(month, day) {
            Ok((month as u32, day as u32))
        } else {
            Err(format!("{m} {month} and {d} {day} are no day of the year"))
        }
    }

    /// A day counted from 1 Jan 2009, as the file's `From`/`To`.
    fn period_day(&self, name: &str) -> Result<NaiveDate, String> {
        let n = self.int(name, None)?;
        u64::try_from(n)
            .ok()
            .and_then(|n| epoch().checked_add_days(Days::new(n)))
            .ok_or_else(|| format!("{name} {n} is no day from 1 Jan 2009 on"))
    }

    /// A date from its year, month and day, as the API's.
    fn date(&self, y: &str, m: &str, d: &str) -> Result<NaiveDate, String> {
        let (year, month, day) = (self.int(y, None)?, self.int(m, None)?, self.int(d, None)?);
        i32::try_from(year)
            .ok()
            .zip(u32::try_from(month).ok())
            .zip(u32::try_from(day).ok())
            .and_then(|((y, m), d)| NaiveDate::from_ymd_opt(y, m, d))
            .ok_or_else(|| format!("{year}-{month}-{day} is no date"))
    }
}

impl EntryDate {
    /// Decode the stored attributes; `get` reads one, `None` when absent.
    /// `Err` says why there is no date: a kind the file format does not
    /// document, or a missing or impossible value.
    pub fn from_attrs<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Result<EntryDate, String> {
        let f = Fields(|n: &str| get(n).map(String::from));
        Ok(match f.int("Mode", Some(0))? {
            0 => {
                let (month, day) = f.day_of_year("Month", "Day")?;
                EntryDate::Fixed { month, day }
            }
            1 => EntryDate::Easter {
                offset: f.int("EasterOffset", Some(0))?,
            },
            3 => EntryDate::Period {
                from: f.period_day("From")?,
                to: f.period_day("To")?,
            },
            4 => {
                let (month, day) = f.day_of_year("Month", "Day")?;
                let (month2, day2) = f.day_of_year("Month2", "Day2")?;
                EntryDate::Yearly {
                    month,
                    day,
                    month2,
                    day2,
                }
            }
            m => return Err(format!("Mode {m} is a kind of date vislox does not know")),
        })
    }

    /// The date as one line: *Every year on 24 Dec*.
    pub fn text(&self) -> String {
        match self {
            EntryDate::Fixed { .. } => format!("Every year on {}", self.short()),
            EntryDate::Yearly { .. } => format!("Every year {}", self.short()),
            _ => self.short(),
        }
    }

    /// The date without *Every year*, for a change between two dates of
    /// the same kind: *24 Dec → 26 Dec*.
    pub fn short(&self) -> String {
        match *self {
            EntryDate::Fixed { month, day } => day_month(month, day),
            EntryDate::Easter { offset } => {
                let rel = match offset {
                    0 => "Easter".to_string(),
                    n if n > 0 => format!("Easter +{n}"),
                    n => format!("Easter −{}", -n),
                };
                match EASTER_DAYS.iter().find(|(o, _)| *o == offset) {
                    Some((0, name)) => name.to_string(),
                    Some((_, name)) => format!("{name} ({rel})"),
                    None => rel,
                }
            }
            EntryDate::Period { from, to } if from == to => full(from),
            EntryDate::Period { from, to } if to < from => {
                format!("{} – {} (ends before it starts)", full(from), full(to))
            }
            EntryDate::Period { from, to } => format!("{} – {}", full(from), full(to)),
            EntryDate::Yearly {
                month,
                day,
                month2,
                day2,
            } => format!("{} – {}", day_month(month, day), day_month(month2, day2)),
        }
    }

    /// The attributes this kind of date reads.
    pub fn attrs(&self) -> &'static [&'static str] {
        match self {
            EntryDate::Fixed { .. } => &["Mode", "Month", "Day"],
            EntryDate::Easter { .. } => &["Mode", "EasterOffset"],
            EntryDate::Period { .. } => &["Mode", "From", "To"],
            EntryDate::Yearly { .. } => &["Mode", "Month", "Day", "Month2", "Day2"],
        }
    }

    /// Whether `other` is the same kind of date.
    pub fn same_kind(&self, other: &EntryDate) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// The request for the heating period, and for the cooling period. Any user
/// with visualization rights may send them; they change nothing.
pub const HEAT_PERIOD: &str = "jdev/sps/calendargetheatperiod";
pub const COOL_PERIOD: &str = "jdev/sps/calendargetcoolperiod";

/// The heating or cooling period, as the Miniserver answered its request:
/// the range every year (`date`, and `text` as it reads: *15 Oct – 15 Apr*),
/// or why there is none (`why`). `value` is the answer as sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Season {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<EntryDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub value: String,
}

impl Season {
    /// From the `LL` object of the Miniserver's answer; `None`: no answer
    /// came.
    pub fn from_reply(ll: Option<&serde_json::Value>) -> Season {
        let Some(ll) = ll else {
            return Season::why(String::new(), "the Miniserver did not answer".into());
        };
        let value = match &ll["value"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            v => v.to_string(),
        };
        let code = match &ll["Code"] {
            serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
            v => v.as_i64(),
        };
        if code != Some(200) {
            let code = code.map_or_else(|| "none".to_string(), |c| c.to_string());
            return Season::why(value, format!("the Miniserver refused it (Code {code})"));
        }
        match season(&value) {
            Ok(d) => Season {
                text: Some(d.short()),
                date: Some(d),
                why: None,
                value,
            },
            Err(e) => Season::why(value, e),
        }
    }

    fn why(value: String, why: String) -> Season {
        Season {
            date: None,
            text: None,
            why: Some(why),
            value,
        }
    }
}

/// `MM-DD/MM-DD` as a range every year (it runs across New Year when the
/// end comes before the start, as a heating period does).
pub fn season(value: &str) -> Result<EntryDate, String> {
    let day = |s: &str| -> Option<(u32, u32)> {
        let (m, d) = s.trim().split_once('-')?;
        let (m, d) = (m.parse::<i64>().ok()?, d.parse::<i64>().ok()?);
        is_day_of_year(m, d).then_some((m as u32, d as u32))
    };
    let bad = || format!("“{value}” is not two days of the year (MM-DD/MM-DD)");
    let (a, b) = value.split_once('/').ok_or_else(bad)?;
    let ((month, day1), (month2, day2)) = (day(a).ok_or_else(bad)?, day(b).ok_or_else(bad)?);
    Ok(EntryDate::Yearly {
        month,
        day: day1,
        month2,
        day2,
    })
}

/// The `Mode=` number of the operating mode an entry switches on.
pub fn mode_number(cal_type: Option<&str>) -> Option<i64> {
    match cal_type {
        None => Some(0),
        Some(v) => v.trim().parse().ok(),
    }
}

/// How a changed date reads: the short forms when both are of one kind
/// (*24 Dec → 26 Dec*), else the full lines.
pub fn change_text(old: &EntryDate, new: &EntryDate) -> String {
    if old.same_kind(new) {
        format!("{} → {}", old.short(), new.short())
    } else {
        format!("{} → {}", old.text(), new.text())
    }
}

// ── Entries changed in the Loxone app ─────────────────────────────────
//
// Admins can add, change and delete entries in the Loxone app. Those edits
// live on the Miniserver only, until a Pull or Sync takes them over. The
// Miniserver lists the entries it uses with `jdev/sps/calendargetentries`
// (admin rights only), in the datasheet's terms: `calMode` 0 a date every
// year (`startMonth`/`startDay`), 1 Easter (`easterOffset`), 2 one date
// (`startYear`/…), 3 one period (`start…` to `endYear`/`endMonth`/`endDay`),
// 4 a range every year (`startMonth`/`startDay` to `endMonth`/`endDay`, as
// a real Miniserver sends it; the datasheet's list differs), 5 a weekday of
// the month. `operatingMode` is the mode's `Mode=` number, the file's
// `CalType=`. The numbering differs from the file's `Mode=`, so entries are
// compared by their decoded dates, never by raw numbers.

/// A calendar entry, from the config file or from the Miniserver.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub uuid: String,
    pub name: String,
    /// The `Mode=` number of the operating mode it switches on; `None`
    /// when that cannot be read.
    pub mode: Option<i64>,
    pub date: Result<EntryDate, String>,
}

impl EntryDate {
    /// Decode an entry of `calendargetentries`. `Err` says why not.
    pub fn from_api(e: &serde_json::Value) -> Result<EntryDate, String> {
        let f = Fields(|n: &str| api_field(e, n));
        Ok(match f.int("calMode", None)? {
            0 => {
                let (month, day) = f.day_of_year("startMonth", "startDay")?;
                EntryDate::Fixed { month, day }
            }
            1 => EntryDate::Easter {
                offset: f.int("easterOffset", Some(0))?,
            },
            2 => {
                let d = f.date("startYear", "startMonth", "startDay")?;
                EntryDate::Period { from: d, to: d }
            }
            3 => EntryDate::Period {
                from: f.date("startYear", "startMonth", "startDay")?,
                to: f.date("endYear", "endMonth", "endDay")?,
            },
            4 => {
                let (month, day) = f.day_of_year("startMonth", "startDay")?;
                let (month2, day2) = f.day_of_year("endMonth", "endDay")?;
                EntryDate::Yearly {
                    month,
                    day,
                    month2,
                    day2,
                }
            }
            5 => {
                return Err(
                    "a weekday of the month (calMode 5), which vislox does not read".into(),
                );
            }
            m => {
                return Err(format!(
                    "calMode {m} is a kind of date vislox does not know"
                ));
            }
        })
    }
}

/// A field of an API entry as text, as the file's attributes are: `None`
/// when absent (or null), a number written out.
fn api_field(e: &serde_json::Value, name: &str) -> Option<String> {
    match &e[name] {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        v => Some(v.to_string()),
    }
}

/// The entries of a `calendargetentries` answer (its `LL.value`).
pub fn parse_api(value: &str) -> Result<Vec<Entry>, String> {
    let v: serde_json::Value =
        serde_json::from_str(value).map_err(|e| format!("not a list of entries: {e}"))?;
    let list = v.as_array().ok_or("not a list of entries")?;
    list.iter()
        .map(|e| {
            let uuid = e["uuid"].as_str().ok_or("an entry without a uuid")?;
            Ok(Entry {
                uuid: uuid.to_ascii_lowercase(),
                name: e["name"].as_str().unwrap_or_default().to_string(),
                // As the file's CalType: absent is 0, a numeric string a number.
                mode: mode_number(api_field(e, "operatingMode").as_deref()),
                date: EntryDate::from_api(e),
            })
        })
        .collect()
}

/// The calendar entries of a config, and its operating modes by `Mode=`
/// number (title), for naming the mode an entry switches on. In document
/// order: when two `Mode` blocks share a number, the first one names it,
/// as in the view.
pub fn of_doc(doc: &loxone_config::doc::LoxoneDoc) -> (Vec<Entry>, HashMap<i64, String>) {
    fn walk(el: &Element, entries: &mut Vec<Entry>, modes: &mut HashMap<i64, String>) {
        for child in el.child_elements().filter(|e| e.name == "C") {
            let title = || {
                child
                    .attr_decoded("Title")
                    .map(|t| t.into_owned())
                    .unwrap_or_default()
            };
            match (child.attr("Type"), child.attr("U")) {
                (Some(ENTRY), Some(uuid)) => entries.push(Entry {
                    uuid: uuid.to_ascii_lowercase(),
                    name: title(),
                    mode: mode_number(child.attr(MODE_ATTR)),
                    date: EntryDate::from_attrs(|n| child.attr(n)),
                }),
                // As in the view: absent is 0, an unreadable number none.
                (Some("Mode"), Some(_)) => {
                    if let Some(n) = mode_number(child.attr("Mode")) {
                        modes.entry(n).or_insert_with(title);
                    }
                }
                _ => {}
            }
            walk(child, entries, modes);
        }
    }
    let (mut entries, mut modes) = (Vec::new(), HashMap::new());
    walk(&doc.xml.root, &mut entries, &mut modes);
    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.uuid.cmp(&b.uuid)));
    (entries, modes)
}

/// How an entry on the Miniserver differs from the config's.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DriftKind {
    /// Added in the app: the config has no entry with its UUID.
    Added,
    /// Changed in the app: name, mode or date.
    Changed,
    /// Deleted in the app: the Miniserver no longer has it.
    Deleted,
}

/// One entry the app changed, as the banner and the entry's properties
/// read it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Drift {
    pub kind: DriftKind,
    pub uuid: String,
    /// The name on the Miniserver (the config's, when deleted).
    pub name: String,
    /// The entry in one line, as the Miniserver has it (the config's, when
    /// deleted): *Every year on 24 Dec · switches on Holiday*.
    pub what: String,
    /// What changed: *name: Party → Fest*, *when: 24 Dec → 26 Dec*,
    /// *switches on: Holiday → Party*.
    pub changes: Vec<String>,
}

fn mode_title(modes: &HashMap<i64, String>, n: Option<i64>) -> String {
    match n {
        Some(n) => modes
            .get(&n)
            .cloned()
            .unwrap_or_else(|| format!("Mode {n} (not in this config)")),
        None => "an unreadable mode".to_string(),
    }
}

fn date_text(d: &Result<EntryDate, String>) -> String {
    match d {
        Ok(d) => d.text(),
        Err(why) => format!("a date vislox cannot read ({why})"),
    }
}

/// Compare the entries the Miniserver uses (`app`) with the config's
/// (`file`), by UUID. A date counts as changed only when both sides
/// decode: one vislox cannot read on either side is not compared.
pub fn drift(file: &[Entry], app: &[Entry], modes: &HashMap<i64, String>) -> Vec<Drift> {
    let what = |e: &Entry| {
        format!(
            "{} · switches on {}",
            date_text(&e.date),
            mode_title(modes, e.mode)
        )
    };
    let by_uuid = |list: &'_ [Entry]| -> HashMap<String, usize> {
        list.iter()
            .enumerate()
            .map(|(i, e)| (e.uuid.clone(), i))
            .collect()
    };
    let (in_file, in_app) = (by_uuid(file), by_uuid(app));
    let mut out = Vec::new();
    for a in app {
        let Some(f) = in_file.get(&a.uuid).map(|&i| &file[i]) else {
            out.push(Drift {
                kind: DriftKind::Added,
                uuid: a.uuid.clone(),
                name: a.name.clone(),
                what: what(a),
                changes: Vec::new(),
            });
            continue;
        };
        let mut changes = Vec::new();
        if f.name != a.name {
            changes.push(format!("name: {} → {}", f.name, a.name));
        }
        if let (Ok(old), Ok(new)) = (&f.date, &a.date)
            && old != new
        {
            changes.push(format!("when: {}", change_text(old, new)));
        }
        if f.mode != a.mode {
            changes.push(format!(
                "switches on: {} → {}",
                mode_title(modes, f.mode),
                mode_title(modes, a.mode)
            ));
        }
        if !changes.is_empty() {
            out.push(Drift {
                kind: DriftKind::Changed,
                uuid: a.uuid.clone(),
                name: a.name.clone(),
                what: what(a),
                changes,
            });
        }
    }
    for f in file {
        if !in_app.contains_key(&f.uuid) {
            out.push(Drift {
                kind: DriftKind::Deleted,
                uuid: f.uuid.clone(),
                name: f.name.clone(),
                what: what(f),
                changes: Vec::new(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(attrs: &[(&str, &str)]) -> Result<EntryDate, String> {
        EntryDate::from_attrs(|n| attrs.iter().find(|(k, _)| *k == n).map(|(_, v)| *v))
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_fixed_date_has_no_mode() {
        // As a real config saves New Year: the unused Month2/Day2 are 1.
        let d = decode(&[
            ("Month", "12"),
            ("Day", "24"),
            ("Month2", "1"),
            ("Day2", "1"),
            ("WeekDayInMonth", "1"),
        ])
        .unwrap();
        assert_eq!(d, EntryDate::Fixed { month: 12, day: 24 });
        assert_eq!(d.text(), "Every year on 24 Dec");
        assert_eq!(d.short(), "24 Dec");
    }

    #[test]
    fn easter_days_have_their_names() {
        let at = |o: &str| {
            decode(&[("Mode", "1"), ("EasterOffset", o)])
                .unwrap()
                .text()
        };
        assert_eq!(at("1"), "Easter Monday (Easter +1)");
        assert_eq!(at("-2"), "Good Friday (Easter −2)");
        assert_eq!(at("60"), "Corpus Christi (Easter +60)");
        assert_eq!(at("5"), "Easter +5");
        assert_eq!(at("-10"), "Easter −10");
        // Easter Sunday is saved without an EasterOffset; the date left in
        // Month/Day (of the year it was saved) does not count.
        let sunday = decode(&[("Mode", "1"), ("Month", "4"), ("Day", "4")]).unwrap();
        assert_eq!(sunday, EntryDate::Easter { offset: 0 });
        assert_eq!(sunday.text(), "Easter Sunday");
    }

    #[test]
    fn a_period_counts_days_from_2009() {
        // lox's notes: 2375 is 4 July 2015.
        let d = decode(&[("Mode", "3"), ("From", "2375"), ("To", "2375")]).unwrap();
        assert_eq!(
            d,
            EntryDate::Period {
                from: ymd(2015, 7, 4),
                to: ymd(2015, 7, 4)
            }
        );
        assert_eq!(d.text(), "4 Jul 2015");
        let summer = decode(&[("Mode", "3"), ("From", "6393"), ("To", "6456")]).unwrap();
        assert_eq!(summer.text(), "4 Jul 2026 – 5 Sep 2026");
        let back = decode(&[("Mode", "3"), ("From", "6456"), ("To", "6393")]).unwrap();
        assert_eq!(
            back.text(),
            "5 Sep 2026 – 4 Jul 2026 (ends before it starts)"
        );
    }

    #[test]
    fn a_yearly_range_may_run_across_new_year() {
        let d = decode(&[
            ("Mode", "4"),
            ("Month", "10"),
            ("Day", "1"),
            ("Month2", "4"),
            ("Day2", "30"),
        ])
        .unwrap();
        assert_eq!(d.text(), "Every year 1 Oct – 30 Apr");
        assert_eq!(d.short(), "1 Oct – 30 Apr");
    }

    #[test]
    fn what_cannot_be_decoded_says_why() {
        assert_eq!(
            decode(&[("Mode", "5"), ("Month", "13")]).unwrap_err(),
            "Mode 5 is a kind of date vislox does not know"
        );
        assert_eq!(
            decode(&[("Month", "2"), ("Day", "30")]).unwrap_err(),
            "Month 2 and Day 30 are no day of the year"
        );
        assert_eq!(decode(&[("Day", "3")]).unwrap_err(), "it has no Month");
        assert_eq!(
            decode(&[("Mode", "3"), ("From", "x")]).unwrap_err(),
            "From is “x”, not a whole number"
        );
        assert_eq!(
            decode(&[("Mode", "3"), ("From", "-1"), ("To", "0")]).unwrap_err(),
            "From -1 is no day from 1 Jan 2009 on"
        );
        // Days far beyond any calendar are refused, not a panic.
        for huge in ["9223372036854775807", "4000000000"] {
            assert_eq!(
                decode(&[("Mode", "3"), ("From", "0"), ("To", huge)]).unwrap_err(),
                format!("To {huge} is no day from 1 Jan 2009 on")
            );
        }
        // A period without both days is no period from 1 Jan 2009.
        assert_eq!(
            decode(&[("Mode", "3"), ("From", "6393")]).unwrap_err(),
            "it has no To"
        );
        assert_eq!(decode(&[("Mode", "3")]).unwrap_err(), "it has no From");
        assert!(decode(&[("Month", "13"), ("Day", "1")]).is_err());
        // 29 Feb is a day of some years.
        assert!(decode(&[("Month", "2"), ("Day", "29")]).is_ok());
    }

    #[test]
    fn a_change_reads_old_to_new() {
        let fixed = |d| EntryDate::Fixed { month: 12, day: d };
        assert_eq!(change_text(&fixed(24), &fixed(26)), "24 Dec → 26 Dec");
        assert_eq!(
            change_text(&fixed(24), &EntryDate::Easter { offset: 1 }),
            "Every year on 24 Dec → Easter Monday (Easter +1)"
        );
    }

    #[test]
    fn the_mode_number_defaults_to_holiday() {
        assert_eq!(mode_number(None), Some(0));
        assert_eq!(mode_number(Some("-5")), Some(-5));
        assert_eq!(mode_number(Some("x")), None);
    }

    #[test]
    fn the_periods_read_as_ranges_every_year() {
        // As a real Miniserver answers (firmware 17).
        let heat: serde_json::Value = serde_json::from_str(
            r#"{"control": "dev/sps/calendargetheatperiod", "value": "10-01/04-30", "Code": "200"}"#,
        )
        .unwrap();
        let s = Season::from_reply(Some(&heat));
        assert_eq!(
            s.date,
            Some(EntryDate::Yearly {
                month: 10,
                day: 1,
                month2: 4,
                day2: 30
            })
        );
        assert_eq!(s.text.as_deref(), Some("1 Oct – 30 Apr"));
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"date":{"kind":"yearly","month":10,"day":1,"month2":4,"day2":30},"text":"1 Oct – 30 Apr","value":"10-01/04-30"}"#
        );
        assert_eq!(season("5-15/9-15").unwrap().short(), "15 May – 15 Sep");
        assert_eq!(season("02-29/03-01").unwrap().short(), "29 Feb – 1 Mar");
        // What cannot be read says why, with the answer.
        for bad in [
            "",
            "10-01",
            "13-01/04-30",
            "10-01/02-30",
            "a-b/c-d",
            "00-00/00-00",
        ] {
            assert_eq!(
                season(bad).unwrap_err(),
                format!("“{bad}” is not two days of the year (MM-DD/MM-DD)")
            );
        }
        let refused: serde_json::Value = serde_json::from_str(
            r#"{"control": "dev/sps/calendargetcoolperiod", "value": "", "Code": "403"}"#,
        )
        .unwrap();
        assert_eq!(
            Season::from_reply(Some(&refused)).why.as_deref(),
            Some("the Miniserver refused it (Code 403)")
        );
        // A numeric Code counts too.
        let num: serde_json::Value =
            serde_json::from_str(r#"{"value": "05-15/09-15", "Code": 200}"#).unwrap();
        assert!(Season::from_reply(Some(&num)).date.is_some());
        let none = Season::from_reply(None);
        assert_eq!(none.why.as_deref(), Some("the Miniserver did not answer"));
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"why":"the Miniserver did not answer","value":""}"#
        );
    }

    #[test]
    fn the_browser_gets_the_date_by_kind() {
        let j = |d: EntryDate| serde_json::to_string(&d).unwrap();
        assert_eq!(
            j(EntryDate::Fixed { month: 12, day: 24 }),
            r#"{"kind":"fixed","month":12,"day":24}"#
        );
        assert_eq!(
            j(EntryDate::Period {
                from: ymd(2026, 7, 4),
                to: ymd(2026, 9, 5)
            }),
            r#"{"kind":"period","from":"2026-07-04","to":"2026-09-05"}"#
        );
    }

    /// Entries as a real Miniserver (firmware 16) sends them.
    const API: &str = r#"[
        {"uuid":"1b344a7c-0374-3516-ffff450b2297409e","name":"Sezon grzewczy","operatingMode":10,"calMode":4,"startMonth":10,"startDay":1,"endMonth":4,"endDay":30},
        {"uuid":"b18873ee-7e61-11e1-b511f20f692e0d8a","name":"PL - Nowy Rok","operatingMode":0,"calMode":0,"startMonth":1,"startDay":1},
        {"uuid":"b4772f6e-7e61-11e1-b592f20f692e0d8a","name":"PL - Poniedziałek Wielkanocny","operatingMode":0,"calMode":1,"easterOffset":1},
        {"uuid":"1860737a-026c-e539-ffffeee000d80263","name":"PL - Pierwszy dzień Wielkiej Nocy","operatingMode":0,"calMode":1,"easterOffset":0}
    ]"#;

    #[test]
    fn the_miniserver_s_entries_decode_to_the_file_s_dates() {
        let e = parse_api(API).unwrap();
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].mode, Some(10));
        assert_eq!(
            e[0].date,
            Ok(EntryDate::Yearly {
                month: 10,
                day: 1,
                month2: 4,
                day2: 30
            })
        );
        assert_eq!(e[1].date, Ok(EntryDate::Fixed { month: 1, day: 1 }));
        assert_eq!(e[2].date, Ok(EntryDate::Easter { offset: 1 }));
        assert_eq!(e[3].date, Ok(EntryDate::Easter { offset: 0 }));
        // The same entries as the file stores them decode to the same dates.
        assert_eq!(
            e[0].date,
            decode(&[
                ("Mode", "4"),
                ("Month", "10"),
                ("Day", "1"),
                ("Month2", "4"),
                ("Day2", "30")
            ])
        );
        assert_eq!(
            e[1].date,
            decode(&[("Month", "1"), ("Day", "1"), ("Month2", "1"), ("Day2", "1")])
        );
    }

    #[test]
    fn one_date_and_a_period_are_periods() {
        let one =
            serde_json::json!({"calMode": 2, "startYear": 2026, "startMonth": 12, "startDay": 31});
        assert_eq!(
            EntryDate::from_api(&one),
            Ok(EntryDate::Period {
                from: ymd(2026, 12, 31),
                to: ymd(2026, 12, 31)
            })
        );
        let span = serde_json::json!({"calMode": 3, "startYear": 2026, "startMonth": 7, "startDay": 4,
            "endYear": 2026, "endMonth": 9, "endDay": 5});
        // As the file's Mode=3, From=6393, To=6456.
        assert_eq!(
            EntryDate::from_api(&span),
            decode(&[("Mode", "3"), ("From", "6393"), ("To", "6456")])
        );
    }

    #[test]
    fn what_the_api_sends_and_vislox_cannot_read_says_why() {
        let d = |v| EntryDate::from_api(&v).unwrap_err();
        assert_eq!(
            d(serde_json::json!({"calMode": 5, "startMonth": 13, "weekDay": 0})),
            "a weekday of the month (calMode 5), which vislox does not read"
        );
        assert_eq!(
            d(serde_json::json!({"calMode": 9})),
            "calMode 9 is a kind of date vislox does not know"
        );
        assert_eq!(d(serde_json::json!({})), "it has no calMode");
        assert_eq!(
            d(
                serde_json::json!({"calMode": 2, "startYear": 2026, "startMonth": 2, "startDay": 30})
            ),
            "2026-2-30 is no date"
        );
        assert_eq!(
            d(serde_json::json!({"calMode": 0, "startMonth": 13, "startDay": 1})),
            "startMonth 13 and startDay 1 are no day of the year"
        );
        assert!(parse_api("{}").is_err());
        assert!(parse_api(r#"[{"name":"x"}]"#).is_err());
    }

    fn entry(uuid: &str, name: &str, mode: i64, date: EntryDate) -> Entry {
        Entry {
            uuid: uuid.into(),
            name: name.into(),
            mode: Some(mode),
            date: Ok(date),
        }
    }

    #[test]
    fn drift_lists_what_the_app_added_changed_and_deleted() {
        let modes = HashMap::from([(0, "Holiday".to_string()), (-1, "Party".to_string())]);
        let xmas = EntryDate::Fixed { month: 12, day: 24 };
        let file = vec![
            entry("a", "Christmas", 0, xmas),
            entry("b", "Easter", 0, EntryDate::Easter { offset: 1 }),
            entry("c", "Same", 0, xmas),
        ];
        let app = vec![
            entry("a", "Xmas", -1, EntryDate::Fixed { month: 12, day: 26 }),
            entry("c", "Same", 0, xmas),
            entry("d", "Fest", -1, EntryDate::Fixed { month: 7, day: 1 }),
        ];
        let d = drift(&file, &app, &modes);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].kind, DriftKind::Changed);
        assert_eq!(
            d[0].changes,
            [
                "name: Christmas → Xmas",
                "when: 24 Dec → 26 Dec",
                "switches on: Holiday → Party"
            ]
        );
        assert_eq!(d[0].what, "Every year on 26 Dec · switches on Party");
        assert_eq!(
            (d[1].kind.clone(), d[1].uuid.as_str()),
            (DriftKind::Added, "d")
        );
        assert_eq!(d[1].what, "Every year on 1 Jul · switches on Party");
        assert_eq!(
            (d[2].kind.clone(), d[2].name.as_str()),
            (DriftKind::Deleted, "Easter")
        );
        assert_eq!(d[2].what, "Easter Monday (Easter +1) · switches on Holiday");
        // Nothing changed, nothing listed.
        assert!(drift(&file, &file, &modes).is_empty());
    }

    #[test]
    fn a_date_either_side_cannot_read_is_not_compared() {
        let modes = HashMap::new();
        let mut a = entry("a", "Weekday", 3, EntryDate::Fixed { month: 1, day: 1 });
        let file = vec![a.clone()];
        a.date = Err("a weekday".into());
        assert!(drift(&file, &[a], &modes).is_empty());
        let unknown = entry("a", "X", 7, EntryDate::Fixed { month: 1, day: 1 });
        let other = Entry {
            mode: Some(8),
            ..unknown.clone()
        };
        assert_eq!(
            drift(&[unknown], &[other], &modes)[0].changes,
            ["switches on: Mode 7 (not in this config) → Mode 8 (not in this config)"]
        );
    }

    #[test]
    fn the_fixture_s_entries_are_read_with_their_modes() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/house.Loxone"
        ))
        .unwrap();
        let doc = loxone_config::doc::LoxoneDoc::parse(src.as_bytes()).unwrap();
        let (entries, modes) = of_doc(&doc);
        let xmas = entries.iter().find(|e| e.name == "Heiligabend").unwrap();
        assert_eq!(xmas.uuid, "10000013-0000-0130-ffff504f94000001");
        assert_eq!(xmas.mode, Some(0));
        assert_eq!(xmas.date, Ok(EntryDate::Fixed { month: 12, day: 24 }));
        assert!(modes.contains_key(&0));
    }

    #[test]
    fn the_api_s_mode_reads_as_the_file_s_cal_type() {
        // Absent is 0 (the holiday mode), as a file without CalType; a
        // numeric string is its number.
        let e = parse_api(
            r#"[{"uuid":"A","calMode":0,"startMonth":1,"startDay":1},
                {"uuid":"b","operatingMode":"0","calMode":0,"startMonth":1,"startDay":1},
                {"uuid":"c","operatingMode":" -1","calMode":"0","startMonth":"1","startDay":1},
                {"uuid":"d","operatingMode":"x","calMode":0,"startMonth":1,"startDay":1}]"#,
        )
        .unwrap();
        assert_eq!(
            e.iter().map(|e| e.mode).collect::<Vec<_>>(),
            [Some(0), Some(0), Some(-1), None]
        );
        assert_eq!(e[0].uuid, "a");
        assert_eq!(e[2].date, Ok(EntryDate::Fixed { month: 1, day: 1 }));
        // So the same entries in the file show no drift.
        let file: Vec<Entry> = ["a", "b", "c"]
            .iter()
            .zip([None, Some("0"), Some("-1")])
            .map(|(u, cal_type)| Entry {
                uuid: u.to_string(),
                name: String::new(),
                mode: mode_number(cal_type),
                date: Ok(EntryDate::Fixed { month: 1, day: 1 }),
            })
            .collect();
        assert!(drift(&file, &e[..3], &HashMap::new()).is_empty());
    }

    #[test]
    fn both_decoders_refuse_alike() {
        let file = decode(&[("Month", "2"), ("Day", "x")]).unwrap_err();
        let api = EntryDate::from_api(
            &serde_json::json!({"calMode": 0, "startMonth": 2, "startDay": "x"}),
        )
        .unwrap_err();
        assert_eq!(file, "Day is “x”, not a whole number");
        assert_eq!(api, "startDay is “x”, not a whole number");
    }

    #[test]
    fn the_first_mode_of_a_number_names_it() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/house.Loxone"
        ))
        .unwrap();
        // Another mode with Party's number (-1), just before it.
        let party = r#"<C Type="Mode" V="178" U="10000011-0000-0110-ffff504f94000001""#;
        assert!(src.contains(party));
        let src = src.replacen(
            party,
            &format!(
                r#"<C Type="Mode" V="178" U="10000099-0000-0110-ffff504f94000001" Title="Erste Party" Mode="-1"/>{party}"#
            ),
            1,
        );
        for _ in 0..8 {
            let doc = loxone_config::doc::LoxoneDoc::parse(src.as_bytes()).unwrap();
            assert_eq!(of_doc(&doc).1[&-1], "Erste Party");
        }
    }
}
