//! Adding blocks (#59): the block types vislox offers, and the files that
//! declare the blocks it adds. The design is in `docs/editing.md`.
//!
//! A block added in vislox is a managed block of the Lox Logic module: a
//! declaration `vx_and_1 = And("Licht", In2: 1)` in a file of vislox's own,
//! one per page, headed by the page's `page "…"` line
//! (`<module>/vislox/<page>.loxl`). `lox logic compile` mints its UUIDs (in
//! the lockfile) and places it on the page. vislox picks the slug, never
//! derived from the name, so that a rename never changes the block's
//! identity.
//!
//! A block added here and not deployed yet can be taken out again: its
//! declaration goes, and a `removed <slug>` line tells lox that is meant
//! (without it lox refuses a lockfile entry the source lost). The line is
//! dropped once the lockfile no longer has the block.
//!
//! A block Loxone Config drew can be adopted into the same file (#72,
//! [`crate::adopt`]): its declaration as `lox logic adopt` writes it, under
//! the file's *adopted* comment, with the wires it brings along (bound in its
//! arguments, or `<-` onto an extern of `edits.loxl`). It keeps its UUIDs.

use loxone_config::connectors::{self, PortDir};
use loxone_config::logic::{
    ArgItem, Attr, Binding, BindingKind, Item, Module, PageDecl, PortRef, Value, WireDecl,
};
use loxone_config::port_names;
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};

/// The slugs vislox picks start with this; nothing else in a module should.
pub const SLUG_PREFIX: &str = "vx_";

/// The argument that says which object a ref mirrors (`mirrors: frost`).
pub const MIRRORS: &str = "mirrors";

/// A push notification drawn on a page (#77): the block, and the object of
/// Loxone Config's *Messages* it mirrors. Not in lox's builtin table (it is a
/// ref), so not in the palette's list of types; added with `mirrors:`.
pub const MESSAGE_REF: &str = "OutputRefLM";
pub const NOTIFICATION: &str = loxone_config::connectors::MESSAGE_REF_TARGET;

// ── the palette ─────────────────────────────────────────────────────────

/// Whether vislox offers `ty` as a new block on a page: a type of lox's
/// verified builtin table that is drawn on a page and that lox creates.
/// Not offered: objects of the object tree (virtual inputs, virtual and
/// Modbus devices, operating modes: #84), blocks a device carries,
/// which exist because Loxone Config paired the device, and [`NEEDS_SETUP`].
pub fn offered(ty: &str) -> bool {
    connectors::BUILTIN_TYPES.contains(&ty)
        && !connectors::object_tree(ty)
        && !connectors::adopt_only(ty)
        && ty != "Mode"
        && !NEEDS_SETUP.contains(&ty)
}

/// Types lox creates that do nothing until they are set up in Loxone
/// Config: an alarm's sensors and its zones, a room controller's and an
/// irrigation's settings, a central block's members (#75). Not offered.
pub const NEEDS_SETUP: &[&str] = &[
    "Alarm",
    "SmokeAlarm",
    "HeatIRoomController2",
    "Irrigation",
    "CentralLight",
    "CentralShade",
];

/// How a setting of a new block (an attribute parameter, `Formula:`,
/// `Type:`, `At:`, …) is written.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// One line of text; `empty` says whether it may be empty.
    Text,
    /// One of a few words.
    Choice,
    /// `on` or `off`.
    OnOff,
    /// A time of day, `HH:MM` or `HH:MM:SS`.
    Time,
    /// A number.
    Number,
    /// A duration: a number with `ms`, `s`, `min` or `h`, or seconds as a
    /// plain number (`12h`, `30min`, `90`).
    Duration,
    /// A program's text (`Code:` on `Code16`, #79): written into a file of
    /// its own next to the block's, `Code: file("vx_code16_1.c")`.
    Program,
}

/// How `param`, a setting of `ty`, is written; `None` for one vislox does
/// not offer.
pub fn setting_kind(ty: &str, param: &str) -> Option<Kind> {
    // A push notification's message (#77): its texts, and the least time
    // between two messages.
    if connectors::message_params(ty).contains(&param) {
        return Some(if param == "MinimumTime" {
            Kind::Duration
        } else {
            Kind::Text
        });
    }
    if connectors::file_params(ty).contains(&param) {
        Some(Kind::Program)
    } else if connectors::clock_params(ty).contains(&param) {
        Some(Kind::Time)
    } else if connectors::choice_values(ty, param).is_some() {
        Some(Kind::Choice)
    } else if connectors::bool_params(ty).contains(&param) {
        Some(Kind::OnOff)
    } else if connectors::text_params(ty).contains(&param) || (ty, param) == ("Formula", "Formula")
    {
        Some(Kind::Text)
    } else {
        Some(Kind::Number)
    }
}

