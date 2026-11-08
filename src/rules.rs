//! Automatic Designer rules (#87): what a rule (`AutopilotRule`) does, read
//! from its `APC=` payload as Lox Logic writes it, in lines that the
//! Properties pane, the change list and the `rule` op (`rule_edit.rs`) share.
//!
//! lox's reading of a payload (`loxone_config::autopilot`) and the app's
//! capability table are private to lox. Its public decompile lifts every rule
//! whose conditions and actions Lox Logic writes into a `rule` declaration
//! (`language.md`, *`rule`*), so vislox reads rules through it, exactly as
//! `lox logic decompile` and `adopt` would write them: the state names and
//! commands are lox's, never guessed here. A rule decompile does not lift
//! (text values, calls, speech output, a block the config lacks) is shown as
//! read-only, with its raw payload in Advanced.
//!
//! A line names its blocks and operating modes by UUID, so the op, `previous`
//! and Revert never depend on how a file spells them.

use loxone_config::LoxoneDoc;
use loxone_config::logic::{
    DecompileOptions, DecompileScope, Item, MatchSpec, ModeName, RULE_BLOCK_TYPE, RuleDecl,
    RuleStmtKind,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RULE: &str = RULE_BLOCK_TYPE;

/// One line of a rule body, as the `rule` op carries it and Properties shows
/// it. Blocks are named by UUID (`block`); a block of vislox's files lox has
/// not pinned yet, by its slug. An operating mode is named by UUID too
/// (`mode`), or, when no mode of the config is that one, as written (a slug
/// or a quoted title), which is then written back exactly so.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Line {
    /// The conditions after it are another group.
    Or,
    /// `when time HH:MM`.
    Time { at: String },
    /// `when weekday mon tue …` (`weekdays`, `weekend`, `daily` as written).
    Weekday { days: Vec<String> },
    /// `when <block>.<state> <op> <value>`: a number, or `on`/`off`.
    State {
        block: String,
        state: String,
        op: String,
        value: String,
    },
    /// `when mode <mode> [off]`.
    Mode { mode: String, on: bool },
    /// `when weather <field> <op> <value>`.
    Weather {
        field: String,
        op: String,
        value: String,
    },
    /// `when day <op> <1…31>`.
    Day { op: String, value: String },
    /// `when daylight [off]`.
    Daylight { on: bool },
    /// `when sunrise|sunset|dawn|dusk`.
    Sun { at: String },
    /// `when date YYYY-MM-DD`.
    Date { date: String },
    /// `then <block> <command>`.
    Then { block: String, command: String },
    /// `then mode <mode> on|off`.
    ThenMode { mode: String, on: bool },
    /// `then notify "<subject>" "<text>"`.
    Notify { subject: String, text: String },
    /// `then mail "<to>" "<subject>" "<text>"`.
    Mail {
        to: String,
        subject: String,
        text: String,
    },
    /// `then wait <duration>` (`500ms`, `2.5s`, `5min`, `1h`).
    Wait { duration: String },
    /// `then message info|warning|error "<subject>" "<text>" [admins]`.
    Message {
        severity: String,
        subject: String,
        text: String,
        #[serde(default)]
        admins: bool,
    },
}

/// What a rule does: whether it is active, and its lines (conditions, `or`
/// between groups, then the actions in order). Empty lines: a rule without
/// conditions or actions (a new one, before its body is written).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct RuleVal {
    pub active: bool,
    #[serde(default)]
    pub lines: Vec<Line>,
}

/// The comparisons a condition takes.
pub const OPS: &[&str] = &["==", "!=", ">", "<", ">=", "<="];
/// The moments of the day `when` takes.
pub const SUN: &[&str] = &["sunrise", "sunset", "dawn", "dusk"];
/// The weekday words `when weekday` takes.
pub const DAYS: &[&str] = &[
    "mon", "tue", "wed", "thu", "fri", "sat", "sun", "weekdays", "weekend", "daily",
];
/// The weather fields lox documents (`language.md`, *`rule`*).
pub const WEATHER: &[&str] = &[
    "temperature",
    "dewpoint",
    "felt_temperature",
    "wind",
    "gusts",
    "wind_direction",
    "irradiance",
    "relative_irradiance",
    "solar_irradiance",
    "precipitation",
    "pressure",
    "sunshine",
];
/// A message's severities.
/// lox 0.83's parser takes these (`logic::rule::severity`).
pub const SEVERITIES: &[&str] = &["info", "warning", "important", "critical"];

impl Line {
    /// A condition (or the `or` between groups), not an action.
    pub fn is_condition(&self) -> bool {
        !self.is_action()
    }

