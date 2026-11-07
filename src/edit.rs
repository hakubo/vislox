//! Editing from Properties: the edit model, what can be edited, and the
//! file the edits are written to. The design is in `docs/editing.md`.
//!
//! An edit is a list of [`Op`]s against a base revision (the blob id of the
//! pending config the browser showed). vislox writes them as Lox Logic claims
//! into a file of its own in the project's module ([`FILE`]), and
//! `lox logic compile` turns the module into the pending `out.Loxone`. So an
//! edit shows as the pending revision, and Apply, Discard and Sync work on it
//! as on any other compile. Nothing here talks to the Miniserver.
//!
//! The claims are `extern` statements pinned by UUID, plus one assignment per
//! value: `x.In2 = 100` (an input's constant), `x.title = "…"`, `x.MinTime = 500`
//! (a setting of a physical input or output). lox records each original value in
//! the lockfile and restores it when the statement leaves the module, so
//! dropping a claim hands the value back to Loxone Config.

use crate::blocks::{self, NewBlock, PageFile};
use loxone_config::connectors::{self, PortDir, PortUnit};
use loxone_config::layout::GRID;
use loxone_config::lock::Layout;
use loxone_config::logic::{ArgItem, Attr, BindingKind, Item, MatchSpec, Module, Value};
use loxone_config::port_names;
use loxone_config::xml::Element;
use loxone_config::{LoxoneDoc, logic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// vislox's own file in the module directory, for the values it claims.
/// The rest of the module is never touched. The blocks vislox adds (#59)
/// are in files of their own next to it, one per page ([`crate::blocks`]).
pub const FILE: &str = "vislox/edits.loxl";

/// vislox's directory in the module: [`FILE`] and the files of new blocks.
pub const DIR: &str = "vislox";

/// The slugs of vislox's files start with this; nothing else in a module should.
const SLUG_PREFIX: &str = crate::blocks::SLUG_PREFIX;

/// One edit. `value: None` drops vislox's claim on the value: lox puts
/// back what Loxone Config had before the claim (see the module comment).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// The object's name (`Title=`).
    Title { uuid: String, value: Option<String> },
    /// The room it is in (`<IoData Pr=…>`), by the room's title.
    Room { uuid: String, value: Option<String> },
    /// Its category (`<IoData Cr=…>`), by the category's title.
    Category { uuid: String, value: Option<String> },
    /// The constant of an unwired input (`Def=` of connector `port`, its `K=`).
    Param {
        uuid: String,
        port: String,
        value: Option<String>,
    },
    /// A setting of a physical input or output, or of a device (`MinTime=`,
    /// `Desc=`, `Installation=`, …).
    Setting {
        uuid: String,
        name: String,
        value: Option<String>,
    },
    /// A new block of `type` on the page with UUID `page` (#59). lox mints
    /// its UUIDs and places it; the reply names it (`added`).
    AddBlock {
        page: String,
        #[serde(rename = "type")]
        ty: String,
        /// Its name; none shows the slug vislox picks.
        #[serde(default)]
        title: Option<String>,
        /// Port key → the constant of an input or parameter. A port left
        /// out gets the value Loxone Config places the block with.
        #[serde(default)]
        params: BTreeMap<String, String>,
        /// Setting (`Formula`, `Type`, `At`, …) → value.
        #[serde(default)]
        settings: BTreeMap<String, String>,
        /// Wires (#61) to draw with it; an empty UUID is this block. The
        /// `previous` of a `remove_block` carries the wires it took out.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        wires: Vec<Ends>,
        /// The UUID of the block this adds again: set in the `previous` of a
        /// `remove_block`. lox mints new UUIDs, so the reply's `added` names
        /// it next to the new one, and undo and redo (#62) can follow the
        /// block. It changes nothing in what is written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restores: Option<String>,
    },
    /// Take out a block added here that is not deployed yet. Its `previous`
    /// is the `add_block` that adds it again.
    RemoveBlock { uuid: String },
    /// Move a block the module manages (#60): its top left corner to
    /// (`x`, `y`) on its page, snapped to Loxone Config's grid. lox keeps a
    /// managed block's position in the lockfile, not in the source, so this
    /// is written there ([`write_moves`]). Its `previous` is the `move` back.
    Move { uuid: String, x: i64, y: i64 },
    /// Lay out the managed blocks of the page with UUID `page` afresh, as
    /// lox places new blocks (`lox logic compile --relayout`). Its
    /// `previous` is the `place` that puts each block back.
    Relayout { page: String },
    /// Move several blocks the module manages at once, each as `move` does:
    /// the undo of a `relayout` (#62), one op however many blocks the page
    /// has. Its `previous` is the `place` that puts them back.
    Place { moves: Vec<Placed> },
    /// Draw a wire (#61) from the output `from` to the input `to`, each
    /// `[object UUID, connector key]` as the page names a wire's ends. It is
    /// written as `to <- from` in [`FILE`]. Its `previous` is the `unwire`.
    Wire { from: [String; 2], to: [String; 2] },
    /// Take out a wire vislox drew (deployed or not). lox removes only the
    /// wires its source draws; one Loxone Config drew is refused. Its
    /// `previous` is the `wire` that draws it again.
    Unwire { from: [String; 2], to: [String; 2] },
    /// Put blocks Loxone Config drew under the module (#72, `lox logic
    /// adopt`): each declared in vislox's file of its page, with its UUIDs.
    /// The config stays exactly as it is. Its `previous` is the `unadopt`.
    Adopt { blocks: Vec<crate::adopt::Pick> },
    /// Adopt every block of the page with UUID `page` that lox can rebuild
    /// exactly; the reply names those it skipped, and why. Its `previous`
    /// is the `unadopt` of those it adopted.
    AdoptPage { page: String },
    /// Stop managing blocks adopted here: their declarations and lockfile
    /// entries go, and Loxone Config owns them again, as the base has them.
    /// Its `previous` is the `adopt` with the same slugs.
    Unadopt { uuids: Vec<String> },
    /// The switching times of the DayTimer `uuid` (#73): a `schedule` set
    /// and the timer's `.schedule` claim in [`FILE`] ([`crate::sched_edit`]).
    /// `None` drops vislox's claim: the timer is as the base has it. Its
    /// `previous` is the `schedule` op with what the timer claimed before.
    Schedule {
        uuid: String,
        value: Option<crate::sched_edit::Sched>,
    },
    /// The moods of the lighting controller `uuid` (#74): a `moods` set and
    /// the controller's `.moods` claim in [`FILE`] ([`crate::mood_edit`]).
    /// `None` drops vislox's claim: the moods are as the base has them. Its
    /// `previous` is the `moods` op with what the controller claimed before.
    Moods {
        uuid: String,
        value: Option<crate::mood_edit::MoodsVal>,
    },
}

/// Where a `place` puts one block: its top left corner, as `move` takes it.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Placed {
    pub uuid: String,
    pub x: i64,
    pub y: i64,
}

impl Op {
    /// The object it changes; `None` for a new block.
    pub fn uuid(&self) -> Option<&str> {
        match self {
            Op::Title { uuid, .. }
            | Op::Room { uuid, .. }
            | Op::Category { uuid, .. }
            | Op::Param { uuid, .. }
            | Op::Setting { uuid, .. }
            | Op::RemoveBlock { uuid }
            | Op::Move { uuid, .. }
            | Op::Schedule { uuid, .. }
            | Op::Moods { uuid, .. } => Some(uuid),
            Op::AddBlock { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. } => None,
        }
    }

    /// The value it sets (`None` also for a block op or a move).
    pub fn value(&self) -> Option<&str> {
        match self {
            Op::Title { value, .. }
            | Op::Room { value, .. }
            | Op::Category { value, .. }
            | Op::Param { value, .. }
            | Op::Setting { value, .. } => value.as_deref(),
            Op::AddBlock { .. }
            | Op::RemoveBlock { .. }
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Schedule { .. }
            | Op::Moods { .. } => None,
        }
    }

    /// The same op with another value (a block op stays as it is).
    pub fn with_value(&self, v: Option<String>) -> Op {
        let mut op = self.clone();
        match &mut op {
            Op::Title { value, .. }
            | Op::Room { value, .. }
            | Op::Category { value, .. }
            | Op::Param { value, .. }
            | Op::Setting { value, .. } => *value = v,
            Op::AddBlock { .. }
            | Op::RemoveBlock { .. }
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Schedule { .. }
            | Op::Moods { .. } => {}
        }
        op
    }
}

/// An attribute of the object itself that vislox claims (`x.title`, `x.room`,
/// `x.category`), in the order the file lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Which {
    Title,
    Room,
    Category,
}

impl Which {
    fn word(self) -> &'static str {
        match self {
            Which::Title => "title",
            Which::Room => "room",
            Which::Category => "category",
        }
    }

    fn op(self, uuid: String, value: Option<String>) -> Op {
        match self {
            Which::Title => Op::Title { uuid, value },
            Which::Room => Op::Room { uuid, value },
            Which::Category => Op::Category { uuid, value },
        }
    }
}

/// `POST /api/edit`: the ops, and the revision they were made against.
#[derive(Deserialize, Debug)]
pub struct Request {
    /// The blob id of the config the browser showed: the pending compile, or
    /// the deployed config when nothing is pending.
    pub base: String,
    pub ops: Vec<Op>,
}

/// The most ops one request takes.
pub const MAX_OPS: usize = 200;

// ── what can be edited ──────────────────────────────────────────────────

/// What Properties may edit on an object. Only the object itself decides
/// (its type, attributes and connectors); the project adds one rule, that
/// blocks the module manages are edited in their own source.
#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Caps {
    pub title: bool,
    /// It has a room, or a category (lox changes them, never adds them).
    pub room: bool,
    pub category: bool,
    pub params: Vec<ParamCap>,
    pub settings: Vec<SettingCap>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ParamCap {
    /// The connector key (`K=`).
    pub key: String,
    /// The documented unit, as a short suffix (`s`, `%`).
    pub unit: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SettingCap {
    pub name: String,
    /// Text (a name, a unit); else a number.
    pub text: bool,
    pub value: Option<String>,
}

/// Physical inputs and outputs whose settings Lox Logic can claim, and the
/// settings: (name, text, may be added where the object has none). From the
/// Lox Logic language spec, "`=` on a setting".
const IO_TYPES: &[&str] = &[
    "LoxAIRsensor",
    "LoxAIRAsensor",
    "LoxAIRactor",
    "LoxAIRAactor",
    "TreeSensor",
    "TreeAsensor",
    "TreeActor",
    "TreeAactor",
    "Actor",
    "DigitalIn",
    "VoltageIn",
    "Lox1wireAsensor",
];
const IO_SETTINGS: &[(&str, bool, bool)] = &[
    ("Desc", true, true),
    ("Unit", true, false),
    ("SourceValLow", false, true),
    ("DestValLow", false, true),
    ("SourceValHigh", false, false),
    ("DestValHigh", false, false),
    ("MinVal", false, true),
    ("MaxVal", false, false),
    ("MinChange", false, false),
    ("MinTime", false, false),
    ("ResetTime", false, true),
    ("AverageTime", false, true),
    ("MaxTime", false, false),
];
const DEVICE_TYPES: &[&str] = &["LoxAIRDevice", "TreeDevice"];
const EXTENSION_TYPES: &[&str] = &["LoxTree", "LoxAIR"];
const PLACE_SETTINGS: &[(&str, bool, bool)] = &[
    ("Installation", true, false),
    ("SwitchBoard", true, false),
    ("SwitchBoardRow", false, false),
    ("SwitchBoardPos", false, false),
];
/// Settings that are a duration, a count or a threshold: never below 0.
const NON_NEGATIVE: &[&str] = &[
    "MinChange",
    "MinTime",
    "ResetTime",
    "AverageTime",
    "MaxTime",
    "SwitchBoardRow",
    "SwitchBoardPos",
];

/// The settings `ty` takes: (name, text, may be added).
fn settings_of(ty: &str) -> Vec<(&'static str, bool, bool)> {
    if IO_TYPES.contains(&ty) {
        IO_SETTINGS.to_vec()
    } else if DEVICE_TYPES.contains(&ty) {
        let mut v = vec![("Desc", true, true)];
        v.extend_from_slice(PLACE_SETTINGS);
        v
    } else if EXTENSION_TYPES.contains(&ty) {
        PLACE_SETTINGS.to_vec()
    } else {
        Vec::new()
    }
}

/// Objects that are not edited from Properties at all: folders and pages,
/// and references, whose title Loxone Config takes from what they mirror.
fn not_editable(ty: &str) -> bool {
    matches!(
        ty,
        "Document" | "Page" | "InputRef" | "OutputRef" | "OutputRefLM"
    ) || ty.ends_with("Caption")
}

pub fn unit_suffix(u: PortUnit) -> String {
    match u {
        PortUnit::Milliseconds => "ms".into(),
        PortUnit::Seconds => "s".into(),
        PortUnit::Minutes => "min".into(),
        PortUnit::Percent => "%".into(),
        PortUnit::Kelvin => "K".into(),
        PortUnit::Degrees => "°".into(),
        PortUnit::Other(u) => u.into(),
    }
}

/// What Properties may edit on `el`, or `None` when nothing.
pub fn caps(el: &Element) -> Option<Caps> {
    let ty = el.attr("Type")?;
    el.attr("U")?;
    if not_editable(ty) {
        return None;
    }
    let specs = connectors::builtin(ty);
    let mut params = Vec::new();
    for co in el.child_elements().filter(|c| c.name == "Co") {
        let Some(key) = co.attr("K") else { continue };
        // Wired: the input takes its value from the wire. Inverted: the
        // connector belongs to Loxone Config (lox refuses to set it).
        if co.child_elements().any(|c| c.name == "In") || co.attr("Inv").is_some() {
            continue;
        }
        let input = match specs {
            Some(specs) => specs
                .iter()
                .find(|s| s.key == key)
                .is_some_and(|s| matches!(s.dir, PortDir::Input | PortDir::Param)),
            // A type lox has no port table for: an input that holds a
            // constant is one; an output never does.
            None => co.attr("Def").is_some(),
        };
        if input {
            params.push(ParamCap {
                key: key.to_string(),
                unit: connectors::port_unit(ty, key).map(unit_suffix),
            });
        }
    }
    let settings = settings_of(ty)
        .into_iter()
        .filter(|(name, _, add)| *add || el.attr(name).is_some())
        .map(|(name, text, _)| SettingCap {
            name: name.into(),
            text,
            value: el.attr_decoded(name).map(|v| v.into_owned()),
        })
        .collect();
    let io = el.child_elements().find(|c| c.name == "IoData");
    let bound = |a: &str| io.and_then(|io| io.attr(a)).is_some_and(|v| !v.is_empty());
    let caps = Caps {
        title: el.attr("Title").is_some(),
        room: bound("Pr"),
        category: bound("Cr"),
        params,
        settings,
    };
    (caps.title
        || caps.room
        || caps.category
        || !caps.params.is_empty()
        || !caps.settings.is_empty())
    .then_some(caps)
}

// ── the claims file ─────────────────────────────────────────────────────

/// What [`FILE`] claims, by object UUID, and the wires it draws (#61).
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claims(
    pub BTreeMap<String, Claim>,
    pub BTreeSet<Wire>,
    /// The switching times it writes (#73).
    pub crate::sched_edit::Scheds,
    /// The lighting moods it writes (#74).
    pub crate::mood_edit::MoodSets,
);

/// A wire vislox draws: `to.port <- from.port`, each end a slug (an
/// extern of [`FILE`] or a block vislox added) and a Lox Logic port name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wire {
    pub to: (String, String),
    pub from: (String, String),
}

impl Wire {
    fn render(&self) -> String {
        format!(
            "{}.{} <- {}.{}",
            self.to.0, self.to.1, self.from.0, self.from.1
        )
    }

    fn touches(&self, slug: &str) -> bool {
        self.to.0 == slug || self.from.0 == slug
    }
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claim {
    /// The object's type, as the extern declares it.
    pub ty: String,
    /// Its title, room and category.
    pub attrs: BTreeMap<Which, String>,
    /// Port (by its Lox Logic name) or setting → value.
    pub sets: BTreeMap<String, Value>,
    /// A block adopted here (#72) names this extern: it stays while it does.
    pub pinned: bool,
}

impl Claim {
    fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.sets.is_empty() && !self.pinned
    }

    /// It claims a value (not only an extern something names).
    pub fn claims(&self) -> bool {
        !self.attrs.is_empty() || !self.sets.is_empty()
    }
}

/// The slug vislox gives the object `uuid`.
pub fn slug(uuid: &str) -> String {
    format!(
        "{SLUG_PREFIX}{}",
        uuid.to_ascii_lowercase().replace('-', "_")
    )
}

/// A value as Properties shows it: a number or text, unquoted.
pub fn plain(v: &Value) -> String {
    match v {
        Value::Number(n) => n.clone(),
        Value::Str(s) => s.clone(),
        other => other.to_string(),
    }
}

impl Claims {
    /// Read [`FILE`]. Only what vislox writes is accepted, so that a
    /// rewrite never drops a statement someone added by hand.
    pub fn parse(text: &str) -> Result<Claims, String> {
        // A fragment: a wire may name a block of another file of vislox's.
        let m = Module::parse_fragment(text).map_err(|e| e.to_string())?;
        let foreign = |what: &str| {
            format!(
                "it holds {what}, which vislox did not write; move it to another file of the module"
            )
        };
        let mut by_slug: BTreeMap<String, String> = BTreeMap::new();
        let mut c = Claims::default();
        let set = |c: &mut Claims,
                   by_slug: &BTreeMap<String, String>,
                   slug: &str,
                   port: &str,
                   v: Value|
         -> Result<(), String> {
            let uuid = by_slug
                .get(slug)
                .ok_or_else(|| foreign(&format!("`{slug}.{port}`")))?;
            let claim = c.0.get_mut(uuid).expect("declared");
            claim.sets.insert(port.to_string(), v);
            Ok(())
        };
        for item in m.items {
            match item {
                Item::Extern(e) => {
                    let MatchSpec::Uuid(uuid) = &e.match_spec else {
                        return Err(foreign(&format!("`extern {}` not pinned by uuid", e.slug)));
                    };
                    if e.slug != slug(uuid) {
                        return Err(foreign(&format!("`extern {}`", e.slug)));
                    }
                    by_slug.insert(e.slug.clone(), uuid.clone());
                    c.0.insert(
                        uuid.clone(),
                        Claim {
                            ty: e.block_type.clone(),
                            ..Default::default()
                        },
                    );
                    // `lox logic fmt` may fold the assignments into the declaration.
                    for arg in e.args {
                        match arg {
                            ArgItem::Binding(b) => match b.kind {
                                BindingKind::Param(v) => {
                                    set(&mut c, &by_slug, &e.slug, &b.port, v)?
                                }
                                // `lox logic fmt` may fold a wire in too.
                                BindingKind::Wire(r) if !r.port.is_empty() => {
                                    c.1.insert(Wire {
                                        to: (e.slug.clone(), b.port.clone()),
                                        from: (r.slug, r.port),
                                    });
                                }
                                _ => {
                                    return Err(foreign(&format!(
                                        "an expression onto `{}`",
                                        e.slug
                                    )));
                                }
                            },
                            _ => return Err(foreign(&format!("`{}`'s arguments", e.slug))),
                        }
                    }
                }
                Item::Set(s) => set(&mut c, &by_slug, &s.target.slug, &s.target.port, s.value)?,
                Item::Wire(w) if !w.from.port.is_empty() && !w.to.port.is_empty() => {
                    c.1.insert(Wire {
                        to: (w.to.slug, w.to.port),
                        from: (w.from.slug, w.from.port),
                    });
                }
                Item::Attr(a) => {
                    let which = match a.attr {
                        Attr::Title => Which::Title,
                        Attr::Room => Which::Room,
                        Attr::Category => Which::Category,
                    };
                    let uuid = by_slug
                        .get(&a.slug)
                        .ok_or_else(|| foreign(&format!("`{a}`")))?;
                    c.0.get_mut(uuid)
                        .expect("declared")
                        .attrs
                        .insert(which, plain(&a.value));
                }
                Item::Schedule(s) if s.name.starts_with(SLUG_PREFIX) => {
                    if c.2.sets.contains_key(&s.name) {
                        return Err(foreign(&format!("a second `schedule {}`", s.name)));
                    }
                    c.2.sets
                        .insert(s.name.clone(), s.lines().cloned().collect());
                }
                Item::ScheduleClaim(sc) if sc.slug.starts_with(SLUG_PREFIX) => {
                    if c.2
                        .claims
                        .insert(sc.slug.clone(), sc.schedule.clone())
                        .is_some()
                    {
                        return Err(foreign(&format!("a second `{}.schedule`", sc.slug)));
                    }
                }
                Item::Moods(m) if m.name.starts_with(SLUG_PREFIX) => {
                    if c.3.sets.contains_key(&m.name) {
                        return Err(foreign(&format!("a second `moods {}`", m.name)));
                    }
                    c.3.sets
                        .insert(m.name.clone(), m.moods().cloned().collect());
                }
                Item::MoodsClaim(mc) if mc.slug.starts_with(SLUG_PREFIX) => {
                    if c.3
                        .claims
                        .insert(mc.slug.clone(), mc.moods.clone())
                        .is_some()
                    {
                        return Err(foreign(&format!("a second `{}.moods`", mc.slug)));
                    }
                }
                Item::Gap | Item::FragmentEnd | Item::Comment(_) => {}
                _ => return Err(foreign("a statement")),
            }
        }
        // A claim names one of the file's sets.
        for (t, set) in &c.2.claims {
            if !c.2.sets.contains_key(set) {
                return Err(foreign(&format!("`{t}.schedule = {set}` without its set")));
            }
        }
        for (t, set) in &c.3.claims {
            if !c.3.sets.contains_key(set) {
                return Err(foreign(&format!("`{t}.moods = {set}` without its set")));
            }
        }
        // A wire's ends are vislox's: its externs, or the blocks it added.
        for w in &c.1 {
            for end in [&w.to.0, &w.from.0] {
                if !end.starts_with(SLUG_PREFIX) {
                    return Err(foreign(&format!("the wire `{}`", w.render())));
                }
            }
        }
        Ok(c)
    }

    /// Whether a wire ends at `slug`.
    pub fn wired(&self, slug: &str) -> bool {
        self.1.iter().any(|w| w.touches(slug))
    }

    /// Whether a wire ends at `slug`, or a schedule (#73) or moods (#74)
    /// claim names it.
    fn named(&self, slug: &str) -> bool {
        self.wired(slug) || self.2.claims.contains_key(slug) || self.3.claims.contains_key(slug)
    }

    /// Whether a set claim of the file (switching times, moods) names `slug`.
    pub fn set_claimed(&self, slug: &str) -> bool {
        self.2.claims.contains_key(slug) || self.3.claims.contains_key(slug)
    }

    /// Drop the externs that claim nothing and no wire or set claim names.
    pub(crate) fn prune(&mut self) {
        let keep: BTreeSet<String> = self
            .0
            .keys()
            .filter(|u| self.named(&slug(u)))
            .cloned()
            .collect();
        self.0.retain(|u, c| !c.is_empty() || keep.contains(u));
    }

