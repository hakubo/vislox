//! Tree and Air hardware from vislox (#85): switching statistics on or off
//! for a Tree or Air input or output, and creating a Tree device that is not
//! paired yet on a Tree extension. The design is in `docs/editing.md`.
//!
//! Lox Logic does both through hardware declarations, which take over a
//! device or an I/O: `Statistics: on` writes the `StatsType=` the type
//! records with, `off` removes it, and `kind:` with `tree:` creates a Tree
//! device. vislox declares them in a file of its own, [`FILE`]:
//!
//! ```text
//! extern vx_hw_20000002_0000_0020_ffff504f94000002 = LoxTree(uuid: "20000002-0000-0020-ffff504f94000002")
//! vx_touchtree_1 = TreeDevice("Touch Flur", kind: "TouchTree", tree: vx_hw_20000002_0000_0020_ffff504f94000002, branch: "left")
//! vx_hw_20000003_0000_0030_ffff504f94000002 = TreeDevice(uuid: "20000003-0000-0030-ffff504f94000002")
//! vx_touchtree_1_i1 = TreeSensor(device: vx_touchtree_1, iname: "I1", Statistics: on)
//! vx_hw_20000003_0000_0031_ffff504f94000002 = TreeAsensor(device: vx_hw_20000003_0000_0030_ffff504f94000002, uuid: "20000003-0000-0031-ffff504f94000002", Statistics: on)
//! removed vx_touchtree_2
//! ```
//!
//! A declaration of a device Loxone Config paired names it by `uuid:` and
//! states nothing else, so the source owns no title, room or setting of it:
//! it is only there for its I/O to name. An I/O of a device created here is
//! named by `iname:`, since lox mints its UUIDs anew on every compile until
//! the device is deployed. Removing a declaration releases the hardware: the
//! element keeps what was written last, which before Apply is the base's.

use crate::edit::{Applied, Ctx, Files, Op, find_ci, op_value};
use loxone_config::logic::{ArgItem, BindingKind, Item, MatchSpec, Module, Value};
use loxone_config::xml::Element;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};

/// vislox's file of the hardware it declares, in its directory. A page file
/// is named after its page's title in letters, digits and dashes
/// ([`crate::blocks::file_name`]), so never this.
pub const FILE: &str = "_hardware.loxl";

/// The slugs vislox gives the externs and declarations it pins by UUID.
const HW_PREFIX: &str = "vx_hw_";

/// The Tree extension a created device goes on.
pub const TREE: &str = "LoxTree";

/// The Tree device type.
pub const TREE_DEVICE: &str = "TreeDevice";

/// The device types whose I/O vislox declares, with those I/O types.
const DEVICES: &[(&str, &[&str])] = &[
    (
        TREE_DEVICE,
        &["TreeSensor", "TreeAsensor", "TreeActor", "TreeAactor"],
    ),
    (
        "LoxAIRDevice",
        &[
            "LoxAIRsensor",
            "LoxAIRAsensor",
            "LoxAIRactor",
            "LoxAIRAactor",
        ],
    ),
];

/// The `StatsType=` an I/O type records with when `Statistics: on` (lox's
/// `hardware::stats_value`): 7 on a digital input or output, 11 on an
/// analog sensor. lox takes `Statistics:` on these types only.
pub fn stats_value(ty: &str) -> Option<&'static str> {
    match ty {
        "TreeSensor" | "TreeActor" | "LoxAIRsensor" | "LoxAIRactor" => Some("7"),
        "TreeAsensor" | "LoxAIRAsensor" => Some("11"),
        _ => None,
    }
}

/// The Tree device kinds lox creates (`tree_kinds`), with the title Loxone
/// Config gives a new one.
pub const KINDS: &[(&str, &str)] = &[
    ("TouchTree", "Touch Tree"),
    ("PresenceSensorTree", "Presence Sensor Tree"),
    ("CorridorLightTree", "LED Ceiling Light RGBW Tree"),
    ("ValveTree", "Valve Actuator Tree"),
];

/// The branches of a Tree extension.
pub const BRANCHES: &[&str] = &["left", "right"];

/// Statistics as Properties shows them: `on` when the I/O records with the
/// `StatsType=` its type writes, `off` when it has none, else the other
/// kind Loxone Config set (`StatsType` as stored).
pub fn stats_state(el: &Element) -> Option<String> {
    let ty = el.attr("Type")?;
    let st = el
        .attr("StatsType")
        .filter(|v| !v.trim().is_empty() && *v != "0");
    match (stats_value(ty), st) {
        (_, None) => stats_value(ty).map(|_| "off".to_string()),
        (Some(w), Some(v)) if w == v => Some("on".to_string()),
        (_, Some(v)) => Some(format!("StatsType {v}")),
    }
}