/// The settings vislox offers on a new block of `ty`, in lox's order.
pub fn settings(ty: &str) -> Vec<(&'static str, Kind)> {
    connectors::attr_params(ty)
        .into_iter()
        .chain(connectors::message_params(ty).iter().copied())
        .filter_map(|p| Some((p, setting_kind(ty, p)?)))
        .collect()
}

/// The ports of `ty` a new block may give a constant: its parameters, and
/// its inputs (a constant until something is wired to them).
pub fn valued_ports(ty: &str) -> impl Iterator<Item = &'static connectors::PortSpec> {
    connectors::builtin(ty)
        .unwrap_or_default()
        .iter()
        .filter(|s| matches!(s.dir, PortDir::Input | PortDir::Param))
}

/// What lox's hover says about a type: its purpose, the link to Loxone's
/// documentation, and the label Loxone Config draws on each port. lox
/// keeps these tables to itself; its language server shows them, so they
/// are read from there. `None` where it says nothing.
struct TypeDoc {
    summary: Option<String>,
    docs: Option<String>,
    labels: BTreeMap<String, String>,
}

fn type_doc(ty: &str) -> TypeDoc {
    use loxone_config::lsp::analysis::Workspace;
    let path = std::path::PathBuf::from("/vislox-palette/types.loxl");
    let mut ws = Workspace::default();
    ws.open(path.clone(), format!("{ty}\n"));
    let md = ws.hover(&path, 0, 0).map(|(m, _)| m).unwrap_or_default();
    let mut doc = TypeDoc {
        summary: None,
        docs: None,
        labels: BTreeMap::new(),
    };
    for line in md.lines().skip(1) {
        let line = line.trim();
        if let Some(url) = line
            .strip_prefix("[Loxone documentation](")
            .and_then(|r| r.strip_suffix(')'))
        {
            doc.docs = Some(url.to_string());
        } else if let Some(rest) = line.strip_prefix("- `") {
            // - `Name` input (Label)
            if let Some((name, rest)) = rest.split_once('`')
                && let Some(label) = rest
                    .rfind(" (")
                    .map(|i| &rest[i + 2..])
                    .and_then(|l| l.strip_suffix(')'))
            {
                doc.labels.insert(name.to_string(), label.to_string());
            }
        } else if !line.is_empty() && doc.summary.is_none() && !line.starts_with("**") {
            doc.summary = Some(line.to_string());
        }
    }
    doc
}

/// `GET /api/block-types`: the types vislox offers on a page, each with
/// what lox knows of it: its purpose, its ports (the source name, the key
/// the config stores, the direction, the unit, the label Loxone Config
/// draws, the value a new block gets) and its settings.
pub fn palette() -> Json {
    let types: Vec<Json> = connectors::BUILTIN_TYPES
        .iter()
        .filter(|t| offered(t))
        .map(|&ty| {
            let doc = type_doc(ty);
            let ports: Vec<Json> = connectors::builtin(ty)
                .unwrap_or_default()
                .iter()
                .map(|s| {
                    json!({
                        "key": s.key,
                        "name": s.name,
                        "dir": match s.dir {
                            PortDir::Input => "input",
                            PortDir::Output => "output",
                            PortDir::Param => "param",
                            PortDir::Api => "api",
                        },
                        "unit": connectors::port_unit(ty, s.key).map(crate::edit::unit_suffix),
                        "label": doc.labels.get(s.name),
                        "default": loxone_config::gui_defaults::gui_default(ty, s.key).unwrap_or("0"),
                    })
                })
                .collect();
            let settings: Vec<Json> = settings(ty)
                .into_iter()
                .map(|(name, kind)| {
                    json!({
                        "name": name,
                        "kind": kind,
                        "choices": connectors::choice_values(ty, name)
                            .map(|c| c.iter().map(|(w, _)| *w).collect::<Vec<_>>()),
                        "empty": connectors::empty_ok(ty, name),
                    })
                })
                .collect();
            json!({
                "type": ty,
                "summary": doc.summary,
                "docs": doc.docs,
                "ports": ports,
                "settings": settings,
            })
        })
        .collect();
    let mut types = types;
    types.push(message_ref_type());
    json!({ "types": types })
}