    /// The file's text: a header, then each object's extern and its values.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# Values edited in vislox (Properties). vislox rewrites this file on every edit;\n\
             # see docs/editing.md in vislox. Deleting a line hands that value back to Loxone Config.\n",
        );
        for (uuid, c) in &self.0 {
            let s = slug(uuid);
            if c.is_empty() && !self.named(&s) {
                continue;
            }
            out.push_str(&format!(
                "\nextern {s} = {}(uuid: {})\n",
                c.ty,
                Value::Str(uuid.clone())
            ));
            for (w, t) in &c.attrs {
                out.push_str(&format!("{s}.{} = {}\n", w.word(), Value::Str(t.clone())));
            }
            for (k, v) in &c.sets {
                out.push_str(&format!("{s}.{k} = {v}\n"));
            }
        }
        out.push_str(&self.2.render());
        out.push_str(&self.3.render());
        if !self.1.is_empty() {
            out.push_str("\n# Wires drawn in vislox. Deleting a line takes the wire out again.\n");
        }
        for w in &self.1 {
            out.push_str(&w.render());
            out.push('\n');
        }
        out
    }

    /// The value claimed for what `op` changes.
    pub fn get(&self, ty: &str, op: &Op) -> Option<String> {
        let c = self.0.get(op.uuid()?)?;
        match op {
            Op::Title { .. } => c.attrs.get(&Which::Title).cloned(),
            Op::Room { .. } => c.attrs.get(&Which::Room).cloned(),
            Op::Category { .. } => c.attrs.get(&Which::Category).cloned(),
            Op::Param { port, .. } => c.sets.get(port_names::name_of(ty, port)).map(plain),
            Op::Setting { name, .. } => c.sets.get(name).map(plain),
            Op::AddBlock { .. }
            | Op::RemoveBlock { .. }
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. }
            | Op::Schedule { .. }
            | Op::Moods { .. } => None,
        }
    }

    /// Every claimed value, as the ops that set it.
    pub fn ops(&self) -> Vec<Op> {
        let mut v = Vec::new();
        for (uuid, c) in &self.0 {
            for (w, t) in &c.attrs {
                v.push(w.op(uuid.clone(), Some(t.clone())));
            }
            let settings = settings_of(&c.ty);
            for (k, val) in &c.sets {
                let value = Some(plain(val));
                if settings.iter().any(|(n, ..)| n == k) {
                    v.push(Op::Setting {
                        uuid: uuid.clone(),
                        name: k.clone(),
                        value,
                    });
                } else {
                    v.push(Op::Param {
                        uuid: uuid.clone(),
                        port: port_names::key_or_raw(&c.ty, k).to_string(),
                        value,
                    });
                }
            }
        }
        v
    }
}

// ── applying an op ──────────────────────────────────────────────────────

/// The element with UUID `uuid`.
pub fn find<'a>(el: &'a Element, uuid: &str) -> Option<&'a Element> {
    if el.attr("U") == Some(uuid) {
        return Some(el);
    }
    el.child_elements().find_map(|c| find(c, uuid))
}

/// How many objects of type `ty` are titled `title`: a room or category is
/// claimed by its title, which must name exactly one.
fn titled(el: &Element, ty: &str, title: &str) -> usize {
    let me = usize::from(
        el.attr("Type") == Some(ty) && el.attr_decoded("Title").is_some_and(|t| t == title),
    );
    me + el
        .child_elements()
        .map(|c| titled(c, ty, title))
        .sum::<usize>()
}

/// Whether `s` is a plain decimal number (`-2.5`), the only spelling lox
/// writes into `Def=` unchanged.
pub(crate) fn number(s: &str) -> Result<String, String> {
    let n = s.trim();
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let body = n.strip_prefix('-').unwrap_or(n);
    let ok = match body.split_once('.') {
        Some((i, f)) => digits(i) && digits(f),
        None => digits(body),
    };
    if ok {
        Ok(n.to_string())
    } else if n.contains(',') {
        Err(format!(
            "`{n}` is not a number: write a decimal point, not a comma"
        ))
    } else {
        Err(format!("`{n}` is not a number (like 12 or -2.5)"))
    }
}

/// A name or other text value: one line, not empty, not too long.
fn text(s: &str, what: &str, empty_ok: bool) -> Result<String, String> {
    if s.chars().any(|c| c.is_control()) {
        return Err(format!("{what} must be one line of text"));
    }
    if !empty_ok && s.trim().is_empty() {
        return Err(format!("{what} cannot be empty"));
    }
    if s.chars().count() > 120 {
        return Err(format!("{what} is longer than 120 characters"));
    }
    Ok(s.to_string())
}