/// How a device is matched.
#[derive(Clone, Debug, PartialEq)]
pub enum DevAt {
    /// One Loxone Config paired (or one deployed earlier), by its UUID.
    Uuid(String),
    /// One lox creates on the Tree extension `tree` (an extern's slug).
    Create {
        kind: String,
        tree: String,
        branch: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub slug: String,
    pub ty: String,
    pub title: Option<String>,
    pub at: DevAt,
}

/// How an I/O is matched inside its device.
#[derive(Clone, Debug, PartialEq)]
pub enum IoAt {
    Uuid(String),
    IName(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Io {
    pub slug: String,
    pub ty: String,
    /// The declaration of its device, by slug.
    pub device: String,
    pub at: IoAt,
    pub stats: Option<bool>,
}

/// [`FILE`]: the Tree extensions it names (by UUID, as written), the
/// devices and I/O it declares, and the created devices it deletes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HwFile {
    pub trees: Vec<String>,
    pub devices: Vec<Device>,
    pub ios: Vec<Io>,
    pub removed: BTreeSet<String>,
}

/// The slug of vislox's extern or declaration of the object `uuid`.
pub fn hw_slug(uuid: &str) -> String {
    format!("{HW_PREFIX}{}", uuid.to_ascii_lowercase().replace('-', "_"))
}

const HEADER: &str = "# Tree and Air hardware vislox declares: the Tree devices it creates, and the inputs\n\
# and outputs it switches statistics on or off for. vislox rewrites this file on every\n\
# edit; see docs/editing.md in vislox.\n";

fn foreign(what: &str) -> String {
    format!(
        "{what} is not something vislox writes here; take it out, or move it into a file of the \
         module's own"
    )
}

fn text_of(v: &Value) -> Option<&str> {
    match v {
        Value::Str(s) | Value::Number(s) => Some(s),
        _ => None,
    }
}

impl HwFile {
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
            && self.devices.is_empty()
            && self.ios.is_empty()
            && self.removed.is_empty()
    }

    pub fn parse(text: &str) -> Result<HwFile, String> {
        let m = Module::parse_fragment(text).map_err(|e| e.to_string())?;
        let mut f = HwFile::default();
        for item in m.items {
            match item {
                Item::Comment(_) | Item::Gap => {}
                Item::Extern(e) => match &e.match_spec {
                    MatchSpec::Uuid(u)
                        if e.block_type == TREE && e.slug == hw_slug(u) && e.args.is_empty() =>
                    {
                        f.trees.push(u.clone())
                    }
                    _ => return Err(foreign(&format!("the extern `{}`", e.slug))),
                },
                Item::Removed(r) if r.slug.starts_with(crate::blocks::SLUG_PREFIX) => {
                    f.removed.insert(r.slug);
                }
                Item::Block(b) if b.slug.starts_with(crate::blocks::SLUG_PREFIX) => {
                    let mut args: BTreeMap<String, Value> = BTreeMap::new();
                    for a in &b.args {
                        match a {
                            ArgItem::Binding(x) => match &x.kind {
                                BindingKind::Param(v) => {
                                    if args.insert(x.port.clone(), v.clone()).is_some() {
                                        return Err(format!("`{}` gives {} twice", b.slug, x.port));
                                    }
                                }
                                _ => return Err(foreign(&format!("a wire onto `{}`", b.slug))),
                            },
                            ArgItem::Comment(_) => {}
                        }
                    }
                    let take = |args: &mut BTreeMap<String, Value>, k: &str| args.remove(k);
                    let bad = || foreign(&format!("`{}` ({})", b.slug, b.block_type));
                    if DEVICES.iter().any(|(d, _)| *d == b.block_type) {
                        let at = match (take(&mut args, "uuid"), take(&mut args, "kind")) {
                            (Some(u), None) => DevAt::Uuid(text_of(&u).ok_or_else(bad)?.into()),
                            (None, Some(k)) if b.block_type == TREE_DEVICE => {
                                let tree = match take(&mut args, "tree") {
                                    Some(Value::Ref(t)) => t,
                                    _ => return Err(bad()),
                                };
                                let branch = match take(&mut args, "branch") {
                                    Some(v) => Some(text_of(&v).ok_or_else(bad)?.to_string()),
                                    None => None,
                                };
                                DevAt::Create {
                                    kind: text_of(&k).ok_or_else(bad)?.into(),
                                    tree,
                                    branch,
                                }
                            }
                            _ => return Err(bad()),
                        };
                        if let DevAt::Uuid(u) = &at
                            && (b.slug != hw_slug(u) || b.title.is_some())
                        {
                            return Err(bad());
                        }
                        if !args.is_empty() {
                            return Err(bad());
                        }
                        f.devices.push(Device {
                            slug: b.slug,
                            ty: b.block_type,
                            title: b.title,
                            at,
                        });
                    } else if stats_value(&b.block_type).is_some() || is_io(&b.block_type) {
                        let device = match take(&mut args, "device") {
                            Some(Value::Ref(d)) => d,
                            _ => return Err(bad()),
                        };
                        let at = match (take(&mut args, "uuid"), take(&mut args, "iname")) {
                            (Some(u), None) => IoAt::Uuid(text_of(&u).ok_or_else(bad)?.into()),
                            (None, Some(i)) => IoAt::IName(text_of(&i).ok_or_else(bad)?.into()),
                            _ => return Err(bad()),
                        };
                        let stats = match take(&mut args, "Statistics") {
                            Some(Value::Bool(v)) => Some(v),
                            Some(Value::Ref(r)) if r == "on" || r == "off" => Some(r == "on"),
                            None => None,
                            _ => return Err(bad()),
                        };
                        if !args.is_empty() || b.title.is_some() {
                            return Err(bad());
                        }
                        f.ios.push(Io {
                            slug: b.slug,
                            ty: b.block_type,
                            device,
                            at,
                            stats,
                        });
                    } else {
                        return Err(bad());
                    }
                }
                Item::Block(b) => return Err(foreign(&format!("`{}`", b.slug))),
                _ => return Err(foreign("a statement")),
            }
        }
        for d in &f.devices {
            if let DevAt::Create { tree, .. } = &d.at
                && !f.trees.iter().any(|u| hw_slug(u) == *tree)
            {
                return Err(format!(
                    "`{}` names the Tree extension `{tree}`, which this file does not; put its \
                     extern back",
                    d.slug
                ));
            }
        }
        for io in &f.ios {
            if !f.devices.iter().any(|d| d.slug == io.device) {
                return Err(format!(
                    "`{}` names the device `{}`, which this file does not declare; put it back",
                    io.slug, io.device
                ));
            }
        }
        Ok(f)
    }