/// The palette's push notification (#77): an `OutputRefLM` that mirrors a
/// `Notification` of Loxone Config's *Messages* (`mirrors`, picked from the
/// config's notifications by the browser), with its message. Its input is
/// wired once it is on the page (#61); it takes no name (the notification's)
/// and no constant.
fn message_ref_type() -> Json {
    let mut list =
        vec![json!({ "name": MIRRORS, "kind": "notification", "choices": null, "empty": false })];
    list.extend(settings(MESSAGE_REF).into_iter().map(|(name, kind)| {
        json!({ "name": name, "kind": kind, "choices": null, "empty": name != "On" })
    }));
    json!({
        "type": MESSAGE_REF,
        "summary": "Push notification: sends the notification's message when its input turns on, and off",
        "docs": null,
        "ports": [
            { "key": "AI", "name": "AI", "dir": "input", "unit": null, "label": null, "default": null, "wire_only": true },
            { "key": "AQ", "name": "AQ", "dir": "output", "unit": null, "label": null, "default": null },
        ],
        "settings": list,
        "mirrors": NOTIFICATION,
    })
}

// ── the files ───────────────────────────────────────────────────────────

/// One block vislox added: `slug = Type("Title", Port: value, …)`, or one
/// it adopted from Loxone Config (#72), which may also bind wires.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewBlock {
    pub slug: String,
    pub ty: String,
    /// The label; `None` shows the slug.
    pub title: Option<String>,
    /// Port (by its source name) or setting → value, in the order written.
    pub args: Vec<(String, Value)>,
    /// Adopted from Loxone Config (#72): it was deployed before vislox
    /// managed it, and keeps its UUIDs.
    pub adopted: bool,
    /// The wires into its inputs its declaration binds (`In1: vx_….AQ`, or
    /// a bare name for the source's default output): port → source. Only an
    /// adopted block has them; a block added here is wired by `<-` in
    /// `edits.loxl` (#61).
    pub wires: Vec<(String, PortRef)>,
    /// The expressions into its inputs (#82): port (by its source name) →
    /// the expression as lox prints it (`vx_a.Q and vx_b.AQ >= 28`). lox
    /// makes their blocks, named `<slug>_<port>__<op><n>`.
    pub exprs: Vec<(String, crate::expr_edit::ExprSrc)>,
}

/// A named signal of a page file (#82): `vx_warm = vx_a.AQ >= 28`, the
/// expression as lox prints it, and its label (`vx_warm.title = "…"`), none
/// showing the expression. Its top block is `slug` itself (hand-owned), the
/// rest `<slug>__<op><n>` (expression-owned).
#[derive(Clone, Debug, PartialEq)]
pub struct SignalDef {
    pub slug: String,
    pub expr: crate::expr_edit::ExprSrc,
    pub title: Option<String>,
}

impl SignalDef {
    pub fn render(&self) -> String {
        let mut out = format!("{} = {}", self.slug, self.expr);
        if let Some(t) = &self.title {
            out.push_str(&format!(
                "\n{}.title = {}",
                self.slug,
                Value::Str(t.clone())
            ));
        }
        out
    }
}

impl NewBlock {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.args.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Set `name`, or drop it with `None`.
    pub fn set(&mut self, name: &str, v: Option<Value>) {
        match (self.args.iter_mut().find(|(k, _)| k == name), v) {
            (Some(slot), Some(v)) => slot.1 = v,
            (None, Some(v)) => self.args.push((name.to_string(), v)),
            (_, None) => self.args.retain(|(k, _)| k != name),
        }
    }

    pub fn render(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = &self.title {
            parts.push(Value::Str(t.clone()).to_string());
        }
        for (k, v) in &self.args {
            parts.push(format!("{k}: {v}"));
        }
        for (k, src) in &self.wires {
            parts.push(format!("{k}: {src}"));
        }
        for (k, e) in &self.exprs {
            parts.push(format!("{k}: {e}"));
        }
        format!("{} = {}({})", self.slug, self.ty, parts.join(", "))
    }
}