/// Apply `op` to `claims`, checked against the pending config `doc`.
/// Returns the value claimed before (`None`: none), which undoes it.
pub fn apply(
    claims: &mut Claims,
    doc: &LoxoneDoc,
    managed: &HashSet<String>,
    op: &Op,
) -> Result<Option<String>, String> {
    let uuid = op
        .uuid()
        .filter(|_| {
            !matches!(
                op,
                Op::RemoveBlock { .. } | Op::Move { .. } | Op::Schedule { .. } | Op::Moods { .. }
            )
        })
        .ok_or_else(|| "not a value".to_string())?;
    let el = find(&doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let ty = el.attr("Type").unwrap_or_default().to_string();
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    if managed.contains(uuid) {
        return Err(format!(
            "{name}: the module manages this block; change it in its .loxl file"
        ));
    }
    let previous = claims.get(&ty, op);
    let caps = caps(el).unwrap_or_default();
    // `None` drops the claim, also where the value can no longer be edited.
    let (key, value): (Result<Which, String>, Option<Value>) = match op {
        Op::Title { value, .. } => {
            let v = match value {
                Some(v) if caps.title => Some(Value::Str(text(v, "The name", false)?)),
                Some(_) => return Err(format!("{name}: the name cannot be edited here")),
                None => None,
            };
            (Ok(Which::Title), v)
        }
        Op::Room { value, .. } | Op::Category { value, .. } => {
            let room = matches!(op, Op::Room { .. });
            let (what, kind, can) = if room {
                ("room", "Place", caps.room)
            } else {
                ("category", "Category", caps.category)
            };
            let v = match value {
                Some(v) if can => {
                    let n = titled(&doc.xml.root, kind, v);
                    if n != 1 {
                        return Err(format!(
                            "{name}: {} {what} is titled \"{v}\"; pick one of the config's {what}s",
                            if n == 0 { "no" } else { "more than one" }
                        ));
                    }
                    Some(Value::Str(v.clone()))
                }
                Some(_) => {
                    return Err(format!(
                        "{name}: it has no {what}; set one in Loxone Config first"
                    ));
                }
                None => None,
            };
            (Ok(if room { Which::Room } else { Which::Category }), v)
        }
        Op::Param { port, value, .. } => {
            let pname = port_names::name_of(&ty, port).to_string();
            let v = match value {
                Some(v) if caps.params.iter().any(|p| &p.key == port) => Some(Value::Number(
                    number(v).map_err(|e| format!("{pname}: {e}"))?,
                )),
                Some(_) => {
                    return Err(format!(
                        "{name}: {pname} cannot be edited here (it is wired, or not an input)"
                    ));
                }
                None => None,
            };
            (Err(pname), v)
        }
        Op::Setting {
            name: setting,
            value,
            ..
        } => {
            let v = match (value, caps.settings.iter().find(|s| &s.name == setting)) {
                // Settings are optional text: empty clears them (`Desc=""`).
                (Some(v), Some(cap)) if cap.text => Some(Value::Str(text(v, setting, true)?)),
                (Some(v), Some(_)) => {
                    let n = number(v).map_err(|e| format!("{setting}: {e}"))?;
                    if NON_NEGATIVE.contains(&setting.as_str()) && n.starts_with('-') {
                        return Err(format!("{setting}: cannot be below 0"));
                    }
                    Some(Value::Number(n))
                }
                (Some(_), None) => {
                    return Err(format!("{name}: {setting} cannot be edited here"));
                }
                (None, _) => None,
            };
            (Err(setting.clone()), v)
        }
        Op::AddBlock { .. }
        | Op::RemoveBlock { .. }
        | Op::Move { .. }
        | Op::Relayout { .. }
        | Op::Place { .. }
        | Op::Wire { .. }
        | Op::Unwire { .. }
        | Op::Adopt { .. }
        | Op::AdoptPage { .. }
        | Op::Unadopt { .. }
        | Op::Schedule { .. }
        | Op::Moods { .. } => {
            unreachable!("not a value")
        }
    };
    let claim = claims.0.entry(uuid.to_string()).or_default();
    claim.ty = ty;
    match (key, value) {
        (Ok(w), Some(v)) => {
            claim.attrs.insert(w, plain(&v));
        }
        (Ok(w), None) => {
            claim.attrs.remove(&w);
        }
        (Err(k), Some(v)) => {
            claim.sets.insert(k, v);
        }
        (Err(k), None) => {
            claim.sets.remove(&k);
        }
    }
    claims.prune();
    Ok(previous)
}

// ── vislox's files, and the blocks it adds ──────────────────────────────

/// What the lockfile says about managed blocks: slug → UUID, the slugs of
/// its removal tombstones (not to be picked again), and where the blocks
/// drawn on a page are.
#[derive(Default, Clone, Debug)]
pub struct Lock {
    pub slugs: BTreeMap<String, String>,
    pub tombstones: BTreeSet<String>,
    /// The blocks lox keeps a position (`layout`) for, so drawn on a page
    /// (#60): slug → the page it pins them to (`page_uuid`; `None` in a lock
    /// from before lox pinned pages). Objects of the object tree have none.
    pub placed: BTreeMap<String, Option<String>>,
    /// The wires lox draws onto the ports of externs (`extern_wires`):
    /// (source connector UUID, sink connector UUID).
    pub wires: BTreeSet<(String, String)>,
    /// The objects the module's externs resolved to (`externals`): slug →
    /// UUID. Loxone Config's, not managed; an extern by title (`Notification(
    /// title: "Frost")`) names its object only here (#68).
    pub externals: BTreeMap<String, String>,
}

impl Lock {
    pub fn read(path: &Path) -> Lock {
        let Ok(bytes) = std::fs::read(path) else {
            return Lock::default();
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Lock::default();
        };
        let slugs = v
            .get("objects")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(s, x)| Some((s.clone(), x.get("uuid")?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let tombstones = v
            .get("removed")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.values()
                    .filter_map(|x| Some(x.get("slug")?.as_str()?.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let placed = v
            .get("objects")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter(|(_, x)| x.get("layout").is_some_and(|l| l.is_object()))
                    .map(|(s, x)| {
                        let page = x
                            .get("page_uuid")
                            .and_then(|p| p.as_str())
                            .map(String::from);
                        (s.clone(), page)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let wires = v
            .get("extern_wires")
            .and_then(|o| o.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|w| {
                        Some((
                            w.get("from")?.as_str()?.to_string(),
                            w.get("to")?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let externals = v
            .get("externals")
            .and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(s, x)| Some((s.clone(), x.get("uuid")?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Lock {
            slugs,
            tombstones,
            placed,
            wires,
            externals,
        }
    }

    pub fn slug_of(&self, uuid: &str) -> Option<&str> {
        self.slugs
            .iter()
            .find(|(_, u)| u.as_str() == uuid)
            .map(|(s, _)| s.as_str())
    }

    pub fn uuid_of(&self, slug: &str) -> Option<&str> {
        self.slugs.get(slug).map(String::as_str)
    }
}

/// vislox's files in the module ([`DIR`]): the values it claims ([`FILE`])
/// and the blocks it added, by file name.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Files {
    pub claims: Claims,
    pub pages: BTreeMap<String, PageFile>,
    /// The files there were. [`FILE`] stays once there, empty or not; a
    /// page file goes with its last block.
    pub present: BTreeSet<String>,
    /// What an edit moves (#60). Not a file of vislox's: lox keeps a managed
    /// block's position in the lockfile, so it is written there.
    pub layout: Placement,
    /// What adopting a block (#72) or its undo changes in the lockfile.
    pub lock_edits: crate::adopt::LockEdits,
}

/// The positions an edit gives managed blocks, and the pages it lays out
/// afresh. lox has no Lox Logic statement for a position: it keeps the
/// rectangle of each block it manages in the lockfile, and takes a move
/// made in Loxone Config over from the base.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Placement {
    /// Slug → its new rectangle.
    pub moves: BTreeMap<String, Layout>,
    /// The pages to lay out afresh: UUID → the title lox takes (`--relayout`).
    pub relayout: BTreeMap<String, String>,
}

/// The name of [`FILE`] in [`DIR`].
const CLAIMS: &str = "edits.loxl";

impl Files {
    /// The files from their texts, by name ([`Files::texts`]). A file
    /// vislox did not write is an error that names it.
    pub fn from_texts(texts: &BTreeMap<String, String>) -> Result<Files, String> {
        let mut f = Files {
            present: texts.keys().cloned().collect(),
            ..Files::default()
        };
        for (name, text) in texts {
            if name == CLAIMS {
                f.claims = Claims::parse(text).map_err(|e| format!("{name}: {e}"))?;
            } else {
                f.pages.insert(
                    name.clone(),
                    PageFile::parse(text).map_err(|e| format!("{name}: {e}"))?,
                );
            }
        }
        // A block adopted here names externs of the claims file, or blocks of
        // vislox's files; those externs stay while it does.
        for (s, file) in f.adopted_names() {
            if f.block(&s).is_some() {
                continue;
            }
            match f.claims.0.iter_mut().find(|(u, _)| slug(u) == s) {
                Some((_, c)) => c.pinned = true,
                None => {
                    return Err(format!(
                        "{file}: a block adopted there names `{s}`, which is neither an extern of \
                         {CLAIMS} nor a block of vislox's files; put its line back"
                    ));
                }
            }
        }
        // A schedule (#73) or moods (#74) claim names an extern of the
        // claims file or a block of vislox's files.
        for (t, what) in f
            .claims
            .2
            .claims
            .keys()
            .map(|t| (t, "schedule"))
            .chain(f.claims.3.claims.keys().map(|t| (t, "moods")))
        {
            if !f.claims.0.keys().any(|u| slug(u) == *t) && f.block(t).is_none() {
                return Err(format!(
                    "{CLAIMS}: `{t}.{what}` names `{t}`, which is neither an extern of this file \
                     nor a block vislox added; put its line back or delete the claim"
                ));
            }
        }
        // Every end of a wire is an extern of the claims file or a block
        // vislox added; anything else would break every compile.
        for w in &f.claims.1 {
            for (s, _) in [&w.from, &w.to] {
                let ext = f.claims.0.keys().any(|u| slug(u) == *s);
                if !ext && f.block(s).is_none() {
                    return Err(format!(
                        "{CLAIMS}: the wire `{}` names `{s}`, which is neither an extern of this \
                         file nor a block vislox added; put its line back or delete the wire",
                        w.render()
                    ));
                }
            }
        }
        Ok(f)
    }

    /// The `.loxl` files of `dir`, by name.
    pub fn texts(dir: &Path) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !name.ends_with(".loxl") || !e.path().is_file() {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                out.insert(name, t);
            }
        }
        out
    }

    /// Each file's text, by name. A page file with no block and no
    /// `removed` line is left out (deleted); [`FILE`] only while it was never
    /// written and claims nothing.
    pub fn render(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if !self.claims.0.values().all(Claim::is_empty)
            || !self.claims.1.is_empty()
            || !self.claims.2.is_empty()
            || !self.claims.3.is_empty()
            || self.present.contains(CLAIMS)
        {
            out.insert(CLAIMS.to_string(), self.claims.render());
        }
        for (name, p) in &self.pages {
            if !p.is_empty() {
                out.insert(name.clone(), p.render());
            }
        }
        out
    }

    /// The text of [`FILE`] with nothing claimed: what keeps a module
    /// directory from having no `.loxl` file at all, which lox refuses.
    pub fn placeholder() -> (String, String) {
        (CLAIMS.to_string(), Claims::default().render())
    }

    /// Tie each page file to its page in `base` by UUID, and follow a page
    /// Loxone Config renamed: `page "…"` names the page by its title. A file
    /// written before it had a UUID takes the first page with its title.
    pub fn follow_pages(&mut self, base: &LoxoneDoc) {
        for p in self.pages.values_mut() {
            match &p.uuid {
                Some(u) => {
                    if let Some(t) = find(&base.xml.root, u)
                        .filter(|e| e.attr("Type") == Some("Page"))
                        .and_then(|e| e.attr_decoded("Title"))
                        .filter(|t| !t.trim().is_empty())
                    {
                        p.page = t.into_owned();
                    }
                }
                None => {
                    let mut same = Vec::new();
                    pages_titled(&base.xml.root, &p.page, &mut same);
                    p.uuid = same.first().and_then(|e| e.attr("U")).map(String::from);
                }
            }
        }
    }

    /// The names the blocks adopted here use for the objects they are wired
    /// to (not themselves), with the file that names them.
    pub fn adopted_names(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (name, p) in &self.pages {
            for b in p.blocks.iter().filter(|b| b.adopted) {
                for (_, src) in &b.wires {
                    out.push((src.slug.clone(), name.clone()));
                }
            }
            for w in &p.wires {
                out.push((w.to.slug.clone(), name.clone()));
            }
        }
        out
    }

    /// Mark the externs the blocks adopted here name, and drop the externs
    /// nothing claims, wires or names any more.
    fn repin(&mut self) {
        let names: BTreeSet<String> = self.adopted_names().into_iter().map(|(s, _)| s).collect();
        for (u, c) in self.claims.0.iter_mut() {
            c.pinned = names.contains(&slug(u));
        }
        self.claims.prune();
    }

    /// The block vislox added as `slug`: its file and declaration.
    pub fn block(&self, slug: &str) -> Option<(&str, &NewBlock)> {
        self.pages.iter().find_map(|(n, p)| {
            p.blocks
                .iter()
                .find(|b| b.slug == slug)
                .map(|b| (n.as_str(), b))
        })
    }

    fn block_mut(&mut self, slug: &str) -> Option<&mut NewBlock> {
        self.pages
            .values_mut()
            .find_map(|p| p.blocks.iter_mut().find(|b| b.slug == slug))
    }

    /// Whether a block or removal of vislox's files is named `slug`.
    pub fn names(&self, slug: &str) -> bool {
        self.pages
            .values()
            .any(|p| p.removed.contains(slug) || p.blocks.iter().any(|b| b.slug == slug))
    }

    /// Drop the `removed` lines lox needs no more: the lockfile has
    /// forgotten the block (or never knew it, when it was added and taken
    /// out before a compile).
    pub fn tidy(&mut self, lock: &Lock) {
        for p in self.pages.values_mut() {
            p.removed.retain(|s| lock.slugs.contains_key(s));
        }
    }

    /// Every value vislox claims or wrote into a block it added (by the
    /// block's UUID in `lock`), as the ops that set it.
    pub fn ops(&self, lock: &Lock) -> Vec<Op> {
        let mut v = self.claims.ops();
        for p in self.pages.values() {
            for b in &p.blocks {
                let Some(uuid) = lock.uuid_of(&b.slug) else {
                    continue;
                };
                if let Some(t) = &b.title {
                    v.push(Op::Title {
                        uuid: uuid.into(),
                        value: Some(t.clone()),
                    });
                }
                let (params, settings) = split_args(b);
                for (port, value) in params {
                    v.push(Op::Param {
                        uuid: uuid.into(),
                        port,
                        value: Some(value),
                    });
                }
                for (name, value) in settings {
                    v.push(Op::Setting {
                        uuid: uuid.into(),
                        name,
                        value: Some(value),
                    });
                }
            }
        }
        v
    }
}

/// A new block's arguments as the ops name them: port key → constant, and
/// setting → value, each as Properties shows it.
pub fn split_args(b: &NewBlock) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let (mut params, mut settings) = (BTreeMap::new(), BTreeMap::new());
    for (k, v) in &b.args {
        let key = blocks::port_key(&b.ty, k);
        if blocks::port_name(&b.ty, key).is_some() {
            params.insert(key.to_string(), plain(v));
        } else {
            settings.insert(k.clone(), plain(v));
        }
    }
    (params, settings)
}

/// The settings Properties edits on a block added here (#59): each with how
/// it is written and the value the declaration gives it.
pub fn new_block_settings(b: &NewBlock) -> serde_json::Value {
    blocks::settings(&b.ty)
        .into_iter()
        .map(|(name, kind)| {
            serde_json::json!({
                "name": name,
                "kind": kind,
                "choices": connectors::choice_values(&b.ty, name)
                    .map(|c| c.iter().map(|(w, _)| *w).collect::<Vec<_>>()),
                "value": b.get(name).map(plain),
            })
        })
        .collect()
}

/// What an op is checked against.
pub struct Ctx<'a> {
    /// The pending config, which the browser showed.
    pub doc: &'a LoxoneDoc,
    /// The project's base: lox compiles the module against it, so a
    /// `page "…"` title must name one of its pages.
    pub base: &'a LoxoneDoc,
    pub lock: &'a Lock,
    /// The deployed revision (the History's: the newest commit of the
    /// base), to tell whether a block added here is deployed. `None`: none.
    pub deployed: Option<&'a LoxoneDoc>,
    /// The names the module's other files declare.
    pub names: &'a BTreeSet<String>,
    /// The wires vislox's file drew when the request began: one the
    /// pending config has and an earlier op of the request took out is not
    /// wired any more (#61).
    pub drawn: &'a BTreeSet<Wire>,
    /// What `lox logic adopt` made of the blocks the request adopts (#72),
    /// by UUID, worked out on a scratch copy before the ops are applied.
    pub adoptions: &'a BTreeMap<String, crate::adopt::Adoption>,
    /// For each `adopt_page`, the blocks it adopts, by page UUID.
    pub page_adoptions: &'a BTreeMap<String, Vec<String>>,
    /// The timers whose switching times the rest of the module claims
    /// (#73): UUID (lowercase) → the claim's `file:line`.
    pub sched_claimed: &'a BTreeMap<String, String>,
    /// The lighting controllers whose moods the rest of the module claims
    /// (#74): UUID (lowercase) → the claim's `file:line`.
    pub moods_claimed: &'a BTreeMap<String, String>,
}

/// What [`apply_op`] did: the value or op that undoes it (`previous`),
/// and the slug of a block it added.
#[derive(Debug, PartialEq)]
pub struct Applied {
    pub previous: serde_json::Value,
    pub added: Option<String>,
}

/// Apply `op` to vislox's files, checked against `ctx`.
pub fn apply_op(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let done = |p: Option<String>| Applied {
        previous: serde_json::json!(p),
        added: None,
    };
    match op {
        Op::AddBlock { .. } => add_block(files, ctx, op),
        Op::RemoveBlock { uuid } => remove_block(files, ctx, uuid),
        Op::Move { uuid, x, y } => move_block(files, ctx, uuid, *x, *y),
        Op::Relayout { page } => relayout(files, ctx, page),
        Op::Place { moves } => place(files, ctx, moves),
        Op::Wire { from, to } => wire(files, ctx, from, to),
        Op::Unwire { from, to } => unwire(files, ctx, from, to),
        Op::Adopt { blocks } => {
            let uuids: Vec<String> = blocks.iter().map(|b| b.uuid.clone()).collect();
            adopt(files, ctx, &uuids)
        }
        Op::AdoptPage { page } => {
            let uuids = ctx
                .page_adoptions
                .get(page)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| "lox can adopt no block of this page".to_string())?;
            adopt(files, ctx, uuids)
        }
        Op::Unadopt { uuids } => unadopt(files, ctx, uuids),
        Op::Schedule { uuid, value } => crate::sched_edit::apply(files, ctx, uuid, value.as_ref()),
        Op::Moods { uuid, value } => crate::mood_edit::apply(files, ctx, uuid, value.as_ref()),
        _ => {
            let uuid = op.uuid().unwrap_or_default();
            match ctx.lock.slug_of(uuid) {
                Some(slug) if files.block(slug).is_some() => {
                    let slug = slug.to_string();
                    edit_new_block(files, ctx, &slug, op).map(done)
                }
                _ => {
                    let managed: HashSet<String> = ctx.lock.slugs.values().cloned().collect();
                    apply(&mut files.claims, ctx.doc, &managed, op).map(done)
                }
            }
        }
    }
}

/// The ops that undo a request (#62), from each op's `previous`: the last
/// op's first, so a request of `unwire` then `wire` is undone by `unwire`
/// then `wire` of the old ends: one op for each op of the request, so it
/// never holds more than [`MAX_OPS`]. A value op's `previous` is the value
/// it replaced; the others' is the op.
/// `minted` is, per op, the UUID lox minted for the block an `add_block`
/// added: it is undone by `remove_block` of that UUID.
///
/// This is only a convenience over `previous`: the browser sends the list
/// back as an ordinary edit, checked and compiled like any other.
pub fn undo_ops(
    ops: &[Op],
    previous: &[serde_json::Value],
    minted: &[Option<String>],
) -> Result<Vec<Op>, String> {
    let mut undo = Vec::new();
    for (i, op) in ops.iter().enumerate().rev() {
        let prev = previous.get(i).cloned().unwrap_or_default();
        let bad = |e: serde_json::Error| {
            format!(
                "op {} ({}): its previous is no op ({e})",
                i + 1,
                op_name(op)
            )
        };
        match op {
            Op::AddBlock { .. } => match minted.get(i).cloned().flatten() {
                Some(uuid) => undo.push(Op::RemoveBlock { uuid }),
                None => {
                    return Err(format!(
                        "op {} (add_block): lox named no UUID for the block it added",
                        i + 1
                    ));
                }
            },
            Op::RemoveBlock { .. }
            | Op::Move { .. }
            | Op::Relayout { .. }
            | Op::Place { .. }
            | Op::Wire { .. }
            | Op::Unwire { .. }
            | Op::Adopt { .. }
            | Op::AdoptPage { .. }
            | Op::Unadopt { .. } => undo.push(serde_json::from_value(prev).map_err(bad)?),
            Op::Moods { uuid, .. } => undo.push(Op::Moods {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            Op::Schedule { uuid, .. } => undo.push(Op::Schedule {
                uuid: uuid.clone(),
                value: serde_json::from_value(prev).map_err(bad)?,
            }),
            _ => undo.push(op.with_value(serde_json::from_value(prev).map_err(bad)?)),
        }
    }
    Ok(undo)
}

/// The name an op has in a request (`title`, `add_block`, …).
fn op_name(op: &Op) -> String {
    serde_json::to_value(op)
        .ok()
        .and_then(|v| v["op"].as_str().map(String::from))
        .unwrap_or_default()
}

/// The pages titled `title`, in document order.
fn pages_titled<'a>(el: &'a Element, title: &str, out: &mut Vec<&'a Element>) {
    if el.attr("Type") == Some("Page") && el.attr_decoded("Title").is_some_and(|t| t == title) {
        out.push(el);
    }
    for c in el.child_elements() {
        pages_titled(c, title, out);
    }
}

/// A name for a new block: one line, and as a Loxone Config save keeps it.
fn block_title(ty: &str, t: &str) -> Result<String, String> {
    let t = text(t, "The name", false)?;
    if let Some(w) = connectors::title_save_rewrite(ty, &t) {
        return Err(format!(
            "The name: a Loxone Config save would write it as \"{w}\" (it drops {{ }}, and \
             turns % < | and spaces at either end into _); write that instead"
        ));
    }
    Ok(t)
}

/// A setting's value as lox takes it.
fn setting_value(ty: &str, name: &str, kind: blocks::Kind, v: &str) -> Result<Value, String> {
    use blocks::Kind;
    Ok(match kind {
        Kind::Text => Value::Str(text(v, name, connectors::empty_ok(ty, name))?),
        Kind::Choice => {
            let c = connectors::choice_values(ty, name).unwrap_or_default();
            if !c.iter().any(|(w, _)| *w == v) {
                return Err(format!("{name}: one of {}", connectors::choice_list(c)));
            }
            Value::Str(v.to_string())
        }
        Kind::OnOff => match v {
            "on" => Value::Bool(true),
            "off" => Value::Bool(false),
            _ => return Err(format!("{name}: on or off")),
        },
        Kind::Time => Value::Clock(
            logic::clock_seconds(v.trim())
                .and_then(logic::clock_text)
                .ok_or_else(|| format!("{name}: a time of day, like 19:00 or 06:30:15"))?,
        ),
        Kind::Number => Value::Number(number(v).map_err(|e| format!("{name}: {e}"))?),
    })
}

fn add_block(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::AddBlock {
        page,
        ty,
        title,
        params,
        settings,
        wires,
        ..
    } = op
    else {
        unreachable!()
    };
    let pg = find(&ctx.base.xml.root, page)
        .filter(|e| e.attr("Type") == Some("Page"))
        .ok_or_else(|| format!("no page {page} in the project's base config"))?;
    let page_title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
    if page_title.trim().is_empty() {
        return Err("the page has no title; name it in Loxone Config first".into());
    }
    // `page "…"` names a page by its title, and lox builds on the first.
    let mut same = Vec::new();
    pages_titled(&ctx.base.xml.root, &page_title, &mut same);
    if same.first().and_then(|e| e.attr("U")) != Some(page.as_str()) {
        return Err(format!(
            "{} pages are titled \"{page_title}\", and lox puts new blocks on the first; \
             rename one in Loxone Config to add blocks here",
            same.len()
        ));
    }
    if !blocks::offered(ty) {
        return Err(format!("vislox does not add blocks of type {ty}"));
    }
    let title = title.as_deref().map(|t| block_title(ty, t)).transpose()?;
    if let Some(key) = params.keys().find(|k| blocks::port_name(ty, k).is_none()) {
        return Err(format!("{ty} has no input {key}"));
    }
    let offered = blocks::settings(ty);
    if let Some(name) = settings
        .keys()
        .find(|n| !offered.iter().any(|(o, _)| o == n))
    {
        return Err(format!("{ty}: {name} cannot be set here"));
    }
    // In the order lox lists them: the ports, then the settings.
    let mut args = Vec::new();
    for spec in blocks::valued_ports(ty) {
        if let Some(v) = params.get(spec.key) {
            let n = spec.name;
            args.push((
                n.to_string(),
                Value::Number(number(v).map_err(|e| format!("{n}: {e}"))?),
            ));
        }
    }
    for (name, kind) in offered {
        if let Some(v) = settings.get(name) {
            args.push((name.to_string(), setting_value(ty, name, kind, v)?));
        }
    }
    let taken = |s: &str| {
        ctx.names.contains(s)
            || ctx.lock.slugs.contains_key(s)
            || ctx.lock.tombstones.contains(s)
            || files.names(s)
    };
    let slug = blocks::new_slug(ty, &taken);
    // The page's file: by its UUID (it may have been renamed), else a new one.
    let file = match files
        .pages
        .iter()
        .find(|(_, p)| p.uuid.as_deref() == Some(page.as_str()))
    {
        Some((n, _)) => n.clone(),
        None => blocks::file_name(&page_title, &|n| n == CLAIMS || files.pages.contains_key(n)),
    };
    let f = files.pages.entry(file).or_insert_with(|| PageFile {
        uuid: Some(page.clone()),
        ..PageFile::new(&page_title)
    });
    f.page = page_title;
    let b = NewBlock {
        slug: slug.clone(),
        ty: ty.clone(),
        title,
        args,
        ..Default::default()
    };
    f.blocks.push(b.clone());
    // Its wires (an undone removal's): an empty UUID is this block.
    let mut geo = Geo::new();
    for w in wires {
        let mut at = |e: &[String; 2]| {
            if e[0].is_empty() {
                new_end(&b, &e[1])
            } else {
                end(files, ctx, e, &mut geo)
            }
        };
        let (src, dst) = (at(&w.from)?, at(&w.to)?);
        connect(files, ctx, &src, &dst)?;
    }
    Ok(Applied {
        previous: serde_json::Value::Null,
        added: Some(slug),
    })
}

fn remove_block(files: &mut Files, ctx: &Ctx, uuid: &str) -> Result<Applied, String> {
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    let slug = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some_and(|(_, b)| b.adopted) => {
            return Err(format!(
                "{name} was adopted from Loxone Config, so it is deployed: vislox takes out only a \
                 block added here and not applied yet. Undo the adoption to hand it back to Loxone Config"
            ));
        }
        Some(s) if files.block(s).is_some() => s.to_string(),
        Some(_) => {
            return Err(format!(
                "{name}: the module manages this block; delete it in its .loxl file"
            ));
        }
        None => return Err(format!("{name}: only a block added here can be taken out")),
    };
    if files.claims.2.claims.contains_key(&slug) {
        return Err(format!(
            "{name}: vislox's file claims its switching times; take them out first (Revert them, \
             or undo the edit)"
        ));
    }
    if files.claims.3.claims.contains_key(&slug) {
        return Err(format!(
            "{name}: vislox's file claims its moods; take them out first (Revert them, or undo the \
             edit)"
        ));
    }
    if ctx
        .deployed
        .is_some_and(|d| find(&d.xml.root, uuid).is_some())
    {
        return Err(format!(
            "{name} is deployed: vislox takes out only a block added here and not applied yet"
        ));
    }
    let (file, b) = files.block(&slug).expect("checked");
    let (file, b) = (file.to_string(), b.clone());
    let pf = &files.pages[&file];
    let page = match &pf.uuid {
        Some(u) => u.clone(),
        None => {
            let mut same = Vec::new();
            pages_titled(&ctx.base.xml.root, &pf.page, &mut same);
            same.first()
                .and_then(|e| e.attr("U"))
                .unwrap_or_default()
                .to_string()
        }
    };
    let (params, settings) = split_args(&b);
    // Its wires, which go with it: drawn again when it is added again.
    let here = |e: &(String, String)| -> Option<[String; 2]> {
        if e.0 == slug {
            Some([
                String::new(),
                port_names::key_or_raw(&b.ty, &e.1).to_string(),
            ])
        } else {
            wire_end_at(files, ctx.lock, e)
        }
    };
    let wires = files
        .claims
        .1
        .iter()
        .filter(|w| w.touches(&slug))
        .filter_map(|w| {
            Some(Ends {
                from: here(&w.from)?,
                to: here(&w.to)?,
            })
        })
        .collect();
    let again = Op::AddBlock {
        page,
        ty: b.ty.clone(),
        title: b.title.clone(),
        params,
        settings,
        wires,
        restores: Some(uuid.to_string()),
    };
    let p = files.pages.get_mut(&file).expect("found");
    p.blocks.retain(|x| x.slug != slug);
    // Its wires go with it, as in Loxone Config.
    files.claims.1.retain(|w| !w.touches(&slug));
    files.claims.prune();
    let p = files.pages.get_mut(&file).expect("found");
    p.removed.insert(slug);
    Ok(Applied {
        previous: op_value(&again),
        added: None,
    })
}

// ── adopting blocks (#72) ───────────────────────────────────────────────

/// Declare the blocks `uuids` in vislox's files, as [`crate::adopt`] worked
/// them out on the scratch copy: each in the file of its page, its externs in
/// [`FILE`], its lockfile entry and tracked wires for the lock.
fn adopt(files: &mut Files, ctx: &Ctx, uuids: &[String]) -> Result<Applied, String> {
    for u in uuids {
        let a = ctx
            .adoptions
            .get(u)
            .ok_or_else(|| format!("{u} was not adopted on the scratch copy"))?;
        if files.block(&a.slug).is_some() {
            return Err(format!("`{}` is a block of vislox's files already", a.slug));
        }
        // `page "…"` names a page by its title, and lox builds on the first.
        let mut same = Vec::new();
        pages_titled(&ctx.base.xml.root, &a.page_title, &mut same);
        if same.first().and_then(|e| e.attr("U")) != Some(a.page.as_str()) {
            return Err(format!(
                "{} pages are titled \"{}\", and lox puts a page's blocks on the first; rename one \
                 in Loxone Config to adopt blocks there",
                same.len(),
                a.page_title
            ));
        }
        let file = match files
            .pages
            .iter()
            .find(|(_, p)| p.uuid.as_deref() == Some(a.page.as_str()))
        {
            Some((n, _)) => n.clone(),
            None => blocks::file_name(&a.page_title, &|n| {
                n == CLAIMS || files.pages.contains_key(n)
            }),
        };
        let f = files.pages.entry(file).or_insert_with(|| PageFile {
            uuid: Some(a.page.clone()),
            ..PageFile::new(&a.page_title)
        });
        f.page = a.page_title.clone();
        f.blocks.push(a.block.clone());
        f.wires.extend(a.wires.iter().cloned());
        for (eu, ty) in &a.externs {
            let c = files.claims.0.entry(eu.clone()).or_default();
            if c.ty.is_empty() {
                c.ty = ty.clone();
            }
            c.pinned = true;
        }
        // Adopted under vislox's extern of it: the wires blocks adopted earlier
        // drew onto it are its declaration's now, and they name it by its slug.
        if a.promoted {
            let ext = slug(u);
            for p in files.pages.values_mut() {
                p.wires.retain(|w| w.to.slug != ext);
                for b in p.blocks.iter_mut() {
                    for (_, src) in b.wires.iter_mut() {
                        if src.slug == ext {
                            src.slug = a.slug.clone();
                        }
                    }
                }
            }
        }
        let le = &mut files.lock_edits;
        le.drop.remove(&a.slug);
        le.add.insert(a.slug.clone(), a.object.clone());
        le.wires.extend(a.extern_wires.iter().cloned());
        le.unwires.extend(a.unwires.iter().cloned());
        le.unexternals.extend(a.unexternals.iter().cloned());
    }
    files.repin();
    Ok(Applied {
        previous: op_value(&Op::Unadopt {
            uuids: uuids.to_vec(),
        }),
        added: None,
    })
}

/// Stop managing blocks adopted here: their declarations and wires leave
/// vislox's file, their lockfile entries and the wires lox tracks for them
/// leave the lock, and Loxone Config owns them again. Only a block whose
/// pending state is the base's: what was changed here goes first.
fn unadopt(files: &mut Files, ctx: &Ctx, uuids: &[String]) -> Result<Applied, String> {
    let name = |u: &str| crate::adopt::name_in(ctx.doc, u);
    let mut picks = Vec::new();
    let mut slugs: BTreeSet<String> = BTreeSet::new();
    for u in uuids {
        let s = ctx
            .lock
            .slug_of(u)
            .ok_or_else(|| format!("{}: the module does not manage it", name(u)))?;
        match files.block(s) {
            Some((_, b)) if b.adopted => {}
            Some(_) => {
                return Err(format!(
                    "{} was added here, not adopted: Remove takes it out",
                    name(u)
                ));
            }
            None => {
                return Err(format!(
                    "{}: the module manages it in its own .loxl file; vislox hands back only a \
                     block it adopted",
                    name(u)
                ));
            }
        }
        if !crate::adopt::same_block(ctx.base, ctx.doc, u) {
            return Err(format!(
                "{} has changes here that the base does not have (values, wires or its place): \
                 revert them first, then hand it back to Loxone Config",
                name(u)
            ));
        }
        if files.claims.2.claims.contains_key(s) {
            return Err(format!(
                "{}: vislox's file claims its switching times; revert them first, then hand it \
                 back to Loxone Config",
                name(u)
            ));
        }
        if files.claims.3.claims.contains_key(s) {
            return Err(format!(
                "{}: vislox's file claims its moods; revert them first, then hand it back to \
                 Loxone Config",
                name(u)
            ));
        }
        slugs.insert(s.to_string());
        picks.push(crate::adopt::Pick {
            uuid: u.clone(),
            slug: Some(s.to_string()),
        });
    }
    let by = |s: &str| {
        ctx.lock
            .uuid_of(s)
            .map(name)
            .unwrap_or_else(|| s.to_string())
    };
    // A wire drawn here (#61) at it would have to change hands too: taken out first.
    if let Some(w) = files
        .claims
        .1
        .iter()
        .find(|w| slugs.contains(&w.to.0) || slugs.contains(&w.from.0))
    {
        let s = if slugs.contains(&w.to.0) {
            &w.to.0
        } else {
            &w.from.0
        };
        return Err(format!(
            "{}: a wire drawn here ends at it ({}); take it out first",
            by(s),
            w.render()
        ));
    }
    // The blocks that stay adopted keep their wires to it: it is an object of
    // Loxone Config's again, so they name it as vislox's extern (`vx_<uuid>`).
    // A wire into it its own declaration bound from such a block becomes that
    // block's wire onto the extern; the rest of its wires are Loxone Config's.
    let mut moved: Vec<(String, blocks::FileWire)> = Vec::new();
    let mut claimed: Vec<Wire> = Vec::new();
    for s in &slugs {
        let (_, b) = files.block(s).expect("checked");
        let ext = slug(ctx.lock.uuid_of(s).expect("checked"));
        for (port, src) in &b.wires {
            if slugs.contains(&src.slug) {
                continue;
            }
            match files.block(&src.slug) {
                Some((_, from)) if from.adopted => moved.push((
                    src.slug.clone(),
                    blocks::FileWire {
                        to: logic::PortRef {
                            slug: ext.clone(),
                            port: port.clone(),
                        },
                        from: src.clone(),
                    },
                )),
                Some(_) if src.port.is_empty() => {
                    return Err(format!(
                        "{} is fed by {}, a block added here, from its default output: vislox \
                         cannot write that wire on its own yet, so it keeps managing it",
                        by(s),
                        by(&src.slug)
                    ));
                }
                Some(_) => claimed.push(Wire {
                    to: (ext.clone(), port.clone()),
                    from: (src.slug.clone(), src.port.clone()),
                }),
                None => {}
            }
        }
    }
    let externs: Vec<(String, String, String)> = slugs
        .iter()
        .map(|s| {
            let u = ctx.lock.uuid_of(s).expect("checked").to_string();
            (s.clone(), slug(&u), u)
        })
        .collect();
    for (s, ext, _) in &externs {
        for p in files.pages.values_mut() {
            for b in p.blocks.iter_mut().filter(|b| !slugs.contains(&b.slug)) {
                for (_, src) in b.wires.iter_mut() {
                    if &src.slug == s {
                        src.slug = ext.clone();
                    }
                }
            }
            for w in p.wires.iter_mut() {
                if &w.to.slug == s && !slugs.contains(&w.from.slug) {
                    w.to.slug = ext.clone();
                }
            }
        }
    }
    for (s, _, u) in &externs {
        let ty = files
            .block(s)
            .map(|(_, b)| b.ty.clone())
            .unwrap_or_default();
        let c = files.claims.0.entry(u.clone()).or_default();
        if c.ty.is_empty() {
            c.ty = ty;
        }
    }
    for s in &slugs {
        for p in files.pages.values_mut() {
            p.blocks.retain(|b| &b.slug != s);
            p.wires.retain(|w| &w.from.slug != s);
        }
        let le = &mut files.lock_edits;
        if let Some(o) = le.add.remove(s) {
            let ports = crate::adopt::ports_of(&o);
            le.wires
                .retain(|w| !ports.contains(w.get("from").and_then(|f| f.as_str()).unwrap_or("")));
        } else {
            le.drop.insert(s.clone());
        }
    }
    for (from, w) in moved {
        if let Some(p) = files
            .pages
            .values_mut()
            .find(|p| p.blocks.iter().any(|b| b.slug == from))
        {
            p.wires.push(w);
        }
    }
    files.claims.1.extend(claimed);
    files.repin();
    Ok(Applied {
        previous: op_value(&Op::Adopt { blocks: picks }),
        added: None,
    })
}

// ── moving blocks (#60) ─────────────────────────────────────────────────

/// The farthest a block is moved from the page's origin, in twips: well
/// past any sheet Loxone Config draws (A0 is about 67 000 wide).
const MAX_COORD: i64 = 1_000_000;

/// `v` on Loxone Config's grid: it snaps positions to multiples of
/// [`GRID`] on save.
pub fn snap(v: i64) -> i64 {
    (v + GRID / 2).div_euclid(GRID) * GRID
}

/// The `move` that puts the block `uuid` back where `doc` draws it.
fn move_back(doc: &LoxoneDoc, uuid: &str) -> Option<Op> {
    let at = Layout::of_element(find(&doc.xml.root, uuid)?)?;
    Some(Op::Move {
        uuid: uuid.to_string(),
        x: at.px,
        y: at.py,
    })
}

/// Where `base` draws the managed block `slug`, on the page the lock pins it
/// to: the rectangle lox takes over unless it is superseded.
fn base_rect(base: &LoxoneDoc, lock: &Lock, slug: &str) -> Option<Layout> {
    let (page, at) = drawn_at(base, lock.uuid_of(slug)?)?;
    lock.placed
        .get(slug)?
        .as_deref()
        .is_none_or(|p| p == page)
        .then_some(at)
}

/// Move a block the module manages. lox writes the position of the blocks
/// it manages only; one Loxone Config drew is moved there (or adopted
/// first, #72).
fn move_block(files: &mut Files, ctx: &Ctx, uuid: &str, x: i64, y: i64) -> Result<Applied, String> {
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    let Some(slug) = ctx.lock.slug_of(uuid) else {
        return Err(format!(
            "{name}: lox moves only the blocks the module manages, and this one was drawn in \
             Loxone Config; move it there"
        ));
    };
    let (Some(now), true) = (Layout::of_element(el), ctx.lock.placed.contains_key(slug)) else {
        return Err(format!("{name} is not drawn on a page"));
    };
    if x.abs() > MAX_COORD || y.abs() > MAX_COORD {
        return Err(format!("{name}: that is too far off the page"));
    }
    // Within a grid step of where the base draws it, lox keeps the base's
    // rectangle as it is: that is where it goes (Revert, on the grid or
    // not). Anywhere else: on the grid, and not past the page's top left.
    let want = Layout::at(x, y, now.w(), now.h());
    let at = match base_rect(ctx.base, ctx.lock, slug) {
        Some(b) if b.same_spot(&want) => b,
        _ => Layout::at(snap(x).max(0), snap(y).max(0), now.w(), now.h()),
    };
    files.layout.moves.insert(slug.to_string(), at);
    Ok(Applied {
        previous: op_value(&Op::Move {
            uuid: uuid.to_string(),
            x: now.px,
            y: now.py,
        }),
        added: None,
    })
}

/// Move several managed blocks, each as [`move_block`] does.
fn place(files: &mut Files, ctx: &Ctx, moves: &[Placed]) -> Result<Applied, String> {
    if moves.is_empty() {
        return Err("place: no blocks to move".into());
    }
    let mut back = Vec::new();
    for m in moves {
        let a = move_block(files, ctx, &m.uuid, m.x, m.y)?;
        match serde_json::from_value(a.previous) {
            Ok(Op::Move { uuid, x, y }) => back.push(Placed { uuid, x, y }),
            _ => unreachable!("a move's previous is a move"),
        }
    }
    Ok(Applied {
        previous: op_value(&Op::Place { moves: back }),
        added: None,
    })
}

/// An op as the reply carries it. An op always serializes: never Null.
fn op_value(op: &Op) -> serde_json::Value {
    serde_json::to_value(op).expect("an op serializes")
}

/// The title lox lays the page `page` out by (`--relayout <title>`), or why
/// it cannot: the base has no such page, another page has its title (lox
/// would lay out both), or the lock pins no block `doc` draws there to it.
pub fn relayout_title(
    base: &LoxoneDoc,
    doc: &LoxoneDoc,
    lock: &Lock,
    page: &str,
) -> Result<String, String> {
    let pg = find(&base.xml.root, page)
        .filter(|e| e.attr("Type") == Some("Page"))
        .ok_or_else(|| format!("no page {page} in the project's base config"))?;
    let title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
    let mut same = Vec::new();
    pages_titled(&base.xml.root, &title, &mut same);
    if title.trim().is_empty() || same.len() != 1 {
        return Err(format!(
            "lox lays out a page by its title, and {} pages are titled \"{title}\"; rename one in \
             Loxone Config first",
            same.len()
        ));
    }
    let drawn_here = |slug: &str| {
        lock.uuid_of(slug)
            .and_then(|u| drawn_at(doc, u))
            .is_some_and(|(q, _)| q == page)
    };
    let pinned = |p: Option<&str>| {
        lock.placed
            .iter()
            .any(|(s, q)| q.as_deref() == p && drawn_here(s))
    };
    if pinned(Some(page)) {
        Ok(title)
    } else if pinned(None) {
        Err(format!(
            "{title}: the lockfile does not say which page its blocks are on (an older lox wrote \
             it); compile the module once with lox logic compile"
        ))
    } else {
        Err(format!(
            "{title}: the module manages no block on this page, and lox lays out only those"
        ))
    }
}

/// Lay out the page's managed blocks afresh, where their wires want them.
/// lox moves only the blocks it manages; one wired to a reference the
/// module does not manage stays beside it.
fn relayout(files: &mut Files, ctx: &Ctx, page: &str) -> Result<Applied, String> {
    let title = relayout_title(ctx.base, ctx.doc, ctx.lock, page)?;
    let back: Vec<Placed> = ctx
        .lock
        .placed
        .iter()
        .filter(|(_, p)| p.as_deref() == Some(page))
        .filter_map(|(s, _)| match move_back(ctx.doc, ctx.lock.uuid_of(s)?)? {
            Op::Move { uuid, x, y } => Some(Placed { uuid, x, y }),
            _ => None,
        })
        .collect();
    // Its undo puts these back; with none, there would be nothing to undo.
    if back.is_empty() {
        return Err(format!(
            "{title}: the pending config draws none of the blocks the module manages on this page"
        ));
    }
    files.layout.relayout.insert(page.to_string(), title);
    Ok(Applied {
        previous: op_value(&Op::Place { moves: back }),
        added: None,
    })
}

/// The page `uuid` is drawn on in `doc`, and its rectangle there.
fn drawn_at(doc: &LoxoneDoc, uuid: &str) -> Option<(String, Layout)> {
    fn walk(el: &Element, page: Option<&str>, uuid: &str) -> Option<(String, Layout)> {
        let page = if el.attr("Type") == Some("Page") {
            el.attr("U")
        } else {
            page
        };
        if el.attr("U") == Some(uuid) {
            return Some((page?.to_string(), Layout::of_element(el)?));
        }
        el.child_elements().find_map(|c| walk(c, page, uuid))
    }
    if uuid.is_empty() {
        return None;
    }
    walk(&doc.xml.root, None, uuid)
}

/// Every object `doc` draws on a page, by UUID: its page and rectangle.
pub fn drawn(doc: &LoxoneDoc) -> HashMap<String, (String, Layout)> {
    fn walk(el: &Element, page: Option<&str>, out: &mut HashMap<String, (String, Layout)>) {
        let page = if el.attr("Type") == Some("Page") {
            el.attr("U")
        } else {
            page
        };
        if let (Some(p), Some(u), Some(at)) = (page, el.attr("U"), Layout::of_element(el)) {
            out.insert(u.to_string(), (p.to_string(), at));
        }
        for c in el.child_elements() {
            walk(c, page, out);
        }
    }
    let mut out = HashMap::new();
    walk(&doc.xml.root, None, &mut out);
    out
}

/// The lockfile `lock` with `moves` written in, the way lox's own placement
/// leaves a block it moved (`--relayout`): the new rectangle (`layout`),
/// and the one the base still draws (`superseded_layout`). Without the
/// latter, the compile would take the base's position back as a move made
/// in Loxone Config. Once the base draws the block at its new place (after
/// Apply), lox drops the line itself.
pub fn write_moves(
    lock: &[u8],
    base: &LoxoneDoc,
    moves: &BTreeMap<String, Layout>,
) -> Result<Vec<u8>, String> {
    let mut v: serde_json::Value =
        serde_json::from_slice(lock).map_err(|e| format!("cannot read the lock: {e}"))?;
    let shown = drawn(base);
    for (slug, at) in moves {
        let obj = v
            .get_mut("objects")
            .and_then(|o| o.get_mut(slug))
            .and_then(|o| o.as_object_mut())
            .ok_or_else(|| format!("the lock has no block {slug}"))?;
        let uuid = obj
            .get("uuid")
            .and_then(|u| u.as_str())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| format!("the lock has no UUID for block {slug}"))?;
        let page = obj.get("page_uuid").and_then(|p| p.as_str());
        let superseded = shown
            .get(uuid)
            .filter(|(p, _)| page.is_none_or(|q| q == p))
            .map(|(_, l)| *l)
            .filter(|l| !l.same_spot(at));
        obj.insert("layout".into(), serde_json::json!(at));
        match superseded {
            Some(l) => obj.insert("superseded_layout".into(), serde_json::json!(l)),
            None => obj.remove("superseded_layout"),
        };
    }
    let mut out = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
    out.push(b'\n');
    Ok(out)
}

/// The moves of `place` that lox did not keep: `out` does not draw the
/// block where it goes (by `lock`'s UUIDs). lox keeps the base's own
/// rectangle for a move within a grid step of it, so `out` drawing it there
/// keeps that move too. A move on a page laid out afresh is not checked:
/// lox places those blocks anew.
pub fn moves_missed(
    out: &LoxoneDoc,
    base: &LoxoneDoc,
    lock: &Lock,
    place: &Placement,
) -> Vec<String> {
    let relaid = |slug: &str| {
        lock.placed
            .get(slug)
            .and_then(|p| p.as_deref())
            .is_some_and(|p| place.relayout.contains_key(p))
    };
    place
        .moves
        .iter()
        .filter(|(slug, _)| !relaid(slug))
        .filter(|(slug, at)| {
            let Some(d) = lock
                .uuid_of(slug)
                .and_then(|u| Layout::of_element(find(&out.xml.root, u)?))
            else {
                return true;
            };
            let base_kept = base_rect(base, lock, slug)
                .is_some_and(|b| b.same_spot(at) && (d.px, d.py) == (b.px, b.py));
            (d.px, d.py) != (at.px, at.py) && !base_kept
        })
        .map(|(s, _)| s.clone())
        .collect()
}

// ── wires (#61) ─────────────────────────────────────────────────────────

/// What a connector carries. Loxone Config wires a digital output to an
/// analog input and back (a value other than 0 is on), but text only to
/// text. `Unknown`: vislox cannot tell, so it does not wire it (lox does not
/// check what an extern's connector carries).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Digital,
    Analog,
    Text,
    Unknown,
}

impl Signal {
    fn word(self) -> &'static str {
        match self {
            Signal::Digital => "digital",
            Signal::Analog => "analog",
            Signal::Text => "text",
            Signal::Unknown => "unknown",
        }
    }
}

/// Loxone draws analog connections differently from digital ones; the
/// connector key is a good tell (`AQ`, `AI`, `AQ1`, `TargetPos`, …).
pub fn analog_key(key: &str) -> bool {
    let k = key.as_bytes();
    (k.len() >= 2 && k[0] == b'A' && (k[1] == b'Q' || k[1] == b'I'))
        || matches!(key, "Txt" | "TQ" | "V" | "Pos" | "Value" | "Temp")
}

/// The text connectors of lox's port table: text outputs (`TQ`, `TQ1`,
/// `TQU`, `OutText`), text inputs (`Text`, `inText`, `TTS`) and message
/// texts (`TM`, `TI`), and `Txt`.
fn text_key(key: &str) -> bool {
    key.starts_with("TQ")
        || matches!(
            key,
            "Txt" | "Text" | "inText" | "OutText" | "TM" | "TI" | "TTS"
        )
}

/// What the connector `key` of an object of type `ty` (its element `el`,
/// if the config has it) carries:
/// - text: a text connector of lox's port table, or a reference to text
///   (`Text="true"`);
/// - an object whose value lox knows (a virtual input, a sensor, a memory
///   flag: the table lox mints references from): that value, and the
///   object's `Analog="true"` or a memory flag's `Tp=`;
/// - a reference: `Analog="true"` or an analog key;
/// - a connector of lox's port table: analog by its key, else digital;
/// - anything else: unknown.
pub fn signal(ty: &str, el: Option<&Element>, key: &str) -> Signal {
    let attr = |a: &str| el.and_then(|e| e.attr(a));
    if attr("Text") == Some("true") || text_key(key) {
        return Signal::Text;
    }
    if let Some((_, v)) = connectors::ref_link_type(ty) {
        return match (v, attr("Tp")) {
            (connectors::RefValue::Text, _) | (_, Some("2")) => Signal::Text,
            (connectors::RefValue::Analog, _) | (_, Some("1")) => Signal::Analog,
            _ if attr("Analog") == Some("true") => Signal::Analog,
            _ => Signal::Digital,
        };
    }
    let is_ref = matches!(ty, "InputRef" | "OutputRef" | "OutputRefLM");
    let known = connectors::builtin(ty).is_some_and(|s| s.iter().any(|s| s.key == key));
    if is_ref || known {
        if analog_key(key) || attr("Analog") == Some("true") {
            Signal::Analog
        } else {
            Signal::Digital
        }
    } else {
        Signal::Unknown
    }
}

/// Who draws a wire vislox's file does not, and so who can take it out.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    /// The rest of the module: a wire lox tracks onto an extern
    /// (`extern_wires`), or one into a block the module declares.
    Module,
    /// Loxone Config.
    LoxoneConfig,
    /// The declaration of a block adopted here (#72): drawn in Loxone Config
    /// before, and brought along by the adoption.
    Adopted,
}

impl Owner {
    /// Why a wire it draws is not changed in vislox.
    pub fn why(self) -> &'static str {
        match self {
            Owner::Module => "The project's module draws this wire: change it in its .loxl file.",
            Owner::LoxoneConfig => {
                "Drawn in Loxone Config. lox removes only the wires its source draws: remove it in \
                 Loxone Config."
            }
            Owner::Adopted => {
                "Drawn in Loxone Config and brought along when the block was adopted: the block's \
                 declaration in vislox's file draws it now. vislox does not take such a wire out yet."
            }
        }
    }
}

/// Who draws the wire from the connector `from` into `to` (connector UUIDs),
/// into the object `sink`, that vislox's file does not draw. One rule for
/// `unwire` and the page (`module_wires` in `GET /api/edits`).
fn owner(lock: &Lock, files: &Files, from: &str, to: &str, source: &str, sink: &str) -> Owner {
    let adopted = |u: &str| {
        lock.slug_of(u)
            .and_then(|s| files.block(s))
            .is_some_and(|(_, b)| b.adopted)
    };
    let tracked = lock.wires.contains(&(from.to_string(), to.to_string()));
    if adopted(sink) || (tracked && adopted(source)) {
        return Owner::Adopted;
    }
    let module_block = lock.slug_of(sink).is_some_and(|s| files.block(s).is_none());
    if module_block || tracked {
        Owner::Module
    } else {
        Owner::LoxoneConfig
    }
}

/// The wires of `doc` that `who` draws, by their ends as the page names
/// them.
pub fn module_wires(
    doc: &LoxoneDoc,
    lock: &Lock,
    files: &Files,
    who: Owner,
) -> Vec<([String; 2], [String; 2])> {
    fn walk<'a>(el: &'a Element, out: &mut HashMap<&'a str, (&'a str, &'a str)>) {
        if let Some(u) = el.attr("U") {
            for co in el.child_elements().filter(|c| c.name == "Co") {
                if let (Some(cu), Some(k)) = (co.attr("U"), co.attr("K")) {
                    out.insert(cu, (u, k));
                }
            }
        }
        for c in el.child_elements() {
            walk(c, out);
        }
    }
    let mut at = HashMap::new();
    walk(&doc.xml.root, &mut at);
    let end = |p: &str| at.get(p).map(|(u, k)| [u.to_string(), k.to_string()]);
    doc.wires()
        .iter()
        .filter_map(|w| {
            let (f, t) = (end(&w.from_port)?, end(&w.to_port)?);
            (owner(lock, files, &w.from_port, &w.to_port, &f[0], &t[0]) == who).then_some((f, t))
        })
        .collect()
}

