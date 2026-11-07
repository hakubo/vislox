//! Objects of the object tree from vislox (#84): virtual inputs, virtual
//! HTTP, UDP and text inputs and their commands, virtual outputs and their
//! commands, Modbus servers, their devices and those devices' sensors and
//! actuators. The design is in `docs/editing.md`.
//!
//! Such an object is a managed block like one added on a page (#59), with
//! no position and no page: lox appends a device to the base's one folder
//! of its type (*Virtual Inputs*, *Virtual Outputs*, *Netzwerkperipherie*)
//! and an I/O to the device its `device:` names, and mints its `IName=`
//! (`VI4`, `VHI1`, …). vislox declares them in a file of its own,
//! [`FILE`], with no `page` line:
//!
//! ```text
//! vx_virtualhttpin_1 = VirtualHttpIn("Zähler", Address: "http://192.0.2.10/status", Poll: 60)
//! vx_virtualhttpincmd_1 = VirtualHttpInCmd("Leistung", device: vx_virtualhttpin_1, Check: "\"power\":\\v")
//! vx_modbusdev_1 = ModbusDev("Regler", device: vx_10000000_0000_0005_ffff504f94000001)
//! ```
//!
//! An I/O below a device Loxone Config made names vislox's extern of it
//! (`vx_<uuid>`, in `edits.loxl`). lox keeps the table of these types to
//! itself (`periphery::PERIPHERY`); [`TYPES`] is the part vislox needs, and
//! a test holds it to lox's public tables.

use crate::edit::{Ctx, Files, extern_key, find_ci, slug};
use loxone_config::connectors;
use loxone_config::logic::Value;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

/// vislox's file of the objects of the object tree, in its directory. A
/// page file is named after its page's title in letters, digits and dashes
/// ([`crate::blocks::file_name`]), so never this.
pub const FILE: &str = "_periphery.loxl";

/// The argument that names the device an I/O hangs below (`device:
/// vx_virtualhttpin_1`): a name, never a wire.
pub const DEVICE: &str = "device";

/// Where an object of a type lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Home {
    /// A device: the base's one folder of this type.
    Folder(&'static str),
    /// An I/O: below an object of one of these types (`device:`).
    Device(&'static [&'static str]),
}

/// One type vislox adds to the object tree: where it lives, the settings a
/// new one cannot do without (lox refuses it without them), and its
/// settings that take text (an address, a command, a check pattern, a text
/// input's value, the name the app shows). Its other settings are numbers
/// (in the unit lox documents, if any) or `on`/`off`.
pub struct Type {
    pub ty: &'static str,
    pub home: Home,
    pub required: &'static [&'static str],
    pub text: &'static [&'static str],
}

/// The types, as lox's table has them (`lox logic types`).
pub const TYPES: &[Type] = &[
    // An operating mode (#86): only its name; lox mints its `Mode=` number.
    Type {
        ty: "Mode",
        home: Home::Folder("ModeCaption"),
        required: &[],
        text: &[],
    },
    // A calendar entry (#86): its mode and date are no settings but two
    // values of their own ([`crate::calendar_edit`]).
    Type {
        ty: "CalendarEntry",
        home: Home::Folder("CalendarCaption"),
        required: &[],
        text: &[],
    },
    Type {
        ty: "VirtualIn",
        home: Home::Folder("VirtualInCaption"),
        required: &[],
        text: &["Desc"],
    },
    Type {
        ty: "VirtualHttpIn",
        home: Home::Folder("VirtualInCaption"),
        required: &["Address", "Poll"],
        text: &["Address"],
    },
    Type {
        ty: "VirtualHttpInCmd",
        home: Home::Device(&["VirtualHttpIn"]),
        required: &["Check"],
        text: &["Check", "Desc"],
    },
    Type {
        ty: "VirtualUdpIn",
        home: Home::Folder("VirtualInCaption"),
        required: &["Port"],
        text: &["Address"],
    },
    Type {
        ty: "VirtualUdpInCmd",
        home: Home::Device(&["VirtualUdpIn"]),
        required: &["Check"],
        text: &["Check", "Desc"],
    },
    Type {
        ty: "VirtualTextIn",
        home: Home::Folder("VirtualInCaption"),
        required: &[],
        text: &["Def"],
    },
    Type {
        ty: "VirtualOut",
        home: Home::Folder("VirtualOutCaption"),
        required: &["Address"],
        text: &["Address", "CmdSep"],
    },
    Type {
        ty: "VirtualOutCmd",
        home: Home::Device(&["VirtualOut"]),
        required: &[],
        text: &[
            "CmdOn",
            "CmdOnHTTP",
            "CmdOnPost",
            "CmdOff",
            "CmdAnswer",
            "Desc",
        ],
    },
    Type {
        ty: "ModbusServer",
        home: Home::Folder("WeatherCaption"),
        required: &["Address"],
        text: &["Address"],
    },
    Type {
        ty: "ModbusDev",
        home: Home::Device(&["ModbusServer"]),
        required: &[],
        text: &["Desc"],
    },
    Type {
        ty: "ModbusASensor",
        home: Home::Device(&["ModbusDev"]),
        required: &["ModbusAddress"],
        text: &["Desc"],
    },
    Type {
        ty: "ModbusSensor",
        home: Home::Device(&["ModbusDev"]),
        required: &["ModbusAddress"],
        text: &[],
    },
    Type {
        ty: "ModbusAActor",
        home: Home::Device(&["ModbusDev"]),
        required: &["ModbusAddress"],
        text: &["Desc"],
    },
];

