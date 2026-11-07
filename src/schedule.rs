//! Switching times: the schedule of a Daytimer, or of a block that holds
//! one (the Intelligent Room Controller's heating and cooling schedule),
//! as the `.Loxone` file stores it, in the terms of the Miniserver's live
//! Daytimer event table so that the two can be compared.
//!
//! lox's `loxone-format.md` (*DayTimer*, *Operating modes*) documents the
//! file, and is followed here:
//!
//! - A `DayTimer` keeps its schedule on itself; other blocks keep it in a
//!   `<Timer>` child, and name the UUID the Miniserver streams it under in
//!   `UUIDTimer=`. A DayTimer's is its own UUID.
//! - `Modes=` lists operating-mode UUIDs, one per column. An `<Entry>`
//!   names its column by index into that list (`Ix=`, absent 0), spans
//!   `From=`…`To=` in minutes of the day (absent 0; `1440` is midnight),
//!   carries `V=`, and optionally `NeedActivate="true"`.
//! - `DefValue=` is the value outside every entry (an analog timer's).
//!
//! The Miniserver sends a Daytimer's schedule as an event table (its
//! `entriesAndDefaultValue` state): the default value, and per entry the
//! operating mode by its `Mode=` number, from, to, need-activate and the
//! value. `Entry` is that form; the file's column index is turned into the
//! mode number with the config's `Mode` objects.

use loxone_config::xml::Element;
use serde::Serialize;

/// One switching time: in column `col` (the operating mode's UUID the
/// file names; `#<n>` for an index past the `Modes=` list), which is
/// operating mode `mode` (its `Mode=` number; `None` when the column names
/// no `Mode` object), from `from` to `to` minutes of the day, value
/// `value`. Columns, not mode numbers, tell entries apart: two columns that
/// name no mode stay two.
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Entry {
    pub col: String,
    pub mode: Option<i64>,
    pub from: u32,
    pub to: u32,
    pub value: f64,
    /// It switches on only when activated (`NeedActivate`).
    pub activate: bool,
}

/// A schedule as the file stores it.
#[derive(Clone, PartialEq, Debug)]
pub struct Timer {
    /// The UUID the Miniserver streams the schedule under.
    pub state: String,
    pub analog: bool,
    /// The value outside every entry, when the file has one.
    pub default: Option<f64>,
    /// The columns: operating-mode UUIDs.
    pub modes: Vec<String>,
    pub entries: Vec<Entry>,
}

fn num<T: std::str::FromStr>(el: &Element, name: &str) -> Option<T> {
    el.attr(name).and_then(|v| v.trim().parse().ok())
}

/// The schedule of block `el` (UUID `uuid`), when it has one: a
/// `DayTimer`, or a block with a `<Timer>` child. `mode_number` gives an
/// operating mode's `Mode=` number from its UUID.
pub fn read(el: &Element, uuid: &str, mode_number: impl Fn(&str) -> Option<i64>) -> Option<Timer> {
    let (t, state) = if el.attr("Type") == Some("DayTimer") {
        (el, uuid.to_string())
    } else {
        let t = el.child_elements().find(|c| c.name == "Timer")?;
        (t, el.attr("UUIDTimer").unwrap_or(uuid).to_string())
    };
    let modes: Vec<String> = t
        .attr("Modes")
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(String::from)
        .collect();
    let entries = t
        .child_elements()
        .filter(|c| c.name == "Entry")
        .map(|e| {
            let ix: usize = num(e, "Ix").unwrap_or(0);
            let col = modes.get(ix).cloned();
            Entry {
                mode: col.as_deref().and_then(&mode_number),
                col: col.unwrap_or_else(|| format!("#{ix}")),
                from: num(e, "From").unwrap_or(0),
                to: num(e, "To").unwrap_or(0),
                value: num(e, "V").unwrap_or(0.0),
                activate: e.attr("NeedActivate") == Some("true"),
            }
        })
        .collect();
    Some(Timer {
        state,
        analog: el.attr("Analog") == Some("true") || t.name == "Timer",
        default: num(t, "DefValue").or_else(|| num(el, "DefValue")),
        modes,
        entries,
    })
}