    pub fn render(&self) -> String {
        let mut out = String::from(HEADER);
        if !self.trees.is_empty() {
            out.push('\n');
            for u in &self.trees {
                out.push_str(&format!(
                    "extern {} = {TREE}(uuid: {})\n",
                    hw_slug(u),
                    Value::Str(u.clone())
                ));
            }
        }
        if !self.devices.is_empty() {
            out.push('\n');
            for d in &self.devices {
                out.push_str(&d.render());
                out.push('\n');
            }
        }
        if !self.ios.is_empty() {
            out.push('\n');
            for io in &self.ios {
                out.push_str(&io.render());
                out.push('\n');
            }
        }
        if !self.removed.is_empty() {
            out.push('\n');
            for s in &self.removed {
                out.push_str(&format!("removed {s}\n"));
            }
        }
        out
    }

    /// Whether this file names `slug`.
    pub fn names(&self, slug: &str) -> bool {
        self.removed.contains(slug)
            || self.devices.iter().any(|d| d.slug == slug)
            || self.ios.iter().any(|i| i.slug == slug)
            || self.trees.iter().any(|u| hw_slug(u) == slug)
    }

    /// The device `slug` vislox created, if it is one.
    pub fn created(&self, slug: &str) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.slug == slug && matches!(d.at, DevAt::Create { .. }))
    }

    /// Drop the Tree externs and the devices by UUID nothing names any more.
    fn prune(&mut self) {
        let ios = &self.ios;
        self.devices
            .retain(|d| !matches!(d.at, DevAt::Uuid(_)) || ios.iter().any(|i| i.device == d.slug));
        let devices = &self.devices;
        self.trees.retain(|u| {
            let s = hw_slug(u);
            devices
                .iter()
                .any(|d| matches!(&d.at, DevAt::Create { tree, .. } if *tree == s))
        });
    }

    /// Every statistics claim, as the op that sets it, and every created
    /// device's name: by the UUID the lockfile has for each.
    pub fn ops(&self, lock: &crate::edit::Lock) -> Vec<Op> {
        let mut v = Vec::new();
        for d in &self.devices {
            if let (DevAt::Create { .. }, Some(t), Some(u)) =
                (&d.at, &d.title, lock.externals.get(&d.slug))
            {
                v.push(Op::Title {
                    uuid: u.clone(),
                    value: Some(t.clone()),
                });
            }
        }
        for io in &self.ios {
            let uuid = match &io.at {
                IoAt::Uuid(u) => Some(u.clone()),
                IoAt::IName(_) => lock.externals.get(&io.slug).cloned(),
            };
            if let (Some(uuid), Some(s)) = (uuid, io.stats) {
                v.push(Op::Statistics {
                    uuid,
                    value: Some(on_off(s).into()),
                });
            }
        }
        v
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn is_io(ty: &str) -> bool {
    DEVICES.iter().any(|(_, ios)| ios.contains(&ty))
}

impl Device {
    fn render(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = &self.title {
            parts.push(Value::Str(t.clone()).to_string());
        }
        match &self.at {
            DevAt::Uuid(u) => parts.push(format!("uuid: {}", Value::Str(u.clone()))),
            DevAt::Create { kind, tree, branch } => {
                parts.push(format!("kind: {}", Value::Str(kind.clone())));
                parts.push(format!("tree: {tree}"));
                if let Some(b) = branch {
                    parts.push(format!("branch: {}", Value::Str(b.clone())));
                }
            }
        }
        format!("{} = {}({})", self.slug, self.ty, parts.join(", "))
    }
}

impl Io {
    fn render(&self) -> String {
        let mut parts = vec![format!("device: {}", self.device)];
        match &self.at {
            IoAt::Uuid(u) => parts.push(format!("uuid: {}", Value::Str(u.clone()))),
            IoAt::IName(i) => parts.push(format!("iname: {}", Value::Str(i.clone()))),
        }
        if let Some(s) = self.stats {
            parts.push(format!("Statistics: {}", on_off(s)));
        }
        format!("{} = {}({})", self.slug, self.ty, parts.join(", "))
    }
}

// ── where an object is ─────────────────────────────────────────────────

/// The element `uuid` and its ancestors, nearest last.
fn path<'a>(root: &'a Element, uuid: &str) -> Option<Vec<&'a Element>> {
    fn walk<'a>(el: &'a Element, uuid: &str, out: &mut Vec<&'a Element>) -> bool {
        out.push(el);
        if el.attr("U").is_some_and(|u| u.eq_ignore_ascii_case(uuid)) {
            return true;
        }
        for c in el.child_elements() {
            if walk(c, uuid, out) {
                return true;
            }
        }
        out.pop();
        false
    }
    let mut out = Vec::new();
    walk(root, uuid, &mut out).then_some(out)
}