/// The most text one setting takes: a command or a check pattern may be a
/// long URL, but stays one line.
const MAX_TEXT: usize = 1000;

/// The type `ty`, if vislox adds it to the object tree.
pub fn of(ty: &str) -> Option<&'static Type> {
    TYPES.iter().find(|t| t.ty == ty)
}

/// Whether a setting of `ty` takes text.
pub fn text_setting(ty: &str, name: &str) -> bool {
    of(ty).is_some_and(|t| t.text.contains(&name))
}

/// The types added under an object of type `parent_ty`: a folder or a
/// device (the browser works it out from the palette the same way).
#[cfg(test)]
fn added_under(parent_ty: &str) -> Vec<&'static str> {
    TYPES
        .iter()
        .filter(|t| match t.home {
            Home::Folder(f) => f == parent_ty,
            Home::Device(ds) => ds.contains(&parent_ty),
        })
        .map(|t| t.ty)
        .collect()
}

/// A number for setting `name` of `ty`: with a unit only where lox
/// documents one for it (`Poll: 1min`), plain elsewhere (`Port`,
/// `ModbusAddress`): lox would read `70%` there as 70.
pub fn number_value(ty: &str, name: &str, v: &str) -> Result<Value, String> {
    if connectors::port_unit(ty, name).is_none() {
        return crate::edit::number(v)
            .map(Value::Number)
            .map_err(|e| format!("{name}: {e}; it takes a plain number, no unit"));
    }
    crate::units::port_value(ty, name, v).map_err(|e| format!("{name}: {e}"))
}

/// A setting's text value: one line, not empty, not too long.
pub fn text_value(name: &str, v: &str) -> Result<String, String> {
    if v.chars().any(|c| c.is_control()) {
        return Err(format!("{name} must be one line of text"));
    }
    if v.trim().is_empty() {
        return Err(format!(
            "{name} cannot be empty; leave it out to keep what Loxone Config writes"
        ));
    }
    if v.chars().count() > MAX_TEXT {
        return Err(format!("{name} is longer than {MAX_TEXT} characters"));
    }
    Ok(v.to_string())
}

/// What a Loxone Config save would make of the settings `args` of an object
/// of type `ty`: lox refuses a value a save rewrites (a fraction of a
/// second on an HTTP input's `Poll`, a digital virtual input's range
/// other than 0 to 1). `None` when a save keeps them all.
pub fn rewritten(ty: &str, args: &[(String, Value)]) -> Option<String> {
    let digital = !matches!(
        args.iter().find(|(k, _)| k == "Analog").map(|(_, v)| v),
        Some(Value::Bool(true))
    );
    for (k, v) in args {
        // A time with its unit is checked in the attribute's own unit
        // (`Poll: 5s` is 5 seconds), as lox checks it.
        let Some(raw) = crate::units::in_own_unit(ty, k, v) else {
            continue;
        };
        if let Some((w, rule)) = connectors::save_rewrite(ty, k, &raw, digital) {
            let unit = connectors::port_unit(ty, k)
                .map(|u| format!(" {}", crate::edit::unit_suffix(u)))
                .unwrap_or_default();
            return Some(format!(
                "{k}: {rule}, so a Loxone Config save would write {w}{unit}; write that instead"
            ));
        }
    }
    None
}