/// Two ends of a wire as the page names them: `[object UUID, connector key]`.
/// In an `add_block`'s `wires`, an empty UUID is the block it adds.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Ends {
    pub from: [String; 2],
    pub to: [String; 2],
}

/// One end of a wire: the object, its connector, and the name lox knows
/// them by.
struct End<'a> {
    /// Empty for the block the same `add_block` adds.
    uuid: String,
    /// The connector (`<Co>`); none on a block not compiled yet.
    co: Option<&'a Element>,
    /// The object's name, for messages.
    name: String,
    ty: String,
    /// The slug the wire names it by: an extern of [`FILE`], or a block
    /// vislox added. `None` for a block the rest of the module manages.
    slug: Option<String>,
    /// A block vislox added: declared in its files, not an extern.
    added: bool,
    /// The port's Lox Logic name (`In1` for `I1`).
    port: String,
    /// Whether the end is an input (a sink), by lox's port table, else by
    /// the side of the block Loxone Config draws it on. `None`: neither.
    input: Option<bool>,
    signal: Signal,
}

impl End<'_> {
    fn label(&self) -> String {
        format!("{}.{}", self.name, self.port)
    }
}

/// The page geometries an op looked at, by page UUID: built once per page.
type Geo = HashMap<String, Option<loxone_config::layout::PageGeometry>>;

/// Whether the connector `key` (`co`) of the object `uuid` is an input: by
/// lox's port table, else by the side of the block its page draws it on
/// (inputs on the left), else by whether it holds a constant or a wire.
fn port_input(
    doc: &LoxoneDoc,
    ty: &str,
    uuid: &str,
    key: &str,
    co: Option<&Element>,
    geo: &mut Geo,
) -> Option<bool> {
    if let Some(spec) = connectors::builtin(ty).and_then(|s| s.iter().find(|s| s.key == key)) {
        return match spec.dir {
            PortDir::Input | PortDir::Param => Some(true),
            PortDir::Output => Some(false),
            PortDir::Api => None,
        };
    }
    let co = co?;
    let side = drawn_at(doc, uuid).and_then(|(page, _)| {
        let g = geo
            .entry(page.clone())
            .or_insert_with(|| loxone_config::layout::PageGeometry::of(doc, &page))
            .as_ref()?;
        let o = g.objects.iter().find(|o| o.uuid == uuid)?;
        o.ports.iter().find(|p| p.key == key).map(|p| p.side)
    });
    match side {
        Some(loxone_config::layout::Side::Left) => Some(true),
        Some(loxone_config::layout::Side::Right) => Some(false),
        None => (co.attr("Def").is_some() || co.child_elements().any(|c| c.name == "In"))
            .then_some(true),
    }
}

/// The end `[uuid, key]` of a wire in the pending config.
fn end<'a>(
    files: &Files,
    ctx: &'a Ctx,
    at: &[String; 2],
    geo: &mut Geo,
) -> Result<End<'a>, String> {
    let [uuid, key] = at;
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let ty = el.attr("Type").unwrap_or_default().to_string();
    let name = el
        .attr_decoded("Title")
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.into_owned())
        .unwrap_or_else(|| ty.clone());
    let co = el
        .child_elements()
        .find(|c| c.name == "Co" && c.attr("K") == Some(key.as_str()))
        .ok_or_else(|| format!("{name} has no connector {key}"))?;
    let (slug, added) = match ctx.lock.slug_of(uuid) {
        Some(s) if files.block(s).is_some() => (Some(s.to_string()), true),
        Some(_) => (None, false),
        None => (Some(slug(uuid)), false),
    };
    Ok(End {
        input: port_input(ctx.doc, &ty, uuid, key, Some(co), geo),
        signal: signal(&ty, Some(el), key),
        port: port_names::name_of(&ty, key).to_string(),
        uuid: uuid.clone(),
        co: Some(co),
        name,
        ty,
        slug,
        added,
    })
}

/// The end `key` of the block `b` an `add_block` adds now: not in the
/// pending config yet, so known by lox's port table alone.
fn new_end<'a>(b: &NewBlock, key: &str) -> Result<End<'a>, String> {
    let name = b.title.clone().unwrap_or_else(|| b.slug.clone());
    if !connectors::builtin(&b.ty).is_some_and(|s| s.iter().any(|s| s.key == key)) {
        return Err(format!("{name} has no connector {key}"));
    }
    let input = connectors::builtin(&b.ty)
        .and_then(|s| s.iter().find(|s| s.key == key))
        .and_then(|s| match s.dir {
            PortDir::Input | PortDir::Param => Some(true),
            PortDir::Output => Some(false),
            PortDir::Api => None,
        });
    Ok(End {
        uuid: String::new(),
        co: None,
        signal: signal(&b.ty, None, key),
        port: port_names::name_of(&b.ty, key).to_string(),
        name,
        ty: b.ty.clone(),
        slug: Some(b.slug.clone()),
        added: true,
        input,
    })
}

/// The wire `to <- from` as vislox's file names it, if both ends have a slug.
fn wire_of(from: &End, to: &End) -> Option<Wire> {
    Some(Wire {
        to: (to.slug.clone()?, to.port.clone()),
        from: (from.slug.clone()?, from.port.clone()),
    })
}

/// Whether the pending config wires `from` into `to`.
fn wired(from: &End, to: &End) -> bool {
    let (Some(src), Some(co)) = (from.co.and_then(|c| c.attr("U")), to.co) else {
        return false;
    };
    co.child_elements()
        .any(|c| c.name == "In" && c.attr("Input") == Some(src))
}

/// Whether `w` is wired now, as the ops of this request so far leave it:
/// in vislox's file, or in the pending config and not one of vislox's
/// wires this request took out.
fn wired_now(files: &Files, ctx: &Ctx, w: &Wire, src: &End, dst: &End) -> bool {
    files.claims.1.contains(w) || (wired(src, dst) && !ctx.drawn.contains(w))
}

/// Draw a wire from an output to an input. Each end is an object Loxone
/// Config drew (an extern, pinned by UUID) or a block added here; a block
/// the rest of the module manages is wired in its own `.loxl`.
fn wire(
    files: &mut Files,
    ctx: &Ctx,
    from: &[String; 2],
    to: &[String; 2],
) -> Result<Applied, String> {
    let mut geo = Geo::new();
    let (src, dst) = (
        end(files, ctx, from, &mut geo)?,
        end(files, ctx, to, &mut geo)?,
    );
    connect(files, ctx, &src, &dst)?;
    Ok(Applied {
        previous: op_value(&Op::Unwire {
            from: from.clone(),
            to: to.clone(),
        }),
        added: None,
    })
}

/// Check the wire `src` → `dst` and write it into vislox's file.
fn connect(files: &mut Files, ctx: &Ctx, src: &End, dst: &End) -> Result<(), String> {
    if src.slug.is_some() && src.slug == dst.slug {
        return Err(format!(
            "{}: a wire goes from one block's output to another block's input",
            src.name
        ));
    }
    match src.input {
        Some(false) => {}
        Some(true) => {
            return Err(format!(
                "{} is an input: a wire starts at an output (on a block's right side)",
                src.label()
            ));
        }
        None => return Err(format!("{} cannot be wired", src.label())),
    }
    match dst.input {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "{} is an output: a wire ends at an input (on a block's left side)",
                dst.label()
            ));
        }
        None => return Err(format!("{} cannot be wired", dst.label())),
    }
    for e in [src, dst] {
        if e.slug.is_none() {
            return Err(format!(
                "{}: the module manages this block; wire it in its .loxl file",
                e.name
            ));
        }
        // lox matches an extern in the base it compiles against.
        if !e.added && find(&ctx.base.xml.root, &e.uuid).is_none() {
            return Err(format!("{} is not in the project's base config", e.name));
        }
    }
    if dst.co.is_some_and(|c| c.attr("Inv").is_some()) {
        return Err(format!(
            "{} is inverted in Loxone Config, which owns that input; lox does not wire it",
            dst.label()
        ));
    }
    for e in [src, dst] {
        if e.signal == Signal::Unknown {
            return Err(format!(
                "vislox cannot tell whether {} carries text, a number or on/off (lox's port table \
                 does not list it); wire it in Loxone Config",
                e.label()
            ));
        }
    }
    if (src.signal == Signal::Text) != (dst.signal == Signal::Text) {
        return Err(format!(
            "{} is a {} output and {} a {} input: Loxone Config wires text only to text",
            src.label(),
            src.signal.word(),
            dst.label(),
            dst.signal.word()
        ));
    }
    let w = wire_of(src, dst).expect("both have a slug");
    if wired_now(files, ctx, &w, src, dst) {
        return Err(format!(
            "{} is already wired to {}",
            src.label(),
            dst.label()
        ));
    }
    // An input takes a wire or a constant, not both (lox refuses both in
    // one declaration; on an extern the constant would be hidden).
    let constant = match files.block(&w.to.0) {
        Some((_, b)) => b.get(&w.to.1).is_some(),
        None => files
            .claims
            .0
            .get(&dst.uuid)
            .is_some_and(|c| c.sets.contains_key(&w.to.1)),
    };
    if constant {
        return Err(format!(
            "{} has a constant edited here; clear it first, then wire the input",
            dst.label()
        ));
    }
    for e in [src, dst] {
        if !e.added {
            files.claims.0.entry(e.uuid.clone()).or_default().ty = e.ty.clone();
        }
    }
    files.claims.1.insert(w);
    Ok(())
}

/// Take out a wire vislox's file draws. lox removes only the wires its
/// source draws (and tracks in the lockfile): one Loxone Config drew stays.
fn unwire(
    files: &mut Files,
    ctx: &Ctx,
    from: &[String; 2],
    to: &[String; 2],
) -> Result<Applied, String> {
    let mut geo = Geo::new();
    let (src, dst) = (
        end(files, ctx, from, &mut geo)?,
        end(files, ctx, to, &mut geo)?,
    );
    let w = wire_of(&src, &dst);
    let mine = w.as_ref().filter(|w| files.claims.1.contains(w));
    let Some(w) = mine.cloned() else {
        let taken = w.as_ref().is_some_and(|w| ctx.drawn.contains(w));
        if taken || !wired(&src, &dst) {
            return Err(format!("{} is not wired to {}", src.label(), dst.label()));
        }
        let port = |e: &End| {
            e.co.and_then(|c| c.attr("U"))
                .unwrap_or_default()
                .to_string()
        };
        let who = owner(
            ctx.lock,
            files,
            &port(&src),
            &port(&dst),
            &src.uuid,
            &dst.uuid,
        );
        return Err(format!("{} → {}: {}", src.label(), dst.label(), who.why()));
    };
    files.claims.1.remove(&w);
    files.claims.prune();
    Ok(Applied {
        previous: op_value(&Op::Wire {
            from: from.clone(),
            to: to.clone(),
        }),
        added: None,
    })
}

/// The end `(slug, port name)` of a wire of vislox's files as the page
/// names it, `[object UUID, connector key]`; `None` when the lock does not
/// know the block (yet).
fn wire_end_at(files: &Files, lock: &Lock, (s, port): &(String, String)) -> Option<[String; 2]> {
    let (uuid, ty) = match files.block(s) {
        Some((_, b)) => (lock.uuid_of(s)?.to_string(), b.ty.clone()),
        None => {
            let (u, c) = files.claims.0.iter().find(|(u, _)| slug(u) == *s)?;
            (u.clone(), c.ty.clone())
        }
    };
    Some([uuid, port_names::key_or_raw(&ty, port).to_string()])
}