fn named(el: &Element) -> String {
    crate::adopt::title_or(
        el.attr_decoded("Title").as_deref(),
        el.attr("U").unwrap_or_default(),
    )
}

/// The device vislox created whose UUID (as the lockfile has it) is `uuid`:
/// its slug.
pub fn created_slug(files: &Files, ctx: &Ctx, uuid: &str) -> Option<String> {
    files
        .hardware
        .devices
        .iter()
        .filter(|d| matches!(d.at, DevAt::Create { .. }))
        .find(|d| {
            ctx.lock
                .externals
                .get(&d.slug)
                .is_some_and(|u| u.eq_ignore_ascii_case(uuid))
        })
        .map(|d| d.slug.clone())
}

/// The slug the rest of the module declares the object `uuid` by, when it
/// does (a hardware declaration owns it).
fn module_owner<'a>(files: &Files, ctx: &'a Ctx, uuid: &str) -> Option<&'a str> {
    ctx.lock
        .externals
        .iter()
        .find(|(s, u)| {
            u.eq_ignore_ascii_case(uuid)
                && ctx.lock.owned.contains(*s)
                && !files.hardware.names(s)
                // vislox's own, from before an earlier op of the request.
                && (ctx.names.contains(*s) || !s.starts_with(crate::blocks::SLUG_PREFIX))
        })
        .map(|(s, _)| s.as_str())
}

// ── statistics ─────────────────────────────────────────────────────────