/// The settings a new object of `ty` cannot do without, missing from
/// `args`.
pub fn missing(ty: &str, args: &[(String, Value)]) -> Vec<&'static str> {
    of(ty)
        .map(|t| {
            t.required
                .iter()
                .copied()
                .filter(|r| !args.iter().any(|(k, _)| k == r))
                .collect()
        })
        .unwrap_or_default()
}

/// Where a new object goes: its folder, or the device it hangs below, as
/// its `device:` names it.
pub enum Parent {
    Folder,
    /// A device of vislox's files, by slug.
    Ours(String),
    /// A device Loxone Config made: vislox's extern of it (its key in
    /// `edits.loxl`) and its type.
    Theirs(String, String),
}

impl Parent {
    /// The name `device:` gives it.
    pub fn name(&self) -> Option<String> {
        match self {
            Parent::Folder => None,
            Parent::Ours(s) => Some(s.clone()),
            Parent::Theirs(k, _) => Some(slug(k)),
        }
    }
}

/// The title of `el`, else its type.
fn named(el: &loxone_config::xml::Element) -> String {
    el.attr_decoded("Title")
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.into_owned())
        .unwrap_or_else(|| el.attr("Type").unwrap_or("object").to_string())
}

/// Where a new object of type `t` goes when it is added under `parent` (a
/// folder's or a device's UUID), checked against the pending config, the
/// base lox compiles against, and the lockfile.
pub fn parent(files: &Files, ctx: &Ctx, t: &Type, parent: &str) -> Result<Parent, String> {
    let el = find_ci(&ctx.doc.xml.root, parent)
        .ok_or_else(|| format!("no object {parent} in the pending config"))?;
    let pty = el.attr("Type").unwrap_or("");
    let name = named(el);
    match t.home {
        Home::Folder(folder) => {
            if pty != folder {
                return Err(format!(
                    "a {} goes into the folder {}, not into {name}",
                    t.ty,
                    folder_name(folder)
                ));
            }
            // lox appends it to the base's one folder of that type.
            let n = count(&ctx.base.xml.root, folder);
            if n != 1 || find_ci(&ctx.base.xml.root, parent).is_none() {
                return Err(format!(
                    "the project's base has {n} folders {}; lox adds to the one there is",
                    folder_name(folder)
                ));
            }
            Ok(Parent::Folder)
        }
        Home::Device(types) => {
            if !types.contains(&pty) {
                return Err(format!(
                    "a {} hangs below a {}, and {name} is a {pty}",
                    t.ty,
                    types.join(" or ")
                ));
            }
            match ctx.lock.slug_of(parent) {
                Some(s) if files.block(s).is_some() => Ok(Parent::Ours(s.to_string())),
                Some(_) => Err(format!(
                    "{name}: the module manages it; add its {} in its .loxl file",
                    t.ty
                )),
                None => {
                    // An extern resolves against the base: the device must be there.
                    if find_ci(&ctx.base.xml.root, parent).is_none() {
                        return Err(format!(
                            "{name} is not in the project's base config: Apply it first, or add \
                             it here"
                        ));
                    }
                    let u = el.attr("U").unwrap_or(parent);
                    let key = extern_key(files, u);
                    if let Some(c) = files.claims.0.get(&key)
                        && !c.ty.is_empty()
                        && c.ty != pty
                    {
                        return Err(format!(
                            "vislox's file names {u} as a {}, not a {pty}",
                            c.ty
                        ));
                    }
                    Ok(Parent::Theirs(key, pty.to_string()))
                }
            }
        }
    }
}