/// The wires of vislox's files, by their ends as the page names them
/// (`[object UUID, connector key]`): from, to. An end the lock no longer
/// knows is left out.
pub fn wire_ends(files: &Files, lock: &Lock) -> Vec<([String; 2], [String; 2])> {
    files
        .claims
        .1
        .iter()
        .filter_map(|w| {
            Some((
                wire_end_at(files, lock, &w.from)?,
                wire_end_at(files, lock, &w.to)?,
            ))
        })
        .collect()
}

/// A value of a block vislox added: its name, an unwired input's constant,
/// or a setting lox takes in the declaration (`Formula:`, `Type:`, `At:`, …).
/// lox has no room or category for a managed block.
fn edit_new_block(
    files: &mut Files,
    ctx: &Ctx,
    slug: &str,
    op: &Op,
) -> Result<Option<String>, String> {
    let uuid = op.uuid().unwrap_or_default();
    let el = find(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = el.attr_decoded("Title").unwrap_or_default().into_owned();
    let caps = caps(el).unwrap_or_default();
    let b = files.block_mut(slug).expect("checked");
    match op {
        Op::Title { value, .. } => {
            let previous = b.title.clone();
            b.title = value
                .as_deref()
                .map(|t| block_title(&b.ty, t))
                .transpose()?;
            Ok(previous)
        }
        Op::Param { port, value, .. } => {
            let pname = blocks::port_name(&b.ty, port)
                .ok_or_else(|| format!("{name}: {port} cannot be edited here"))?;
            let previous = b.get(pname).map(plain);
            let v = match value {
                Some(v) if caps.params.iter().any(|p| &p.key == port) => Some(Value::Number(
                    number(v).map_err(|e| format!("{pname}: {e}"))?,
                )),
                Some(_) => {
                    return Err(format!(
                        "{name}: {pname} cannot be edited here (it is wired, or not an input)"
                    ));
                }
                None => None,
            };
            b.set(pname, v);
            Ok(previous)
        }
        Op::Setting {
            name: setting,
            value,
            ..
        } => {
            // A lighting controller's circuit (#74): `Circuit<n>: "dimmer"`,
            // which lox's table of settings does not list.
            if connectors::circuit_param(&b.ty, setting).is_some() {
                let previous = b.get(setting).map(plain);
                let v = match value.as_deref().map(str::trim) {
                    Some(v) => {
                        crate::mood_edit::circuit_setting(el, setting, v)
                            .map_err(|e| format!("{name}: {e}"))?;
                        Some(Value::Str(v.to_string()))
                    }
                    None => None,
                };
                b.set(setting, v);
                return Ok(previous);
            }
            let kind = blocks::settings(&b.ty)
                .into_iter()
                .find(|(n, _)| n == setting)
                .map(|(_, k)| k)
                .ok_or_else(|| format!("{name}: {setting} cannot be edited here"))?;
            let previous = b.get(setting).map(plain);
            let v = value
                .as_deref()
                .map(|v| setting_value(&b.ty, setting, kind, v))
                .transpose()?;
            b.set(setting, v);
            Ok(previous)
        }
        _ => Err(format!(
            "{name}: a block added here has no room or category lox can set; \
             only its name, its inputs' constants and its settings are edited"
        )),
    }
}

/// The names the module declares (blocks, externs, signals, instances,
/// constants), so that a new slug is none of them. Empty when the module
/// does not read; then the check after writing says why.
pub fn module_names(module: &Path) -> BTreeSet<String> {
    module_decls(module).0
}

/// [`module_names`], and of them the `let` constants (a bare name in an
/// argument names one of those before an object).
pub fn module_decls(module: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let Some(m) = module
        .to_str()
        .and_then(|p| loxone_config::cli::read_module(p).ok())
    else {
        return Default::default();
    };
    let mut names = BTreeSet::new();
    let mut lets = BTreeSet::new();
    for i in &m.items {
        match i {
            Item::Block(b) | Item::Instance(b) => names.insert(b.slug.clone()),
            Item::Extern(e) => names.insert(e.slug.clone()),
            Item::Signal(s) => names.insert(s.slug.clone()),
            // The sets share the namespace (#73 picks a set's name).
            Item::Schedule(s) => names.insert(s.name.clone()),
            Item::Tiles(s) => names.insert(s.name.clone()),
            Item::Moods(s) => names.insert(s.name.clone()),
            Item::Access(s) => names.insert(s.name.clone()),
            Item::Let(l) => {
                lets.insert(l.name.clone());
                names.insert(l.name.clone())
            }
            _ => false,
        };
    }
    (names, lets)
}

/// Check the whole module, with vislox's files as written, the way `lox logic
/// check` and `lint` would: syntax, ports, and the documented range of each
/// value vislox claims. Faster than a compile, and its messages name the value.
pub fn check_module(module: &Path) -> Result<(), String> {
    let path = module
        .to_str()
        .ok_or_else(|| "the module's path is not UTF-8".to_string())?;
    let m = loxone_config::cli::read_module(path).map_err(|e| e.to_string())?;
    logic::validate_ports(&m).map_err(|e| e.to_string())?;
    let out_of_range: Vec<String> = logic::lint_source(&m)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|f| f.kind == logic::LintKind::OutOfRange && f.slug.starts_with(SLUG_PREFIX))
        .map(|f| f.detail)
        .collect();
    if out_of_range.is_empty() {
        Ok(())
    } else {
        Err(out_of_range.join("\n"))
    }
}

/// A `.Loxone` file's (or backup's) program, parsed.
pub fn parse_doc(bytes: &[u8]) -> Result<LoxoneDoc, String> {
    let xml = crate::source::program_xml(bytes).map_err(|e| e.to_string())?;
    LoxoneDoc::parse(&xml).map_err(|e| e.to_string())
}

/// The value `op` changes, as `doc` has it (`None`: no such object, or the
/// value is not set). An input without `Def=` holds 0.
pub fn current(doc: &LoxoneDoc, op: &Op) -> Option<String> {
    let el = find(&doc.xml.root, op.uuid()?)?;
    let ty = el.attr("Type").unwrap_or_default();
    let decoded = |e: &Element, a: &str| e.attr_decoded(a).map(|v| v.into_owned());
    let place = |a: &str| {
        let io = el.child_elements().find(|c| c.name == "IoData")?;
        let id = io.attr(a).filter(|v| !v.is_empty())?;
        decoded(find(&doc.xml.root, id)?, "Title")
    };
    match op {
        Op::Title { .. } => decoded(el, "Title"),
        Op::Room { .. } => place("Pr"),
        Op::Category { .. } => place("Cr"),
        Op::Param { port, .. } => {
            let key = port_names::key_or_raw(ty, port);
            let co = el
                .child_elements()
                .find(|c| c.name == "Co" && c.attr("K") == Some(key))?;
            Some(co.attr("Def").unwrap_or("0").to_string())
        }
        // A block setting may be stored under another name (`At:` is `Sec=`).
        Op::Setting { name, .. } => decoded(el, connectors::attr_name(ty, name)),
        Op::AddBlock { .. }
        | Op::RemoveBlock { .. }
        | Op::Move { .. }
        | Op::Relayout { .. }
        | Op::Place { .. }
        | Op::Wire { .. }
        | Op::Unwire { .. }
        | Op::Adopt { .. }
        | Op::AdoptPage { .. }
        | Op::Unadopt { .. }
        | Op::Schedule { .. }
        | Op::Moods { .. } => None,
    }
}