    pub fn is_action(&self) -> bool {
        matches!(
            self,
            Line::Then { .. }
                | Line::ThenMode { .. }
                | Line::Notify { .. }
                | Line::Mail { .. }
                | Line::Wait { .. }
                | Line::Message { .. }
        )
    }

    /// The block it watches or drives, as named.
    pub fn block(&self) -> Option<&str> {
        match self {
            Line::State { block, .. } | Line::Then { block, .. } => Some(block),
            _ => None,
        }
    }

    /// The operating mode it names, as named.
    pub fn mode(&self) -> Option<&str> {
        match self {
            Line::Mode { mode, .. } | Line::ThenMode { mode, .. } => Some(mode),
            _ => None,
        }
    }
}

/// `s` in Lox Logic's quotes, as lox writes a string.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A lowercase word: an identifier lox reads as one token.
fn word(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|c| c.is_ascii_alphabetic())
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A number lox reads as one: digits, one point, a leading minus.
fn number(s: &str) -> bool {
    let t = s.strip_prefix('-').unwrap_or(s);
    !t.is_empty()
        && t.chars().all(|c| c.is_ascii_digit() || c == '.')
        && t.chars().filter(|c| *c == '.').count() <= 1
        && !t.starts_with('.')
        && !t.ends_with('.')
}

/// How a line names its block and mode in a file: the caller's spelling of
/// each (a slug of the file), or why it cannot be named.
pub struct Names<'a> {
    pub block: &'a dyn Fn(&str) -> Result<String, String>,
    pub mode: &'a dyn Fn(&str) -> Result<String, String>,
}

/// The line as Lox Logic writes it (`when time 09:00`, `then vx_… on`), its
/// parts checked so that it is exactly one statement. lox's parser checks the
/// rest (a time of day, a duration's range, a weekday word) when the body is
/// read back.
pub fn text(line: &Line, names: &Names) -> Result<String, String> {
    let op = |o: &str| {
        if OPS.contains(&o) {
            Ok(o.to_string())
        } else {
            Err(format!("`{o}` is no comparison: {}", OPS.join(" ")))
        }
    };
    let value = |v: &str| {
        let v = v.trim();
        if v == "on" || v == "off" || number(v) {
            Ok(v.to_string())
        } else {
            Err(format!("`{v}` is no number, `on` or `off`"))
        }
    };
    let on_off = |on: bool| if on { "on" } else { "off" };
    Ok(match line {
        Line::Or => "or".into(),
        Line::Time { at } => {
            let at = at.trim();
            if !at.chars().all(|c| c.is_ascii_digit() || c == ':') || at.is_empty() {
                return Err(format!("`{at}` is no time of day (HH:MM)"));
            }
            format!("when time {at}")
        }
        Line::Weekday { days } => {
            if days.is_empty() {
                return Err("a weekday condition names a day at least".into());
            }
            for d in days {
                if !DAYS.contains(&d.as_str()) {
                    return Err(format!("`{d}` is no day: {}", DAYS.join(" ")));
                }
            }
            format!("when weekday {}", days.join(" "))
        }
        Line::State {
            block,
            state,
            op: o,
            value: v,
        } => {
            if !word(state) {
                return Err(format!("`{state}` is no state name"));
            }
            format!(
                "when {}.{state} {} {}",
                (names.block)(block)?,
                op(o)?,
                value(v)?
            )
        }
        Line::Mode { mode, on } => {
            let m = (names.mode)(mode)?;
            if *on {
                format!("when mode {m}")
            } else {
                format!("when mode {m} off")
            }
        }
        Line::Weather {
            field,
            op: o,
            value: v,
        } => {
            if !WEATHER.contains(&field.as_str()) {
                return Err(format!(
                    "`{field}` is no weather field: {}",
                    WEATHER.join(", ")
                ));
            }
            format!("when weather {field} {} {}", op(o)?, value(v)?)
        }
        Line::Day { op: o, value: v } => {
            let v = v.trim();
            match v.parse::<u8>() {
                Ok(1..=31) => format!("when day {} {v}", op(o)?),
                _ => return Err(format!("`{v}`: a day of the month is 1 … 31")),
            }
        }
        Line::Daylight { on: true } => "when daylight".into(),
        Line::Daylight { on: false } => "when daylight off".into(),
        Line::Sun { at } => {
            if !SUN.contains(&at.as_str()) {
                return Err(format!("`{at}` is none of {}", SUN.join(", ")));
            }
            format!("when {at}")
        }
        Line::Date { date } => {
            let d = date.trim();
            let shape = d.len() == 10
                && d.char_indices().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                });
            if !shape {
                return Err(format!("`{d}` is no date (YYYY-MM-DD)"));
            }
            format!("when date {d}")
        }
        Line::Then { block, command } => {
            let c = command.trim();
            if c.is_empty() {
                return Err("an action on a block sends a command".into());
            }
            format!("then {} {}", (names.block)(block)?, quote(c))
        }
        Line::ThenMode { mode, on } => {
            format!("then mode {} {}", (names.mode)(mode)?, on_off(*on))
        }
        Line::Notify { subject, text } => {
            format!("then notify {} {}", quote(subject), quote(text))
        }
        Line::Mail { to, subject, text } => {
            format!(
                "then mail {} {} {}",
                quote(to.trim()),
                quote(subject),
                quote(text)
            )
        }
        Line::Wait { duration } => {
            let d = duration.trim();
            if d.is_empty() || !d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
                return Err(format!("`{d}` is no duration (500ms, 2.5s, 5min, 1h)"));
            }
            format!("then wait {d}")
        }
        Line::Message {
            severity,
            subject,
            text,
            admins,
        } => {
            if !SEVERITIES.contains(&severity.as_str()) {
                return Err(format!(
                    "`{severity}` is no severity: {}",
                    SEVERITIES.join(", ")
                ));
            }
            // The command carries them `/`-separated: lox refuses a `/`.
            if subject.contains('/') || text.contains('/') {
                return Err(
                    "a message's subject and text go into a command path, so they cannot \
                     contain `/`"
                        .into(),
                );
            }
            format!(
                "then message {severity} {} {}{}",
                quote(subject),
                quote(text),
                if *admins { " admins" } else { "" }
            )
        }
    })
}

