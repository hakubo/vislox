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

use loxone_config::connectors::{self, PortDir, PortUnit};
use loxone_config::logic::{ArgItem, Attr, BindingKind, Item, MatchSpec, Module, Value};
use loxone_config::port_names;
use loxone_config::xml::Element;
use loxone_config::{LoxoneDoc, logic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// vislox's own file in the module directory. The rest of the module is
/// never touched. Later edits (new blocks, #59) get their own files next to it.
pub const FILE: &str = "vislox/edits.loxl";

/// The slugs of [`FILE`] start with this; nothing else in a module should.
const SLUG_PREFIX: &str = "vx_";

/// One edit. `value: None` drops vislox's claim on the value: lox puts
/// back what Loxone Config had before the claim (see the module comment).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// The object's name (`Title=`).
    Title { uuid: String, value: Option<String> },
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
}

impl Op {
    pub fn uuid(&self) -> &str {
        match self {
            Op::Title { uuid, .. } | Op::Param { uuid, .. } | Op::Setting { uuid, .. } => uuid,
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

fn unit_suffix(u: PortUnit) -> String {
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
    let caps = Caps {
        title: el.attr("Title").is_some(),
        params,
        settings,
    };
    (caps.title || !caps.params.is_empty() || !caps.settings.is_empty()).then_some(caps)
}

// ── the claims file ─────────────────────────────────────────────────────

/// What [`FILE`] claims, by object UUID.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claims(pub BTreeMap<String, Claim>);

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Claim {
    /// The object's type, as the extern declares it.
    pub ty: String,
    pub title: Option<String>,
    /// Port (by its Lox Logic name) or setting → value.
    pub sets: BTreeMap<String, Value>,
}

impl Claim {
    fn is_empty(&self) -> bool {
        self.title.is_none() && self.sets.is_empty()
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
                Item::Attr(a) if a.attr == Attr::Title => {
                    let uuid = by_slug
                        .get(&a.slug)
                        .ok_or_else(|| foreign(&format!("`{}.title`", a.slug)))?;
                    c.0.get_mut(uuid).expect("declared").title = Some(plain(&a.value));
                }
                Item::Gap | Item::FragmentEnd | Item::Comment(_) => {}
                Item::Attr(a) => return Err(foreign(&format!("`{a}`"))),
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
            if let Some(t) = &c.title {
                out.push_str(&format!("{s}.title = {}\n", Value::Str(t.clone())));
            }
            for (k, v) in &c.sets {
                out.push_str(&format!("{s}.{k} = {v}\n"));
            }
        }
        out
    }

    /// The value claimed for what `op` changes.
    pub fn get(&self, ty: &str, op: &Op) -> Option<String> {
        let c = self.0.get(op.uuid())?;
        match op {
            Op::Title { .. } => c.title.clone(),
            Op::Param { port, .. } => c.sets.get(port_names::name_of(ty, port)).map(plain),
            Op::Setting { name, .. } => c.sets.get(name).map(plain),
        }
    }

    /// Every claimed value, as the ops that set it.
    pub fn ops(&self) -> Vec<Op> {
        let mut v = Vec::new();
        for (uuid, c) in &self.0 {
            if let Some(t) = &c.title {
                v.push(Op::Title {
                    uuid: uuid.clone(),
                    value: Some(t.clone()),
                });
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
    let uuid = op.uuid();
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
    let (key, value): (Option<String>, Option<Value>) = match op {
        Op::Title { value, .. } => {
            let v = match value {
                Some(v) if caps.title => Some(Value::Str(text(v, "The name", false)?)),
                Some(_) => return Err(format!("{name}: the name cannot be edited here")),
                None => None,
            };
            (None, v)
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
            (Some(pname), v)
        }
        Op::Setting {
            name: setting,
            value,
            ..
        } => {
            let v = match (value, caps.settings.iter().find(|s| &s.name == setting)) {
                (Some(v), Some(cap)) if cap.text => Some(Value::Str(text(
                    v,
                    setting,
                    setting == "Installation" || setting.starts_with("SwitchBoard"),
                )?)),
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
            (Some(setting.clone()), v)
        }
    };
    let claim = claims.0.entry(uuid.to_string()).or_default();
    claim.ty = ty;
    match (key, value) {
        (None, v) => claim.title = v.map(|v| plain(&v)),
        (Some(k), Some(v)) => {
            claim.sets.insert(k, v);
        }
        (Some(k), None) => {
            claim.sets.remove(&k);
        }
    }
    if claim.is_empty() {
        claims.0.remove(uuid);
    }
    Ok(previous)
}

/// Check the whole module, with [`FILE`] as written, the way `lox logic
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

/// The UUIDs of the blocks the module manages, from its lockfile.
pub fn managed(lock: &Path) -> HashSet<String> {
    let Ok(bytes) = std::fs::read(lock) else {
        return HashSet::new();
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return HashSet::new();
    };
    v.get("objects")
        .and_then(|o| o.as_object())
        .map(|o| {
            o.values()
                .filter_map(|x| x.get("uuid")?.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
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

    #[test]
    fn managed_blocks_come_from_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = tmp.path().join("logic.lock.json");
        std::fs::write(&lock, r#"{"objects":{"a":{"uuid":"U1","type":"And"}}}"#).unwrap();
        assert_eq!(managed(&lock), HashSet::from(["U1".to_string()]));
        assert!(managed(&tmp.path().join("none")).is_empty());
    }
}