/// Every file of the module directory (not hidden ones, as lox skips them),
/// with its modification time.
pub fn module_files(dir: &Path, found: &mut dyn FnMut(&Path, std::time::SystemTime)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            module_files(&path, found);
        } else if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
            found(&path, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loxone_config::{Lockfile, logic::CompileOptions};

    const DIV: &str = "10000006-0000-0060-ffff504f94000001";
    const AND: &str = "10000004-0000-0040-ffff504f94000001";
    const VI: &str = "10000001-0000-0010-ffff504f94000001";
    const REF: &str = "10000003-0000-0030-ffff504f94000001";
    const AI_IN: &str = "20000005-0000-0051-ffff504f94000002";
    const TOUCH: &str = "20000003-0000-0030-ffff504f94000002";

    fn doc(name: &str) -> LoxoneDoc {
        let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        LoxoneDoc::parse(&std::fs::read(p).unwrap()).unwrap()
    }

    fn compile(base: &LoxoneDoc, src: &str) -> String {
        let m = Module::parse(src).unwrap();
        logic::validate_ports(&m).unwrap();
        let opts = CompileOptions {
            machine: [0x50, 0x4f, 0x94, 0, 0, 1],
            mint_time_unix: 0,
            page_title: None,
            allow_removals: false,
            accept_version: None,
            relayout: vec![],
        };
        let out = logic::compile(base, &m, &mut Lockfile::default(), &opts).unwrap();
        String::from_utf8(out.to_bytes()).unwrap()
    }

    fn param(uuid: &str, port: &str, v: Option<&str>) -> Op {
        Op::Param {
            uuid: uuid.into(),
            port: port.into(),
            value: v.map(Into::into),
        }
    }

    #[test]
    fn what_can_be_edited() {
        let d = doc("house.Loxone");
        let div = caps(find(&d.xml.root, DIV).unwrap()).unwrap();
        assert!(div.title);
        // Input1 is wired, AQ an output: only the divisor.
        assert_eq!(
            div.params.iter().map(|p| &p.key[..]).collect::<Vec<_>>(),
            ["Input2"]
        );
        let and = caps(find(&d.xml.root, AND).unwrap()).unwrap();
        assert_eq!(
            and.params.iter().map(|p| &p.key[..]).collect::<Vec<_>>(),
            ["I2"]
        );
        // A reference takes its title from what it mirrors.
        assert_eq!(caps(find(&d.xml.root, REF).unwrap()), None);
        // A virtual input: its name only (its settings stay with Loxone Config).
        let vi = caps(find(&d.xml.root, VI).unwrap()).unwrap();
        assert!(vi.title && vi.params.is_empty() && vi.settings.is_empty());

        let h = doc("hardware.Loxone");
        let ai = caps(find(&h.xml.root, AI_IN).unwrap()).unwrap();
        let names: Vec<_> = ai.settings.iter().map(|s| &s.name[..]).collect();
        // Those it has, and those Lox Logic may add; not Unit, which it lacks.
        assert!(
            names.contains(&"MinChange") && names.contains(&"Desc"),
            "{names:?}"
        );
        assert!(!names.contains(&"Unit"), "{names:?}");
        let min = ai.settings.iter().find(|s| s.name == "MinChange").unwrap();
        assert_eq!((min.text, min.value.as_deref()), (false, Some("0.25")));
        let touch = caps(find(&h.xml.root, TOUCH).unwrap()).unwrap();
        let names: Vec<_> = touch.settings.iter().map(|s| &s.name[..]).collect();
        assert_eq!(names, ["Desc", "Installation"]);
    }

    #[test]
    fn ops_are_checked_and_return_what_they_replace() {
        let d = doc("house.Loxone");
        let none = HashSet::new();
        let mut c = Claims::default();
        assert_eq!(
            apply(&mut c, &d, &none, &param(DIV, "Input2", Some("100"))),
            Ok(None)
        );
        assert_eq!(
            apply(&mut c, &d, &none, &param(DIV, "Input2", Some("50"))),
            Ok(Some("100".into()))
        );
        let err = |op: Op| apply(&mut c.clone(), &d, &none, &op).unwrap_err();
        assert!(err(param(DIV, "Input2", Some("1,5"))).contains("decimal point"));
        assert!(err(param(DIV, "Input2", Some("abc"))).contains("not a number"));
        assert!(err(param(DIV, "Input1", Some("3"))).contains("wired"));
        assert!(err(param(DIV, "AQ", Some("3"))).contains("cannot be edited"));
        let title = |v: &str| Op::Title {
            uuid: DIV.into(),
            value: Some(v.into()),
        };
        assert!(err(title("  ")).contains("cannot be empty"));
        assert!(err(title("a\nb")).contains("one line"));
        assert!(err(param("nope", "I2", Some("1"))).contains("no object"));
        let managed = HashSet::from([DIV.to_string()]);
        assert!(
            apply(&mut c.clone(), &d, &managed, &title("x"))
                .unwrap_err()
                .contains("module manages")
        );
        // Dropping the claim hands the value back; the object leaves the file.
        assert_eq!(
            apply(&mut c, &d, &none, &param(DIV, "Input2", None)),
            Ok(Some("50".into()))
        );
        assert!(c.0.is_empty());

        let h = doc("hardware.Loxone");
        let set = |n: &str, v: &str| Op::Setting {
            uuid: AI_IN.into(),
            name: n.into(),
            value: Some(v.into()),
        };
        let mut c = Claims::default();
        assert!(
            apply(&mut c, &h, &none, &set("MinChange", "-1"))
                .unwrap_err()
                .contains("below 0")
        );
        assert!(
            apply(&mut c, &h, &none, &set("Unit", "V"))
                .unwrap_err()
                .contains("cannot be edited")
        );
        apply(&mut c, &h, &none, &set("MinChange", "0.5")).unwrap();
        apply(&mut c, &h, &none, &set("Desc", "Pegel")).unwrap();
        assert_eq!(c.ops().len(), 2);
        // An optional text clears to empty (lox writes `Desc=""`); a name cannot.
        apply(&mut c, &h, &none, &set("Desc", "")).unwrap();
        assert!(c.render().contains(".Desc = \"\""), "{}", c.render());
        assert!(compile(&h, &c.render()).contains(r#"Desc="""#));
        let no_name = Op::Title {
            uuid: AI_IN.into(),
            value: Some(String::new()),
        };
        assert!(
            apply(&mut c, &h, &none, &no_name)
                .unwrap_err()
                .contains("cannot be empty")
        );
    }

    #[test]
    fn the_file_round_trips_and_compiles_with_lox() {
        let d = doc("house.Loxone");
        let none = HashSet::new();
        let mut c = Claims::default();
        for op in [
            param(DIV, "Input2", Some("100")),
            Op::Title {
                uuid: DIV.into(),
                value: Some("Teiler \"neu\"".into()),
            },
            param(AND, "I2", Some("1")),
        ] {
            apply(&mut c, &d, &none, &op).unwrap();
        }
        let text = c.render();
        assert!(
            text.contains("vx_10000006_0000_0060_ffff504f94000001.In2 = 100"),
            "{text}"
        );
        assert_eq!(Claims::parse(&text).unwrap(), c);
        assert_eq!(Claims::parse(&text).unwrap().ops().len(), 3);
        // lox's own compiler writes exactly these values.
        let out = compile(&d, &text);
        assert!(out.contains(r#"Title="Teiler &quot;neu&quot;""#), "{out}");
        assert!(out.contains(r#"<Co K="Input2" Def="100""#), "{out}");
        assert!(out.contains(r#"<Co K="I2" Def="1""#), "{out}");

        let h = doc("hardware.Loxone");
        let mut c = Claims::default();
        apply(
            &mut c,
            &h,
            &none,
            &Op::Setting {
                uuid: AI_IN.into(),
                name: "MinChange".into(),
                value: Some("0.5".into()),
            },
        )
        .unwrap();
        assert!(compile(&h, &c.render()).contains(r#"MinChange="0.5""#));
    }

    #[test]
    fn room_and_category_are_claimed_by_title() {
        let xml = std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace(
            "\t\t<C Type=\"Page\" V=\"178\" U=\"10000002",
            "\t\t<C Type=\"Place\" U=\"30000a01-0000-0000-ffff504f94000001\" Title=\"Küche\"/>\n\
             \t\t<C Type=\"Place\" U=\"30000a02-0000-0000-ffff504f94000001\" Title=\"Bad\"/>\n\
             \t\t<C Type=\"Place\" U=\"30000a04-0000-0000-ffff504f94000001\" Title=\"Flur\"/>\n\
             \t\t<C Type=\"Place\" U=\"30000a05-0000-0000-ffff504f94000001\" Title=\"Flur\"/>\n\
             \t\t<C Type=\"Category\" U=\"30000a03-0000-0000-ffff504f94000001\" Title=\"Licht\"/>\n\
             \t\t<C Type=\"Page\" V=\"178\" U=\"10000002",
        )
        .replace(
            "<Co K=\"Input1\" Nc=\"1\"",
            "<IoData Visu=\"true\" Pr=\"30000a01-0000-0000-ffff504f94000001\" Cr=\"30000a03-0000-0000-ffff504f94000001\"/><Co K=\"Input1\" Nc=\"1\"",
        );
        let d = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let div = caps(find(&d.xml.root, DIV).unwrap()).unwrap();
        assert!(div.room && div.category);
        assert!(!caps(find(&d.xml.root, AND).unwrap()).unwrap().room);
        let none = HashSet::new();
        let room = |uuid: &str, v: &str| Op::Room {
            uuid: uuid.into(),
            value: Some(v.into()),
        };
        let mut c = Claims::default();
        let err = |c: &mut Claims, op: Op| apply(c, &d, &none, &op).unwrap_err();
        assert!(err(&mut c, room(DIV, "Keller")).contains("no room"));
        assert!(err(&mut c, room(DIV, "Flur")).contains("more than one"));
        assert!(err(&mut c, room(AND, "Bad")).contains("has no room"));
        apply(&mut c, &d, &none, &room(DIV, "Bad")).unwrap();
        let text = c.render();
        assert!(text.contains(".room = \"Bad\""), "{text}");
        assert_eq!(Claims::parse(&text).unwrap(), c);
        assert!(compile(&d, &text).contains(r#"Pr="30000a02-0000-0000-ffff504f94000001""#));
    }

    #[test]
    fn current_values_are_read_as_a_config_has_them() {
        let d = doc("house.Loxone");
        let get = |op: Op| current(&d, &op);
        assert_eq!(get(param(DIV, "Input2", None)).as_deref(), Some("1000"));
        // No `Def=`: 0.
        assert_eq!(get(param(AND, "I2", None)).as_deref(), Some("0"));
        let t = Op::Title {
            uuid: DIV.into(),
            value: None,
        };
        assert_eq!(get(t).as_deref(), Some("O1002"));
        let room = Op::Room {
            uuid: DIV.into(),
            value: None,
        };
        assert_eq!(get(room), None);
        assert_eq!(get(param("nope", "I1", None)), None);
    }

    #[test]
    fn statements_vislox_did_not_write_are_refused() {
        assert!(
            Claims::parse("a = And()\n")
                .unwrap_err()
                .contains("did not write")
        );
        assert!(
            Claims::parse("extern x = Div(uuid: \"10000006-0000-0060-ffff504f94000001\")\n")
                .unwrap_err()
                .contains("did not write")
        );
        assert!(Claims::parse("x.title = \"a\"\n").is_err());
        assert_eq!(Claims::parse("# nothing yet\n").unwrap(), Claims::default());
    }

    #[test]
    fn the_module_check_names_a_value_out_of_its_range() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pages");
        std::fs::create_dir_all(dir.join("vislox")).unwrap();
        let uuid = "1000000a-0000-00a0-ffff504f94000001";
        let s = slug(uuid);
        std::fs::write(
            dir.join(FILE),
            format!("extern {s} = Monoflop(uuid: \"{uuid}\")\n{s}.Time = -3\n"),
        )
        .unwrap();
        let e = check_module(&dir).unwrap_err();
        assert!(e.contains("outside the documented range"), "{e}");
        std::fs::write(
            dir.join(FILE),
            format!("extern {s} = Monoflop(uuid: \"{uuid}\")\n{s}.Time = 3\n"),
        )
        .unwrap();
        check_module(&dir).unwrap();
    }

    const EG: &str = "10000002-0000-0020-ffff504f94000001";
    const OG: &str = "10000008-0000-0080-ffff504f94000001";

    /// Compile vislox's files (by name, in `<module>/vislox`) with lox's own
    /// compiler, as `lox logic compile` would, keeping the lock.
    fn compile_files(
        base: &LoxoneDoc,
        files: &Files,
        lock: &mut Lockfile,
    ) -> Result<LoxoneDoc, String> {
        let frags: Vec<Module> = files
            .render()
            .values()
            .map(|t| Module::parse_fragment(t).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        let m = Module::from_fragments(frags);
        logic::validate_ports(&m).map_err(|e| e.to_string())?;
        let opts = CompileOptions {
            machine: [0x50, 0x4f, 0x94, 0, 0, 1],
            mint_time_unix: 0,
            page_title: None,
            allow_removals: false,
            accept_version: None,
            relayout: vec![],
        };
        let out = logic::compile(base, &m, lock, &opts).map_err(|e| e.to_string())?;
        LoxoneDoc::parse(&out.to_bytes()).map_err(|e| e.to_string())
    }

    fn lock_of(l: &Lockfile) -> Lock {
        Lock {
            slugs: l
                .objects
                .iter()
                .map(|(s, o)| (s.clone(), o.uuid.clone()))
                .collect(),
            tombstones: l.removed.values().map(|t| t.slug.clone()).collect(),
            placed: l
                .objects
                .iter()
                .filter(|(_, o)| o.layout.is_some())
                .map(|(s, o)| (s.clone(), o.page_uuid.clone()))
                .collect(),
            wires: l
                .extern_wires
                .iter()
                .map(|w| (w.from.clone(), w.to.clone()))
                .collect(),
            externals: l
                .externals
                .iter()
                .map(|(s, e)| (s.clone(), e.uuid.clone()))
                .collect(),
        }
    }

    fn add(
        page: &str,
        ty: &str,
        title: Option<&str>,
        params: &[(&str, &str)],
        settings: &[(&str, &str)],
    ) -> Op {
        let m = |v: &[(&str, &str)]| {
            v.iter()
                .map(|(k, x)| (k.to_string(), x.to_string()))
                .collect()
        };
        Op::AddBlock {
            page: page.into(),
            ty: ty.into(),
            title: title.map(Into::into),
            params: m(params),
            settings: m(settings),
            wires: vec![],
            restores: None,
        }
    }

    #[test]
    fn the_undo_of_a_request_is_each_previous_last_op_first() {
        let t = |v: Option<&str>| Op::Title {
            uuid: DIV.into(),
            value: v.map(Into::into),
        };
        // A relayout of however many blocks is undone by one `place`: the
        // undo of a request never has more ops than the request.
        let place = Op::Place {
            moves: (0..500)
                .map(|i| Placed {
                    uuid: format!("u{i}"),
                    x: 96 * i,
                    y: 0,
                })
                .collect(),
        };
        let ops = vec![
            t(Some("A")),
            add("p", "And", None, &[], &[]),
            Op::Relayout { page: "p".into() },
            Op::RemoveBlock { uuid: "r".into() },
        ];
        let mut again = add("p", "Or", None, &[], &[]);
        if let Op::AddBlock { restores, .. } = &mut again {
            *restores = Some("r".into());
        }
        let previous = vec![
            serde_json::json!(null),
            serde_json::json!(null),
            serde_json::to_value(place.clone()).unwrap(),
            serde_json::to_value(&again).unwrap(),
        ];
        let minted = [None, Some("n".to_string()), None, None];
        assert_eq!(
            undo_ops(&ops, &previous, &minted).unwrap(),
            vec![again, place, Op::RemoveBlock { uuid: "n".into() }, t(None)]
        );
        assert_eq!(undo_ops(&ops, &previous, &minted).unwrap().len(), ops.len());
        // A block lox minted no UUID for cannot be taken out again; and a
        // previous that is no op says which op.
        let e = undo_ops(&ops, &previous, &[]).unwrap_err();
        assert!(e.contains("op 2 (add_block)"), "{e}");
        let mut broken = previous.clone();
        broken[2] = serde_json::Value::Null;
        let e = undo_ops(&ops, &broken, &minted).unwrap_err();
        assert!(e.contains("op 3 (relayout)"), "{e}");
        // `restores` is echoed, not written: it round-trips, and is left out when none.
        let v = serde_json::to_value(add("p", "And", None, &[], &[])).unwrap();
        assert_eq!(v.get("restores"), None);
    }

    /// The page an object is drawn on.
    fn page_of(doc: &LoxoneDoc, uuid: &str) -> Option<String> {
        fn walk(el: &Element, page: Option<&str>, uuid: &str) -> Option<String> {
            let page = if el.attr("Type") == Some("Page") {
                el.attr("U")
            } else {
                page
            };
            if el.attr("U") == Some(uuid) {
                return page.map(String::from);
            }
            el.child_elements().find_map(|c| walk(c, page, uuid))
        }
        walk(&doc.xml.root, None, uuid)
    }

    #[test]
    fn every_offered_type_is_created_by_lox() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut failed = Vec::new();
        for t in connectors::BUILTIN_TYPES
            .iter()
            .filter(|t| blocks::offered(t))
        {
            let mut files = Files::default();
            let lock = Lock::default();
            let ctx = Ctx {
                doc: &d,
                lock: &lock,
                base: &d,
                deployed: None,
                names: &names,
                drawn: &Default::default(),
                adoptions: &Default::default(),
                page_adoptions: &Default::default(),
                sched_claimed: &Default::default(),
                moods_claimed: &Default::default(),
            };
            apply_op(&mut files, &ctx, &add(EG, t, None, &[], &[])).unwrap();
            let mut l = Lockfile::default();
            match compile_files(&d, &files, &mut l) {
                Ok(out) => {
                    let uuid = &l.objects.values().next().unwrap().uuid;
                    let el = find(&out.xml.root, uuid).unwrap();
                    assert_eq!(el.attr("Type"), Some(*t));
                    assert_eq!(page_of(&out, uuid).as_deref(), Some(EG), "{t}");
                }
                Err(e) => failed.push(format!("{t}: {e}")),
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }

    #[test]
    fn a_block_is_added_named_and_given_values_then_taken_out() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let a = apply_op(
            &mut files,
            &ctx,
            &add(OG, "Monoflop", Some("Nachlauf"), &[("Time", "30")], &[]),
        )
        .unwrap();
        assert_eq!(
            a,
            Applied {
                previous: serde_json::Value::Null,
                added: Some("vx_monoflop_1".into())
            }
        );
        let b = apply_op(
            &mut files,
            &ctx,
            &add(
                OG,
                "PulseAt",
                None,
                &[],
                &[("Pulse", "time"), ("At", "7:05")],
            ),
        )
        .unwrap();
        assert_eq!(b.added.as_deref(), Some("vx_pulseat_1"));
        let text = &files.render()["obergeschoss.loxl"];
        assert!(text.starts_with("# Blocks added in vislox"), "{text}");
        assert!(text.contains("page \"Obergeschoss\"\n"), "{text}");
        assert!(
            text.contains("vx_monoflop_1 = Monoflop(\"Nachlauf\", Time: 30)\n"),
            "{text}"
        );
        assert!(
            text.contains("vx_pulseat_1 = PulseAt(At: 07:05, Pulse: \"time\")\n"),
            "{text}"
        );
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().pages,
            files.pages
        );

        let out = compile_files(&d, &files, &mut l).unwrap();
        let mono = l.objects["vx_monoflop_1"].uuid.clone();
        let el = find(&out.xml.root, &mono).unwrap();
        assert_eq!(el.attr("Title"), Some("Nachlauf"));
        assert_eq!(page_of(&out, &mono).as_deref(), Some(OG));
        let time = el
            .child_elements()
            .find(|c| c.attr("K") == Some("Time"))
            .unwrap();
        assert_eq!(time.attr("Def"), Some("30"));
        let pulse = find(&out.xml.root, &l.objects["vx_pulseat_1"].uuid).unwrap();
        assert_eq!(pulse.attr("Sec"), Some("25500"));

        // Edited like any block, in its declaration; the module's own
        // blocks are not.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: Some(&d),
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let title = Op::Title {
            uuid: mono.clone(),
            value: Some("Treppe".into()),
        };
        assert_eq!(
            apply_op(&mut files, &ctx, &title).unwrap().previous,
            "Nachlauf"
        );
        let p = param(&mono, "Time", Some("45"));
        assert_eq!(apply_op(&mut files, &ctx, &p).unwrap().previous, "30");
        // A setting is written into the declaration too.
        let pulse = l.objects["vx_pulseat_1"].uuid.clone();
        let at = Op::Setting {
            uuid: pulse.clone(),
            name: "At".into(),
            value: Some("8:00".into()),
        };
        assert_eq!(apply_op(&mut files, &ctx, &at).unwrap().previous, "07:05");
        let bad = Op::Setting {
            uuid: pulse.clone(),
            name: "Pulse".into(),
            value: Some("often".into()),
        };
        assert!(
            apply_op(&mut files, &ctx, &bad)
                .unwrap_err()
                .contains("\"time\"")
        );
        assert!(files.claims.0.is_empty());
        // Title, Time; At, Pulse.
        assert_eq!(files.ops(&lock).len(), 4);
        let room = Op::Room {
            uuid: mono.clone(),
            value: Some("Bad".into()),
        };
        assert!(
            apply_op(&mut files, &ctx, &room)
                .unwrap_err()
                .contains("only its name")
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let el = find(&out.xml.root, &mono).unwrap();
        assert_eq!(el.attr("Title"), Some("Treppe"));
        let at = Op::Setting {
            uuid: pulse.clone(),
            name: "At".into(),
            value: None,
        };
        assert_eq!(current(&out, &at).as_deref(), Some("28800"));

        // Taken out: `removed` tells lox, and the op that adds it again comes back.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: Some(&d),
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: mono.clone() }).unwrap();
        // It names the block it adds again (#62): lox mints new UUIDs for it.
        let mut again = add(OG, "Monoflop", Some("Treppe"), &[("Time", "45")], &[]);
        if let Op::AddBlock { restores, .. } = &mut again {
            *restores = Some(mono.clone());
        }
        assert_eq!(serde_json::from_value::<Op>(r.previous).unwrap(), again);
        assert!(files.render()["obergeschoss.loxl"].contains("removed vx_monoflop_1\n"));
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(find(&out.xml.root, &mono).is_none());
        // Once lox has forgotten it, the line goes.
        files.tidy(&lock_of(&l));
        assert!(!files.render()["obergeschoss.loxl"].contains("removed"));
        // A deployed block is not taken out here.
        let pulse = l.objects["vx_pulseat_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: Some(&out),
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let e = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: pulse }).unwrap_err();
        assert!(e.contains("is deployed"), "{e}");
        let e = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: DIV.into() }).unwrap_err();
        assert!(e.contains("only a block added here"), "{e}");
    }

    #[test]
    fn a_page_file_follows_its_page_and_goes_with_its_last_block() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            base: &d,
            lock: &lock,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", Some("Licht"), &[], &[])).unwrap();
        let text = files.render()["obergeschoss.loxl"].clone();
        assert!(
            text.contains(&format!("# page uuid: {OG}\npage \"Obergeschoss\"")),
            "{text}"
        );
        compile_files(&d, &files, &mut l).unwrap();
        // Loxone Config renames the page: the same file follows it, by UUID.
        let renamed = LoxoneDoc::parse(
            std::fs::read_to_string(format!(
                "{}/tests/fixtures/house.Loxone",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap()
            .replace("Title=\"Obergeschoss\"", "Title=\"Oben\"")
            .as_bytes(),
        )
        .unwrap();
        let mut files = Files::from_texts(&files.render()).unwrap();
        files.follow_pages(&renamed);
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &renamed,
            base: &renamed,
            lock: &lock,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "Or", None, &[], &[])).unwrap();
        let r = files.render();
        assert_eq!(r.keys().collect::<Vec<_>>(), ["obergeschoss.loxl"]);
        assert!(r["obergeschoss.loxl"].contains("page \"Oben\"\n"), "{r:?}");
        let out = compile_files(&renamed, &files, &mut l).unwrap();
        for s in ["vx_and_1", "vx_or_1"] {
            assert_eq!(
                page_of(&out, &l.objects[s].uuid).as_deref(),
                Some(OG),
                "{s}"
            );
        }
        // A file written before it had a UUID takes the page with its title.
        let mut old = Files::from_texts(&BTreeMap::from([(
            "oben.loxl".to_string(),
            "page \"Oben\"\nvx_and_1 = And()\n".to_string(),
        )]))
        .unwrap();
        old.follow_pages(&renamed);
        assert_eq!(old.pages["oben.loxl"].uuid.as_deref(), Some(OG));

        // Both taken out, and forgotten by lox: the file goes.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            base: &renamed,
            lock: &lock,
            deployed: Some(&renamed),
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        for s in ["vx_and_1", "vx_or_1"] {
            let uuid = l.objects[s].uuid.clone();
            apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid }).unwrap();
        }
        assert!(files.render().contains_key("obergeschoss.loxl"));
        compile_files(&renamed, &files, &mut l).unwrap();
        files.tidy(&lock_of(&l));
        assert!(files.render().is_empty(), "{:?}", files.render());
    }

    #[test]
    fn a_new_block_is_checked_before_lox_sees_it() {
        let d = doc("house.Loxone");
        let names = BTreeSet::from(["vx_and_1".to_string()]);
        let lock = Lock::default();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let mut files = Files::default();
        let err = |op: Op| apply_op(&mut files.clone(), &ctx, &op).unwrap_err();
        assert!(err(add(DIV, "And", None, &[], &[])).contains("no page"));
        assert!(err(add(EG, "VirtualIn", None, &[], &[])).contains("does not add"));
        assert!(err(add(EG, "And", Some("a|b"), &[], &[])).contains("\"a_b\""));
        assert!(err(add(EG, "And", Some(" "), &[], &[])).contains("cannot be empty"));
        assert!(err(add(EG, "And", None, &[("Q", "1")], &[])).contains("no input Q"));
        assert!(err(add(EG, "And", None, &[("I1", "1,5")], &[])).contains("decimal point"));
        assert!(err(add(EG, "Memory", None, &[], &[("Type", "big")])).contains("\"digital\""));
        assert!(err(add(EG, "PulseAt", None, &[], &[("At", "25:00")])).contains("time of day"));
        assert!(err(add(EG, "Code16", None, &[], &[("Code", "x")])).contains("cannot be set"));
        // A slug the module has is not picked again; a port by its key.
        let a = apply_op(&mut files, &ctx, &add(EG, "And", None, &[("I2", "1")], &[])).unwrap();
        assert_eq!(a.added.as_deref(), Some("vx_and_2"));
        assert!(files.render()["erdgeschoss.loxl"].contains("vx_and_2 = And(In2: 1)"));

        // Two pages with one title: lox would build on the first.
        let xml = std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace("Title=\"Obergeschoss\"", "Title=\"Erdgeschoss\"");
        let d = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let e = apply_op(&mut Files::default(), &ctx, &add(OG, "And", None, &[], &[])).unwrap_err();
        assert!(e.contains("2 pages are titled"), "{e}");
        apply_op(&mut Files::default(), &ctx, &add(EG, "And", None, &[], &[])).unwrap();
    }

    #[test]
    fn managed_blocks_come_from_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("logic.lock.json");
        std::fs::write(
            &lock,
            r#"{"objects":{"a":{"uuid":"U1","type":"And"}},"removed":{"U0":{"slug":"b","type":"Or"}}}"#,
        )
        .unwrap();
        let l = Lock::read(&lock);
        assert_eq!((l.slug_of("U1"), l.uuid_of("a")), (Some("a"), Some("U1")));
        assert!(l.placed.is_empty());
        std::fs::write(
            &lock,
            r#"{"objects":{"a":{"uuid":"U1","type":"And","page_uuid":"P","layout":{"px":96,"py":192,"px2":960,"py2":768}}}}"#,
        )
        .unwrap();
        assert_eq!(Lock::read(&lock).placed["a"].as_deref(), Some("P"));
        assert!(l.tombstones.contains("b"));
        assert!(Lock::read(&tmp.path().join("none")).slugs.is_empty());
    }

    /// `lock` with `moves` written in, as the next compile reads it.
    fn moved(l: &Lockfile, base: &LoxoneDoc, moves: &BTreeMap<String, Layout>) -> Lockfile {
        serde_json::from_slice(&write_moves(l.to_json().as_bytes(), base, moves).unwrap()).unwrap()
    }

    #[test]
    fn a_managed_block_is_moved_through_the_lock() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", Some("Licht"), &[], &[])).unwrap();
        // Deployed: the base draws it where lox placed it.
        let base = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.objects["vx_and_1"].uuid.clone();
        let placed = Layout::of_element(find(&base.xml.root, &uuid).unwrap()).unwrap();
        let lock = lock_of(&l);
        assert_eq!(lock.placed["vx_and_1"].as_deref(), Some(OG));
        let ctx = Ctx {
            doc: &base,
            lock: &lock,
            base: &base,
            deployed: Some(&base),
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        // Snapped to the grid, the size kept; `previous` moves it back.
        let mut files = Files::from_texts(&files.render()).unwrap();
        let (x, y) = (placed.px + 2000, placed.py + 1010);
        let a = apply_op(
            &mut files,
            &ctx,
            &Op::Move {
                uuid: uuid.clone(),
                x,
                y,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            Op::Move {
                uuid: uuid.clone(),
                x: placed.px,
                y: placed.py
            }
        );
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px % GRID, to.py % GRID), (0, 0));
        assert_eq!((to.px, to.py), (snap(x), snap(y)));
        assert_eq!((to.w(), to.h()), (placed.w(), placed.h()));
        // Not a file: vislox's files are as they were.
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().pages,
            files.pages
        );

        // The lock alone is not enough: lox takes the base's position back
        // as a move made in Loxone Config.
        let mut plain = l.clone();
        plain.objects.get_mut("vx_and_1").unwrap().layout = Some(to);
        let out = compile_files(&base, &files, &mut plain).unwrap();
        let at = Layout::of_element(find(&out.xml.root, &uuid).unwrap()).unwrap();
        assert_eq!((at.px, at.py), (placed.px, placed.py));
        // With the position the base still shows superseded, the move holds,
        // and keeps holding on the next compile.
        let mut l2 = moved(&l, &base, &files.layout.moves);
        assert_eq!(l2.objects["vx_and_1"].superseded_layout, Some(placed));
        for _ in 0..2 {
            let out = compile_files(&base, &files, &mut l2).unwrap();
            let at = Layout::of_element(find(&out.xml.root, &uuid).unwrap()).unwrap();
            assert_eq!((at.px, at.py), (to.px, to.py));
            assert!(moves_missed(&out, &base, &lock_of(&l2), &files.layout).is_empty());
        }
        // Once applied, the base draws it there, and lox drops the line.
        let applied = compile_files(&base, &files, &mut l2).unwrap();
        compile_files(&applied, &files, &mut l2).unwrap();
        assert_eq!(l2.objects["vx_and_1"].superseded_layout, None);

        // Back where the base draws it: nothing superseded.
        let back = Placement {
            moves: BTreeMap::from([("vx_and_1".to_string(), placed)]),
            ..Default::default()
        };
        let mut l3 = moved(&l2, &base, &back.moves);
        assert_eq!(l3.objects["vx_and_1"].superseded_layout, None);
        let out = compile_files(&base, &files, &mut l3).unwrap();
        assert!(moves_missed(&out, &base, &lock_of(&l3), &back).is_empty());
        assert!(!moves_missed(&out, &base, &lock_of(&l3), &files.layout).is_empty());
    }

    #[test]
    fn only_managed_blocks_on_a_page_are_moved_or_laid_out() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        apply_op(&mut files, &ctx, &add(OG, "Or", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let err = |op: Op| apply_op(&mut files.clone(), &ctx, &op).unwrap_err();
        // Drawn in Loxone Config: lox does not move it.
        let e = err(Op::Move {
            uuid: DIV.into(),
            x: 0,
            y: 0,
        });
        assert!(e.contains("drawn in Loxone Config"), "{e}");
        let and = l.objects["vx_and_1"].uuid.clone();
        let e = err(Op::Move {
            uuid: and.clone(),
            x: 5_000_000,
            y: 0,
        });
        assert!(e.contains("too far"), "{e}");
        assert!(
            err(Op::Move {
                uuid: "nope".into(),
                x: 0,
                y: 0
            })
            .contains("no object")
        );
        // A page without managed blocks has nothing lox lays out.
        let e = err(Op::Relayout { page: EG.into() });
        assert!(e.contains("manages no block on this page"), "{e}");
        assert!(err(Op::Relayout { page: DIV.into() }).contains("no page"));
        let a = apply_op(&mut files, &ctx, &Op::Relayout { page: OG.into() }).unwrap();
        assert_eq!(
            files.layout.relayout,
            BTreeMap::from([(OG.to_string(), "Obergeschoss".to_string())])
        );
        // `previous` puts each block back, in one `place` (#62).
        let Op::Place { moves: back } = serde_json::from_value(a.previous).unwrap() else {
            panic!("not a place")
        };
        assert_eq!(back.len(), 2);
        assert!(back.iter().any(|m| m.uuid == and));
        // A `place` moves each block as `move` does, and its previous is the
        // `place` back.
        let mut again = files.clone();
        let p = apply_op(
            &mut again,
            &ctx,
            &Op::Place {
                moves: back.clone(),
            },
        )
        .unwrap();
        assert!(
            matches!(serde_json::from_value(p.previous).unwrap(), Op::Place { moves } if moves.len() == 2)
        );
        assert_eq!(again.layout.moves.len(), 2);
        let e = apply_op(&mut again, &ctx, &Op::Place { moves: vec![] }).unwrap_err();
        assert!(e.contains("no blocks"), "{e}");
    }

    #[test]
    fn snapping_follows_loxone_configs_grid() {
        assert_eq!(snap(0), 0);
        assert_eq!(snap(47), 0);
        assert_eq!(snap(48), 96);
        assert_eq!(snap(1000), 960);
        assert_eq!(snap(-47), 0);
        assert_eq!(snap(-49), -96);
    }

    /// `doc` with the element `uuid` drawn at (`px`, `py`), its size kept.
    fn drawn_moved(doc: &LoxoneDoc, uuid: &str, px: i64, py: i64) -> LoxoneDoc {
        let at = Layout::of_element(find(&doc.xml.root, uuid).unwrap()).unwrap();
        let text = String::from_utf8(doc.to_bytes()).unwrap();
        let line = text
            .lines()
            .find(|l| l.contains(&format!("U=\"{uuid}\"")))
            .unwrap();
        let new = line
            .replace(&format!("Px=\"{}\"", at.px), &format!("Px=\"{px}\""))
            .replace(&format!("Py=\"{}\"", at.py), &format!("Py=\"{py}\""))
            .replace(
                &format!("Px2=\"{}\"", at.px2),
                &format!("Px2=\"{}\"", px + at.w()),
            )
            .replace(
                &format!("Py2=\"{}\"", at.py2),
                &format!("Py2=\"{}\"", py + at.h()),
            );
        LoxoneDoc::parse(text.replace(line, &new).as_bytes()).unwrap()
    }

    #[test]
    fn a_block_off_the_grid_is_moved_away_and_back_and_never_past_the_top_left() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let uuid = l.objects["vx_and_1"].uuid.clone();
        // Deployed off the grid (moved in Loxone Config): lox takes it over.
        let placed = Layout::of_element(find(&out.xml.root, &uuid).unwrap()).unwrap();
        let (bx, by) = (placed.px + 4800 + 48, placed.py + 960 + 24);
        let base = drawn_moved(&out, &uuid, bx, by);
        let out = compile_files(&base, &files, &mut l).unwrap();
        assert_eq!(
            Layout::of_element(find(&out.xml.root, &uuid).unwrap()).map(|a| (a.px, a.py)),
            Some((bx, by))
        );
        let mv = |files: &mut Files, l: &Lockfile, out: &LoxoneDoc, x, y| {
            let lock = lock_of(l);
            let ctx = Ctx {
                doc: out,
                lock: &lock,
                base: &base,
                deployed: Some(&base),
                names: &names,
                drawn: &Default::default(),
                adoptions: &Default::default(),
                page_adoptions: &Default::default(),
                sched_claimed: &Default::default(),
                moods_claimed: &Default::default(),
            };
            files.layout = Placement::default();
            apply_op(
                files,
                &ctx,
                &Op::Move {
                    uuid: uuid.clone(),
                    x,
                    y,
                },
            )
            .unwrap()
        };
        // Away: snapped to the grid.
        let a = mv(&mut files, &l, &out, bx + 2000, by);
        assert_eq!(
            serde_json::from_value::<Op>(a.previous).unwrap(),
            Op::Move {
                uuid: uuid.clone(),
                x: bx,
                y: by
            }
        );
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px, to.py), (snap(bx + 2000), snap(by)));
        let mut l2 = moved(&l, &base, &files.layout.moves);
        let away = compile_files(&base, &files, &mut l2).unwrap();
        assert!(moves_missed(&away, &base, &lock_of(&l2), &files.layout).is_empty());

        // Back (Revert): to the deployed position as it is, off the grid; lox
        // keeps the base's rectangle, and that counts as kept.
        mv(&mut files, &l2, &away, bx, by);
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px, to.py), (bx, by));
        let mut l3 = moved(&l2, &base, &files.layout.moves);
        assert_eq!(l3.objects["vx_and_1"].superseded_layout, None);
        let back = compile_files(&base, &files, &mut l3).unwrap();
        assert!(moves_missed(&back, &base, &lock_of(&l3), &files.layout).is_empty());
        // A snapped target within a grid step of it is kept the same way.
        let near = Placement {
            moves: BTreeMap::from([(
                "vx_and_1".to_string(),
                Layout::at(snap(bx), snap(by), to.w(), to.h()),
            )]),
            ..Default::default()
        };
        assert!(moves_missed(&back, &base, &lock_of(&l3), &near).is_empty());

        // Never past the page's top left.
        mv(&mut files, &l3, &back, -500, -10);
        let to = files.layout.moves["vx_and_1"];
        assert_eq!((to.px, to.py), (0, 0));
    }

    #[test]
    fn only_moves_on_a_page_laid_out_afresh_go_unchecked() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        apply_op(&mut files, &ctx, &add(EG, "Or", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        // Neither move is in `out`: only the one on the page not laid out is missed.
        let far = Layout::at(19200, 9600, 1344, 696);
        let place = Placement {
            moves: BTreeMap::from([("vx_and_1".to_string(), far), ("vx_or_1".to_string(), far)]),
            relayout: BTreeMap::from([(OG.to_string(), "Obergeschoss".to_string())]),
        };
        assert_eq!(moves_missed(&out, &d, &lock, &place), ["vx_or_1"]);
        let none = Placement {
            relayout: BTreeMap::new(),
            ..place
        };
        assert_eq!(
            moves_missed(&out, &d, &lock, &none),
            ["vx_and_1", "vx_or_1"]
        );
    }

    #[test]
    fn a_lock_entry_without_a_uuid_is_refused() {
        let d = doc("house.Loxone");
        let moves = BTreeMap::from([("a".to_string(), Layout::at(0, 0, 96, 96))]);
        for lock in [
            r#"{"objects":{"a":{"type":"And"}}}"#,
            r#"{"objects":{"a":{"uuid":"","type":"And"}}}"#,
        ] {
            let e = write_moves(lock.as_bytes(), &d, &moves).unwrap_err();
            assert!(e.contains("no UUID for block a"), "{e}");
        }
        let e = write_moves(br#"{"objects":{}}"#, &d, &moves).unwrap_err();
        assert!(e.contains("no block a"), "{e}");
    }

    #[test]
    fn arrange_says_why_a_page_cannot_be_laid_out() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let mut lock = lock_of(&l);
        assert_eq!(
            relayout_title(&out, &out, &lock, OG).unwrap(),
            "Obergeschoss"
        );
        assert!(
            relayout_title(&out, &out, &lock, EG)
                .unwrap_err()
                .contains("manages no block")
        );
        // A lock from before lox pinned pages.
        lock.placed.insert("vx_and_1".into(), None);
        assert!(
            relayout_title(&out, &out, &lock, OG)
                .unwrap_err()
                .contains("older lox")
        );
        // Another page with its title.
        let twice = LoxoneDoc::parse(
            String::from_utf8(out.to_bytes())
                .unwrap()
                .replace("Title=\"Erdgeschoss\"", "Title=\"Obergeschoss\"")
                .as_bytes(),
        )
        .unwrap();
        let lock = lock_of(&l);
        assert!(
            relayout_title(&twice, &twice, &lock, OG)
                .unwrap_err()
                .contains("2 pages are titled")
        );
    }

    // ── wires (#61) ─────────────────────────────────────────────────────

    const OR: &str = "10000009-0000-0090-ffff504f94000001";

    fn ends(f: (&str, &str), t: (&str, &str)) -> ([String; 2], [String; 2]) {
        (
            [f.0.to_string(), f.1.to_string()],
            [t.0.to_string(), t.1.to_string()],
        )
    }

    fn wire_op(f: (&str, &str), t: (&str, &str)) -> Op {
        let (from, to) = ends(f, t);
        Op::Wire { from, to }
    }

    fn unwire_op(f: (&str, &str), t: (&str, &str)) -> Op {
        let (from, to) = ends(f, t);
        Op::Unwire { from, to }
    }

    /// Whether `doc` wires the connector `from` of one object into `to` of another.
    fn has_wire(doc: &LoxoneDoc, f: (&str, &str), t: (&str, &str)) -> bool {
        let co = |u: &str, k: &str| {
            find(&doc.xml.root, u)
                .unwrap()
                .child_elements()
                .find(|c| c.name == "Co" && c.attr("K") == Some(k))
                .unwrap()
        };
        let src = co(f.0, f.1).attr("U").unwrap().to_string();
        co(t.0, t.1)
            .child_elements()
            .any(|c| c.name == "In" && c.attr("Input") == Some(src.as_str()))
    }

    #[test]
    fn a_wire_between_objects_loxone_config_drew_is_compiled_by_lox_and_taken_out_again() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        // The Divide block's output onto the upper floor's OR: another page.
        let w = wire_op((DIV, "AQ"), (OR, "I2"));
        let a = apply_op(&mut files, &ctx, &w).unwrap();
        assert_eq!(
            a.previous,
            serde_json::to_value(unwire_op((DIV, "AQ"), (OR, "I2"))).unwrap()
        );
        let text = &files.render()["edits.loxl"];
        let (sd, so) = (slug(DIV), slug(OR));
        assert!(
            text.contains(&format!("extern {sd} = Div(uuid: \"{DIV}\")\n")),
            "{text}"
        );
        assert!(text.contains(&format!("{so}.In2 <- {sd}.AQ\n")), "{text}");
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        assert_eq!(
            wire_ends(&files, &lock),
            vec![ends((DIV, "AQ"), (OR, "I2"))]
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(has_wire(&out, (DIV, "AQ"), (OR, "I2")));
        // The wire Loxone Config drew into the other input stays.
        assert!(has_wire(
            &out,
            ("10000004-0000-0040-ffff504f94000001", "Q"),
            (OR, "I1")
        ));
        assert_eq!(l.extern_wires.len(), 1);

        // Taken out again: the statement and the externs go, and lox takes the wire out.
        let lock = lock_of(&l);
        let drawn = files.claims.1.clone();
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &drawn,
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        // One request may take a wire out and draw it again (an undo and a
        // redo, a reconnect to the same port): the ops see each other.
        let u = unwire_op((DIV, "AQ"), (OR, "I2"));
        let mut again = files.clone();
        apply_op(&mut again, &ctx, &u).unwrap();
        let e = apply_op(&mut again.clone(), &ctx, &u).unwrap_err();
        assert!(e.contains("is not wired"), "{e}");
        apply_op(&mut again, &ctx, &w).unwrap();
        assert_eq!(again, files);
        let e = apply_op(&mut again, &ctx, &w).unwrap_err();
        assert!(e.contains("already wired"), "{e}");
        assert_eq!(
            apply_op(&mut files, &ctx, &u).unwrap().previous,
            serde_json::to_value(&w).unwrap()
        );
        assert_eq!(files.claims, Claims::default());
        let again = compile_files(&d, &files, &mut l).unwrap();
        assert!(!has_wire(&again, (DIV, "AQ"), (OR, "I2")));
        assert!(l.extern_wires.is_empty());
    }

    #[test]
    fn a_wire_is_refused_where_loxone_config_or_lox_would_not_have_it() {
        const AND_Q: (&str, &str) = (AND, "Q");
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let lock = Lock::default();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        assert!(err(wire_op((DIV, "Input2"), (OR, "I2"))).contains("is an input"));
        assert!(err(wire_op((DIV, "AQ"), (AND, "Q"))).contains("is an output"));
        assert!(err(wire_op((DIV, "AQ"), (DIV, "Input2"))).contains("another block"));
        assert!(err(wire_op((DIV, "AQ"), (OR, "Nope"))).contains("no connector"));
        // Loxone Config drew this one already.
        assert!(err(wire_op(AND_Q, (OR, "I1"))).contains("already wired"));
        // ...and only Loxone Config can take it out.
        let e = err(unwire_op(AND_Q, (OR, "I1")));
        assert!(e.contains("Drawn in Loxone Config"), "{e}");
        assert!(err(unwire_op((DIV, "AQ"), (OR, "I2"))).contains("not wired"));

        // A text output goes only into a text input.
        let xml = std::fs::read_to_string(format!(
            "{}/tests/fixtures/house.Loxone",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace(
            r#"Analog="true" LinkRefType="71""#,
            r#"Text="true" LinkRefType="71""#,
        );
        let t = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        let ref_el = find(&t.xml.root, REF).unwrap();
        assert_eq!(signal("InputRef", Some(ref_el), "AQ"), Signal::Text);
        let el = |u: &str| find(&d.xml.root, u);
        assert_eq!(signal("InputRef", el(REF), "AQ"), Signal::Analog);
        assert_eq!(signal("And", el(AND), "Q"), Signal::Digital);
        // From lox's tables: a virtual input's value, a text output, a
        // memory flag's type; a connector lox does not list is unknown.
        assert_eq!(signal("VirtualIn", el(VI), "Q"), Signal::Digital);
        assert_eq!(signal("VirtualTextIn", None, "Q"), Signal::Text);
        assert_eq!(signal("Status", None, "TQ"), Signal::Text);
        let mem =
            loxone_config::xml::XmlDocument::parse(br#"<C Type="Memory" Tp="2" U="x"/>"#).unwrap();
        assert_eq!(signal("Memory", Some(&mem.root), "Q"), Signal::Text);
        assert_eq!(signal("Banana", None, "Q"), Signal::Unknown);
        assert_eq!(signal("And", None, "Nope"), Signal::Unknown);
        let tctx = Ctx {
            doc: &t,
            lock: &lock,
            base: &t,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let e = apply_op(
            &mut Files::default(),
            &tctx,
            &wire_op((REF, "AQ"), (OR, "I2")),
        )
        .unwrap_err();
        assert!(e.contains("text only to text"), "{e}");
        // Analog into digital is fine: a value other than 0 is on.
        apply_op(
            &mut Files::default(),
            &ctx,
            &wire_op((REF, "AQ"), (OR, "I2")),
        )
        .unwrap();

        // A block the rest of the module manages is wired in its own source,
        // and the wires it draws are taken out there.
        let managed = Lock {
            slugs: BTreeMap::from([("teiler".to_string(), DIV.to_string())]),
            wires: BTreeSet::from([(
                "10000003-0000-0032-01ff504f94000001".to_string(),
                "10000006-0000-0061-00ff504f94000001".to_string(),
            )]),
            ..Lock::default()
        };
        let mctx = Ctx {
            doc: &d,
            lock: &managed,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let e = apply_op(
            &mut Files::default(),
            &mctx,
            &wire_op((DIV, "AQ"), (OR, "I2")),
        )
        .unwrap_err();
        assert!(e.contains("module manages"), "{e}");
        let e = apply_op(
            &mut Files::default(),
            &mctx,
            &unwire_op((REF, "AQ"), (DIV, "Input1")),
        )
        .unwrap_err();
        assert!(e.contains("module draws this wire"), "{e}");
        // An input with a constant edited here is cleared first.
        let mut files = Files::default();
        apply_op(&mut files, &ctx, &param(OR, "I2", Some("1"))).unwrap();
        let e = apply_op(&mut files, &ctx, &wire_op((DIV, "AQ"), (OR, "I2"))).unwrap_err();
        assert!(e.contains("clear it first"), "{e}");
    }

    #[test]
    fn a_block_added_here_is_wired_both_ways_and_its_wires_go_with_it() {
        let d = doc("house.Loxone");
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        apply_op(&mut files, &ctx, &add(OG, "And", None, &[("I2", "1")], &[])).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let new = l.objects["vx_and_1"].uuid.clone();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        // Its input with a constant takes no wire; the other does.
        let e = apply_op(&mut files, &ctx, &wire_op((DIV, "AQ"), (&new, "I2"))).unwrap_err();
        assert!(e.contains("clear it first"), "{e}");
        apply_op(&mut files, &ctx, &wire_op((DIV, "AQ"), (&new, "I1"))).unwrap();
        apply_op(&mut files, &ctx, &wire_op((&new, "Q"), (OR, "I2"))).unwrap();
        let text = &files.render()["edits.loxl"];
        assert!(text.contains("vx_and_1.In1 <- "), "{text}");
        assert!(
            text.contains(&format!("{}.In2 <- vx_and_1.Q\n", slug(OR))),
            "{text}"
        );
        let mut ends_now = wire_ends(&files, &lock);
        ends_now.sort();
        let mut want = vec![
            ends((DIV, "AQ"), (&new, "I1")),
            ends((&new, "Q"), (OR, "I2")),
        ];
        want.sort();
        assert_eq!(ends_now, want);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(has_wire(&out, (DIV, "AQ"), (&new, "I1")));
        assert!(has_wire(&out, (&new, "Q"), (OR, "I2")));

        // Taken out, the block takes its wires along, and the externs they needed.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let undo = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: new.clone() })
            .unwrap()
            .previous;
        assert!(
            files.claims.1.is_empty() && files.claims.0.is_empty(),
            "{:?}",
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert!(find(&out.xml.root, &new).is_none());
        assert!(!has_wire(&out, (DIV, "AQ"), (OR, "I2")));

        // Its `previous` adds it again with its wires: a complete undo. lox
        // mints new UUIDs, so the block names itself by an empty UUID.
        let undo: Op = serde_json::from_value(undo).unwrap();
        let Op::AddBlock { wires, .. } = &undo else {
            panic!("{undo:?}")
        };
        let mut got: Vec<_> = wires
            .iter()
            .map(|w| (w.from.clone(), w.to.clone()))
            .collect();
        got.sort();
        let mut want = vec![ends((DIV, "AQ"), ("", "I1")), ends(("", "Q"), (OR, "I2"))];
        want.sort();
        assert_eq!(got, want);
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let a = apply_op(&mut files, &ctx, &undo).unwrap();
        let slug2 = a.added.unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let back = l.objects[&slug2].uuid.clone();
        assert_ne!(back, new);
        assert!(has_wire(&out, (DIV, "AQ"), (&back, "I1")));
        assert!(has_wire(&out, (&back, "Q"), (OR, "I2")));
        assert_eq!(
            find(&out.xml.root, &back).unwrap().attr("Type"),
            Some("And")
        );
    }

    #[test]
    fn a_wire_whose_end_the_files_do_not_declare_is_refused() {
        let s = slug(DIV);
        let page = "# page uuid: 10000008-0000-0080-ffff504f94000001\npage \"Obergeschoss\"\n\nvx_and_1 = And()\n";
        let files = |claims: String| {
            BTreeMap::from([
                ("edits.loxl".to_string(), claims),
                ("obergeschoss.loxl".to_string(), page.to_string()),
            ])
        };
        let ok = format!("extern {s} = Div(uuid: \"{DIV}\")\nvx_and_1.In1 <- {s}.AQ\n");
        assert_eq!(Files::from_texts(&files(ok)).unwrap().claims.1.len(), 1);
        // Its extern deleted by hand: refused, not written back to break the compile.
        let e = Files::from_texts(&files(format!("vx_and_1.In1 <- {s}.AQ\n"))).unwrap_err();
        assert!(e.contains(&format!("names `{s}`")), "{e}");
        // A block no file of vislox's declares.
        let e = Files::from_texts(&files(format!(
            "extern {s} = Div(uuid: \"{DIV}\")\nvx_or_9.In1 <- {s}.AQ\n"
        )))
        .unwrap_err();
        assert!(e.contains("names `vx_or_9`"), "{e}");
    }

    #[test]
    fn wires_in_the_file_are_read_back_and_foreign_ones_refused() {
        let u = DIV;
        let s = slug(u);
        let text =
            format!("extern {s} = Div(uuid: \"{u}\", Input2: vx_and_1.Q)\nvx_or_1.In1 <- {s}.AQ\n");
        let c = Claims::parse(&text).unwrap();
        assert_eq!(c.1.len(), 2);
        assert!(c.0[u].is_empty());
        // Rendered with the wires as statements, and read back the same.
        assert_eq!(Claims::parse(&c.render()).unwrap(), c);
        let e = Claims::parse(&format!(
            "extern {s} = Div(uuid: \"{u}\")\n{s}.In2 <- licht.Q\n"
        ))
        .unwrap_err();
        assert!(e.contains("did not write"), "{e}");
    }

    /// Handing back one of two blocks adopted together (#72): the other keeps
    /// every wire between them, onto vislox's extern of the one handed back
    /// (a loop's wire, a binding in either declaration).
    #[test]
    fn a_block_handed_back_leaves_its_neighbours_wires_on_an_extern() {
        const METER: &str = "10000005-0000-0050-ffff504f94000001";
        let d = doc("house.Loxone");
        let lock = Lock {
            slugs: BTreeMap::from([
                ("vx_a".to_string(), AND.to_string()),
                ("vx_b".to_string(), METER.to_string()),
            ]),
            ..Default::default()
        };
        let text = |a: &str, b: &str, wire: &str| {
            format!(
                "page \"Erdgeschoss\"\n# Adopted from Loxone Config\nvx_a = And(\"O1001\"{a})\nvx_b = MeterAbsUni(\"Zähler\"{b})\n{wire}"
            )
        };
        let files = |page: String| {
            let mut f =
                Files::from_texts(&BTreeMap::from([("eg.loxl".to_string(), page)])).unwrap();
            f.repin();
            f
        };
        let (adoptions, pages) = (Default::default(), Default::default());
        let ctx = Ctx {
            doc: &d,
            base: &d,
            lock: &lock,
            deployed: None,
            names: &Default::default(),
            drawn: &Default::default(),
            adoptions: &adoptions,
            page_adoptions: &pages,
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let back = |f: &mut Files, u: &str| {
            apply_op(
                f,
                &ctx,
                &Op::Unadopt {
                    uuids: vec![u.into()],
                },
            )
            .unwrap();
            f.render()
        };
        let ext_a = slug(AND);
        // A loop's wire, out of the block that stays into the one handed back.
        let mut f = files(text("", "", "vx_a.In2 <- vx_b\n"));
        let out = back(&mut f, AND);
        assert!(
            out["eg.loxl"].contains(&format!("{ext_a}.In2 <- vx_b\n")),
            "{out:?}"
        );
        assert!(!out["eg.loxl"].contains("vx_a "), "{out:?}");
        assert!(
            out[CLAIMS].contains(&format!("extern {ext_a} = And(uuid: \"{AND}\")")),
            "{out:?}"
        );
        assert!(f.lock_edits.drop.contains("vx_a"));
        // A binding in the other block's declaration names its extern.
        let mut f = files(text("", ", Power: vx_a", ""));
        let out = back(&mut f, AND);
        assert!(
            out["eg.loxl"].contains(&format!("Power: {ext_a})")),
            "{out:?}"
        );
        // A binding in its own declaration, fed by the block that stays: that
        // block's wire onto its extern now.
        let mut f = files(text("", ", Power: vx_a", ""));
        let out = back(&mut f, METER);
        let ext_b = slug(METER);
        assert!(
            out["eg.loxl"].contains(&format!("{ext_b}.Power <- vx_a\n")),
            "{out:?}"
        );
        assert!(!out["eg.loxl"].contains("vx_b"), "{out:?}");
        // Everything the files say still reads back.
        Files::from_texts(&out).unwrap();
    }

    /// An interval in a test: from, to, value.
    type Sp = (u32, u32, Option<&'static str>);

    const UHR: &str = "10000020-0000-0200-ffff504f94000001";
    const HEIZ: &str = "10000021-0000-0210-ffff504f94000001";
    const LICHT: &str = "10000022-0000-0220-ffff504f94000001";
    const PARTY: &str = "10000011-0000-0110-ffff504f94000001";

    /// The fixture with three Daytimers on the upper floor: Uhr and Heizung
    /// (analog; Uhr with a Monday and a Party column), Licht (digital).
    fn timers() -> LoxoneDoc {
        let p = format!("{}/tests/fixtures/house.Loxone", env!("CARGO_MANIFEST_DIR"));
        let mut s = std::fs::read_to_string(p).unwrap();
        let timer = |u: &str, n: u32, title: &str, analog: bool, modes: &str, body: &str| {
            format!(
                "\t\t\t<C Type=\"DayTimer\" V=\"178\" U=\"{u}\" Title=\"{title}\" Px=\"960\" Py=\"{}\" \
                 Px2=\"3648\" Py2=\"{}\" Nio=\"2\" WF=\"16384\"{}{}>\
                 <Co K=\"Mode\" U=\"{}\"/><Co K=\"AQ\" U=\"{}\"/>{body}</C>\n",
                4800 + 960 * n,
                5600 + 960 * n,
                if analog {
                    " Analog=\"true\" DefValue=\"18\""
                } else {
                    ""
                },
                if modes.is_empty() {
                    String::new()
                } else {
                    format!(" Modes=\"{modes}\"")
                },
                u.replacen("0-ffff", "1-00ff", 1),
                u.replacen("0-ffff", "2-01ff", 1),
            )
        };
        let all = timer(
            UHR,
            0,
            "Uhr",
            true,
            &format!("00000000-0000-0004-1500000000000000,{PARTY}"),
            "<Entry From=\"390\" To=\"480\" V=\"21\"/><Entry From=\"1080\" To=\"1380\" Ix=\"1\" V=\"22.5\"/>",
        ) + &timer(HEIZ, 1, "Heizung", true, "", "")
            + &timer(LICHT, 2, "Licht", false, "", "");
        let end = "\n\t\t</C>\n\t</C>\n</ControlList>";
        let at = s.rfind(end).expect("end of the upper floor");
        s.insert_str(at + 1, &all);
        LoxoneDoc::parse(s.as_bytes()).unwrap()
    }

    fn sched(set: Option<&str>, cols: &[(&str, &[Sp])]) -> Option<crate::sched_edit::Sched> {
        Some(crate::sched_edit::Sched {
            set: set.map(Into::into),
            modes: (!cols.is_empty()).then(|| {
                cols.iter()
                    .map(|(u, spans)| {
                        let v = spans
                            .iter()
                            .map(|(from, to, value)| crate::sched_edit::Span {
                                from: *from,
                                to: *to,
                                value: value.map(Into::into),
                                activate: false,
                            })
                            .collect();
                        (u.to_string(), v)
                    })
                    .collect()
            }),
            ..Default::default()
        })
    }

    fn entries(doc: &LoxoneDoc, uuid: &str) -> Vec<(u32, u32, f64)> {
        let t = crate::schedule::read(find(&doc.xml.root, uuid).unwrap(), uuid, |_| None).unwrap();
        let mut v: Vec<_> = t.entries.iter().map(|e| (e.from, e.to, e.value)).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }

    #[test]
    fn switching_times_are_a_schedule_set_lox_writes_onto_the_timer() {
        use crate::sched_edit::builtin_uuid;
        let d = timers();
        let names = BTreeSet::new();
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let claimed = BTreeMap::new();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &claimed,
            moods_claimed: &Default::default(),
        };
        let (mon, sat, hol) = (builtin_uuid(4), builtin_uuid(9), builtin_uuid(1));
        let wk: Vec<(String, Vec<Sp>)> = (4..=8)
            .map(|n| {
                (
                    builtin_uuid(n),
                    vec![(360, 480, Some("21.5")), (1020, 1350, Some("21.5"))],
                )
            })
            .collect();
        let mut cols: Vec<(&str, &[Sp])> =
            wk.iter().map(|(u, v)| (u.as_str(), v.as_slice())).collect();
        let we = [(480, 1380, Some("20"))];
        cols.push((&sat, &we));
        let sun = builtin_uuid(10);
        cols.push((&sun, &we));
        cols.push((&hol, &[]));
        let party = [(1080, 1440, Some("23"))];
        cols.push((PARTY, &party));
        let op = Op::Schedule {
            uuid: UHR.into(),
            value: sched(None, &cols),
        };
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let s = slug(UHR);
        for want in [
            format!("extern {s} = DayTimer(uuid: \"{UHR}\")\n"),
            "schedule vx_uhr_schedule\n\tweekdays 06:00-08:00 = 21.5, 17:00-22:30 = 21.5\n\
             \tweekend 08:00-23:00 = 20\n\tholiday off\n\t\"Party\" 18:00-24:00 = 23\nend\n"
                .to_string(),
            format!("{s}.schedule = vx_uhr_schedule\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        // Read back as written.
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let e = entries(&out, UHR);
        assert!(
            e.contains(&(360, 480, 21.5)) && e.contains(&(1080, 1440, 23.0)),
            "{e:?}"
        );
        assert_eq!(e.len(), 5 * 2 + 2 + 1);
        assert!(crate::sched_edit::missed(&files, &out, &lock, &[UHR.into()], &[]).is_empty());
        assert!(crate::sched_edit::differs(&out, &d, UHR));

        // Changed again: its own set is rewritten, and the previous is what it claimed.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = Op::Schedule {
            uuid: UHR.into(),
            value: sched(None, &[(&mon, &[(0, 60, Some("5"))])]),
        };
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: Option<crate::sched_edit::Sched> =
            serde_json::from_value(a2.previous.clone()).unwrap();
        let prev = prev.unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_uhr_schedule"));
        assert_eq!(prev.modes.as_ref().unwrap()[&mon].len(), 2);
        assert_eq!(prev.modes.as_ref().unwrap()[PARTY][0].to, 1440);
        assert_eq!(files.claims.2.sets.len(), 1);
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Schedule {
                uuid: UHR.into(),
                value: Some(prev)
            }]
        );

        // Heizung shares Uhr's set (Use the same times as …); Licht is digital and may not.
        let share = Op::Schedule {
            uuid: HEIZ.into(),
            value: sched(Some("vx_uhr_schedule"), &[]),
        };
        apply_op(&mut files, &ctx2, &share).unwrap();
        let e = apply_op(
            &mut files.clone(),
            &ctx2,
            &Op::Schedule {
                uuid: LICHT.into(),
                value: sched(Some("vx_uhr_schedule"), &[]),
            },
        )
        .unwrap_err();
        assert!(e.contains("Licht is digital"), "{e}");
        // Heizung on its own: a new set, Uhr's stays.
        apply_op(
            &mut files,
            &ctx2,
            &Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("19"))])]),
            },
        )
        .unwrap();
        assert_eq!(files.claims.2.sets.len(), 2);
        assert_eq!(files.claims.2.claims[&slug(HEIZ)], "vx_heizung_schedule");
        // Both on Uhr's set, rewritten for both.
        apply_op(&mut files, &ctx2, &share).unwrap();
        assert_eq!(
            files.claims.2.sets.len(),
            1,
            "Heizung's own set went with its last claim"
        );
        apply_op(
            &mut files,
            &ctx2,
            &Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(Some("vx_uhr_schedule"), &[(&sat, &[(0, 1440, Some("16"))])]),
            },
        )
        .unwrap();
        let mut l2 = l.clone();
        let out2 = compile_files(&d, &files, &mut l2).unwrap();
        assert_eq!(entries(&out2, UHR), entries(&out2, HEIZ));
        assert_eq!(entries(&out2, UHR), [(0, 1440, 16.0)]);
        assert!(crate::sched_edit::missed(&files, &out2, &lock, &[HEIZ.into()], &[]).is_empty());

        // A digital timer: no values.
        let e = apply_op(
            &mut files.clone(),
            &ctx2,
            &Op::Schedule {
                uuid: LICHT.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("3"))])]),
            },
        )
        .unwrap_err();
        assert!(e.contains("digital"), "{e}");
        apply_op(
            &mut files,
            &ctx2,
            &Op::Schedule {
                uuid: LICHT.into(),
                value: sched(None, &[(&mon, &[(60, 120, None)])]),
            },
        )
        .unwrap();
        assert!(files.render()["edits.loxl"].contains("\tmon 01:00-02:00\n"));

        // Dropped: the timer is as the base has it, and the set and extern go.
        for u in [UHR, HEIZ, LICHT] {
            apply_op(
                &mut files,
                &ctx2,
                &Op::Schedule {
                    uuid: u.into(),
                    value: None,
                },
            )
            .unwrap();
        }
        assert!(files.claims.2.is_empty() && files.claims.0.is_empty());
        let out3 = compile_files(&d, &files, &mut l2).unwrap();
        assert_eq!(entries(&out3, UHR), entries(&d, UHR));

        // Not a DayTimer, a timer the module claims elsewhere, a mode the config lacks.
        assert!(
            apply_op(
                &mut files,
                &ctx,
                &Op::Schedule {
                    uuid: DIV.into(),
                    value: sched(None, &[(&mon, &[])])
                }
            )
            .unwrap_err()
            .contains("only a DayTimer")
        );
        let elsewhere = BTreeMap::from([(UHR.to_string(), "pages/uhr.loxl:4".to_string())]);
        let e = apply_op(
            &mut files,
            &Ctx {
                sched_claimed: &elsewhere,
                ..ctx
            },
            &op,
        )
        .unwrap_err();
        assert!(e.contains("pages/uhr.loxl:4 claims"), "{e}");
        let e = apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[("00000000-0000-0000-0000-000000000000", &[])]),
            },
        )
        .unwrap_err();
        assert!(e.contains("no operating mode"), "{e}");
    }

    /// Rewriting a set other timers share is undone whole: the undo puts
    /// their switching times back too, not only this timer's claim (#73).
    #[test]
    fn undoing_a_rewrite_of_a_shared_set_gives_the_others_their_times_back() {
        use crate::sched_edit::builtin_uuid;
        let d = timers();
        let names = BTreeSet::new();
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let mon = builtin_uuid(4);
        let mut files = Files::default();
        // Uhr on its set; Heizung on one of its own.
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("20"))])]),
            },
        )
        .unwrap();
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(None, &[(&mon, &[(0, 30, Some("17"))])]),
            },
        )
        .unwrap();
        let start = files.clone();
        let uhr_before = entries(&compile_files(&d, &files, &mut l.clone()).unwrap(), UHR);
        for (label, heiz) in [("its own set", true), ("no claim", false)] {
            files = start.clone();
            if !heiz {
                apply_op(
                    &mut files,
                    &ctx,
                    &Op::Schedule {
                        uuid: HEIZ.into(),
                        value: None,
                    },
                )
                .unwrap();
            }
            let was = files.clone();
            // Heizung rewrites Uhr's set: both get 05:00-06:00.
            let op = Op::Schedule {
                uuid: HEIZ.into(),
                value: sched(
                    Some("vx_uhr_schedule"),
                    &[(&mon, &[(300, 360, Some("21"))])],
                ),
            };
            let a = apply_op(&mut files, &ctx, &op).unwrap();
            let out = compile_files(&d, &files, &mut l).unwrap();
            assert_eq!(entries(&out, UHR), [(300, 360, 21.0)], "{label}");
            let undo = undo_ops(
                std::slice::from_ref(&op),
                std::slice::from_ref(&a.previous),
                &[],
            )
            .unwrap();
            let Op::Schedule { value: Some(v), .. } = &undo[0] else {
                panic!("{label}: {undo:?}")
            };
            assert_eq!(
                v.restore[&"vx_uhr_schedule".to_string()][&mon][0].from,
                60,
                "{label}"
            );
            assert_eq!(v.set.is_some(), heiz, "{label}");
            // Undone: Uhr has its times again, Heizung its claim as it was.
            let b = apply_op(&mut files, &ctx, &undo[0]).unwrap();
            assert_eq!(files.claims, was.claims, "{label}");
            let out = compile_files(&d, &files, &mut l).unwrap();
            assert_eq!(entries(&out, UHR), uhr_before, "{label}");
            assert!(
                crate::sched_edit::missed(
                    &files,
                    &out,
                    &lock,
                    &[HEIZ.into()],
                    &["vx_uhr_schedule".into()]
                )
                .is_empty()
            );
            // And the undo's own previous redoes it whole.
            let redo = undo_ops(std::slice::from_ref(&undo[0]), &[b.previous], &[]).unwrap();
            apply_op(&mut files, &ctx, &redo[0]).unwrap();
            let out = compile_files(&d, &files, &mut l).unwrap();
            assert_eq!(entries(&out, UHR), [(300, 360, 21.0)], "{label}");
            assert_eq!(entries(&out, HEIZ), [(300, 360, 21.0)], "{label}");
        }
    }

    /// A timer the module's own file claims too: vislox's claim can still be
    /// dropped (every compile fails while both claim it), not changed (#73).
    #[test]
    fn a_claim_on_a_timer_the_module_claims_too_can_be_dropped() {
        use crate::sched_edit::builtin_uuid;
        let d = timers();
        let names = BTreeSet::new();
        let lock = Lock::default();
        let mut ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: &d,
            deployed: None,
            names: &names,
            drawn: &Default::default(),
            adoptions: &Default::default(),
            page_adoptions: &Default::default(),
            sched_claimed: &Default::default(),
            moods_claimed: &Default::default(),
        };
        let mut files = Files::default();
        let mon = builtin_uuid(4);
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[(&mon, &[(60, 120, Some("20"))])]),
            },
        )
        .unwrap();
        let elsewhere = BTreeMap::from([(UHR.to_string(), "pages/uhr.loxl:4".to_string())]);
        ctx.sched_claimed = &elsewhere;
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: sched(None, &[(&mon, &[])]),
            },
        )
        .unwrap_err();
        assert!(e.contains("pages/uhr.loxl:4 claims"), "{e}");
        apply_op(
            &mut files,
            &ctx,
            &Op::Schedule {
                uuid: UHR.into(),
                value: None,
            },
        )
        .unwrap();
        assert!(files.claims.2.is_empty() && files.claims.0.is_empty());
    }

    #[test]
    fn a_schedule_claim_vislox_did_not_write_is_refused() {
        let s = slug(UHR);
        let ext = format!("extern {s} = DayTimer(uuid: \"{UHR}\")\n");
        let set = "schedule vx_a\n\tmon 01:00-02:00\nend\n";
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        assert!(read(format!("{ext}{set}{s}.schedule = vx_a\n")).is_ok());
        // A set of the module's own name, a claim without its set or its extern.
        assert!(
            read(format!(
                "{ext}schedule abend\n\tmon 01:00-02:00\nend\n{s}.schedule = abend\n"
            ))
            .is_err()
        );
        assert!(read(format!("{ext}{s}.schedule = vx_b\n")).is_err());
        assert!(
            read(format!("{set}{s}.schedule = vx_a\n"))
                .unwrap_err()
                .contains("neither an extern")
        );
    }

    // ── lighting moods (#74) ───────────────────────────────────────────

    fn mrow(name: &str, values: &str) -> crate::mood_edit::MoodRow {
        crate::mood_edit::MoodRow {
            name: name.into(),
            values: values.into(),
        }
    }

    fn moods_op(uuid: &str, set: Option<&str>, rows: &[(&str, &str)]) -> Op {
        Op::Moods {
            uuid: uuid.into(),
            value: Some(crate::mood_edit::MoodsVal {
                set: set.map(Into::into),
                moods: (!rows.is_empty()).then(|| rows.iter().map(|(n, v)| mrow(n, v)).collect()),
                ..Default::default()
            }),
        }
    }

    /// A controller's moods in `doc`: (name, SID, rendered values), off mood included.
    fn moods_in(doc: &LoxoneDoc, uuid: &str) -> Vec<(String, String, String)> {
        let m = loxone_config::moods::Moods::read(find(&doc.xml.root, uuid).unwrap()).unwrap();
        m.moods
            .iter()
            .map(|x| {
                (
                    x.name.clone(),
                    x.sid.clone().unwrap_or_default(),
                    loxone_config::moods::render(&x.qs),
                )
            })
            .collect()
    }

    fn mood_ctx<'a>(
        d: &'a LoxoneDoc,
        lock: &'a Lock,
        names: &'a BTreeSet<String>,
        claimed: &'a BTreeMap<String, String>,
    ) -> Ctx<'a> {
        Ctx {
            doc: d,
            lock,
            base: d,
            deployed: None,
            names,
            drawn: &EMPTY_WIRES,
            adoptions: &EMPTY_ADOPT,
            page_adoptions: &EMPTY_PAGES,
            sched_claimed: &EMPTY_CLAIMS,
            moods_claimed: claimed,
        }
    }

    static EMPTY_WIRES: BTreeSet<Wire> = BTreeSet::new();
    static EMPTY_ADOPT: BTreeMap<String, crate::adopt::Adoption> = BTreeMap::new();
    static EMPTY_PAGES: BTreeMap<String, Vec<String>> = BTreeMap::new();
    static EMPTY_CLAIMS: BTreeMap<String, String> = BTreeMap::new();

    #[test]
    fn moods_are_a_set_lox_writes_onto_the_controller() {
        use crate::mood_edit::fixture::{FLUR, LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        // "Kino" deleted, "Neu" added, "Lesen" first.
        let op = moods_op(
            LICHT,
            None,
            &[
                ("Lesen", "AQ1 on, AQ2 50%"),
                ("Viel Licht", "all on"),
                ("Neu", "AQ3 temp(80%, 3000K)"),
            ],
        );
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = &files.render()["edits.loxl"];
        let s = slug(LICHT);
        for want in [
            format!("extern {s} = LightController2(uuid: \"{LICHT}\")\n"),
            "moods vx_licht_moods\n\t\"Lesen\": AQ1 on, AQ2 50%\n\t\"Viel Licht\": all on\n\t\"Neu\": AQ3 temp(80%, 3000K)\nend\n"
                .to_string(),
            format!("{s}.moods = vx_licht_moods\n"),
        ] {
            assert!(text.contains(&want), "{want} in\n{text}");
        }
        assert_eq!(
            Files::from_texts(&files.render()).unwrap().claims,
            files.claims
        );
        let out = compile_files(&d, &files, &mut l).unwrap();
        let got = moods_in(&out, LICHT);
        // The off mood stays last; Lesen keeps its scene id, and Neu takes the
        // lowest free one: Kino's, which a button on Sel2 would now select.
        assert_eq!(
            got.iter()
                .map(|m| (m.0.as_str(), m.1.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Lesen", "1"),
                ("Viel Licht", "777"),
                ("Neu", "2"),
                ("Aus", "778")
            ]
        );
        assert_eq!(got[0].2, "AQ1 on, AQ2 50%");
        // `all on` fits switches only: the dimmer and the white circuit stay off.
        assert_eq!(got[1].2, "all on, AQ2 off, AQ3 off");
        assert!(crate::mood_edit::missed(&files, &out, &lock, &[LICHT.into()], &[]).is_empty());
        assert!(crate::mood_edit::missed(&files, &d, &lock, &[LICHT.into()], &[]) == ["Licht"]);
        assert!(crate::mood_edit::differs(&out, &d, LICHT));
        assert!(!crate::mood_edit::differs(&out, &d, FLUR));

        // Changed again: its own set is rewritten; previous is what it claimed.
        let ctx2 = Ctx { doc: &out, ..ctx };
        let op2 = moods_op(LICHT, None, &[("Viel Licht", "all off")]);
        let a2 = apply_op(&mut files, &ctx2, &op2).unwrap();
        let prev: crate::mood_edit::MoodsVal =
            serde_json::from_value::<Option<_>>(a2.previous.clone())
                .unwrap()
                .unwrap();
        assert_eq!(prev.set.as_deref(), Some("vx_licht_moods"));
        assert_eq!(prev.moods.as_ref().unwrap().len(), 3);
        assert_eq!(files.claims.3.sets.len(), 1);
        let undo = undo_ops(std::slice::from_ref(&op2), &[a2.previous], &[]).unwrap();
        assert_eq!(
            undo,
            [Op::Moods {
                uuid: LICHT.into(),
                value: Some(prev)
            }]
        );
        // Dropped: the controller is as the base has it; the set and extern go.
        apply_op(
            &mut files,
            &ctx2,
            &Op::Moods {
                uuid: LICHT.into(),
                value: None,
            },
        )
        .unwrap();
        assert!(files.claims.3.is_empty() && files.claims.0.is_empty());
        let out3 = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out3, LICHT), moods_in(&d, LICHT));
    }

    #[test]
    fn moods_lox_would_refuse_are_refused_first() {
        use crate::mood_edit::fixture::{BAD, LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock = Lock::default();
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let err = |op: Op| apply_op(&mut Files::default(), &ctx, &op).unwrap_err();
        // "Viel Licht" is built in: it stays (a rename is a delete).
        let e = err(moods_op(LICHT, None, &[("Lesen", "all on")]));
        assert!(e.contains("\"Viel Licht\" is a built-in mood"), "{e}");
        let e = err(moods_op(
            LICHT,
            None,
            &[("Bright", "all on"), ("Lesen", "")],
        ));
        assert!(e.contains("cannot be deleted or renamed"), "{e}");
        // The off mood is never in the set.
        let e = err(moods_op(
            LICHT,
            None,
            &[("Viel Licht", "all on"), ("Aus", "")],
        ));
        assert!(e.contains("off mood"), "{e}");
        // A value the circuit cannot take: a colour on a switch, on on a dimmer.
        let e = err(moods_op(LICHT, None, &[("Viel Licht", "AQ1 rgb(1, 2, 3)")]));
        assert!(
            e.contains("AQ1 is") && e.contains("switch or dimmer"),
            "{e}"
        );
        let e = err(moods_op(BAD, None, &[("Viel Licht", "AQ1 on")]));
        assert!(
            e.starts_with("Bad: ") && e.contains("dimmer circuit"),
            "{e}"
        );
        let e = err(moods_op(LICHT, None, &[("Viel Licht", "AQ19 on")]));
        assert!(e.contains("18 circuits"), "{e}");
        let e = err(moods_op(LICHT, None, &[("Viel Licht", "AQ1 bright")]));
        assert!(e.contains("mood \"Viel Licht\""), "{e}");
        // `all on` sets only the circuits it fits: fine on a dimmer.
        apply_op(
            &mut Files::default(),
            &ctx,
            &moods_op(BAD, None, &[("Viel Licht", "all on")]),
        )
        .unwrap();
        // Not a controller; one the module claims.
        assert!(err(moods_op(DIV, None, &[("A", "")])).contains("only a lighting controller"));
        let elsewhere = BTreeMap::from([(LICHT.to_string(), "pages/licht.loxl:4".to_string())]);
        let e = apply_op(
            &mut Files::default(),
            &mood_ctx(&d, &lock, &names, &elsewhere),
            &moods_op(LICHT, None, &[("Viel Licht", "all on")]),
        )
        .unwrap_err();
        assert!(e.contains("pages/licht.loxl:4 claims its moods"), "{e}");
        // Dropping vislox's claim on it is always let through.
        apply_op(
            &mut Files::default(),
            &mood_ctx(&d, &lock, &names, &elsewhere),
            &Op::Moods {
                uuid: LICHT.into(),
                value: None,
            },
        )
        .unwrap();
    }

    /// Rewriting a set other controllers share is undone whole, and a
    /// controller whose circuits do not take the set is refused (#74).
    #[test]
    fn a_shared_mood_set_is_undone_whole_and_checked_for_every_controller() {
        use crate::mood_edit::fixture::{BAD, FLUR, LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let mut l = Lockfile::default();
        let lock = lock_of(&l);
        let ctx = mood_ctx(&d, &lock, &names, &claimed);
        let mut files = Files::default();
        apply_op(
            &mut files,
            &ctx,
            &moods_op(
                LICHT,
                None,
                &[("Viel Licht", "all on"), ("Lesen", "AQ1 on")],
            ),
        )
        .unwrap();
        // Flur takes the same set; Bad cannot (AQ1 on does not fit its dimmer).
        apply_op(
            &mut files,
            &ctx,
            &moods_op(FLUR, Some("vx_licht_moods"), &[]),
        )
        .unwrap();
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &moods_op(BAD, Some("vx_licht_moods"), &[]),
        )
        .unwrap_err();
        assert!(e.contains("Bad") && e.contains("dimmer circuit"), "{e}");
        let was = files.clone();
        let licht_before = moods_in(&compile_files(&d, &files, &mut l.clone()).unwrap(), LICHT);
        // Flur rewrites the shared set: both change; the undo gives Licht its moods back.
        let op = moods_op(FLUR, Some("vx_licht_moods"), &[("Viel Licht", "AQ2 40%")]);
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out, LICHT)[0].2, "AQ2 40%");
        assert_eq!(moods_in(&out, FLUR)[0].2, "AQ2 40%");
        let undo = undo_ops(
            std::slice::from_ref(&op),
            std::slice::from_ref(&a.previous),
            &[],
        )
        .unwrap();
        let Op::Moods { value: Some(v), .. } = &undo[0] else {
            panic!("{undo:?}")
        };
        assert_eq!(v.set.as_deref(), Some("vx_licht_moods"));
        let b = apply_op(&mut files, &ctx, &undo[0]).unwrap();
        assert_eq!(files.claims, was.claims);
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out, LICHT), licht_before);
        // Flur on a set of its own: Licht's stays; and the redo rewrites it whole again.
        let redo = undo_ops(std::slice::from_ref(&undo[0]), &[b.previous], &[]).unwrap();
        apply_op(&mut files, &ctx, &redo[0]).unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        assert_eq!(moods_in(&out, LICHT)[0].2, "AQ2 40%");
        apply_op(
            &mut files,
            &ctx,
            &moods_op(FLUR, None, &[("Viel Licht", "all off")]),
        )
        .unwrap();
        assert_eq!(files.claims.3.sets.len(), 2);
        assert_eq!(files.claims.3.claims[&slug(FLUR)], "vx_flur_moods");
        // A mood set and a schedule set share one namespace.
        assert!(crate::sched_edit::taken(&files, &ctx, "vx_flur_moods"));
    }

    #[test]
    fn a_moods_claim_vislox_did_not_write_is_refused() {
        use crate::mood_edit::fixture::LICHT;
        let s = slug(LICHT);
        let ext = format!("extern {s} = LightController2(uuid: \"{LICHT}\")\n");
        let set = "moods vx_a\n\t\"Viel Licht\": all on\nend\n";
        let read = |t: String| Files::from_texts(&BTreeMap::from([("edits.loxl".to_string(), t)]));
        assert!(read(format!("{ext}{set}{s}.moods = vx_a\n")).is_ok());
        assert!(
            read(format!(
                "{ext}moods wohnen\n\t\"A\": all on\nend\n{s}.moods = wohnen\n"
            ))
            .is_err()
        );
        assert!(read(format!("{ext}{s}.moods = vx_b\n")).is_err());
        assert!(
            read(format!("{set}{s}.moods = vx_a\n"))
                .unwrap_err()
                .contains("neither an extern")
        );
    }

    /// `Circuit<n>:` on a controller vislox added: switch or dimmer, never on
    /// a colour circuit (#74).
    #[test]
    fn a_circuit_is_declared_as_switch_or_dimmer_on_a_managed_controller() {
        use crate::mood_edit::{circuit_setting, fixture::LICHT};
        let d = LoxoneDoc::parse(crate::mood_edit::fixture::house().as_bytes()).unwrap();
        let el = find(&d.xml.root, LICHT).unwrap();
        assert!(circuit_setting(el, "Circuit2", "dimmer").is_ok());
        assert!(
            circuit_setting(el, "Circuit3", "dimmer")
                .unwrap_err()
                .contains("colour circuit")
        );
        assert!(
            circuit_setting(el, "Circuit1", "rgb")
                .unwrap_err()
                .contains("\"switch\" or \"dimmer\"")
        );
        assert!(circuit_setting(el, "Circuit6", "switch").is_err());
        // Written into the declaration of a controller added here.
        let mut files = Files::default();
        let mut l = Lockfile::default();
        let (names, claimed) = (BTreeSet::new(), BTreeMap::new());
        let lock0 = lock_of(&l);
        let ctx = mood_ctx(&d, &lock0, &names, &claimed);
        apply_op(
            &mut files,
            &ctx,
            &add(OG, "LightController2", Some("Neu"), &[], &[]),
        )
        .unwrap();
        let out = compile_files(&d, &files, &mut l).unwrap();
        let lock = lock_of(&l);
        let (slug_new, uuid) = l
            .objects
            .iter()
            .find(|(s, _)| s.starts_with("vx_lightcontroller2"))
            .map(|(s, o)| (s.clone(), o.uuid.clone()))
            .unwrap();
        let ctx = Ctx {
            doc: &out,
            ..mood_ctx(&d, &lock, &names, &claimed)
        };
        let op = Op::Setting {
            uuid: uuid.clone(),
            name: "Circuit1".into(),
            value: Some("dimmer".into()),
        };
        let a = apply_op(&mut files, &ctx, &op).unwrap();
        assert_eq!(a.previous, serde_json::Value::Null);
        let text = files
            .render()
            .into_iter()
            .find(|(k, _)| k != "edits.loxl")
            .unwrap()
            .1;
        assert!(
            text.contains(&format!(
                "{slug_new} = LightController2(\"Neu\", Circuit1: \"dimmer\")"
            )),
            "{text}"
        );
        let e = apply_op(
            &mut files.clone(),
            &ctx,
            &Op::Setting {
                uuid,
                name: "Circuit1".into(),
                value: Some("rgb".into()),
            },
        )
        .unwrap_err();
        assert!(e.contains("\"switch\" or \"dimmer\""), "{e}");
    }
}
