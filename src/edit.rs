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
use loxone_config::logic::{ArgItem, Attr, BindingKind, Item, MatchSpec, Module, Value};
use loxone_config::port_names;
use loxone_config::xml::Element;
use loxone_config::{LoxoneDoc, logic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
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
    },
    /// Take out a block added here that is not deployed yet. Its `previous`
    /// is the `add_block` that adds it again.
    RemoveBlock { uuid: String },
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
            | Op::RemoveBlock { uuid } => Some(uuid),
            Op::AddBlock { .. } => None,
        }
    }

    /// The value it sets (`None` also for a block op).
    pub fn value(&self) -> Option<&str> {
        match self {
            Op::Title { value, .. }
            | Op::Room { value, .. }
            | Op::Category { value, .. }
            | Op::Param { value, .. }
            | Op::Setting { value, .. } => value.as_deref(),
            Op::AddBlock { .. } | Op::RemoveBlock { .. } => None,
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
            Op::AddBlock { .. } | Op::RemoveBlock { .. } => {}
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

/// What [`FILE`] claims, by object UUID.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claims(pub BTreeMap<String, Claim>);

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claim {
    /// The object's type, as the extern declares it.
    pub ty: String,
    /// Its title, room and category.
    pub attrs: BTreeMap<Which, String>,
    /// Port (by its Lox Logic name) or setting → value.
    pub sets: BTreeMap<String, Value>,
}

impl Claim {
    fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.sets.is_empty()
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
        let m = Module::parse(text).map_err(|e| e.to_string())?;
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
                                _ => return Err(foreign(&format!("a wire onto `{}`", e.slug))),
                            },
                            _ => return Err(foreign(&format!("`{}`'s arguments", e.slug))),
                        }
                    }
                }
                Item::Set(s) => set(&mut c, &by_slug, &s.target.slug, &s.target.port, s.value)?,
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
                Item::Gap | Item::FragmentEnd | Item::Comment(_) => {}
                _ => return Err(foreign("a statement")),
            }
        }
        Ok(c)
    }

    /// The file's text: a header, then each object's extern and its values.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# Values edited in vislox (Properties). vislox rewrites this file on every edit;\n\
             # see docs/editing.md in vislox. Deleting a line hands that value back to Loxone Config.\n",
        );
        for (uuid, c) in &self.0 {
            if c.is_empty() {
                continue;
            }
            let s = slug(uuid);
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
            Op::AddBlock { .. } | Op::RemoveBlock { .. } => None,
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
fn number(s: &str) -> Result<String, String> {
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
        .filter(|_| !matches!(op, Op::RemoveBlock { .. }))
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
        Op::AddBlock { .. } | Op::RemoveBlock { .. } => unreachable!("not a value"),
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
    if claim.is_empty() {
        claims.0.remove(uuid);
    }
    Ok(previous)
}

// ── vislox's files, and the blocks it adds ──────────────────────────────

/// What the lockfile says about managed blocks: slug → UUID, and the slugs
/// of its removal tombstones (not to be picked again).
#[derive(Default, Clone, Debug)]
pub struct Lock {
    pub slugs: BTreeMap<String, String>,
    pub tombstones: BTreeSet<String>,
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
        Lock { slugs, tombstones }
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
    /// The files there were: they stay, empty or not (a module directory
    /// with no `.loxl` file is an error to lox).
    pub present: BTreeSet<String>,
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

    /// Each file's text, by name; a new file with nothing in it is left out.
    pub fn render(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if !self.claims.0.values().all(Claim::is_empty) || self.present.contains(CLAIMS) {
            out.insert(CLAIMS.to_string(), self.claims.render());
        }
        for (name, p) in &self.pages {
            if !p.is_empty() || self.present.contains(name) {
                out.insert(name.clone(), p.render());
            }
        }
        out
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
    fn names(&self, slug: &str) -> bool {
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
                for (name, val) in &b.args {
                    let key = blocks::port_key(&b.ty, name);
                    if blocks::port_name(&b.ty, key).is_some() {
                        v.push(Op::Param {
                            uuid: uuid.into(),
                            port: key.to_string(),
                            value: Some(plain(val)),
                        });
                    }
                }
            }
        }
        v
    }
}

