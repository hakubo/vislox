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

/// One switching time: in operating mode `mode` (its `Mode=` number;
/// `None` when the file's column names no `Mode` object), from `from` to
/// `to` minutes of the day, value `value`.
#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Entry {
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
    /// Entries, by column index into `modes`.
    pub entries: Vec<(usize, Entry)>,
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
            let entry = Entry {
                mode: modes.get(ix).and_then(|m| mode_number(m)),
                from: num(e, "From").unwrap_or(0),
                to: num(e, "To").unwrap_or(0),
                value: num(e, "V").unwrap_or(0.0),
                activate: e.attr("NeedActivate") == Some("true"),
            };
            (ix, entry)
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

/// One mode's entries in one line, in time order: `06:30–08:00 21.5,
/// 17:00–22:00 21.5 (activate)`; `analog` adds each entry's value.
pub fn line(entries: &[Entry], analog: bool) -> String {
    let mut es: Vec<&Entry> = entries.iter().collect();
    es.sort_by_key(|e| (e.from, e.to));
    if es.is_empty() {
        return "no entries".into();
    }
    es.iter()
        .map(|e| {
            let mut s = format!("{}–{}", clock(e.from), clock(e.to));
            if analog {
                s.push_str(&format!(" {}", trim(e.value)));
            }
            if e.activate {
                s.push_str(" (activate)");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn trim(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The entries of mode `mode`, in time order.
pub fn of_mode(entries: &[Entry], mode: Option<i64>) -> Vec<Entry> {
    let mut v: Vec<Entry> = entries.iter().filter(|e| e.mode == mode).copied().collect();
    v.sort_by(|a, b| {
        (a.from, a.to)
            .cmp(&(b.from, b.to))
            .then(a.value.total_cmp(&b.value))
    });
    v
}

/// The modes, in first-seen order, whose entries differ between `a` and
/// `b` (order within a mode does not count).
pub fn changed_modes(a: &[Entry], b: &[Entry]) -> Vec<Option<i64>> {
    let mut modes: Vec<Option<i64>> = Vec::new();
    for e in a.iter().chain(b) {
        if !modes.contains(&e.mode) {
            modes.push(e.mode);
        }
    }
    modes
        .into_iter()
        .filter(|m| of_mode(a, *m) != of_mode(b, *m))
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
             </C>"
        ));
        let t = read(first(&d, "DayTimer"), "dt", mode_number).expect("a timer");
        assert_eq!(t.state, "dt");
        assert!(t.analog);
        assert_eq!(t.default, Some(38.0));
        assert_eq!(t.modes.len(), 3);
        let es: Vec<Entry> = t.entries.iter().map(|(_, e)| *e).collect();
        assert_eq!(
            es[0],
            Entry {
                mode: Some(3),
                from: 0,
                to: 1440,
                value: 40.0,
                activate: false
            }
        );
        assert_eq!(
            es[1],
            Entry {
                mode: Some(4),
                from: 390,
                to: 480,
                value: 45.5,
                activate: true
            }
        );
        // A column that names no mode has no number.
        assert_eq!(es[2].mode, None);
        assert_eq!(
            line(&of_mode(&es, Some(4)), true),
            "06:30–08:00 45.5 (activate)"
        );
        assert_eq!(line(&of_mode(&es, Some(3)), false), "00:00–24:00");
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
        assert_eq!(t.entries[0].1.mode, Some(4));
        assert!(read(first(&d, "And"), "and", mode_number).is_none());
    }

    #[test]
    fn only_modes_whose_entries_differ_count_as_changed() {
        let e = |mode, from, to| Entry {
            mode: Some(mode),
            from,
            to,
            value: 1.0,
            activate: false,
        };
        let a = [e(3, 390, 480), e(3, 1000, 1100), e(4, 0, 1440)];
        // Same entries of Monday in another order; Tuesday changed.
        let b = [e(3, 1000, 1100), e(3, 390, 480), e(4, 0, 1380)];
        assert_eq!(changed_modes(&a, &b), vec![Some(4)]);
        assert!(changed_modes(&a, &a).is_empty());
        // A mode only one side has.
        assert_eq!(changed_modes(&a, &a[..2]), vec![Some(4)]);
    }
}