/// `hh:mm` of a minute of the day (`1440` is `24:00`).
pub fn clock(min: u32) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

/// Entries in one line, in the order given: `06:30–08:00 21.5,
/// 17:00–22:00 21.5 (activate)`; `analog` adds each entry's value (a
/// digital timer's is always on).
pub fn line(entries: &[Entry], analog: bool) -> String {
    if entries.is_empty() {
        return "no entries".into();
    }
    entries
        .iter()
        .map(|e| {
            let mut s = format!("{}–{}", clock(e.from), clock(e.to));
            if analog {
                s.push_str(&format!(" {}", crate::live::number(e.value)));
            }
            if e.activate {
                s.push_str(" (activate)");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The entries of column `col`, in time order.
pub fn of_col(entries: &[Entry], col: &str) -> Vec<Entry> {
    let mut v: Vec<Entry> = entries.iter().filter(|e| e.col == col).cloned().collect();
    v.sort_by(|a, b| {
        (a.from, a.to)
            .cmp(&(b.from, b.to))
            .then(a.value.total_cmp(&b.value))
    });
    v
}

/// Column `col`'s entries as they are compared: from, to, the value (an
/// analog timer's only; a digital timer's is always on) and activation, in
/// time order.
pub fn col_key(entries: &[Entry], col: &str, analog: bool) -> Vec<(u32, u32, Option<u64>, bool)> {
    of_col(entries, col)
        .iter()
        .map(|e| (e.from, e.to, analog.then(|| e.value.to_bits()), e.activate))
        .collect()
}

/// The columns, in first-seen order, whose entries differ between `a` and
/// `b` (order within a column does not count; a digital timer's values do
/// not either, as [`line`] does not show them).
pub fn changed_cols(a: &[Entry], b: &[Entry], analog: bool) -> Vec<String> {
    let mut cols: Vec<&str> = Vec::new();
    for e in a.iter().chain(b) {
        if !cols.contains(&e.col.as_str()) {
            cols.push(&e.col);
        }
    }
    cols.into_iter()
        .filter(|c| col_key(a, c, analog) != col_key(b, c, analog))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use loxone_config::doc::LoxoneDoc;

    const MON: &str = "00000000-0000-0004-1500000000000000";
    const TUE: &str = "00000000-0000-0005-1500000000000000";

    fn mode_number(u: &str) -> Option<i64> {
        match u {
            MON => Some(3),
            TUE => Some(4),
            _ => None,
        }
    }

    fn first<'a>(doc: &'a LoxoneDoc, ty: &str) -> &'a Element {
        fn find<'a>(el: &'a Element, ty: &str) -> Option<&'a Element> {
            for c in el.child_elements() {
                if c.name == "C" && c.attr("Type") == Some(ty) {
                    return Some(c);
                }
                if let Some(f) = find(c, ty) {
                    return Some(f);
                }
            }
            None
        }
        find(&doc.xml.root, ty).expect("block")
    }

    fn doc(body: &str) -> LoxoneDoc {
        let src = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ControlList Version=\"1\">\
             <C Type=\"Document\" V=\"1\" U=\"d\" Title=\"T\">{body}</C></ControlList>"
        );
        LoxoneDoc::parse(src.as_bytes()).expect("parses")
    }

    #[test]
    fn a_daytimer_keeps_its_entries_on_itself() {
        let d = doc(&format!(
            "<C Type=\"DayTimer\" U=\"dt\" Title=\"Uhr\" Analog=\"true\" DefValue=\"38\" N=\"3\" Modes=\"{MON},{TUE},x\">\
               <Co K=\"AQ\" U=\"dt-aq\"/>\
               <Entry To=\"1440\" V=\"40\"/>\
               <Entry From=\"390\" To=\"480\" Ix=\"1\" NeedActivate=\"true\" V=\"45.5\"/>\
               <Entry From=\"60\" To=\"120\" Ix=\"2\" V=\"1\"/>\
               <Entry From=\"60\" To=\"120\" Ix=\"7\" V=\"1\"/>\
             </C>"
        ));
        let t = read(first(&d, "DayTimer"), "dt", mode_number).expect("a timer");
        assert_eq!(t.state, "dt");
        assert!(t.analog);
        assert_eq!(t.default, Some(38.0));
        assert_eq!(t.modes.len(), 3);
        let es = &t.entries;
        assert_eq!(
            (es[0].col.as_str(), es[0].mode, es[0].from, es[0].to),
            (MON, Some(3), 0, 1440)
        );
        assert_eq!(
            (es[1].mode, es[1].value, es[1].activate),
            (Some(4), 45.5, true)
        );
        // A column that names no mode has no number, but keeps its UUID;
        // an index past the list is a column of its own.
        assert_eq!((es[2].col.as_str(), es[2].mode), ("x", None));
        assert_eq!((es[3].col.as_str(), es[3].mode), ("#7", None));
        assert_eq!(line(&of_col(es, TUE), true), "06:30–08:00 45.5 (activate)");
        assert_eq!(line(&of_col(es, MON), false), "00:00–24:00");
        assert_eq!(line(&[], false), "no entries");
    }

    #[test]
    fn a_room_controller_keeps_its_schedule_in_a_timer_child() {
        let d = doc(&format!(
            "<C Type=\"HeatIRoomController2\" U=\"irc\" Title=\"Bad\" UUIDTimer=\"irc-timer\">\
               <Timer M=\"5\" DefValue=\"0\" N=\"1\" Modes=\"{TUE}\"><Entry From=\"360\" To=\"480\" V=\"1\"/></Timer>\
             </C><C Type=\"And\" U=\"and\" Title=\"O1\"/>"
        ));
        let t = read(first(&d, "HeatIRoomController2"), "irc", mode_number).expect("a timer");
        assert_eq!(t.state, "irc-timer");
        assert_eq!(t.default, Some(0.0));
        assert_eq!(t.entries[0].mode, Some(4));
        assert!(read(first(&d, "And"), "and", mode_number).is_none());
    }

    fn e(col: &str, from: u32, to: u32, value: f64) -> Entry {
        Entry {
            col: col.into(),
            mode: None,
            from,
            to,
            value,
            activate: false,
        }
    }

    #[test]
    fn only_columns_whose_entries_differ_count_as_changed() {
        let a = [
            e(MON, 390, 480, 1.0),
            e(MON, 1000, 1100, 1.0),
            e(TUE, 0, 1440, 1.0),
        ];
        // Same entries of Monday in another order; Tuesday changed.
        let b = [
            e(MON, 1000, 1100, 1.0),
            e(MON, 390, 480, 1.0),
            e(TUE, 0, 1380, 1.0),
        ];
        assert_eq!(changed_cols(&a, &b, true), vec![TUE]);
        assert!(changed_cols(&a, &a, true).is_empty());
        // A column only one side has.
        assert_eq!(changed_cols(&a, &a[..2], true), vec![TUE]);
    }

    #[test]
    fn two_columns_without_a_mode_stay_apart() {
        // An entry moves from one unknown column to another: both changed.
        let a = [e("x", 60, 120, 1.0)];
        let b = [e("y", 60, 120, 1.0)];
        assert_eq!(changed_cols(&a, &b, true), vec!["x", "y"]);
        assert_eq!(line(&of_col(&b, "x"), false), "no entries");
    }

    #[test]
    fn a_digital_timers_value_is_no_change() {
        // Not shown for a digital timer, so not a change either.
        let a = [e(MON, 390, 480, 1.0)];
        let b = [e(MON, 390, 480, 0.0)];
        assert!(changed_cols(&a, &b, false).is_empty());
        assert_eq!(changed_cols(&a, &b, true), vec![MON]);
    }
}