/// The lines of a declaration's body, its blocks and modes named back by
/// `block` and `mode` (a slug → UUID, else as written).
pub fn lines_of(
    decl: &RuleDecl,
    block: &dyn Fn(&str) -> String,
    mode: &dyn Fn(&ModeName) -> String,
) -> RuleVal {
    let mut v = RuleVal::default();
    for st in decl.stmts() {
        let line = match &st.kind {
            RuleStmtKind::Active(a) => {
                v.active = *a;
                continue;
            }
            RuleStmtKind::Or => Line::Or,
            RuleStmtKind::Time(m) => Line::Time {
                at: format!("{:02}:{:02}", m / 60, m % 60),
            },
            RuleStmtKind::Weekday(d) => Line::Weekday { days: d.clone() },
            RuleStmtKind::State {
                target,
                state,
                op,
                value,
            } => Line::State {
                block: block(target),
                state: state.clone(),
                op: op.symbol().into(),
                value: value.clone(),
            },
            RuleStmtKind::Mode { mode: m, on } => Line::Mode {
                mode: mode(m),
                on: *on,
            },
            RuleStmtKind::ThenMode { mode: m, on } => Line::ThenMode {
                mode: mode(m),
                on: *on,
            },
            RuleStmtKind::Weather { field, op, value } => Line::Weather {
                field: field.clone(),
                op: op.symbol().into(),
                value: value.clone(),
            },
            RuleStmtKind::Day { op, value } => Line::Day {
                op: op.symbol().into(),
                value: value.clone(),
            },
            RuleStmtKind::Daylight(on) => Line::Daylight { on: *on },
            RuleStmtKind::Sun(w) => Line::Sun { at: w.clone() },
            RuleStmtKind::Date(d) => Line::Date { date: d.clone() },
            RuleStmtKind::Then { target, command } => Line::Then {
                block: block(target),
                command: command.clone(),
            },
            RuleStmtKind::Notify { subject, text } => Line::Notify {
                subject: subject.clone(),
                text: text.clone(),
            },
            RuleStmtKind::Mail { to, subject, text } => Line::Mail {
                to: to.clone(),
                subject: subject.clone(),
                text: text.clone(),
            },
            RuleStmtKind::Wait(d) => Line::Wait {
                duration: d.clone(),
            },
            RuleStmtKind::Message {
                severity,
                subject,
                text,
                admins,
            } => Line::Message {
                severity: severity.clone(),
                subject: subject.clone(),
                text: text.clone(),
                admins: *admins,
            },
        };
        v.lines.push(line);
    }
    v
}

/// Whether `v` holds together as a rule, as lox's checker wants one: a
/// condition in every group (between `or`s) and an action at least. The
/// conditions come first.
pub fn check_shape(v: &RuleVal) -> Result<(), String> {
    if v.lines.is_empty() {
        return Err("a rule needs a condition and an action".into());
    }
    let first_action = v.lines.iter().position(Line::is_action);
    let Some(first_action) = first_action else {
        return Err("a rule does something: add an action".into());
    };
    if v.lines[first_action..].iter().any(Line::is_condition) {
        return Err("a rule's conditions come before its actions".into());
    }
    let conditions = &v.lines[..first_action];
    if conditions.is_empty() {
        return Err("a rule runs on a condition: add one".into());
    }
    for group in conditions.split(|l| *l == Line::Or) {
        if group.is_empty() {
            return Err("every group of conditions (between `or`s) has one at least".into());
        }
    }
    Ok(())
}