/// `statistics`: switch the statistics of the Tree or Air I/O `uuid` on or
/// off, or drop vislox's claim (`None`). A value op: its `previous` is what
/// vislox's file claimed before.
pub fn set_statistics(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&str>,
) -> Result<Applied, String> {
    let want = match value {
        None => None,
        Some("on") => Some(true),
        Some("off") => Some(false),
        Some(v) => return Err(format!("Statistics: on or off, not \"{v}\"")),
    };
    // The claim as vislox's file has it, found by UUID or (below a device
    // created here) by its designation.
    let at = files.hardware.ios.iter().position(|io| {
        let u = match &io.at {
            IoAt::Uuid(u) => Some(u.as_str()),
            IoAt::IName(_) => ctx.lock.externals.get(&io.slug).map(String::as_str),
        };
        u.is_some_and(|u| u.eq_ignore_ascii_case(uuid))
    });
    let previous = at
        .and_then(|i| files.hardware.ios[i].stats)
        .map(|s| on_off(s).to_string());
    // Dropping a claim needs no object: one gone since (an undo) drops nothing.
    if want.is_none() {
        // Dropping vislox's claim is always allowed.
        if let Some(i) = at {
            files.hardware.ios[i].stats = None;
            files.hardware.ios.remove(i);
            files.hardware.prune();
        }
        return Ok(Applied {
            previous: json!(previous),
            added: None,
            warnings: Vec::new(),
        });
    }
    let chain = path(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let el = *chain.last().expect("found");
    let ty = el.attr("Type").unwrap_or_default().to_string();
    let name = named(el);
    if stats_value(&ty).is_none() {
        return Err(format!(
            "{name}: Lox Logic switches statistics only on a Tree or Air input, a digital output \
             and an analog sensor, not on a {ty}"
        ));
    }
    let dev = chain
        .iter()
        .rev()
        .skip(1)
        .find(|e| {
            DEVICES
                .iter()
                .any(|(d, ios)| e.attr("Type") == Some(*d) && ios.contains(&ty.as_str()))
        })
        .ok_or_else(|| format!("{name}: it hangs below no Tree or Air device"))?;
    let dev_uuid = dev.attr("U").unwrap_or_default().to_string();
    let dev_ty = dev.attr("Type").unwrap_or_default().to_string();
    for (u, what) in [(uuid, name.clone()), (dev_uuid.as_str(), named(dev))] {
        if let Some(s) = module_owner(files, ctx, u) {
            return Err(format!(
                "{name}: the module declares {what} in its own file (`{s}`); switch its \
                 statistics there"
            ));
        }
    }
    if let Some(i) = at {
        files.hardware.ios[i].stats = want;
    } else if let Some(dev_slug) = created_slug(files, ctx, &dev_uuid) {
        // Below a device created here: by its designation, since lox mints
        // its UUIDs anew until the device is deployed.
        let iname = el
            .attr("IName")
            .filter(|i| !i.trim().is_empty())
            .ok_or_else(|| format!("{name}: it has no designation (IName) to declare it by"))?
            .to_string();
        let slug = format!("{dev_slug}_{}", iname.to_ascii_lowercase());
        if files.hardware.names(&slug) || ctx.names.contains(&slug) {
            return Err(format!("{name}: the name `{slug}` is taken"));
        }
        files.hardware.ios.push(Io {
            slug,
            ty,
            device: dev_slug,
            at: IoAt::IName(iname),
            stats: want,
        });
    } else {
        if find_ci(&ctx.base.xml.root, uuid).is_none() {
            return Err(format!(
                "{name}: it is not in the project's base config yet; Apply it first"
            ));
        }
        let dev_slug = hw_slug(&dev_uuid);
        if !files.hardware.devices.iter().any(|d| d.slug == dev_slug) {
            files.hardware.devices.push(Device {
                slug: dev_slug.clone(),
                ty: dev_ty,
                title: None,
                at: DevAt::Uuid(dev_uuid),
            });
        }
        files.hardware.ios.push(Io {
            slug: hw_slug(uuid),
            ty,
            device: dev_slug,
            at: IoAt::Uuid(el.attr("U").unwrap_or(uuid).to_string()),
            stats: want,
        });
    }
    Ok(Applied {
        previous: json!(previous),
        added: None,
        warnings: Vec::new(),
    })
}

// ── creating a Tree device ─────────────────────────────────────────────

/// Why a Tree device cannot be added on the Tree extension `tree` now
/// (`Ok`: it can).
fn tree_ok(ctx: &Ctx, tree: &str) -> Result<(), String> {
    let el = find_ci(&ctx.doc.xml.root, tree)
        .filter(|e| e.attr("Type") == Some(TREE))
        .ok_or_else(|| format!("no Tree extension {tree} in the pending config"))?;
    if find_ci(&ctx.base.xml.root, tree).is_none() {
        return Err(format!(
            "{}: it is not in the project's base config; Apply it first",
            named(el)
        ));
    }
    Ok(())
}

/// For each Tree extension of the config shown, whether a device can be
/// added there now (`null`), or why not (`GET /api/edits`' `add_under`).
pub fn add_refusals(ctx: &Ctx) -> serde_json::Map<String, Json> {
    fn walk(el: &Element, out: &mut Vec<String>) {
        if el.attr("Type") == Some(TREE)
            && let Some(u) = el.attr("U")
        {
            out.push(u.to_string());
        }
        for c in el.child_elements() {
            walk(c, out);
        }
    }
    let mut at = Vec::new();
    walk(&ctx.doc.xml.root, &mut at);
    at.into_iter()
        .map(|u| {
            let why = tree_ok(ctx, &u).err();
            (u, json!(why))
        })
        .collect()
}

/// The setting of `add_block` that switches the statistics of a new
/// device's I/O on or off: `<iname>.Statistics`. The undo of a delete
/// carries them, so the device comes back as it was.
const STATS_SUFFIX: &str = ".Statistics";

/// `add_block` of a `TreeDevice` on the Tree extension `page`: a device lox
/// creates, unpaired. Its settings are `kind` and `branch`.
pub fn add_device(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::AddBlock {
        page: tree,
        title,
        params,
        settings,
        wires,
        ..
    } = op
    else {
        unreachable!()
    };
    tree_ok(ctx, tree)?;
    if !params.is_empty() || !wires.is_empty() {
        return Err("a Tree device takes no constants or wires here".into());
    }
    let mut settings = settings.clone();
    let kind = settings
        .remove("kind")
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "say which kind of Tree device to create: {}",
                KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
            )
        })?;
    if !KINDS.iter().any(|(k, _)| *k == kind) {
        return Err(format!(
            "Lox Logic cannot create a {kind}; it creates {}. Pair others in Loxone Config.",
            KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
        ));
    }
    let branch = settings.remove("branch").filter(|b| !b.trim().is_empty());
    if let Some(b) = &branch
        && !BRANCHES.contains(&b.as_str())
    {
        return Err(format!("branch: left or right, not \"{b}\""));
    }
    let mut stats = Vec::new();
    for (k, v) in &settings {
        let Some(iname) = k
            .strip_suffix(STATS_SUFFIX)
            .filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric()))
        else {
            return Err(format!("a Tree device: {k} cannot be set here"));
        };
        let on = match v.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err(format!("{k}: on or off, not \"{v}\"")),
        };
        stats.push((iname.to_string(), on));
    }
    let title = title
        .as_deref()
        .map(|t| crate::edit::block_title(TREE_DEVICE, t))
        .transpose()?;
    let tree_el = find_ci(&ctx.doc.xml.root, tree).expect("checked");
    let tree_uuid = tree_el.attr("U").unwrap_or(tree).to_string();
    let taken = |s: &str| {
        ctx.names.contains(s)
            || ctx.lock.slugs.contains_key(s)
            || ctx.lock.externals.contains_key(s)
            || ctx.lock.tombstoned(s)
            || files.names(s)
    };
    let slug = crate::blocks::new_slug(&kind, &taken);
    // Everything is checked: from here on `files` changes.
    let hw = &mut files.hardware;
    if !hw.trees.iter().any(|u| u.eq_ignore_ascii_case(&tree_uuid)) {
        hw.trees.push(tree_uuid.clone());
    }
    hw.devices.push(Device {
        slug: slug.clone(),
        ty: TREE_DEVICE.into(),
        title,
        at: DevAt::Create {
            kind,
            tree: hw_slug(&tree_uuid),
            branch,
        },
    });
    for (iname, on) in stats {
        hw.ios.push(Io {
            slug: format!("{slug}_{}", iname.to_ascii_lowercase()),
            ty: io_type(&iname),
            device: slug.clone(),
            at: IoAt::IName(iname),
            stats: Some(on),
        });
    }
    Ok(Applied {
        previous: Json::Null,
        added: Some(slug),
        warnings: Vec::new(),
    })
}