/// One file of blocks vislox added: the page they are on, the blocks, and
/// the removals of blocks added here that lox has not forgotten yet.
#[derive(Clone, Debug, PartialEq)]
pub struct PageFile {
    /// The page's UUID, from a `# page uuid: …` line: the file follows its
    /// page when Loxone Config renames it (`page "…"` names it by title).
    pub uuid: Option<String>,
    pub page: String,
    /// `page "…" (room: …, category: …)`: kept as written (#88 writes them).
    pub room: Option<String>,
    pub category: Option<String>,
    pub blocks: Vec<NewBlock>,
    /// `removed <slug>`: a block of this file deleted (#59, #80), until the
    /// lockfile no longer has it.
    pub removed: BTreeSet<String>,
    /// `moved <old> -> <new>`: a block of this file renamed (#80), old slug →
    /// new, until the lockfile no longer has the old slug.
    pub moved: BTreeMap<String, String>,
    /// The wires out of the blocks adopted here onto objects Loxone Config
    /// drew (#72): `vx_<uuid>.Port <- slug` (an extern of `edits.loxl`), as
    /// `lox logic adopt` writes them. lox tracks them in the lockfile
    /// (`extern_wires`).
    pub wires: Vec<FileWire>,
    /// The named signals written here (#82).
    pub signals: Vec<SignalDef>,
    /// Of `removed`, the signals dropped here (#82): only making one again
    /// takes its line back. Written under a comment of their own.
    pub dropped: BTreeSet<String>,
}

/// `to <- from`: a wire of a page file, out of a block adopted there.
#[derive(Clone, Debug, PartialEq)]
pub struct FileWire {
    pub to: PortRef,
    pub from: PortRef,
}

impl FileWire {
    pub fn render(&self) -> String {
        format!("{} <- {}", self.to, self.from)
    }
}

/// The comment that ties a file to its page: `# page uuid: …`.
const UUID_LINE: &str = "page uuid:";

/// The comment above the blocks adopted from Loxone Config (#72): the
/// blocks after it are adopted, not added.
pub const ADOPTED_LINE: &str = "Adopted from Loxone Config";

/// The comment above the `removed` lines of signals dropped here (#82).
pub const DROPPED_LINE: &str = "Signals dropped here";

fn foreign(what: &str) -> String {
    format!("it holds {what}, which vislox did not write; move it to another file of the module")
}