/// What a rule of a config does, as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// Lox Logic writes it: its lines, blocks and modes by UUID.
    Rule(RuleVal),
    /// Lox Logic cannot write it: why.
    Not(String),
}

/// Every rule of `doc`, by UUID (lowercase): what it does, or why Lox Logic
/// cannot write it. Read with lox's decompile, so only for a config that has
/// rules.
pub fn read(doc: &LoxoneDoc) -> BTreeMap<String, Read> {
    let objects = doc.objects();
    let rules: Vec<_> = objects.iter().filter(|o| o.block_type == RULE).collect();
    let mut out: BTreeMap<String, Read> = BTreeMap::new();
    if rules.is_empty() {
        return out;
    }
    for r in &rules {
        let empty = empty_rule(doc, &r.path);
        out.insert(
            r.uuid.to_ascii_lowercase(),
            match empty {
                Some(active) => Read::Rule(RuleVal {
                    active,
                    lines: Vec::new(),
                }),
                None => Read::Not(
                    "Lox Logic cannot write this rule: it has conditions or actions the language \
                     does not know (text values, calls, speech output), or names a block the \
                     config does not have. Change it in the app."
                        .into(),
                ),
            },
        );
    }
    // Only rules and the blocks they name are lifted: no managed type.
    let opts = DecompileOptions {
        managed_types: Default::default(),
        scope: DecompileScope::ManagedOnly,
        ..Default::default()
    };
    let Ok((module, _)) = loxone_config::logic::decompile(doc, &opts) else {
        return out;
    };
    // Decompile pins an extern by title or designation where that names one
    // object; resolve it as lox would, on type and that name.
    let one = |ty: &str, f: &dyn Fn(&loxone_config::doc::ObjectSummary) -> bool| {
        let mut found = objects.iter().filter(|o| o.block_type == ty && f(o));
        let first = found.next()?;
        found.next().is_none().then_some(first.uuid.as_str())
    };
    let externs: BTreeMap<&str, &str> = module
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Extern(e) => {
                let uuid = match &e.match_spec {
                    MatchSpec::Uuid(u) => Some(u.as_str()),
                    MatchSpec::Title(t) => {
                        one(&e.block_type, &|o| o.title.as_deref() == Some(t.as_str()))
                    }
                    MatchSpec::IName(n) => {
                        one(&e.block_type, &|o| o.iname.as_deref() == Some(n.as_str()))
                    }
                    _ => None,
                }?;
                Some((e.slug.as_str(), uuid))
            }
            _ => None,
        })
        .collect();
    let modes = modes(doc);
    let block = |s: &str| {
        externs
            .get(s)
            .map(|u| u.to_ascii_lowercase())
            .unwrap_or_else(|| s.to_string())
    };
    let mode = |m: &ModeName| match m {
        ModeName::Slug(s) => externs
            .get(s.as_str())
            .map(|u| u.to_ascii_lowercase())
            .unwrap_or_else(|| s.clone()),
        ModeName::Title(t) => mode_by_title(&modes, t).unwrap_or_else(|| quote(t)),
    };
    for decl in module.rule_decls() {
        let uuid = match &decl.title {
            // A rule declared by its name, which no other rule has.
            Some(t) => rules
                .iter()
                .find(|r| r.title.as_deref() == Some(t.as_str()))
                .map(|r| r.uuid.to_ascii_lowercase()),
            None => externs
                .get(decl.slug.as_str())
                .map(|u| u.to_ascii_lowercase()),
        };
        if let Some(u) = uuid {
            out.insert(u, Read::Rule(lines_of(decl, &block, &mode)));
        }
    }
    out
}

/// A rule without conditions or actions (one lox created and nothing wrote
/// yet), which decompile does not lift: whether it is active.
fn empty_rule(doc: &LoxoneDoc, path: &[usize]) -> Option<bool> {
    let el = doc.element_at(path)?;
    let apc = el.attr_decoded("APC")?;
    let v: serde_json::Value = serde_json::from_str(&apc).ok()?;
    let groups = v.get("eventGroups")?.as_array()?;
    let none = groups.iter().all(|g| {
        g.get("events")
            .and_then(|e| e.as_array())
            .is_none_or(|e| e.is_empty())
    });
    let no_actions = v
        .get("actions")
        .and_then(|a| a.as_array())
        .is_none_or(|a| a.is_empty());
    (none && no_actions).then(|| v.get("active").and_then(|a| a.as_bool()).unwrap_or(false))
}