/// The I/O type of a created device's designation: `AI…` an analog sensor,
/// `AQ…` an analog output, `Q…` a digital output, else a digital input.
fn io_type(iname: &str) -> String {
    if iname.starts_with("AI") {
        "TreeAsensor"
    } else if iname.starts_with("AQ") {
        "TreeAactor"
    } else if iname.starts_with('Q') {
        "TreeActor"
    } else {
        "TreeSensor"
    }
    .into()
}

/// The name of a created device, written into its declaration.
pub fn set_title(files: &mut Files, slug: &str, value: Option<&str>) -> Result<Applied, String> {
    let title = value
        .map(|t| crate::edit::block_title(TREE_DEVICE, t))
        .transpose()?;
    let d = files
        .hardware
        .devices
        .iter_mut()
        .find(|d| d.slug == slug)
        .expect("a created device");
    let previous = std::mem::replace(&mut d.title, title);
    Ok(Applied {
        previous: json!(previous),
        added: None,
        warnings: Vec::new(),
    })
}

// ── deleting it ────────────────────────────────────────────────────────

/// Whether the created device `slug` (UUID `uuid`) can be deleted or taken
/// out now: `Err` says why not. Changes nothing.
pub fn removal(files: &Files, ctx: &Ctx, slug: &str, uuid: &str) -> Result<bool, String> {
    let el = find_ci(&ctx.doc.xml.root, uuid)
        .ok_or_else(|| format!("no object {uuid} in the pending config"))?;
    let name = named(el);
    // Its objects' connectors: a wire from one of them to anything outside
    // keeps the device (lox does not delete a device that feeds a wire
    // drawn in Loxone Config), and a claim of vislox's on one would name
    // nothing.
    let mut inside: BTreeSet<String> = BTreeSet::new();
    let mut objects: Vec<&Element> = Vec::new();
    fn collect<'a>(el: &'a Element, ports: &mut BTreeSet<String>, objs: &mut Vec<&'a Element>) {
        if el.name == "C" {
            objs.push(el);
        }
        for c in el.child_elements() {
            if c.name == "Co"
                && let Some(u) = c.attr("U")
            {
                ports.insert(u.to_ascii_lowercase());
            }
            collect(c, ports, objs);
        }
    }
    collect(el, &mut inside, &mut objects);
    for w in ctx.doc.wires() {
        let from = inside.contains(&w.from_port.to_ascii_lowercase());
        let to = inside.contains(&w.to_port.to_ascii_lowercase());
        if from != to {
            let io = objects
                .iter()
                .find(|o| {
                    o.child_elements().any(|c| {
                        c.name == "Co"
                            && c.attr("U").is_some_and(|u| {
                                u.eq_ignore_ascii_case(&w.from_port)
                                    || u.eq_ignore_ascii_case(&w.to_port)
                            })
                    })
                })
                .map(|o| named(o))
                .unwrap_or_else(|| name.clone());
            return Err(format!(
                "{name}: its {io} is wired to a block; lox does not delete a device that feeds a \
                 wire, so take that wire out first"
            ));
        }
    }
    for o in &objects {
        let u = o.attr("U").unwrap_or_default();
        if files.claims.0.keys().any(|k| k.eq_ignore_ascii_case(u)) {
            return Err(format!(
                "{name}: vislox's file claims a value of {}; Revert it first",
                named(o)
            ));
        }
    }
    let deployed = ctx
        .deployed
        .is_some_and(|d| find_ci(&d.xml.root, uuid).is_some());
    if deployed && !ctx.lock.externals.contains_key(slug) {
        return Err(format!(
            "{name}: the lockfile has no entry for `{slug}`; compile the module first"
        ));
    }
    Ok(deployed)
}