impl PageFile {
    pub fn new(page: &str) -> PageFile {
        PageFile {
            uuid: None,
            page: page.to_string(),
            room: None,
            category: None,
            blocks: Vec::new(),
            removed: BTreeSet::new(),
            moved: BTreeMap::new(),
            wires: Vec::new(),
            signals: Vec::new(),
            dropped: BTreeSet::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
            && self.removed.is_empty()
            && self.moved.is_empty()
            && self.signals.is_empty()
    }

    /// Read a file vislox wrote. Only what vislox writes is accepted, so
    /// that a rewrite never drops a statement someone added by hand.
    #[cfg(test)]
    pub fn parse(text: &str) -> Result<PageFile, String> {
        PageFile::parse_with(text, &BTreeSet::new())
    }

    /// [`PageFile::parse`], knowing vislox's constants (#81): a bare name of
    /// one on an adopted block is its value, not a wire from an object.
    pub fn parse_with(text: &str, consts: &BTreeSet<String>) -> Result<PageFile, String> {
        // A fragment: an adopted block names the externs of `edits.loxl`.
        let m = Module::parse_fragment(text).map_err(|e| e.to_string())?;
        let mut file: Option<PageFile> = None;
        let mut uuid = None;
        let mut adopted = false;
        // Under the comment of the signals dropped here (#82).
        let mut dropping = false;
        for item in m.items {
            match item {
                Item::Comment(c) if file.is_none() && c.trim().starts_with(UUID_LINE) => {
                    uuid = Some(c.trim()[UUID_LINE.len()..].trim().to_string());
                }
                Item::Comment(c) if file.is_some() && c.trim().starts_with(ADOPTED_LINE) => {
                    adopted = true;
                    dropping = false;
                }
                Item::Comment(c) if file.is_some() => {
                    dropping = c.trim().starts_with(DROPPED_LINE);
                }
                Item::Page(p) if file.is_none() => {
                    file = Some(PageFile {
                        uuid: uuid.take(),
                        room: p.room,
                        category: p.category,
                        ..PageFile::new(&p.title)
                    })
                }
                Item::Block(b) => {
                    let f = file
                        .as_mut()
                        .ok_or_else(|| foreign(&format!("`{}` before its page line", b.slug)))?;
                    if !b.slug.starts_with(SLUG_PREFIX) {
                        return Err(foreign(&format!("`{}`", b.slug)));
                    }
                    let mut args = Vec::new();
                    let mut wires = Vec::new();
                    let mut exprs = Vec::new();
                    for a in &b.args {
                        match a {
                            // `mirrors:` names the object a ref is (a push
                            // notification's `OutputRefLM`, #77): a value,
                            // never a wire.
                            ArgItem::Binding(Binding {
                                port,
                                kind: BindingKind::Param(v @ Value::Ref(_)),
                                ..
                            }) if port == MIRRORS => args.push((port.clone(), v.clone())),
                            // A bare name of vislox's on an adopted block: the
                            // default output of the object it names, unless it
                            // is one of vislox's constants (#81). Any other bare
                            // name is a value: a `let` constant, a day word.
                            ArgItem::Binding(Binding {
                                port,
                                kind: BindingKind::Param(Value::Ref(src)),
                                ..
                            }) if adopted
                                && src.starts_with(SLUG_PREFIX)
                                && !consts.contains(src) =>
                            {
                                wires.push((
                                    port.clone(),
                                    PortRef {
                                        slug: src.clone(),
                                        port: String::new(),
                                    },
                                ))
                            }
                            ArgItem::Binding(Binding {
                                port,
                                kind: BindingKind::Param(v),
                                ..
                            }) => args.push((port.clone(), v.clone())),
                            ArgItem::Binding(Binding {
                                port,
                                kind: BindingKind::Wire(src),
                                ..
                            }) if adopted => wires.push((port.clone(), src.clone())),
                            // An expression into an input (#82).
                            ArgItem::Binding(Binding {
                                port,
                                kind: BindingKind::Expr(e),
                                ..
                            }) => exprs.push((port.clone(), crate::expr_edit::ExprSrc::of(e))),
                            ArgItem::Binding(_) => {
                                return Err(foreign(&format!("a wire onto `{}`", b.slug)));
                            }
                            ArgItem::Comment(_) => {}
                        }
                    }
                    f.blocks.push(NewBlock {
                        slug: b.slug,
                        ty: b.block_type,
                        title: b.title,
                        args,
                        adopted,
                        wires,
                        exprs,
                    });
                }
                // A named signal (#82), and its label.
                Item::Signal(x) if x.slug.starts_with(SLUG_PREFIX) => {
                    file.as_mut()
                        .ok_or_else(|| foreign(&format!("`{}` before its page line", x.slug)))?
                        .signals
                        .push(SignalDef {
                            slug: x.slug,
                            expr: crate::expr_edit::ExprSrc::of(&x.expr),
                            title: None,
                        });
                }
                Item::Attr(a)
                    if a.attr == Attr::Title
                        && matches!(a.value, Value::Str(_))
                        && file.as_ref().is_some_and(|f| {
                            f.signals
                                .iter()
                                .any(|x| x.slug == a.slug && x.title.is_none())
                        }) =>
                {
                    let Value::Str(t) = a.value else {
                        unreachable!("matched")
                    };
                    let f = file.as_mut().expect("checked");
                    if let Some(x) = f.signals.iter_mut().find(|x| x.slug == a.slug) {
                        x.title = Some(t);
                    }
                }
                // A wire out of a block adopted here (#72).
                Item::Wire(WireDecl { to, from, .. })
                    if adopted
                        && !to.port.is_empty()
                        && file.as_ref().is_some_and(|f| {
                            f.blocks.iter().any(|b| b.adopted && b.slug == from.slug)
                        }) =>
                {
                    file.as_mut()
                        .expect("checked")
                        .wires
                        .push(FileWire { to, from });
                }
                Item::Removed(r) if r.slug.starts_with(SLUG_PREFIX) => {
                    let f = file
                        .as_mut()
                        .ok_or_else(|| foreign("`removed` before the page line"))?;
                    if dropping {
                        f.dropped.insert(r.slug.clone());
                    }
                    f.removed.insert(r.slug);
                }
                Item::Moved(m)
                    if m.from.starts_with(SLUG_PREFIX) && m.to.starts_with(SLUG_PREFIX) =>
                {
                    file.as_mut()
                        .ok_or_else(|| foreign("`moved` before the page line"))?
                        .moved
                        .insert(m.from, m.to);
                }
                Item::Gap | Item::FragmentEnd | Item::Comment(_) => {}
                _ => return Err(foreign("a statement")),
            }
        }
        file.ok_or_else(|| "it has no page line".to_string())
    }

    pub fn render(&self) -> String {
        let page = PageDecl {
            title: self.page.clone(),
            room: self.room.clone(),
            category: self.category.clone(),
            comment: None,
        };
        let mut out = String::from(
            "# Blocks added in vislox on this page. vislox rewrites this file on every edit;\n\
             # see docs/editing.md in vislox.\n",
        );
        if let Some(u) = &self.uuid {
            out.push_str(&format!("# {UUID_LINE} {u}\n"));
        }
        out.push_str(&render_page(&page));
        out.push('\n');
        let (adopted, added): (Vec<&NewBlock>, Vec<&NewBlock>) =
            self.blocks.iter().partition(|b| b.adopted);
        if !added.is_empty() {
            out.push('\n');
        }
        for b in added {
            out.push_str(&b.render());
            out.push('\n');
        }
        if !adopted.is_empty() {
            out.push_str(&format!(
                "\n# {ADOPTED_LINE}: drawn there, managed here since; they keep their UUIDs.\n"
            ));
        }
        for b in adopted {
            out.push_str(&b.render());
            out.push('\n');
            for w in self.wires.iter().filter(|w| w.from.slug == b.slug) {
                out.push_str(&w.render());
                out.push('\n');
            }
        }
        if !self.signals.is_empty() {
            out.push_str(
                "\n# Named signals written in vislox: each names an expression's result.\n",
            );
        }
        for x in &self.signals {
            out.push_str(&x.render());
            out.push('\n');
        }
        let (signals, deleted): (Vec<&String>, Vec<&String>) =
            self.removed.iter().partition(|s| self.dropped.contains(*s));
        if !deleted.is_empty() {
            out.push_str("\n# Deleted here: lox takes them out of the config.\n");
        }
        for s in deleted {
            out.push_str(&format!("removed {s}\n"));
        }
        if !signals.is_empty() {
            out.push_str(&format!(
                "\n# {DROPPED_LINE}: lox takes them out of the config.\n"
            ));
        }
        for s in signals {
            out.push_str(&format!("removed {s}\n"));
        }
        if !self.moved.is_empty() {
            out.push_str("\n# Renamed here: the blocks keep their UUIDs.\n");
        }
        for (from, to) in &self.moved {
            out.push_str(&format!("moved {from} -> {to}\n"));
        }
        out
    }
}

/// `page "…"` for a page titled `title`, as a file of vislox's has it.
pub fn render_page_line(title: &str) -> String {
    render_page(&PageDecl {
        title: title.to_string(),
        room: None,
        category: None,
        comment: None,
    })
}

fn render_page(p: &PageDecl) -> String {
    let mut args = Vec::new();
    if let Some(r) = &p.room {
        args.push(format!("room: {}", Value::Str(r.clone())));
    }
    if let Some(c) = &p.category {
        args.push(format!("category: {}", Value::Str(c.clone())));
    }
    let t = Value::Str(p.title.clone());
    if args.is_empty() {
        format!("page {t}")
    } else {
        format!("page {t} ({})", args.join(", "))
    }
}

/// The file name for the blocks of page `title`: its title in plain
/// letters, digits and dashes (`Küche Licht` → `kueche-licht.loxl`), unless
/// another page's file has that name already.
pub fn file_name(title: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let mut s = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => s.push_str("ae"),
            'ö' => s.push_str("oe"),
            'ü' => s.push_str("ue"),
            'ß' => s.push_str("ss"),
            c if c.is_ascii_alphanumeric() => s.push(c),
            _ if !s.ends_with('-') => s.push('-'),
            _ => {}
        }
    }
    let mut stem: String = s.trim_matches('-').chars().take(40).collect();
    stem = stem.trim_end_matches('-').to_string();
    // `edits.loxl` is the file of #58's claims.
    if stem.is_empty() || stem == "edits" {
        stem = format!("page-{stem}").trim_end_matches('-').to_string();
    }
    let mut name = format!("{stem}.loxl");
    let mut n = 2;
    while taken(&name) {
        name = format!("{stem}-{n}.loxl");
        n += 1;
    }
    name
}