/// The config's operating modes: UUID (lowercase), title.
pub fn modes(doc: &LoxoneDoc) -> Vec<(String, String)> {
    doc.objects()
        .into_iter()
        .filter(|o| o.block_type == "Mode")
        .map(|o| (o.uuid.to_ascii_lowercase(), o.title.unwrap_or_default()))
        .collect()
}

/// The one mode titled `t` (lox compares titles exactly).
fn mode_by_title(modes: &[(String, String)], t: &str) -> Option<String> {
    let mut found = modes.iter().filter(|(_, x)| x == t);
    let first = found.next()?;
    found.next().is_none().then(|| first.0.clone())
}

/// Whether `s` is a Loxone UUID (8-4-4-16 hex digits).
pub fn is_uuid(s: &str) -> bool {
    let groups: Vec<usize> = s.split('-').map(str::len).collect();
    groups == [8, 4, 4, 16] && s.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit())
}

/// A line in plain words, its block and mode by the name `title` gives
/// (*at 09:00*, *Pumpe: on*, *while “Urlaub” is on*).
pub fn words(line: &Line, title: &dyn Fn(&str) -> String) -> String {
    fn cmp(o: &str) -> &str {
        match o {
            "==" => "is",
            "!=" => "is not",
            ">" => "above",
            "<" => "below",
            ">=" => "at least",
            "<=" => "at most",
            other => other,
        }
    }
    let q = |s: &str| format!("“{s}”");
    match line {
        Line::Or => "or".into(),
        Line::Time { at } => format!("at {at}"),
        Line::Weekday { days } => {
            let name = |d: &str| {
                match d {
                    "mon" => "Monday",
                    "tue" => "Tuesday",
                    "wed" => "Wednesday",
                    "thu" => "Thursday",
                    "fri" => "Friday",
                    "sat" => "Saturday",
                    "sun" => "Sunday",
                    "weekdays" => "weekdays",
                    "weekend" => "the weekend",
                    "daily" => "every day",
                    other => other,
                }
                .to_string()
            };
            format!(
                "on {}",
                days.iter().map(|d| name(d)).collect::<Vec<_>>().join(", ")
            )
        }
        Line::State {
            block,
            state,
            op,
            value,
        } => format!("{} {state} {} {value}", title(block), cmp(op)),
        Line::Mode { mode, on } => format!(
            "while {} is {}",
            q(&title(mode)),
            if *on { "on" } else { "off" }
        ),
        Line::Weather { field, op, value } => {
            format!("weather {} {} {value}", field.replace('_', " "), cmp(op))
        }
        Line::Day { op, value } => format!("day of the month {} {value}", cmp(op)),
        Line::Daylight { on: true } => "during the day".into(),
        Line::Daylight { on: false } => "at night".into(),
        Line::Sun { at } => format!("at {at}"),
        Line::Date { date } => format!("on {date}"),
        Line::Then { block, command } => format!("{}: {command}", title(block)),
        Line::ThenMode { mode, on } => format!(
            "switch {} {}",
            q(&title(mode)),
            if *on { "on" } else { "off" }
        ),
        Line::Notify { subject, .. } => format!("push notification {}", q(subject)),
        Line::Mail { to, subject, .. } => format!("mail {} to {to}", q(subject)),
        Line::Wait { duration } => format!("wait {duration}"),
        Line::Message {
            severity,
            subject,
            admins,
            ..
        } => format!(
            "{severity} message {}{}",
            q(subject),
            if *admins { " (administrators)" } else { "" }
        ),
    }
}

/// The rule in one sentence: its groups of conditions, then its actions
/// (*At 09:00 and on weekdays: Pumpe: on*).
pub fn summary(v: &RuleVal, title: &dyn Fn(&str) -> String) -> String {
    let first_action = v
        .lines
        .iter()
        .position(Line::is_action)
        .unwrap_or(v.lines.len());
    let groups: Vec<String> = v.lines[..first_action]
        .split(|l| *l == Line::Or)
        .filter(|g| !g.is_empty())
        .map(|g| {
            g.iter()
                .map(|l| words(l, title))
                .collect::<Vec<_>>()
                .join(" and ")
        })
        .collect();
    let actions: Vec<String> = v.lines[first_action..]
        .iter()
        .map(|l| words(l, title))
        .collect();
    if groups.is_empty() && actions.is_empty() {
        return "No conditions and no actions yet".into();
    }
    let mut s = groups.join(", or ");
    if let Some(c) = s.get(..1) {
        s = c.to_uppercase() + &s[1..];
    }
    format!("{s}: {}", actions.join(", then "))
}

