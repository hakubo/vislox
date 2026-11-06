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

use chrono::{Days, NaiveDate};
use serde::Serialize;

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

impl EntryDate {
    /// Decode the stored attributes; `get` reads one, `None` when absent.
    /// `Err` says why there is no date: a kind the file format does not
    /// document, or a missing or impossible value.
    pub fn from_attrs<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Result<EntryDate, String> {
        let int = |name: &str, absent: Option<i64>| -> Result<i64, String> {
            match get(name) {
                Some(v) => v
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| format!("{name} is “{v}”, not a whole number")),
                None => absent.ok_or_else(|| format!("it has no {name}")),
            }
        };
        let day_of_year = |m: &str, d: &str| -> Result<(u32, u32), String> {
            let (month, day) = (int(m, None)?, int(d, None)?);
            if is_day_of_year(month, day) {
                Ok((month as u32, day as u32))
            } else {
                Err(format!("{m} {month} and {d} {day} are no day of the year"))
            }
        };
        let period_day = |name: &str| -> Result<NaiveDate, String> {
            let n = int(name, None)?;
            u64::try_from(n)
                .ok()
                .and_then(|n| epoch().checked_add_days(Days::new(n)))
                .ok_or_else(|| format!("{name} {n} is no day from 1 Jan 2009 on"))
        };
        Ok(match int("Mode", Some(0))? {
            0 => {
                let (month, day) = day_of_year("Month", "Day")?;
                EntryDate::Fixed { month, day }
            }
            1 => EntryDate::Easter {
                offset: int("EasterOffset", Some(0))?,
            },
            3 => EntryDate::Period {
                from: period_day("From")?,
                to: period_day("To")?,
            },
            4 => {
                let (month, day) = day_of_year("Month", "Day")?;
                let (month2, day2) = day_of_year("Month2", "Day2")?;
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
}