/// The UUID of where the object `b` of vislox's periphery file is: its
/// folder in `base`, or its device (a block of vislox's files by its
/// lockfile UUID, or the object vislox's extern names). What `add_block`
/// and `restore_block` take as `page`.
pub fn parent_uuid(files: &Files, ctx: &Ctx, b: &crate::blocks::NewBlock) -> String {
    let Some(t) = of(&b.ty) else {
        return String::new();
    };
    match t.home {
        Home::Folder(folder) => first(&ctx.base.xml.root, folder)
            .and_then(|e| e.attr("U"))
            .unwrap_or_default()
            .to_string(),
        Home::Device(_) => match b.get(DEVICE) {
            Some(Value::Ref(n)) => ctx
                .lock
                .uuid_of(n)
                .map(String::from)
                .or_else(|| files.claims.0.keys().find(|k| slug(k) == *n).cloned())
                .unwrap_or_default(),
            _ => String::new(),
        },
    }
}

/// The objects of vislox's files below each device they name: device
/// name → their slugs. Built once per request, not once per block.
pub type Below = BTreeMap<String, Vec<String>>;

pub fn below(files: &Files) -> Below {
    let mut out = Below::new();
    for b in files.pages.values().flat_map(|p| p.blocks.iter()) {
        if let Some(Value::Ref(n)) = b.get(DEVICE) {
            out.entry(n.clone()).or_default().push(b.slug.clone());
        }
    }
    out
}

/// The objects below the device `uuid` (in the pending config or the
/// base) that vislox's files do not declare: Loxone Config made them, and
/// deleting the device would delete them with it. By name, each once.
pub fn others_below(files: &Files, ctx: &Ctx, uuid: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for doc in [ctx.doc, ctx.base] {
        let Some(el) = find_ci(&doc.xml.root, uuid) else {
            continue;
        };
        for c in el.child_elements().filter(|c| c.name == "C") {
            let Some(u) = c.attr("U") else { continue };
            let ours = ctx
                .lock
                .slug_of(u)
                .is_some_and(|s| files.block(s).is_some());
            if !ours && seen.insert(u.to_ascii_lowercase()) {
                out.push(named(c));
            }
        }
    }
    out
}

/// For each folder and device of the config shown that takes new objects,
/// whether one can be added there now (`null`), or why not: the checks
/// `add_block` makes on where it goes (`GET /api/edits`' `add_under`).
pub fn add_refusals(files: &Files, ctx: &Ctx) -> serde_json::Map<String, Json> {
    fn walk(el: &loxone_config::xml::Element, out: &mut Vec<(String, &'static Type)>) {
        if let (Some(u), Some(ty)) = (el.attr("U"), el.attr("Type"))
            && let Some(t) = TYPES.iter().find(|t| match t.home {
                Home::Folder(f) => f == ty,
                Home::Device(ds) => ds.contains(&ty),
            })
        {
            out.push((u.to_string(), t));
        }
        for c in el.child_elements() {
            walk(c, out);
        }
    }
    let mut at = Vec::new();
    walk(&ctx.doc.xml.root, &mut at);
    at.into_iter()
        .map(|(u, t)| {
            let why = parent(files, ctx, t, &u).err();
            (u, json!(why))
        })
        .collect()
}

fn count(el: &loxone_config::xml::Element, ty: &str) -> usize {
    usize::from(el.attr("Type") == Some(ty))
        + el.child_elements().map(|c| count(c, ty)).sum::<usize>()
}

fn first<'a>(
    el: &'a loxone_config::xml::Element,
    ty: &str,
) -> Option<&'a loxone_config::xml::Element> {
    if el.attr("Type") == Some(ty) {
        return Some(el);
    }
    el.child_elements().find_map(|c| first(c, ty))
}

/// How Loxone Config names a folder type in English.
pub fn folder_name(ty: &str) -> &'static str {
    match ty {
        "VirtualInCaption" => "Virtual Inputs",
        "VirtualOutCaption" => "Virtual Outputs",
        "WeatherCaption" => "Network Devices (Netzwerkperipherie)",
        "ModeCaption" => "Operating Modes (Betriebsmodi)",
        "CalendarCaption" => "Calendar (Betriebszeiten)",
        _ => "of its type",
    }
}