/// A changed rule, as the change list reads it: active ↔ inactive, and each
/// condition and action, old → new, added or removed, by its place.
pub fn changes(old: &RuleVal, new: &RuleVal, title: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut out = Vec::new();
    let state = |a: bool| if a { "active" } else { "inactive" };
    if old.active != new.active {
        out.push(format!("{} → {}", state(old.active), state(new.active)));
    }
    let split = |v: &RuleVal| -> (Vec<Vec<Line>>, Vec<Line>) {
        let at = v
            .lines
            .iter()
            .position(Line::is_action)
            .unwrap_or(v.lines.len());
        let groups = v.lines[..at]
            .split(|l| *l == Line::Or)
            .map(<[Line]>::to_vec)
            .filter(|g| !g.is_empty())
            .collect();
        (groups, v.lines[at..].to_vec())
    };
    let (og, oa) = split(old);
    let (ng, na) = split(new);
    let many = og.len().max(ng.len()) > 1;
    let mut pair = |what: String, o: &[Line], n: &[Line]| {
        for i in 0..o.len().max(n.len()) {
            match (o.get(i), n.get(i)) {
                (Some(a), Some(b)) if a != b => out.push(format!(
                    "{what} changed: {} → {}",
                    words(a, title),
                    words(b, title)
                )),
                (Some(a), None) => out.push(format!("{what} removed: {}", words(a, title))),
                (None, Some(b)) => out.push(format!("{what} added: {}", words(b, title))),
                _ => {}
            }
        }
    };
    for g in 0..og.len().max(ng.len()) {
        let what = if many {
            format!("condition (group {})", g + 1)
        } else {
            "condition".into()
        };
        pair(
            what,
            og.get(g).map(Vec::as_slice).unwrap_or_default(),
            ng.get(g).map(Vec::as_slice).unwrap_or_default(),
        );
    }
    pair("action".into(), &oa, &na);
    out
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A synthetic config shaped like lox's own rule fixture (its
    //! `tests/rule.rs`): a push button the app shows as a switch, a pulse
    //! button, two rules as the app writes them, an operating mode and the
    //! Miniserver's time zone.
    pub const PAGE: &str = "40000000-0000-0002-ffff504f94112233";
    pub const PUMP: &str = "40000001-0000-0010-ffff504f94112233";
    pub const COVER: &str = "40000003-0000-0030-ffff504f94112233";
    pub const RULE_ON: &str = "40000002-0000-0020-ffff504f94112233";
    pub const RULE_OFF: &str = "40000005-0000-0050-ffff504f94112233";
    pub const RULE_ODD: &str = "40000006-0000-0060-ffff504f94112233";
    pub const PARTY: &str = "40000007-0000-0070-ffff504f94112233";
    pub const CONTAINER: &str = "40000008-0000-0080-ffff504f94112233";

    fn esc(json: &str) -> String {
        json.replace('"', "&quot;")
    }

    pub fn rule_on_apc() -> String {
        esc(&format!(
            r#"{{"type":1,"id":"{RULE_ON}","active":true,"activeIfValid":true,"name":"Pumpe Ein","description":"","operator":3,"invalid":false,"eventGroups":[{{"operator":1,"events":[{{"type":3,"descID":270001,"id":270,"value":9999999999,"operator":1,"forceEdge":false,"arrValues":[0,1,2,3,4]}},{{"type":5,"descID":272001,"id":272,"value":540,"operator":1,"forceEdge":false}}]}}],"actions":[{{"type":0,"descID":342101,"id":"{PUMP}","command":"on"}}]}}"#
        ))
    }

    pub fn rule_off_apc() -> String {
        esc(&format!(
            r#"{{"type":1,"id":"{RULE_OFF}","active":false,"activeIfValid":false,"name":"Pumpe Aus","description":"","operator":3,"invalid":false,"eventGroups":[{{"operator":1,"events":[{{"type":5,"descID":272001,"id":272,"value":1020,"operator":1,"forceEdge":false}}]}}],"actions":[{{"type":0,"descID":342102,"id":"{PUMP}","command":"off"}}]}}"#
        ))
    }

    /// A rule with an action Lox Logic does not write (speech output).
    pub fn rule_odd_apc() -> String {
        esc(&format!(
            r#"{{"type":1,"id":"{RULE_ODD}","active":true,"activeIfValid":true,"name":"Ansage","description":"","operator":3,"invalid":false,"eventGroups":[{{"operator":1,"events":[{{"type":5,"descID":272001,"id":272,"value":600,"operator":1,"forceEdge":false}}]}}],"actions":[{{"type":9,"descID":999999,"id":0,"command":"say"}}]}}"#
        ))
    }

    pub fn house_with(on: &str, off: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
             <ControlList Version=\"1\" NextObj=\"9\" NextConst=\"1\" NextNote=\"1\" NextMem=\"1\">\r\n\
             \t<C Type=\"Document\" V=\"175\" U=\"40000000-0000-0001-ffff504f94112233\">\r\n\
             \t\t<C Type=\"LoxLIVE\" V=\"175\" U=\"40000000-0000-0009-ffff504f94112233\" Title=\"Miniserver\" Timezone=\"Europe/Vienna\"/>\r\n\
             \t\t<C Type=\"ModeCaption\" V=\"175\" U=\"40000000-0000-000a-ffff504f94112233\" Title=\"Betriebsmodi\">\r\n\
             \t\t\t<C Type=\"Mode\" V=\"175\" U=\"{PARTY}\" Title=\"Party\" Mode=\"-1\"/>\r\n\
             \t\t</C>\r\n\
             \t\t<C Type=\"AutoPilot\" V=\"175\" U=\"{CONTAINER}\" Title=\"Regeln\"/>\r\n\
             \t\t<C Type=\"Page\" V=\"175\" U=\"{PAGE}\" Title=\"Automatik-Regeln\" SpNo=\"1\">\r\n\
             \t\t\t<C Type=\"PushButton\" V=\"175\" U=\"{PUMP}\" Title=\"Pumpe\" SpStates=\"40000001-0000-0013-ffffaaaaaaaa0001,40000001-0000-0014-ffffaaaaaaaa0001,40000001-0000-0015-ffffaaaaaaaa0001\">\r\n\
             \t\t\t\t<Co K=\"Reset\" U=\"40000001-0000-0012-00ffaaaaaaaa0001\"/>\r\n\
             \t\t\t\t<Co K=\"Q\" U=\"40000001-0000-0011-01ffaaaaaaaa0001\"/>\r\n\
             \t\t\t\t<IoData Visu=\"true\" Pr=\"40000000-0000-0003-ffff504f94112233\" Cr=\"40000000-0000-0004-ffff504f94112233\"/>\r\n\
             \t\t\t\t<Display Type=\"1\"/>\r\n\
             \t\t\t</C>\r\n\
             \t\t\t<C Type=\"PButtonT\" V=\"175\" U=\"{COVER}\" Title=\"Abdeckung\">\r\n\
             \t\t\t\t<Co K=\"Reset\" U=\"40000003-0000-0032-00ffaaaaaaaa0003\"/>\r\n\
             \t\t\t\t<Co K=\"Q\" U=\"40000003-0000-0031-01ffaaaaaaaa0003\"/>\r\n\
             \t\t\t\t<IoData Visu=\"true\" Pr=\"40000000-0000-0003-ffff504f94112233\" Cr=\"40000000-0000-0004-ffff504f94112233\"/>\r\n\
             \t\t\t</C>\r\n\
             \t\t\t<C Type=\"AutopilotRule\" V=\"175\" U=\"{RULE_ON}\" Title=\"Pumpe Ein\" Px=\"960\" Py=\"960\" Px2=\"2304\" Py2=\"1656\" Nio=\"1\" APC=\"{on}\" STA=\"1\" ref=\"{CONTAINER}\">\r\n\
             \t\t\t\t<Co K=\"Q\" U=\"40000002-0000-0021-00ffaaaaaaaa0002\"/>\r\n\
             \t\t\t</C>\r\n\
             \t\t\t<C Type=\"AutopilotRule\" V=\"175\" U=\"{RULE_OFF}\" Title=\"Pumpe Aus\" Px=\"960\" Py=\"1920\" Px2=\"2304\" Py2=\"2616\" Nio=\"1\" APC=\"{off}\" ref=\"{CONTAINER}\">\r\n\
             \t\t\t\t<Co K=\"Q\" U=\"40000005-0000-0051-00ffaaaaaaaa0005\"/>\r\n\
             \t\t\t</C>\r\n\
             \t\t\t<C Type=\"AutopilotRule\" V=\"175\" U=\"{RULE_ODD}\" Title=\"Ansage\" Px=\"960\" Py=\"2880\" Px2=\"2304\" Py2=\"3576\" Nio=\"1\" APC=\"{odd}\" STA=\"1\" ref=\"{CONTAINER}\">\r\n\
             \t\t\t\t<Co K=\"Q\" U=\"40000006-0000-0061-00ffaaaaaaaa0006\"/>\r\n\
             \t\t\t</C>\r\n\
             \t\t</C>\r\n\
             \t</C>\r\n\
             </ControlList>\r\n",
            odd = rule_odd_apc()
        )
    }

    pub fn house() -> String {
        house_with(&rule_on_apc(), &rule_off_apc())
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    fn doc() -> LoxoneDoc {
        LoxoneDoc::parse(house().as_bytes()).unwrap()
    }

    fn title(u: &str) -> String {
        match u {
            PUMP => "Pumpe".into(),
            PARTY => "Party".into(),
            other => other.into(),
        }
    }

    #[test]
    fn rules_read_as_lox_lifts_them_blocks_by_uuid() {
        let r = read(&doc());
        let on = RuleVal {
            active: true,
            lines: vec![
                Line::Weekday {
                    days: vec!["weekdays".into()],
                },
                Line::Time { at: "09:00".into() },
                Line::Then {
                    block: PUMP.into(),
                    command: "on".into(),
                },
            ],
        };
        assert_eq!(r[RULE_ON], Read::Rule(on.clone()));
        assert!(matches!(&r[RULE_OFF], Read::Rule(v) if !v.active && v.lines.len() == 2));
        assert!(matches!(&r[RULE_ODD], Read::Not(_)), "{:?}", r[RULE_ODD]);
        assert_eq!(summary(&on, &title), "On weekdays and at 09:00: Pumpe: on");
        // A config without rules reads nothing, and costs no decompile.
        let none = LoxoneDoc::parse(include_bytes!("../tests/fixtures/house.Loxone")).unwrap();
        assert!(read(&none).is_empty());
    }

    #[test]
    fn a_changed_rule_reads_per_condition_and_action() {
        let old = RuleVal {
            active: true,
            lines: vec![
                Line::Time { at: "09:00".into() },
                Line::Then {
                    block: PUMP.into(),
                    command: "on".into(),
                },
            ],
        };
        let new = RuleVal {
            active: false,
            lines: vec![
                Line::Time { at: "10:00".into() },
                Line::Or,
                Line::Mode {
                    mode: PARTY.into(),
                    on: true,
                },
                Line::Then {
                    block: PUMP.into(),
                    command: "on".into(),
                },
                Line::Wait {
                    duration: "5s".into(),
                },
            ],
        };
        assert_eq!(
            changes(&old, &new, &title),
            vec![
                "active → inactive",
                "condition (group 1) changed: at 09:00 → at 10:00",
                "condition (group 2) added: while “Party” is on",
                "action added: wait 5s",
            ]
        );
    }

    #[test]
    fn a_line_is_one_statement_or_refused() {
        let names = Names {
            block: &|u| Ok(format!("vx_{}", u.len())),
            mode: &|m| Ok(quote(m)),
        };
        let t = |l: Line| text(&l, &names);
        assert_eq!(
            t(Line::Then {
                block: "x".into(),
                command: "Mood 2".into()
            })
            .unwrap(),
            "then vx_1 \"Mood 2\""
        );
        assert_eq!(
            t(Line::Notify {
                subject: "Frost \"jetzt\"".into(),
                text: "a\nb".into()
            })
            .unwrap(),
            "then notify \"Frost \\\"jetzt\\\"\" \"a\\nb\""
        );
        for bad in [
            Line::State {
                block: "x".into(),
                state: "a b".into(),
                op: "==".into(),
                value: "1".into(),
            },
            Line::State {
                block: "x".into(),
                state: "active".into(),
                op: "=".into(),
                value: "1".into(),
            },
            Line::State {
                block: "x".into(),
                state: "active".into(),
                op: "==".into(),
                value: "1\nthen".into(),
            },
            Line::Weekday {
                days: vec!["monday".into()],
            },
            Line::Day {
                op: "==".into(),
                value: "32".into(),
            },
            Line::Wait {
                duration: "5 s\n".into(),
            },
            Line::Message {
                severity: "info".into(),
                subject: "a/b".into(),
                text: "".into(),
                admins: false,
            },
        ] {
            assert!(t(bad.clone()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_body_holds_together() {
        let then = Line::Then {
            block: PUMP.into(),
            command: "on".into(),
        };
        let at = Line::Time { at: "09:00".into() };
        let v = |lines: Vec<Line>| RuleVal {
            active: true,
            lines,
        };
        assert!(check_shape(&v(vec![])).is_err());
        assert!(check_shape(&v(vec![at.clone(), then.clone()])).is_ok());
        assert!(check_shape(&v(vec![at.clone()])).is_err());
        assert!(check_shape(&v(vec![then.clone()])).is_err());
        assert!(check_shape(&v(vec![at.clone(), Line::Or, then.clone()])).is_err());
        assert!(check_shape(&v(vec![then.clone(), at.clone()])).is_err());
    }
}