/// `remove_block` of the device `slug` vislox created, with `removed
/// <slug>`: when it is not deployed, its `previous` is the `add_block` that
/// adds it again (under a new UUID); when it is, the `restore_block` that
/// brings it back with its UUID.
pub fn remove_device(
    files: &mut Files,
    ctx: &Ctx,
    slug: &str,
    uuid: &str,
) -> Result<Applied, String> {
    let deployed = removal(files, ctx, slug, uuid)?;
    let hw = &mut files.hardware;
    let d = hw.created(slug).expect("checked").clone();
    let DevAt::Create { kind, tree, branch } = &d.at else {
        unreachable!()
    };
    let tree_uuid = hw
        .trees
        .iter()
        .find(|u| hw_slug(u) == *tree)
        .cloned()
        .unwrap_or_default();
    let ios: Vec<Io> = hw
        .ios
        .iter()
        .filter(|i| i.device == slug)
        .cloned()
        .collect();
    let previous = if deployed {
        let mut source = d.render();
        for io in &ios {
            source.push('\n');
            source.push_str(&io.render());
        }
        let title = find_ci(&ctx.doc.xml.root, uuid)
            .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()));
        Op::RestoreBlock {
            uuid: uuid.to_string(),
            page: tree_uuid,
            adopted: false,
            source,
            program: None,
            wires: Vec::new(),
            externs: BTreeMap::new(),
            lock: json!({ "uuid": uuid, "matched_by": "created", "title_at_match": title }),
            lock_wires: Vec::new(),
        }
    } else {
        let mut settings = BTreeMap::from([("kind".to_string(), kind.clone())]);
        if let Some(b) = branch {
            settings.insert("branch".into(), b.clone());
        }
        for io in &ios {
            if let (IoAt::IName(i), Some(s)) = (&io.at, io.stats) {
                settings.insert(format!("{i}{STATS_SUFFIX}"), on_off(s).into());
            }
        }
        Op::AddBlock {
            page: tree_uuid,
            ty: TREE_DEVICE.into(),
            title: d.title.clone(),
            params: BTreeMap::new(),
            settings,
            wires: Vec::new(),
            restores: Some(uuid.to_string()),
        }
    };
    hw.devices.retain(|x| x.slug != slug);
    hw.ios.retain(|i| i.device != slug);
    // lox keeps a created device in the lockfile from its first compile on:
    // without `removed` it refuses to let the declaration go.
    hw.removed.insert(slug.to_string());
    hw.prune();
    Ok(Applied {
        previous: op_value(&previous),
        added: None,
        warnings: Vec::new(),
    })
}

/// Whether a `restore_block`'s source is a device of [`FILE`].
pub fn restores_device(source: &str) -> bool {
    source
        .lines()
        .next()
        .is_some_and(|l| l.contains(&format!("= {TREE_DEVICE}(")) && l.contains("kind:"))
}