/// What an op is checked against.
pub struct Ctx<'a> {
    /// The pending config, which the browser showed.
    pub doc: &'a LoxoneDoc,
    pub lock: &'a Lock,
    /// What the Miniserver runs (the project's base), to tell whether a
    /// block added here is deployed. Only read for `remove_block`.
    pub base: Option<&'a LoxoneDoc>,
    /// The names the module's other files declare.
    pub names: &'a BTreeSet<String>,
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
    } = op
    else {
        unreachable!()
    };
    let pg = find(&ctx.doc.xml.root, page)
        .filter(|e| e.attr("Type") == Some("Page"))
        .ok_or_else(|| format!("no page {page} in the pending config"))?;
    let page_title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
    if page_title.trim().is_empty() {
        return Err("the page has no title; name it in Loxone Config first".into());
    }
    // `page "…"` names a page by its title, and lox builds on the first.
    let mut same = Vec::new();
    pages_titled(&ctx.doc.xml.root, &page_title, &mut same);
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
    let file = match files.pages.iter().find(|(_, p)| p.page == page_title) {
        Some((n, _)) => n.clone(),
        None => blocks::file_name(&page_title, &|n| n == CLAIMS || files.pages.contains_key(n)),
    };
    files
        .pages
        .entry(file)
        .or_insert_with(|| PageFile::new(&page_title))
        .blocks
        .push(NewBlock {
            slug: slug.clone(),
            ty: ty.clone(),
            title,
            args,
        });
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
        Some(s) if files.block(s).is_some() => s.to_string(),
        Some(_) => {
            return Err(format!(
                "{name}: the module manages this block; delete it in its .loxl file"
            ));
        }
        None => return Err(format!("{name}: only a block added here can be taken out")),
    };
    let base = ctx
        .base
        .ok_or_else(|| "cannot read the deployed config".to_string())?;
    if find(&base.xml.root, uuid).is_some() {
        return Err(format!(
            "{name} is deployed: vislox takes out only a block added here and not applied yet"
        ));
    }
    let (file, b) = files.block(&slug).expect("checked");
    let (file, b) = (file.to_string(), b.clone());
    let page = files.pages[&file].page.clone();
    let mut same = Vec::new();
    pages_titled(&ctx.doc.xml.root, &page, &mut same);
    let (mut params, mut settings) = (BTreeMap::new(), BTreeMap::new());
    for (k, v) in &b.args {
        let key = blocks::port_key(&b.ty, k);
        if blocks::port_name(&b.ty, key).is_some() {
            params.insert(key.to_string(), plain(v));
        } else {
            settings.insert(k.clone(), plain(v));
        }
    }
    let again = Op::AddBlock {
        page: same
            .first()
            .and_then(|e| e.attr("U"))
            .unwrap_or_default()
            .to_string(),
        ty: b.ty.clone(),
        title: b.title.clone(),
        params,
        settings,
    };
    let p = files.pages.get_mut(&file).expect("found");
    p.blocks.retain(|x| x.slug != slug);
    p.removed.insert(slug);
    Ok(Applied {
        previous: serde_json::to_value(&again).unwrap_or_default(),
        added: None,
    })
}

/// A value of a block vislox added: its name, or an unwired input's constant.
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
        _ => Err(format!(
            "{name}: on a block added here, only its name and its inputs' constants are edited"
        )),
    }
}

/// The names the module declares (blocks, externs, signals, instances,
/// constants), so that a new slug is none of them. Empty when the module
/// does not read; then the check after writing says why.
pub fn module_names(module: &Path) -> BTreeSet<String> {
    let Some(m) = module
        .to_str()
        .and_then(|p| loxone_config::cli::read_module(p).ok())
    else {
        return BTreeSet::new();
    };
    m.items
        .iter()
        .filter_map(|i| match i {
            Item::Block(b) | Item::Instance(b) => Some(b.slug.clone()),
            Item::Extern(e) => Some(e.slug.clone()),
            Item::Signal(s) => Some(s.slug.clone()),
            Item::Let(l) => Some(l.name.clone()),
            _ => None,
        })
        .collect()
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
        Op::Setting { name, .. } => decoded(el, name),
        Op::AddBlock { .. } | Op::RemoveBlock { .. } => None,
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
        }
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
                base: None,
                names: &names,
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
            base: None,
            names: &names,
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
            base: Some(&d),
            names: &names,
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
        assert!(files.claims.0.is_empty());
        assert_eq!(files.ops(&lock).len(), 2);
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

        // Taken out: `removed` tells lox, and the op that adds it again comes back.
        let lock = lock_of(&l);
        let ctx = Ctx {
            doc: &out,
            lock: &lock,
            base: Some(&d),
            names: &names,
        };
        let r = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: mono.clone() }).unwrap();
        assert_eq!(
            serde_json::from_value::<Op>(r.previous).unwrap(),
            add(OG, "Monoflop", Some("Treppe"), &[("Time", "45")], &[])
        );
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
            base: Some(&out),
            names: &names,
        };
        let e = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: pulse }).unwrap_err();
        assert!(e.contains("is deployed"), "{e}");
        let e = apply_op(&mut files, &ctx, &Op::RemoveBlock { uuid: DIV.into() }).unwrap_err();
        assert!(e.contains("only a block added here"), "{e}");
    }

    #[test]
    fn a_new_block_is_checked_before_lox_sees_it() {
        let d = doc("house.Loxone");
        let names = BTreeSet::from(["vx_and_1".to_string()]);
        let lock = Lock::default();
        let ctx = Ctx {
            doc: &d,
            lock: &lock,
            base: None,
            names: &names,
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
            base: None,
            names: &names,
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
        assert!(l.tombstones.contains("b"));
        assert!(Lock::read(&tmp.path().join("none")).slugs.is_empty());
    }
}