/// `GET /api/block-types`' `periphery`: each type vislox adds to the object
/// tree, where it goes (`folder`: the folder type, or `devices`: the
/// device types it hangs below), its purpose, and its settings with how each
/// is written, its unit and whether a new one needs it.
pub fn palette(doc: &dyn Fn(&str) -> Option<String>) -> Json {
    TYPES
        .iter()
        .map(|t| {
            let settings: Vec<Json> = crate::blocks::settings(t.ty)
                .into_iter()
                .map(|(name, kind)| {
                    json!({
                        "name": name,
                        "kind": kind,
                        "choices": null,
                        "empty": false,
                        "required": t.required.contains(&name),
                        "unit": connectors::port_unit(t.ty, name).map(crate::edit::unit_suffix),
                    })
                })
                .collect();
            let (folder, devices) = match t.home {
                Home::Folder(f) => (Some(f), None),
                Home::Device(d) => (None, Some(d)),
            };
            json!({
                "type": t.ty,
                "summary": doc(t.ty),
                "folder": folder,
                "devices": devices,
                "settings": settings,
                // A calendar entry is added in the Operating modes view, where
                // its mode and date are picked (#86).
                "view": (t.ty == crate::calendar_edit::ENTRY).then_some("modes"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vislox's table is lox's: each type is of the object tree, a device
    /// lives in the folder lox appends it to, every required and text
    /// setting is one lox takes, and every other setting of lox's is a
    /// number or on/off.
    #[test]
    fn the_table_is_loxs() {
        for t in TYPES {
            assert!(connectors::object_tree(t.ty), "{}", t.ty);
            match t.home {
                Home::Folder(f) => {
                    assert_eq!(connectors::container_type(t.ty), Some(f), "{}", t.ty)
                }
                Home::Device(ds) => {
                    assert_eq!(connectors::container_type(t.ty), None, "{}", t.ty);
                    for d in ds {
                        assert!(of(d).is_some(), "{d}");
                    }
                }
            }
            let settings = connectors::attr_params(t.ty);
            for r in t.required {
                assert!(settings.contains(r), "{} {r}", t.ty);
            }
            for s in t.text {
                assert!(settings.contains(s), "{} {s}", t.ty);
                assert!(!connectors::bool_params(t.ty).contains(s), "{} {s}", t.ty);
                assert!(connectors::port_unit(t.ty, s).is_none(), "{} {s}", t.ty);
            }
        }
        // Every periphery type lox creates.
        for ty in connectors::BUILTIN_TYPES {
            if connectors::object_tree(ty) {
                assert!(of(ty).is_some(), "{ty}");
            }
        }
        assert_eq!(added_under("ModeCaption"), ["Mode"]);
        assert_eq!(added_under("CalendarCaption"), ["CalendarEntry"]);
        assert_eq!(added_under("VirtualInCaption").len(), 4);
        assert_eq!(
            added_under("ModbusDev"),
            ["ModbusASensor", "ModbusSensor", "ModbusAActor"]
        );
        assert!(added_under("Page").is_empty());
    }

    #[test]
    fn a_save_rewrites_are_refused_by_name() {
        let n = |k: &str, v: &str| (k.to_string(), Value::Number(v.into()));
        assert!(rewritten("VirtualHttpIn", &[n("Poll", "60")]).is_none());
        let e = rewritten("VirtualHttpIn", &[n("Poll", "5")]).unwrap();
        assert!(e.contains("Poll") && e.contains("10"), "{e}");
        // A digital virtual input reads 0 to 1.
        let e = rewritten("VirtualIn", &[n("MaxVal", "30")]).unwrap();
        assert!(e.contains("MaxVal") && e.contains("would write 1"), "{e}");
        assert!(
            rewritten(
                "VirtualIn",
                &[("Analog".into(), Value::Bool(true)), n("MaxVal", "30")]
            )
            .is_none()
        );
        assert_eq!(missing("VirtualHttpIn", &[n("Poll", "60")]), ["Address"]);
        assert!(missing("VirtualIn", &[]).is_empty());
    }
}