/// `restore_block` of a device vislox created and deleted while lox keeps
/// its tombstone: its declaration and its I/O's as they were, and its
/// lockfile entries rebuilt here (never taken from the request), so lox
/// brings it back under its UUID. Its `previous` is the `remove_block`.
pub fn restore_device(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::RestoreBlock {
        uuid, page, source, ..
    } = op
    else {
        unreachable!()
    };
    let (tomb_uuid, tomb_slug, tomb_ty) = ctx.lock.tomb(uuid).ok_or_else(|| {
        format!("lox keeps no tombstone of {uuid}: it can no longer be brought back")
    })?;
    if tomb_ty != TREE_DEVICE {
        return Err(format!("{uuid} was no Tree device"));
    }
    let tree_hw = format!(
        "extern {} = {TREE}(uuid: {})\n",
        hw_slug(page),
        Value::Str(page.clone())
    );
    let parsed = HwFile::parse(&format!("{tree_hw}{source}\n"))
        .map_err(|e| format!("the device to bring back: {e}"))?;
    let [d] = parsed.devices.as_slice() else {
        return Err("the device to bring back is one declaration".into());
    };
    if d.slug != tomb_slug || !matches!(d.at, DevAt::Create { .. }) {
        return Err(format!(
            "the device to bring back is `{tomb_slug}`, created here, not `{}`",
            d.slug
        ));
    }
    if parsed.ios.iter().any(|i| i.device != d.slug) || !parsed.removed.is_empty() {
        return Err("the device to bring back names only its own inputs and outputs".into());
    }
    let deployed = ctx
        .deployed
        .and_then(|doc| path(&doc.xml.root, uuid))
        .ok_or_else(|| format!("{uuid} is not in the deployed config"))?;
    let el = *deployed.last().expect("found");
    if !deployed
        .iter()
        .any(|e| e.attr("U").is_some_and(|u| u.eq_ignore_ascii_case(page)))
    {
        return Err(format!("{} is not on that Tree extension", named(el)));
    }
    if let DevAt::Create { kind, .. } = &d.at
        && el.attr("TId") != Some(kind.as_str())
    {
        return Err(format!("{} is no {kind}", named(el)));
    }
    tree_ok(ctx, page)?;
    let in_use = |s: &str| {
        ctx.names.contains(s)
            || ctx.lock.slugs.contains_key(s)
            || ctx.lock.externals.contains_key(s)
            || files.names_except_removed(s)
    };
    if let Some(s) = std::iter::once(&d.slug)
        .chain(parsed.ios.iter().map(|i| &i.slug))
        .find(|s| in_use(s))
    {
        return Err(format!("the name `{s}` is taken now"));
    }
    let hw = &mut files.hardware;
    let tree_uuid = page.clone();
    if !hw.trees.iter().any(|u| u.eq_ignore_ascii_case(&tree_uuid)) {
        hw.trees.push(tree_uuid);
    }
    hw.removed.remove(&d.slug);
    hw.devices.push(d.clone());
    hw.ios.extend(parsed.ios.iter().cloned());
    files.lock_edits.untomb.insert(tomb_uuid.to_string());
    files.lock_edits.created.insert(
        d.slug.clone(),
        (
            el.attr("U").unwrap_or(uuid).to_string(),
            el.attr_decoded("Title").map(|t| t.into_owned()),
        ),
    );
    Ok(Applied {
        previous: op_value(&Op::RemoveBlock { uuid: uuid.clone() }),
        added: None,
        warnings: Vec::new(),
    })
}

/// `GET /api/edits`' `blocks` entries for the devices vislox created: as a
/// block of vislox's files is listed, with `hardware` set and whether the
/// device is paired.
pub fn listed(
    files: &Files,
    lock: &crate::edit::Lock,
    rel_dir: &str,
    deployed: Option<&loxone_config::LoxoneDoc>,
    shown: Option<&loxone_config::LoxoneDoc>,
    was: &Files,
) -> Vec<Json> {
    files
        .hardware
        .devices
        .iter()
        .filter_map(|d| {
            let DevAt::Create { kind, branch, .. } = &d.at else {
                return None;
            };
            let uuid = lock.externals.get(&d.slug);
            let serial = uuid
                .and_then(|u| shown.and_then(|s| find_ci(&s.xml.root, u)))
                .and_then(|e| e.attr("Serial").map(String::from));
            Some(json!({
                "slug": d.slug,
                "uuid": uuid,
                "type": TREE_DEVICE,
                "page": null,
                "periphery": true,
                "hardware": true,
                "kind": kind,
                "branch": branch,
                "paired": !crate::hardware::unpaired(serial.as_deref()),
                "file": format!("{rel_dir}/{FILE}"),
                // lox's rename of a created device is not offered here.
                "renames": false,
                "settings": [],
                "adopted": false,
                "committed": was.hardware.created(&d.slug).is_some(),
                "deployed": uuid.is_some_and(|u| deployed
                    .is_some_and(|doc| find_ci(&doc.xml.root, u).is_some())),
            }))
        })
        .collect()
}

/// `GET /api/block-types`' `periphery` entry for a Tree device: it goes on a
/// Tree extension, with its kind and branch.
pub fn palette() -> Json {
    json!({
        "type": TREE_DEVICE,
        "summary": "A Tree device lox creates unpaired; pair it with a Tree search in Loxone Config.",
        "folder": null,
        "devices": [TREE],
        "hardware": true,
        "settings": [
            { "name": "kind", "kind": "choice", "choices": KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
              "empty": false, "required": true, "unit": null },
            { "name": "branch", "kind": "choice", "choices": BRANCHES, "empty": false,
              "required": true, "unit": null },
        ],
    })
}