/// A new slug for a block of `ty`: `vx_<type>_<n>`, the lowest `n` no name
/// of the module, the lockfile (its tombstones included) or vislox's files
/// has.
pub fn new_slug(ty: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let stem: String = ty
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(|c| c.to_lowercase())
        .collect();
    (1..)
        .map(|n| format!("{SLUG_PREFIX}{stem}_{n}"))
        .find(|s| !taken(s))
        .expect("an unused slug")
}

/// The source name of port `key` of `ty`, if a new block may give it a
/// constant.
pub fn port_name(ty: &str, key: &str) -> Option<&'static str> {
    valued_ports(ty).find(|s| s.key == key).map(|s| s.name)
}

/// The key of the port `name` (a source name) of `ty`.
pub fn port_key<'a>(ty: &str, name: &'a str) -> &'a str {
    port_names::key_or_raw(ty, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_offers_page_blocks_lox_creates() {
        assert!(offered("And") && offered("Formula") && offered("PulseAt"));
        // The object tree, and blocks a device carries.
        assert!(!offered("VirtualIn") && !offered("ModbusDev") && !offered("Mode"));
        assert!(!offered("NfcCodeTouch") && !offered("TpfController"));
        // Set up in Loxone Config before they do anything.
        assert!(!offered("Alarm") && !offered("Irrigation") && !offered("CentralLight"));
        assert!(!offered("Banana"));
        let p = palette();
        let types = p["types"].as_array().unwrap();
        // A push notification (#77): picked by the notification it mirrors.
        let lm = types.iter().find(|t| t["type"] == "OutputRefLM").unwrap();
        let names: Vec<_> = lm["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["mirrors", "On", "Off", "MinimumTime"]);
        assert_eq!(lm["settings"][3]["kind"], "duration");
        let and = types.iter().find(|t| t["type"] == "And").unwrap();
        assert!(
            and["summary"].as_str().unwrap().contains("AND"),
            "lox's hover changed: {and}"
        );
        let names: Vec<_> = and["ports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["In1", "In2", "Q"]);
        let mono = types.iter().find(|t| t["type"] == "Monoflop").unwrap();
        let time = mono["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["key"] == "Time")
            .unwrap();
        assert_eq!(time["unit"], "s");
        let pulse = types.iter().find(|t| t["type"] == "PulseAt").unwrap();
        let kinds: Vec<_> = pulse["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["kind"].as_str().unwrap()))
            .collect();
        assert!(kinds.contains(&("At", "time")), "{kinds:?}");
        assert!(kinds.contains(&("Pulse", "choice")), "{kinds:?}");
        // A program's text (#79): written into a file of its own.
        let code = types.iter().find(|t| t["type"] == "Code16").unwrap();
        assert!(
            code["settings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == "Code" && s["kind"] == "program")
        );
        assert_eq!(setting_kind("Formula", "Formula"), Some(Kind::Text));
    }

    #[test]
    fn a_page_file_round_trips() {
        let mut f = PageFile::new("Küche \"Licht\"");
        f.uuid = Some("10000008-0000-0080-ffff504f94000001".into());
        f.blocks.push(NewBlock {
            slug: "vx_and_1".into(),
            ty: "And".into(),
            title: Some("Licht an".into()),
            args: vec![("In2".into(), Value::Number("1".into()))],
            ..Default::default()
        });
        f.blocks.push(NewBlock {
            slug: "vx_or_1".into(),
            ty: "Or".into(),
            title: None,
            args: vec![],
            ..Default::default()
        });
        f.removed.insert("vx_not_1".into());
        let text = f.render();
        assert!(text.contains("page \"Küche \\\"Licht\\\"\"\n"), "{text}");
        assert!(
            text.contains("# page uuid: 10000008-0000-0080-ffff504f94000001\npage "),
            "{text}"
        );
        assert!(
            text.contains("vx_and_1 = And(\"Licht an\", In2: 1)\n"),
            "{text}"
        );
        assert!(text.contains("vx_or_1 = Or()\n"), "{text}");
        assert!(text.contains("removed vx_not_1\n"), "{text}");
        assert_eq!(PageFile::parse(&text).unwrap(), f);
    }

    #[test]
    fn an_adopted_block_round_trips_with_its_wires() {
        let mut f = PageFile::new("Erdgeschoss");
        f.uuid = Some("10000002-0000-0020-ffff504f94000001".into());
        f.blocks.push(NewBlock {
            slug: "vx_and_1".into(),
            ty: "And".into(),
            ..Default::default()
        });
        let ext = |s: &str, p: &str| PortRef {
            slug: s.into(),
            port: p.into(),
        };
        f.blocks.push(NewBlock {
            slug: "vx_gate".into(),
            ty: "And".into(),
            title: Some("O1001".into()),
            args: vec![("In2".into(), Value::Number("1".into()))],
            adopted: true,
            wires: vec![
                (
                    "In1".into(),
                    ext("vx_10000001_0000_0010_ffff504f94000001", ""),
                ),
                (
                    "In3".into(),
                    ext("vx_10000006_0000_0060_ffff504f94000001", "AQ"),
                ),
            ],
            exprs: vec![],
        });
        f.wires.push(FileWire {
            to: ext("vx_10000005_0000_0050_ffff504f94000001", "Power"),
            from: ext("vx_gate", ""),
        });
        let text = f.render();
        assert!(
            text.contains(
                "vx_gate = And(\"O1001\", In2: 1, In1: vx_10000001_0000_0010_ffff504f94000001, \
                 In3: vx_10000006_0000_0060_ffff504f94000001.AQ)\n\
                 vx_10000005_0000_0050_ffff504f94000001.Power <- vx_gate\n"
            ),
            "{text}"
        );
        assert!(text.contains("# Adopted from Loxone Config"), "{text}");
        assert_eq!(PageFile::parse(&text).unwrap(), f);
        // Before the adopted comment, a block with a wire is not vislox's.
        assert!(
            PageFile::parse("page \"A\"\nvx_and_1 = And(In1: x.Q)\n")
                .unwrap_err()
                .contains("did not write")
        );
        // A wire out of a block that is not adopted here is not either.
        assert!(
            PageFile::parse("page \"A\"\n# Adopted from Loxone Config\nvx_b.In1 <- vx_c\n")
                .unwrap_err()
                .contains("did not write")
        );
    }

    /// `mirrors:` on a push notification's block is a value naming
    /// vislox's extern, on a block added or adopted here, never a wire (#77).
    #[test]
    fn mirrors_is_a_value_not_a_wire() {
        let text = "page \"Meldungen\"\n\nvx_outputreflm_1 = OutputRefLM(mirrors: vx_6000, On: \"a\")\n\n\
                    # Adopted from Loxone Config: drawn there, managed here since; they keep their UUIDs.\n\
                    vx_frost = OutputRefLM(mirrors: vx_6001, AI: vx_x.Q, On: \"b\", MinimumTime: 12h)\n";
        let f = PageFile::parse(text).unwrap();
        for (b, n) in f.blocks.iter().zip(["vx_6000", "vx_6001"]) {
            assert_eq!(b.get(MIRRORS), Some(&Value::Ref(n.into())));
            assert!(b.wires.iter().all(|(p, _)| p != MIRRORS));
        }
        assert_eq!(f.blocks[1].wires.len(), 1);
        assert!(f.render().contains(
            "vx_frost = OutputRefLM(mirrors: vx_6001, On: \"b\", MinimumTime: 12h, AI: vx_x.Q)"
        ));
        assert_eq!(PageFile::parse(&f.render()).unwrap(), f);
        // The message is edited as settings: texts and a duration.
        assert_eq!(
            settings("OutputRefLM"),
            [
                ("On", Kind::Text),
                ("Off", Kind::Text),
                ("MinimumTime", Kind::Duration)
            ]
        );
    }

    #[test]
    fn statements_vislox_did_not_write_are_refused() {
        let bad = |t: &str| PageFile::parse(t).unwrap_err();
        assert!(bad("page \"A\"\nlicht = And()\n").contains("did not write"));
        assert!(
            bad("page \"A\"\nvx_and_1 = And(In1: x.Q)\nextern x = And(uuid: \"u\")\n")
                .contains("did not write")
        );
        assert!(bad("vx_and_1 = And()\n").contains("page line"));
        assert!(bad("# nothing\n").contains("no page line"));
    }

    #[test]
    fn names_are_plain_and_unique() {
        let none = |_: &str| false;
        assert_eq!(file_name("Küche Licht", &none), "kueche-licht.loxl");
        assert_eq!(file_name("  ", &none), "page.loxl");
        assert_eq!(file_name("Edits", &none), "page-edits.loxl");
        let taken = |n: &str| n == "bad.loxl" || n == "bad-2.loxl";
        assert_eq!(file_name("Bad", &taken), "bad-3.loxl");
        let used = |s: &str| s == "vx_greaterequal_1";
        assert_eq!(new_slug("GreaterEqual", &used), "vx_greaterequal_2");
        assert_eq!(new_slug("And", &none), "vx_and_1");
    }
}
