//! The view model the browser draws: every page with its blocks, ports,
//! wires and off-page links, a properties record per object, the project
//! tree, and — when a base revision is given — what changed against it.

use loxone_config::diff::{self, DocDiff};
use loxone_config::doc::{LoxoneDoc, PortView, WireView, ports};
use loxone_config::layout::{GeoObject, Layout, PageGeometry, ROW, Shape, Side};
use loxone_config::port_names;
use loxone_config::xml::Element;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const ADDED: &str = "added";
const REMOVED: &str = "removed";
const CHANGED: &str = "changed";
const MOVED: &str = "moved";

/// Attributes that are geometry, compared separately.
const GEOMETRY: &[&str] = &["Px", "Py", "Px2", "Py2"];

#[derive(Serialize, Default)]
pub struct View {
    pub title: String,
    pub pages: Vec<Page>,
    pub tree: Vec<TreeNode>,
    pub objects: BTreeMap<String, ObjInfo>,
    pub changes: Vec<Change>,
    pub stats: Stats,
    pub compared: bool,
    /// How values flow where the Miniserver streams none (live values).
    pub live: LiveGraph,
}

/// What the browser needs to give a wire a live value the Miniserver does
/// not stream: references and memory flags carry another connector's value,
/// constants are in the config, and simple gates can be computed from their
/// inputs. Keyed by output connector UUID.
#[derive(Serialize, Default)]
pub struct LiveGraph {
    /// Output → the connector whose value it passes on (a reference or a
    /// memory flag, from its input's wire).
    pub pass: BTreeMap<String, String>,
    /// Output of a reference → the object it refers to, whose app control
    /// may stream its value.
    pub refs: BTreeMap<String, String>,
    /// Output → its constant value.
    pub consts: BTreeMap<String, f64>,
    /// Output → the stateless gate that computes it.
    pub gates: BTreeMap<String, Gate>,
    /// A lighting controller's Mood output → the controller, whose app
    /// control streams its active moods.
    pub moods: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct Gate {
    /// `and`, `or`, `xor`, `not`, `eq`, `ne`, `gt`, `ge`, `lt`, `le`, `add`,
    /// `sub`, `mul`, `div`.
    pub op: &'static str,
    pub ins: Vec<GateIn>,
}

#[derive(Serialize)]
pub struct GateIn {
    /// The wire's source, if wired.
    pub src: Option<String>,
    /// The parameter value, if not wired.
    pub def: Option<f64>,
    /// The input is inverted (wired), or a constant 1 (unwired).
    pub inv: bool,
}

#[derive(Serialize, Default)]
pub struct Stats {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub moved: usize,
    pub wires_added: usize,
    pub wires_removed: usize,
}

#[derive(Serialize)]
pub struct Page {
    pub uuid: String,
    pub title: String,
    pub old_title: Option<String>,
    pub status: &'static str,
    pub changes: usize,
    pub objects: Vec<PObj>,
    pub wires: Vec<PWire>,
    pub labels: Vec<PLabel>,
    pub sheet: Option<[i64; 4]>,
}

#[derive(Serialize)]
pub struct PObj {
    pub uuid: String,
    pub ty: String,
    pub title: String,
    pub shape: &'static str,
    pub rect: [i64; 4],
    /// Where it was before a move.
    pub old_rect: Option<[i64; 4]>,
    pub color: Option<String>,
    pub status: &'static str,
    pub ports: Vec<PPort>,
    /// The object a ref tag mirrors.
    pub target: Option<Target>,
    /// A ghost of an object that now sits on another page.
    pub moved_to: Option<Target>,
}

#[derive(Serialize)]
pub struct PPort {
    pub key: String,
    pub name: String,
    pub side: &'static str,
    pub x: i64,
    pub y: i64,
    pub def: Option<String>,
    pub inv: bool,
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct PWire {
    pub from: [String; 2],
    pub to: [String; 2],
    pub points: Vec<[i64; 2]>,
    pub analog: bool,
    pub status: &'static str,
    /// The source connector: its UUID is the state the Miniserver streams.
    pub src: String,
}

#[derive(Serialize)]
pub struct PLabel {
    pub obj: String,
    pub port: String,
    pub side: &'static str,
    pub x: i64,
    pub y: i64,
    pub target: Target,
    /// The signal flows into this page's port (the far end is the source).
    pub incoming: bool,
    pub status: &'static str,
    /// The source connector (live value).
    pub src: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Target {
    pub uuid: String,
    pub title: String,
    pub ty: String,
    /// Short address (`I1`, `VI3`, `AQ1`).
    pub iname: Option<String>,
    pub port: Option<String>,
    pub page: Option<String>,
    pub page_title: Option<String>,
    /// Folder path for objects that are not on a page (periphery).
    pub path: Option<String>,
    pub removed: bool,
}

#[derive(Serialize)]
pub struct ObjInfo {
    pub uuid: String,
    pub ty: String,
    pub title: String,
    pub old_title: Option<String>,
    pub status: &'static str,
    pub page: Option<String>,
    pub path: String,
    pub room: Option<String>,
    pub category: Option<String>,
    pub attrs: Vec<AttrInfo>,
    pub ports: Vec<PortInfo>,
    /// Ref tags that mirror this object: (page uuid, ref object uuid).
    pub mirrored_by: Vec<[String; 2]>,
    pub target: Option<Target>,
}

#[derive(Serialize)]
pub struct AttrInfo {
    pub name: String,
    pub value: Option<String>,
    pub old: Option<String>,
    pub changed: bool,
}

#[derive(Serialize)]
pub struct PortInfo {
    pub key: String,
    pub uuid: String,
    pub name: String,
    pub def: Option<String>,
    pub old_def: Option<String>,
    pub def_changed: bool,
    pub inv: bool,
    pub conns: Vec<Conn>,
}

#[derive(Serialize)]
pub struct Conn {
    /// `in`: the far port feeds this one; `out`: this port feeds it.
    pub dir: &'static str,
    pub target: Target,
    pub status: &'static str,
    /// The wire's source connector (live value).
    pub src: String,
}

#[derive(Serialize)]
pub struct TreeNode {
    pub uuid: String,
    pub title: String,
    pub ty: String,
    pub status: &'static str,
    pub page: bool,
    pub children: Vec<TreeNode>,
}

#[derive(Serialize)]
pub struct Change {
    pub kind: &'static str,
    pub text: String,
    pub obj: Option<String>,
    pub page: Option<String>,
}

fn rect(l: &Layout) -> [i64; 4] {
    [l.px, l.py, l.w(), l.h()]
}

fn shape_name(s: Shape) -> &'static str {
    match s {
        Shape::Tag => "tag",
        Shape::Small => "small",
        Shape::Large => "large",
    }
}

fn side_name(s: Side) -> &'static str {
    match s {
        Side::Left => "L",
        Side::Right => "R",
    }
}

/// Loxone draws analog connections differently from digital ones; the
/// connector key is a good tell (`AQ`, `AI`, `AQ1`, `TargetPos`, …).
fn analog_key(key: &str) -> bool {
    let k = key.as_bytes();
    (k.len() >= 2 && k[0] == b'A' && (k[1] == b'Q' || k[1] == b'I'))
        || matches!(key, "Txt" | "TQ" | "V" | "Pos" | "Value" | "Temp")
}

/// One document, indexed for the view.
struct Ctx<'a> {
    doc: &'a LoxoneDoc,
    /// uuid → element
    els: HashMap<String, &'a Element>,
    /// uuid → nearest enclosing object's uuid
    parent: HashMap<String, String>,
    /// uuid → page uuid, for objects placed on a page
    page_of: HashMap<String, String>,
    /// page uuids in document order
    pages: Vec<String>,
    /// port uuid → (owner uuid, key)
    port_owner: HashMap<String, (String, String)>,
    wires: Vec<WireView>,
    /// ref target uuid → refs (page, ref uuid)
    mirrors: HashMap<String, Vec<[String; 2]>>,
    /// Document child order of each object, for the tree.
    order: Vec<String>,
    /// Ports with at least one wire.
    wired: HashSet<String>,
}

impl<'a> Ctx<'a> {
    fn new(doc: &'a LoxoneDoc) -> Ctx<'a> {
        let mut c = Ctx {
            doc,
            els: HashMap::new(),
            parent: HashMap::new(),
            page_of: HashMap::new(),
            pages: Vec::new(),
            port_owner: HashMap::new(),
            wires: doc.wires(),
            mirrors: HashMap::new(),
            order: Vec::new(),
            wired: HashSet::new(),
        };
        c.wired = c
            .wires
            .iter()
            .flat_map(|w| [w.from_port.clone(), w.to_port.clone()])
            .collect();
        c.walk(&doc.xml.root, None, None);
        c
    }

    fn walk(&mut self, el: &'a Element, parent: Option<&str>, page: Option<&str>) {
        for child in el.child_elements().filter(|e| e.name == "C") {
            let Some(uuid) = child.attr("U") else {
                continue;
            };
            let ty = child.attr("Type").unwrap_or("");
            self.els.insert(uuid.to_string(), child);
            self.order.push(uuid.to_string());
            if let Some(p) = parent {
                self.parent.insert(uuid.to_string(), p.to_string());
            }
            if let Some(pg) = page {
                self.page_of.insert(uuid.to_string(), pg.to_string());
                if let Some(r) = child.attr("Ref") {
                    self.mirrors
                        .entry(r.to_string())
                        .or_default()
                        .push([pg.to_string(), uuid.to_string()]);
                }
            }
            for p in ports(child) {
                self.port_owner
                    .insert(p.uuid.clone(), (uuid.to_string(), p.key.clone()));
            }
            let is_page = ty == "Page";
            if is_page {
                self.pages.push(uuid.to_string());
            }
            let child_page = if is_page { Some(uuid) } else { None };
            self.walk(child, Some(uuid), child_page);
        }
    }

    fn el(&self, uuid: &str) -> Option<&'a Element> {
        self.els.get(uuid).copied()
    }

    fn title(&self, uuid: &str) -> String {
        self.el(uuid)
            .and_then(|e| e.attr_decoded("Title").map(|t| t.into_owned()))
            .filter(|t| !t.is_empty())
            .or_else(|| self.el(uuid).and_then(|e| e.attr("Type").map(String::from)))
            .unwrap_or_else(|| uuid.to_string())
    }

    fn ty(&self, uuid: &str) -> String {
        self.el(uuid)
            .and_then(|e| e.attr("Type"))
            .unwrap_or("")
            .to_string()
    }

    /// Folder path of an object, e.g. `Miniserver / Virtual inputs`.
    fn path(&self, uuid: &str) -> String {
        let mut parts = Vec::new();
        let mut cur = self.parent.get(uuid);
        while let Some(p) = cur {
            if self.ty(p) == "Document" {
                break;
            }
            parts.push(self.title(p));
            cur = self.parent.get(p);
        }
        parts.reverse();
        parts.join(" / ")
    }

    fn target(&self, uuid: &str, port: Option<&str>) -> Target {
        let page = self.page_of.get(uuid).cloned();
        Target {
            uuid: uuid.to_string(),
            title: self.title(uuid),
            ty: self.ty(uuid),
            iname: self
                .el(uuid)
                .and_then(|e| e.attr_decoded("IName").map(|t| t.into_owned())),
            port: port.map(|p| port_names::name_of(&self.ty(uuid), p).to_string()),
            page_title: page.as_ref().map(|p| self.title(p)),
            path: if page.is_none() {
                Some(self.path(uuid))
            } else {
                None
            },
            page,
            removed: false,
        }
    }

    fn port_target(&self, port_uuid: &str) -> Option<Target> {
        let (owner, key) = self.port_owner.get(port_uuid)?;
        Some(self.target(owner, Some(key)))
    }

    /// Title of the Place/Category an `<IoData>` binding names.
    fn binding(&self, el: &Element, attr: &str) -> Option<String> {
        let io = el.child_elements().find(|c| c.name == "IoData")?;
        let id = io.attr(attr)?;
        Some(
            self.el(id)
                .map(|_| self.title(id))
                .unwrap_or_else(|| id.to_string()),
        )
    }
}

/// Loxone Config lets the user hide unused connectors, and saves the block
/// at the height of the rows it shows. The geometry model draws every
/// connector of a known type, so a block with hidden connectors comes out
/// taller than Loxone draws it and runs into its neighbours. When the saved
/// height is smaller, keep it: show the wired connectors (they cannot be
/// hidden), then unwired ones in order while rows are left.
fn fit_written(g: &GeoObject, el: &Element, wired: &HashSet<String>) -> GeoObject {
    let mut g = g.clone();
    let Some(written) = Layout::of_element(el) else {
        return g;
    };
    if g.shape == Shape::Tag {
        return g;
    }
    // Math blocks (Divide, Average, …) are small in Loxone Config whatever
    // the model says: the saved width tells.
    let mut width = g.rect.w();
    if g.shape == Shape::Large && written.w() <= Shape::Small.width() {
        g.shape = Shape::Small;
        width = written.w();
    }
    // Footprint height = base + ROW × rows (see layout::Shape::height).
    let base = match g.shape {
        Shape::Small => 312,
        _ => 504,
    };
    let capacity = ((written.h() - base) / ROW).max(1);
    let mut rows_used = 0;
    for side in [Side::Left, Side::Right] {
        let on_side: Vec<usize> = (0..g.ports.len())
            .filter(|&i| g.ports[i].side == side)
            .collect();
        let n_wired = on_side
            .iter()
            .filter(|&&i| wired.contains(&g.ports[i].uuid))
            .count() as i64;
        let hide = on_side.len() as i64 > capacity;
        let mut spare = (capacity - n_wired).max(0);
        let mut keep = Vec::new();
        for &i in &on_side {
            if !hide || wired.contains(&g.ports[i].uuid) {
                keep.push(i);
            } else if spare > 0 {
                spare -= 1;
                keep.push(i);
            }
        }
        for (row, &i) in keep.iter().enumerate() {
            g.ports[i].row = row as i64;
        }
        rows_used = rows_used.max(keep.len() as i64);
        let drop: HashSet<usize> = on_side.into_iter().filter(|i| !keep.contains(i)).collect();
        g.ports = g
            .ports
            .iter()
            .enumerate()
            .filter(|(i, _)| !drop.contains(i))
            .map(|(_, p)| p.clone())
            .collect();
    }
    let h = (base + ROW * rows_used.max(1)).max(written.h());
    g.rect = Layout::at(written.px, written.py, width, h);
    g
}

fn layout_of(el: &Element) -> Option<Layout> {
    Layout::of_element(el)
}

fn attrs_of(el: &Element) -> BTreeMap<String, String> {
    el.attrs
        .iter()
        .filter(|a| !GEOMETRY.contains(&a.name.as_str()) && a.name != "U")
        .map(|a| {
            (
                a.name.clone(),
                el.attr_decoded(&a.name)
                    .map(|v| v.into_owned())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

/// The orthogonal route of a wire, as Loxone Config draws it: out of the
/// source, across at the midpoint, into the sink; a wire running right to
/// left loops around.
fn route(
    a: (i64, i64),
    b: (i64, i64),
    ra: &Layout,
    rb: &Layout,
    leaves_right: bool,
    enters_left: bool,
) -> Vec<[i64; 2]> {
    const LOOP: i64 = 240;
    let ((x1, y1), (x2, y2)) = (a, b);
    if leaves_right && enters_left && x2 - x1 >= 2 * LOOP {
        let xm = (x1 + x2) / 2;
        return vec![[x1, y1], [xm, y1], [xm, y2], [x2, y2]];
    }
    let out = if leaves_right { x1 + LOOP } else { x1 - LOOP };
    let into = if enters_left { x2 - LOOP } else { x2 + LOOP };
    let ym = if ra.py2 <= rb.py || rb.py2 <= ra.py {
        (ra.py2.min(rb.py2) + ra.py.max(rb.py)) / 2
    } else {
        ra.py2.max(rb.py2) + ROW
    };
    vec![
        [x1, y1],
        [out, y1],
        [out, ym],
        [into, ym],
        [into, y2],
        [x2, y2],
    ]
}

/// One changed attribute: (name, old value, new value).
type AttrDelta = (String, Option<String>, Option<String>);

/// What changed between two documents, keyed for lookups.
#[derive(Default)]
struct Delta {
    on: bool,
    added: HashSet<String>,
    removed: HashSet<String>,
    renamed: HashMap<String, Option<String>>,
    /// (object, port key) → old Def
    params: HashMap<(String, String), Option<String>>,
    /// object → attr changes (name, old, new)
    attrs: HashMap<String, Vec<AttrDelta>>,
    moved: HashMap<String, Layout>,
    /// Objects now on another page than before.
    repaged: HashSet<String>,
    wires_added: HashSet<(String, String)>,
    wires_removed: HashSet<(String, String)>,
    placements: HashSet<String>,
}

impl Delta {
    fn status(&self, uuid: &str) -> &'static str {
        if !self.on {
            ""
        } else if self.added.contains(uuid) {
            ADDED
        } else if self.removed.contains(uuid) {
            REMOVED
        } else if self.renamed.contains_key(uuid)
            || self.attrs.contains_key(uuid)
            || self.placements.contains(uuid)
            || self.params.keys().any(|(o, _)| o == uuid)
        {
            CHANGED
        } else if self.moved.contains_key(uuid) {
            MOVED
        } else {
            ""
        }
    }

    fn wire(&self, from: &str, to: &str) -> &'static str {
        let k = (from.to_string(), to.to_string());
        if self.wires_added.contains(&k) {
            ADDED
        } else if self.wires_removed.contains(&k) {
            REMOVED
        } else {
            ""
        }
    }
}

fn delta(new: &Ctx, old: &Ctx, d: &DocDiff) -> Delta {
    let mut x = Delta {
        on: true,
        ..Delta::default()
    };
    x.added = d.added.iter().map(|o| o.uuid.clone()).collect();
    x.removed = d.removed.iter().map(|o| o.uuid.clone()).collect();
    // Objects the semantic diff skips (users, hardware) still count.
    for u in new.els.keys() {
        if !old.els.contains_key(u) {
            x.added.insert(u.clone());
        }
    }
    for u in old.els.keys() {
        if !new.els.contains_key(u) {
            x.removed.insert(u.clone());
        }
    }
    for r in &d.renamed {
        x.renamed.insert(r.uuid.clone(), r.from.clone());
    }
    for p in &d.placements {
        x.placements.insert(p.object_uuid.clone());
    }
    for w in &d.wires_added {
        x.wires_added
            .insert((w.from_port.clone(), w.to_port.clone()));
    }
    for w in &d.wires_removed {
        x.wires_removed
            .insert((w.from_port.clone(), w.to_port.clone()));
    }
    // Wires the semantic diff folds away still show on the page.
    let nw: HashSet<(String, String)> = new
        .wires
        .iter()
        .map(|w| (w.from_port.clone(), w.to_port.clone()))
        .collect();
    let ow: HashSet<(String, String)> = old
        .wires
        .iter()
        .map(|w| (w.from_port.clone(), w.to_port.clone()))
        .collect();
    x.wires_added.extend(nw.difference(&ow).cloned());
    x.wires_removed.extend(ow.difference(&nw).cloned());

    for (u, nel) in &new.els {
        let Some(oel) = old.el(u) else { continue };
        let (na, oa) = (attrs_of(nel), attrs_of(oel));
        let mut ch = Vec::new();
        for k in na.keys().chain(oa.keys()).collect::<BTreeSet<_>>() {
            if k == "Title" && x.renamed.contains_key(u) {
                continue;
            }
            if na.get(k) != oa.get(k) {
                ch.push((k.clone(), oa.get(k).cloned(), na.get(k).cloned()));
            }
        }
        if k_title_changed(&na, &oa) && !x.renamed.contains_key(u) {
            x.renamed.insert(u.clone(), oa.get("Title").cloned());
            ch.retain(|(k, ..)| k != "Title");
        }
        if !ch.is_empty() {
            x.attrs.insert(u.clone(), ch);
        }
        let (np, op): (Vec<PortView>, Vec<PortView>) = (ports(nel), ports(oel));
        for p in &np {
            let od = op
                .iter()
                .find(|q| q.key == p.key)
                .and_then(|q| q.def.clone());
            if od != p.def {
                x.params.insert((u.clone(), p.key.clone()), od);
            }
        }
        if let (Some(a), Some(b)) = (layout_of(nel), layout_of(oel)) {
            if new.page_of.get(u) != old.page_of.get(u) {
                x.repaged.insert(u.clone());
                x.moved.insert(u.clone(), b);
            } else if (a.px, a.py) != (b.px, b.py) {
                x.moved.insert(u.clone(), b);
            }
        }
    }
    x
}

fn k_title_changed(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> bool {
    a.get("Title") != b.get("Title")
}

/// Build the view of `new`, compared with `old` when given.
pub fn build(new: &LoxoneDoc, old: Option<&LoxoneDoc>, title: &str) -> View {
    let nc = Ctx::new(new);
    let oc = old.map(Ctx::new);
    let d = match (&oc, old) {
        (Some(oc), Some(od)) => delta(&nc, oc, &diff::diff(od, new)),
        _ => Delta::default(),
    };

    let mut view = View {
        title: title.to_string(),
        compared: d.on,
        ..View::default()
    };

    // Pages: the new document's, then removed ones.
    let mut page_ids: Vec<(String, bool)> = nc.pages.iter().map(|p| (p.clone(), false)).collect();
    if let Some(oc) = &oc {
        for p in &oc.pages {
            if !nc.els.contains_key(p) {
                page_ids.push((p.clone(), true));
            }
        }
    }
    for (uuid, gone) in page_ids {
        let page = if gone {
            page_view(oc.as_ref().unwrap(), None, &uuid, &d, true)
        } else {
            page_view(&nc, oc.as_ref(), &uuid, &d, false)
        };
        if let Some(p) = page {
            view.pages.push(p);
        }
    }

    // Properties of every object.
    for u in &nc.order {
        view.objects
            .insert(u.clone(), obj_info(&nc, oc.as_ref(), u, &d, false));
    }
    if let Some(oc) = &oc {
        for u in &oc.order {
            if !nc.els.contains_key(u) {
                view.objects
                    .insert(u.clone(), obj_info(oc, None, u, &d, true));
            }
        }
    }

    view.live = live_graph(&nc);
    view.tree = tree(&nc, oc.as_ref(), &d);
    if d.on {
        changes(&mut view, &nc, oc.as_ref().unwrap(), &d);
    }
    view
}

fn page_view(c: &Ctx, old: Option<&Ctx>, uuid: &str, d: &Delta, gone: bool) -> Option<Page> {
    let geo = PageGeometry::of(c.doc, uuid)?;
    let old_geo = old.and_then(|o| PageGeometry::of(o.doc, uuid));
    let mut objects = Vec::new();
    // port uuid → (x, y, side, rect)
    let mut port_pos: HashMap<String, ((i64, i64), Side, Layout)> = HashMap::new();
    let add_obj = |g: &GeoObject,
                   ctx: &Ctx,
                   status: &'static str,
                   objects: &mut Vec<PObj>,
                   port_pos: &mut HashMap<String, ((i64, i64), Side, Layout)>| {
        let el = ctx.el(&g.uuid);
        let fitted = el.map(|e| fit_written(g, e, &ctx.wired));
        let g = fitted.as_ref().unwrap_or(g);
        let defs: HashMap<String, PortView> = el
            .map(|e| ports(e).into_iter().map(|p| (p.uuid.clone(), p)).collect())
            .unwrap_or_default();
        let mut pp = Vec::new();
        for (i, p) in g.ports.iter().enumerate() {
            let (x, y) = g.port_point(i);
            port_pos
                .entry(p.uuid.clone())
                .or_insert(((x, y), p.side, g.rect));
            let pv = defs.get(&p.uuid);
            pp.push(PPort {
                key: p.key.clone(),
                name: port_names::name_of(&g.block_type, &p.key).to_string(),
                side: side_name(p.side),
                x,
                y,
                def: pv.and_then(|v| v.def.clone()),
                inv: pv.is_some_and(|v| v.inv),
                status: if d.on && d.params.contains_key(&(g.uuid.clone(), p.key.clone())) {
                    CHANGED
                } else {
                    ""
                },
            });
        }
        let target = el.and_then(|e| e.attr("Ref")).map(|r| ctx.target(r, None));
        let title = match (&target, el.and_then(|e| e.attr("Title"))) {
            (Some(t), None) => t.title.clone(),
            _ => g.title.clone(),
        };
        objects.push(PObj {
            uuid: g.uuid.clone(),
            ty: g.block_type.clone(),
            title,
            shape: shape_name(g.shape),
            rect: rect(&g.rect),
            old_rect: d
                .moved
                .get(&g.uuid)
                .filter(|_| !d.repaged.contains(&g.uuid))
                .map(rect),
            color: el.and_then(|e| e.attr("Cl")).map(String::from),
            status,
            ports: pp,
            target,
            moved_to: None,
        });
    };

    for g in &geo.objects {
        let status = if gone { REMOVED } else { d.status(&g.uuid) };
        add_obj(g, c, status, &mut objects, &mut port_pos);
    }
    // Ghosts: objects that were on this page and are not any more.
    if let (Some(og), Some(oc)) = (&old_geo, old) {
        let here: HashSet<&str> = geo.objects.iter().map(|o| o.uuid.as_str()).collect();
        for g in &og.objects {
            if !here.contains(g.uuid.as_str()) {
                add_obj(g, oc, REMOVED, &mut objects, &mut port_pos);
                if c.page_of.contains_key(&g.uuid) {
                    let ghost = objects.last_mut().unwrap();
                    ghost.status = MOVED;
                    ghost.moved_to = Some(c.target(&g.uuid, None));
                }
            }
        }
    }

    // Wires drawn on the page, and links to elsewhere.
    let mut wires = Vec::new();
    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    let tag_objs: HashSet<String> = objects
        .iter()
        .filter(|o| o.shape == "tag")
        .map(|o| o.uuid.clone())
        .collect();
    let mut emit = |w: &WireView,
                    ctx: &Ctx,
                    status: &'static str,
                    wires: &mut Vec<PWire>,
                    labels: &mut Vec<PLabel>| {
        let a = port_pos.get(&w.from_port);
        let b = port_pos.get(&w.to_port);
        let key = |p: &str| ctx.port_owner.get(p).cloned().unwrap_or_default();
        let (fo, fk) = key(&w.from_port);
        let (to, tk) = key(&w.to_port);
        match (a, b) {
            (Some(&(pa, sa, ra)), Some(&(pb, sb, rb))) => {
                if fo == to || !seen.insert((w.from_port.clone(), w.to_port.clone())) {
                    return;
                }
                // Orient source (right side) → sink (left side).
                let (pa, sa, ra, pb, sb, rb, from, to_) = if sa == Side::Left && sb == Side::Right {
                    (
                        pb,
                        sb,
                        rb,
                        pa,
                        sa,
                        ra,
                        [to.clone(), tk.clone()],
                        [fo.clone(), fk.clone()],
                    )
                } else {
                    (
                        pa,
                        sa,
                        ra,
                        pb,
                        sb,
                        rb,
                        [fo.clone(), fk.clone()],
                        [to.clone(), tk.clone()],
                    )
                };
                wires.push(PWire {
                    points: route(pa, pb, &ra, &rb, sa == Side::Right, sb == Side::Left),
                    analog: analog_key(&from[1]),
                    from,
                    to: to_,
                    status,
                    src: w.from_port.clone(),
                });
            }
            (Some(&(p, s, _)), None) | (None, Some(&(p, s, _))) => {
                let here_is_from = a.is_some();
                let (here_obj, here_key, far_port) = if here_is_from {
                    (fo.clone(), fk.clone(), &w.to_port)
                } else {
                    (to.clone(), tk.clone(), &w.from_port)
                };
                // A ref tag *is* the drawing of its far end.
                if tag_objs.contains(&here_obj) {
                    return;
                }
                if !seen.insert((w.from_port.clone(), w.to_port.clone())) {
                    return;
                }
                let mut target = ctx.port_target(far_port).unwrap_or_default();
                target.removed = status == REMOVED;
                labels.push(PLabel {
                    obj: here_obj,
                    port: here_key,
                    side: side_name(s),
                    x: p.0,
                    y: p.1,
                    target,
                    incoming: !here_is_from,
                    status,
                    src: w.from_port.clone(),
                });
            }
            _ => {}
        }
    };
    for w in &c.wires {
        let status = if gone {
            REMOVED
        } else {
            d.wire(&w.from_port, &w.to_port)
        };
        emit(w, c, status, &mut wires, &mut labels);
    }
    if let Some(oc) = old {
        for w in &oc.wires {
            if d.wires_removed
                .contains(&(w.from_port.clone(), w.to_port.clone()))
            {
                emit(w, oc, REMOVED, &mut wires, &mut labels);
            }
        }
    }
    let _ = geo.labels; // recomputed above with their targets

    let old_title = old
        .filter(|_| d.renamed.contains_key(uuid))
        .map(|o| o.title(uuid));
    let status = if gone {
        REMOVED
    } else if d.added.contains(uuid) {
        ADDED
    } else {
        ""
    };
    let changes = objects.iter().filter(|o| !o.status.is_empty()).count()
        + wires.iter().filter(|w| !w.status.is_empty()).count()
        + labels.iter().filter(|l| !l.status.is_empty()).count()
        + objects
            .iter()
            .flat_map(|o| &o.ports)
            .filter(|p| !p.status.is_empty())
            .count();
    Some(Page {
        uuid: uuid.to_string(),
        title: geo.title.clone(),
        old_title,
        status: if status.is_empty() && changes > 0 {
            CHANGED
        } else {
            status
        },
        changes,
        objects,
        wires,
        labels,
        sheet: geo.sheet.as_ref().map(rect),
    })
}

fn obj_info(c: &Ctx, old: Option<&Ctx>, uuid: &str, d: &Delta, gone: bool) -> ObjInfo {
    let el = c.el(uuid).expect("indexed object");
    let ty = c.ty(uuid);
    let attr_changes = d.attrs.get(uuid);
    let mut attrs: Vec<AttrInfo> = attrs_of(el)
        .into_iter()
        .map(|(name, value)| {
            let ch = attr_changes.and_then(|v| v.iter().find(|(n, ..)| *n == name));
            AttrInfo {
                old: ch.and_then(|(_, o, _)| o.clone()),
                changed: ch.is_some(),
                name,
                value: Some(value),
            }
        })
        .collect();
    if let Some(v) = attr_changes {
        for (n, o, new) in v {
            if new.is_none() {
                attrs.push(AttrInfo {
                    name: n.clone(),
                    value: None,
                    old: o.clone(),
                    changed: true,
                });
            }
        }
    }
    let mut port_infos = Vec::new();
    for p in ports(el) {
        let mut conns = Vec::new();
        let mut push = |w: &WireView, ctx: &Ctx, status: &'static str| {
            if w.to_port == p.uuid {
                if let Some(t) = ctx.port_target(&w.from_port) {
                    conns.push(Conn {
                        dir: "in",
                        target: t,
                        status,
                        src: w.from_port.clone(),
                    });
                }
            } else if w.from_port == p.uuid
                && let Some(t) = ctx.port_target(&w.to_port)
            {
                conns.push(Conn {
                    dir: "out",
                    target: t,
                    status,
                    src: w.from_port.clone(),
                });
            }
        };
        for w in &c.wires {
            let st = if gone {
                REMOVED
            } else {
                d.wire(&w.from_port, &w.to_port)
            };
            push(w, c, st);
        }
        if let Some(oc) = old {
            for w in &oc.wires {
                if d.wires_removed
                    .contains(&(w.from_port.clone(), w.to_port.clone()))
                {
                    push(w, oc, REMOVED);
                }
            }
        }
        let k = (uuid.to_string(), p.key.clone());
        port_infos.push(PortInfo {
            name: port_names::name_of(&ty, &p.key).to_string(),
            def_changed: d.on && d.params.contains_key(&k),
            old_def: d.params.get(&k).cloned().flatten(),
            def: p.def.clone(),
            inv: p.inv,
            uuid: p.uuid.clone(),
            key: p.key,
            conns,
        });
    }
    let status = if gone { REMOVED } else { d.status(uuid) };
    ObjInfo {
        uuid: uuid.to_string(),
        title: c.title(uuid),
        old_title: d.renamed.get(uuid).cloned().flatten(),
        status,
        page: c.page_of.get(uuid).cloned(),
        path: c.path(uuid),
        room: c.binding(el, "Pr"),
        category: c.binding(el, "Cr"),
        attrs,
        ports: port_infos,
        mirrored_by: c.mirrors.get(uuid).cloned().unwrap_or_default(),
        target: el.attr("Ref").map(|r| c.target(r, None)),
        ty,
    }
}

fn tree(c: &Ctx, old: Option<&Ctx>, d: &Delta) -> Vec<TreeNode> {
    fn node(
        c: &Ctx,
        uuid: &str,
        d: &Delta,
        kids: &HashMap<String, Vec<String>>,
        status: &'static str,
    ) -> TreeNode {
        let ty = c.ty(uuid);
        let children = kids
            .get(uuid)
            .map(|v| {
                v.iter()
                    .map(|k| {
                        node(
                            c,
                            k,
                            d,
                            kids,
                            if status == REMOVED {
                                REMOVED
                            } else {
                                d.status(k)
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        TreeNode {
            uuid: uuid.to_string(),
            title: c.title(uuid),
            page: ty == "Page",
            ty,
            status,
            children,
        }
    }
    let kids = |ctx: &Ctx| {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for u in &ctx.order {
            if let Some(p) = ctx.parent.get(u) {
                m.entry(p.clone()).or_default().push(u.clone());
            }
        }
        m
    };
    let nk = kids(c);
    let root: Vec<&String> = c
        .order
        .iter()
        .filter(|u| !c.parent.contains_key(*u))
        .collect();
    let mut out: Vec<TreeNode> = root
        .iter()
        .map(|u| node(c, u, d, &nk, d.status(u)))
        .collect();
    // Removed objects under their former parent, where it still exists.
    if let Some(oc) = old {
        let ok = kids(oc);
        fn insert(n: &mut TreeNode, parent: &str, child: TreeNode) -> Option<TreeNode> {
            if n.uuid == parent {
                n.children.push(child);
                return None;
            }
            let mut child = Some(child);
            for k in &mut n.children {
                child = insert(k, parent, child.take()?);
            }
            child
        }
        for u in &oc.order {
            if c.els.contains_key(u) {
                continue;
            }
            let Some(p) = oc.parent.get(u) else { continue };
            if !c.els.contains_key(p) {
                continue; // drawn with its removed parent
            }
            let mut n = Some(node(oc, u, d, &ok, REMOVED));
            for r in &mut out {
                n = insert(r, p, n.take().unwrap());
                if n.is_none() {
                    break;
                }
            }
        }
    }
    // Skip the single Document root: show its children.
    if out.len() == 1 && out[0].ty == "Document" {
        return std::mem::take(&mut out[0].children);
    }
    out
}

fn changes(view: &mut View, nc: &Ctx, oc: &Ctx, d: &Delta) {
    let mut v = Vec::new();
    let page_for = |u: &str| nc.page_of.get(u).or_else(|| oc.page_of.get(u)).cloned();
    let name = |ctx: &Ctx, u: &str| format!("{} ({})", ctx.title(u), ctx.ty(u));
    let sorted = |set: &HashSet<String>, ctx: &Ctx| {
        let mut x: Vec<String> = set
            .iter()
            .filter(|u| ctx.els.contains_key(*u))
            .cloned()
            .collect();
        x.sort_by_key(|u| ctx.order.iter().position(|o| o == u));
        x
    };
    // Leave out ports' owners' children noise: only report top objects whose parent is not also added.
    for u in sorted(&d.added, nc) {
        if nc.parent.get(&u).is_some_and(|p| d.added.contains(p)) {
            continue;
        }
        view.stats.added += 1;
        v.push(Change {
            kind: ADDED,
            text: name(nc, &u),
            page: page_for(&u),
            obj: Some(u),
        });
    }
    for u in sorted(&d.removed, oc) {
        if oc.parent.get(&u).is_some_and(|p| d.removed.contains(p)) {
            continue;
        }
        view.stats.removed += 1;
        v.push(Change {
            kind: REMOVED,
            text: name(oc, &u),
            page: page_for(&u),
            obj: Some(u),
        });
    }
    let mut changed: Vec<String> = nc
        .order
        .iter()
        .filter(|u| d.status(u) == CHANGED)
        .cloned()
        .collect();
    changed.dedup();
    for u in changed {
        view.stats.changed += 1;
        if let Some(Some(from)) = d.renamed.get(&u) {
            v.push(Change {
                kind: "renamed",
                text: format!("{from} → {}", nc.title(&u)),
                page: page_for(&u),
                obj: Some(u.clone()),
            });
        }
        let ty = nc.ty(&u);
        let mut params: Vec<_> = d.params.iter().filter(|((o, _), _)| *o == u).collect();
        params.sort_by(|a, b| a.0.1.cmp(&b.0.1));
        for ((_, key), old) in params {
            let new = nc
                .el(&u)
                .and_then(|e| ports(e).into_iter().find(|p| &p.key == key))
                .and_then(|p| p.def);
            v.push(Change {
                kind: "param",
                text: format!(
                    "{} · {}: {} → {}",
                    nc.title(&u),
                    port_names::name_of(&ty, key),
                    old.as_deref().unwrap_or("—"),
                    new.as_deref().unwrap_or("—")
                ),
                page: page_for(&u),
                obj: Some(u.clone()),
            });
        }
        if let Some(a) = d.attrs.get(&u) {
            for (n, o, new) in a {
                v.push(Change {
                    kind: "attr",
                    text: attr_change_text(
                        &nc.title(&u),
                        &nc.ty(&u),
                        n,
                        o.as_deref(),
                        new.as_deref(),
                    ),
                    page: page_for(&u),
                    obj: Some(u.clone()),
                });
            }
        }
        if d.placements.contains(&u) {
            v.push(Change {
                kind: "attr",
                text: format!("{} · room/category binding", nc.title(&u)),
                page: page_for(&u),
                obj: Some(u.clone()),
            });
        }
    }
    let wire_text = |ctx: &Ctx, w: &(String, String)| {
        let end = |p: &str| {
            ctx.port_owner
                .get(p)
                .map(|(o, k)| format!("{}.{}", ctx.title(o), port_names::name_of(&ctx.ty(o), k)))
                .unwrap_or_else(|| p.to_string())
        };
        format!("{} → {}", end(&w.0), end(&w.1))
    };
    let wire_page = |ctx: &Ctx, w: &(String, String)| {
        let owner = |p: &str| ctx.port_owner.get(p).map(|(o, _)| o.clone());
        owner(&w.1)
            .and_then(|o| ctx.page_of.get(&o).cloned())
            .or_else(|| owner(&w.0).and_then(|o| ctx.page_of.get(&o).cloned()))
    };
    let wire_obj =
        |ctx: &Ctx, w: &(String, String)| ctx.port_owner.get(&w.1).map(|(o, _)| o.clone());
    let mut wa: Vec<_> = d.wires_added.iter().collect();
    wa.sort();
    for w in wa {
        view.stats.wires_added += 1;
        v.push(Change {
            kind: "wire+",
            text: wire_text(nc, w),
            page: wire_page(nc, w),
            obj: wire_obj(nc, w),
        });
    }
    let mut wr: Vec<_> = d.wires_removed.iter().collect();
    wr.sort();
    for w in wr {
        view.stats.wires_removed += 1;
        v.push(Change {
            kind: "wire-",
            text: wire_text(oc, w),
            page: wire_page(oc, w),
            obj: wire_obj(oc, w),
        });
    }
    let mut moved: Vec<_> = d
        .moved
        .keys()
        .filter(|u| d.status(u) == MOVED)
        .cloned()
        .collect();
    moved.sort_by_key(|u| nc.order.iter().position(|o| o == u));
    for u in moved {
        view.stats.moved += 1;
        let (from, to) = (oc.page_of.get(&u), nc.page_of.get(&u));
        let text = match (from, to) {
            (Some(a), Some(b)) if a != b => {
                format!("{} · page {} → {}", nc.title(&u), oc.title(a), nc.title(b))
            }
            _ => nc.title(&u),
        };
        v.push(Change {
            kind: MOVED,
            text,
            page: page_for(&u),
            obj: Some(u),
        });
    }
    view.changes = v;
}

/// One changed attribute, for the change list. Long or multi-line text
/// (notes, program code) shows the lines that changed, not two cut-off
/// copies of the same beginning.
fn attr_change_text(
    title: &str,
    ty: &str,
    name: &str,
    old: Option<&str>,
    new: Option<&str>,
) -> String {
    let label = if ty == "Text" && name == "Text" {
        "note"
    } else {
        name
    };
    let long = |v: Option<&str>| v.is_some_and(|v| v.contains('\n') || v.chars().count() > 40);
    if !(long(old) || long(new)) {
        return format!("{title} · {label}: {} → {}", short(old), short(new));
    }
    let clean = |v: Option<&str>| v.unwrap_or("").replace('\r', "");
    let lines: Vec<String> = diff::text_changes(&clean(old), &clean(new))
        .into_iter()
        .skip(1)
        .filter(|l| l.trim() != "+" && l.trim() != "-")
        .take(4)
        .collect();
    if lines.is_empty() {
        format!("{title} · {label}: whitespace changed")
    } else {
        format!("{title} · {label}: {}", lines.join(" · "))
    }
}

fn short(s: Option<&str>) -> String {
    match s {
        None => "—".into(),
        Some(s) if s.chars().count() > 40 => format!("{}…", s.chars().take(40).collect::<String>()),
        Some(s) => s.to_string(),
    }
}

/// The gate op of a block type, and its inputs in order.
fn gate_of(ty: &str) -> Option<(&'static str, &'static [&'static str])> {
    const LOGIC: &[&str] = &["I1", "I2", "I3", "I4", "I5", "I6", "I7", "I8"];
    const TWO: &[&str] = &["Input1", "Input2"];
    Some(match ty {
        "And" => ("and", LOGIC),
        "Or" => ("or", LOGIC),
        "Xor" => ("xor", LOGIC),
        "Not" => ("not", &["I"]),
        "Equal" => ("eq", TWO),
        "NotEqual" => ("ne", TWO),
        "Greater" => ("gt", TWO),
        "GreaterEqual" => ("ge", TWO),
        "Less" => ("lt", TWO),
        "LessEqual" => ("le", TWO),
        "Add" => ("add", TWO),
        "Sub" => ("sub", TWO),
        "Mult" => ("mul", TWO),
        "Div" => ("div", TWO),
        _ => return None,
    })
}

fn live_graph(c: &Ctx) -> LiveGraph {
    let mut g = LiveGraph::default();
    for el in c.els.values() {
        let Some(ty) = el.attr("Type") else {
            continue;
        };
        let ps = ports(el);
        let by = |k: &str| ps.iter().find(|p| p.key == k);
        let src = |k: &str| by(k).and_then(|p| p.inputs.first().cloned());
        let outs = |keys: &[&str]| -> Vec<String> {
            keys.iter()
                .filter_map(|k| by(k))
                .map(|p| p.uuid.clone())
                .collect()
        };
        match ty {
            // A reference's input is wired from the object it shows.
            "InputRef" => {
                let from = src("AI").or_else(|| src("I"));
                for o in outs(&["AQ", "Q"]) {
                    if let Some(f) = &from {
                        g.pass.insert(o.clone(), f.clone());
                    }
                    if let Some(r) = el.attr("Ref") {
                        g.refs.insert(o, r.to_string());
                    }
                }
            }
            // An output reference sends what is wired to it on to the object
            // it refers to (a mode, a system variable, an actuator).
            "OutputRef" | "OutputRefLM" => {
                for (i, o) in [("AI", "AQ"), ("I", "Q")] {
                    if let (Some(f), Some(p)) = (src(i), by(o)) {
                        g.pass.insert(p.uuid.clone(), f);
                    }
                }
                if let Some(r) = el.attr("Ref") {
                    for o in outs(&["AQ", "Q"]) {
                        g.refs.insert(o, r.to_string());
                    }
                }
            }
            // A mode is on while its input (set by an output reference) is.
            "Mode" => {
                if let (Some(f), Some(p)) = (src("I"), by("Q")) {
                    g.pass.insert(p.uuid.clone(), f);
                }
            }
            // A system variable set from the program, when it does not scale.
            "SysVar" => {
                let num = |k: &str, d: f64| el.attr(k).and_then(|v| v.parse().ok()).unwrap_or(d);
                let same = num("SourceValLow", 0.0) == num("DestValLow", 0.0)
                    && num("SourceValHigh", 1.0) == num("DestValHigh", 1.0);
                if let (true, Some(f), Some(p)) = (same, src("AI"), by("AQ")) {
                    g.pass.insert(p.uuid.clone(), f);
                }
            }
            // A memory flag holds what its input is set to.
            "Memory" => {
                if let Some(f) = src("Input") {
                    for o in outs(&["AQ", "Q"]) {
                        g.pass.insert(o, f.clone());
                    }
                }
            }
            "LightController2" => {
                if let (Some(p), Some(u)) = (by("Scene"), el.attr("U")) {
                    g.moods.insert(p.uuid.clone(), u.to_string());
                }
            }
            "Constant" => {
                let v = el
                    .attr("AnalogConstant")
                    .or_else(|| el.attr("DigitalConstant"))
                    .and_then(|v| v.parse::<f64>().ok());
                if let Some(v) = v {
                    for o in outs(&["AQ", "Q"]) {
                        g.consts.insert(o, v);
                    }
                }
            }
            _ => {
                let Some((op, keys)) = gate_of(ty) else {
                    continue;
                };
                let ins = keys
                    .iter()
                    .filter_map(|k| by(k))
                    .map(|p| GateIn {
                        src: p.inputs.first().cloned(),
                        def: p.def.as_deref().and_then(|d| d.parse().ok()),
                        inv: p.inv,
                    })
                    .collect();
                let out = by("Q").or_else(|| by("AQ"));
                if let Some(o) = out {
                    g.gates.insert(o.uuid.clone(), Gate { op, ins });
                }
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUSE: &str = include_str!("../tests/fixtures/house.Loxone");
    const AND: &str = "10000004-0000-0040-ffff504f94000001";
    const METER: &str = "10000005-0000-0050-ffff504f94000001";
    const DIV: &str = "10000006-0000-0060-ffff504f94000001";
    const NOTE: &str = "10000007-0000-0070-ffff504f94000001";
    const OR: &str = "10000009-0000-0090-ffff504f94000001";
    const PAGE_EG: &str = "10000002-0000-0020-ffff504f94000001";
    const PAGE_OG: &str = "10000008-0000-0080-ffff504f94000001";

    fn doc(src: &str) -> LoxoneDoc {
        LoxoneDoc::parse(src.as_bytes()).expect("fixture parses")
    }

    fn page<'a>(v: &'a View, uuid: &str) -> &'a Page {
        v.pages.iter().find(|p| p.uuid == uuid).expect("page")
    }

    fn obj<'a>(p: &'a Page, uuid: &str) -> &'a PObj {
        p.objects
            .iter()
            .find(|o| o.uuid == uuid && o.moved_to.is_none())
            .expect("object on page")
    }

    /// The fixture with each `(from, to)` replacement applied exactly once.
    fn edited(edits: &[(&str, &str)]) -> String {
        let mut s = HOUSE.to_string();
        for (from, to) in edits {
            assert_eq!(
                s.matches(from).count(),
                1,
                "edit target `{from}` must be unique"
            );
            s = s.replacen(from, to, 1);
        }
        s
    }

    #[test]
    fn live_graph_passes_references_on_and_knows_the_gates() {
        let v = build(&doc(HOUSE), None, "t");
        let g = &v.live;
        // The Temperatur reference passes on VirtualIn VI1's output.
        assert_eq!(
            g.pass["10000003-0000-0032-01ff504f94000001"],
            "10000001-0000-0011-00ff504f94000001"
        );
        assert_eq!(
            g.refs["10000003-0000-0032-01ff504f94000001"],
            "10000001-0000-0010-ffff504f94000001"
        );
        let and = &g.gates["10000004-0000-0043-02ff504f94000001"];
        assert_eq!(and.op, "and");
        assert_eq!(and.ins.len(), 2);
        assert_eq!(
            and.ins[0].src.as_deref(),
            Some("10000003-0000-0032-01ff504f94000001")
        );
        assert!(and.ins[1].src.is_none());
        assert!(g.moods.is_empty());
        let div = &g.gates["10000006-0000-0063-02ff504f94000001"];
        assert_eq!(div.op, "div");
        assert_eq!(div.ins[1].def, Some(1000.0));
    }

    #[test]
    fn live_graph_follows_output_references_through_modes() {
        // Push button Q → OutputRef → Mode; the Mode's Q → InputRef elsewhere.
        // A system variable that scales does not pass its input on.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ControlList Version="1">
	<C Type="Document" V="1" U="d" Title="T">
		<C Type="Mode" V="1" U="m" Title="Night">
			<Co K="I" Nc="1" U="m-i"><In Input="o-aq"/></Co>
			<Co K="Q" U="m-q"/>
		</C>
		<C Type="SysVar" V="1" U="s" Title="Scaled" SourceValHigh="10" DestValHigh="100">
			<Co K="AI" Nc="1" U="s-ai"><In Input="o-aq"/></Co>
			<Co K="AQ" U="s-aq"/>
			<Co K="Q" U="s-q"/>
		</C>
		<C Type="Page" V="1" U="p" Title="P">
			<C Type="PushButton" V="1" U="b" Title="Night" Px="0" Py="0" Px2="960" Py2="960">
				<Co K="Q" U="b-q"/>
			</C>
			<C Type="OutputRef" V="1" U="o" Title="Night" Ref="m" Px="2000" Py="0" Px2="3000" Py2="192">
				<Co K="AI" Nc="1" U="o-ai"><In Input="b-q"/></Co>
				<Co K="AQ" U="o-aq"/>
			</C>
			<C Type="InputRef" V="1" U="r" Title="Night" Ref="m" Px="0" Py="2000" Px2="1000" Py2="2192">
				<Co K="AI" Nc="1" U="r-ai"><In Input="m-q"/></Co>
				<Co K="Q" U="r-q"/>
			</C>
		</C>
	</C>
</ControlList>"#;
        let g = build(&doc(xml), None, "t").live;
        assert_eq!(g.pass["o-aq"], "b-q");
        assert_eq!(g.refs["o-aq"], "m");
        assert_eq!(g.pass["m-q"], "o-aq");
        assert_eq!(g.pass["r-q"], "m-q");
        assert!(!g.pass.contains_key("s-aq"));
    }

    #[test]
    fn plain_view_has_pages_blocks_and_no_marks() {
        let v = build(&doc(HOUSE), None, "Testhaus");
        assert_eq!(v.title, "Testhaus");
        assert!(!v.compared);
        let titles: Vec<&str> = v.pages.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Erdgeschoss", "Obergeschoss"]);
        let eg = page(&v, PAGE_EG);
        assert_eq!(eg.objects.len(), 5);
        assert!(eg.objects.iter().all(|o| o.status.is_empty()));
        assert!(v.changes.is_empty());
        assert_eq!(eg.wires.len(), 3, "ref→AND, ref→Div, AND→meter");
    }

    #[test]
    fn ref_tag_names_what_it_mirrors() {
        let v = build(&doc(HOUSE), None, "");
        let tag = obj(page(&v, PAGE_EG), "10000003-0000-0030-ffff504f94000001");
        assert_eq!(tag.shape, "tag");
        let t = tag.target.as_ref().expect("ref target");
        assert_eq!(t.title, "Temperatur");
        assert_eq!(t.iname.as_deref(), Some("VI1"));
        assert_eq!(t.page, None, "a virtual input lives in the periphery");
        assert_eq!(t.path.as_deref(), Some("Virtuelle Eingänge"));
    }

    #[test]
    fn wire_to_another_page_becomes_a_link_on_both_pages() {
        let v = build(&doc(HOUSE), None, "");
        let out = page(&v, PAGE_EG)
            .labels
            .iter()
            .find(|l| l.obj == AND && l.port == "Q")
            .expect("link at AND.Q");
        assert!(!out.incoming);
        assert_eq!(out.target.uuid, OR);
        assert_eq!(out.target.page.as_deref(), Some(PAGE_OG));
        assert_eq!(out.target.page_title.as_deref(), Some("Obergeschoss"));

        let inc = page(&v, PAGE_OG)
            .labels
            .iter()
            .find(|l| l.obj == OR)
            .expect("link at OR.I1");
        assert!(inc.incoming);
        assert_eq!(inc.target.uuid, AND);
        assert_eq!(inc.target.page.as_deref(), Some(PAGE_EG));
    }

    #[test]
    fn blocks_keep_the_size_loxone_saved() {
        let v = build(&doc(HOUSE), None, "");
        let eg = page(&v, PAGE_EG);
        // Saved 1272 high: header plus four rows. Wired Pf stays, unwired outputs fill the rows left.
        let meter = obj(eg, METER);
        assert_eq!(meter.rect, [6720, 768, 2688, 1272]);
        assert!(meter.ports.iter().any(|p| p.key == "Pf"));
        assert!(meter.ports.iter().filter(|p| p.side == "R").count() <= 4);
        // Divide is saved at the small block width.
        let div = obj(eg, DIV);
        assert_eq!(div.shape, "small");
        assert_eq!(div.rect[2], 1344);
        // Ports sit on the block's edges.
        for p in &div.ports {
            let edge = if p.side == "L" {
                div.rect[0]
            } else {
                div.rect[0] + div.rect[2]
            };
            assert_eq!(p.x, edge, "{} on its edge", p.key);
        }
    }

    #[test]
    fn note_carries_its_text() {
        let v = build(&doc(HOUSE), None, "");
        let info = &v.objects[NOTE];
        assert_eq!(info.ty, "Text");
        let text = info
            .attrs
            .iter()
            .find(|a| a.name == "Text")
            .and_then(|a| a.value.as_deref());
        assert_eq!(text, Some("Erste Zeile\nZweite Zeile"));
    }

    #[test]
    fn properties_list_connections_both_ways() {
        let v = build(&doc(HOUSE), None, "");
        let and = &v.objects[AND];
        assert_eq!(and.page.as_deref(), Some(PAGE_EG));
        let q = and.ports.iter().find(|p| p.key == "Q").expect("Q");
        let targets: Vec<&str> = q.conns.iter().map(|c| c.target.uuid.as_str()).collect();
        assert!(targets.contains(&METER) && targets.contains(&OR));
        assert!(q.conns.iter().all(|c| c.dir == "out"));
        let div = &v.objects[DIV];
        let input2 = div
            .ports
            .iter()
            .find(|p| p.key == "Input2")
            .expect("Input2");
        assert_eq!(input2.def.as_deref(), Some("1000"));
    }

    #[test]
    fn comparing_marks_every_kind_of_change() {
        let base = doc(HOUSE);
        let new = doc(&edited(&[
            // rename
            ("Title=\"Licht OG\"", "Title=\"Licht oben\""),
            // parameter
            ("K=\"Input2\" Def=\"1000\"", "K=\"Input2\" Def=\"100\""),
            // move
            (
                "Title=\"Zähler\" Px=\"6720\" Py=\"768\" Px2=\"9408\"",
                "Title=\"Zähler\" Px=\"7680\" Py=\"768\" Px2=\"10368\"",
            ),
            // note text
            ("Zweite Zeile\"", "Zweite Zeile\nDritte Zeile\""),
            // remove the AND → OR wire (OR.I1 left unwired)
            (
                "<Co K=\"I1\" Nc=\"1\" U=\"10000009-0000-0091-00ff504f94000001\">\n\t\t\t\t\t<In Input=\"10000004-0000-0043-02ff504f94000001\"/>\n\t\t\t\t</Co>",
                "<Co K=\"I1\" U=\"10000009-0000-0091-00ff504f94000001\"/>",
            ),
        ]));
        let v = build(&new, Some(&base), "");
        assert!(v.compared);
        let kinds: Vec<&str> = v.changes.iter().map(|c| c.kind).collect();
        for k in ["renamed", "param", "moved", "attr", "wire-"] {
            assert!(kinds.contains(&k), "missing a `{k}` change in {kinds:?}");
        }
        let og = page(&v, PAGE_OG);
        assert_eq!(obj(og, OR).status, CHANGED);
        assert_eq!(og.status, CHANGED);
        assert_eq!(v.objects[OR].old_title.as_deref(), Some("Licht OG"));

        let eg = page(&v, PAGE_EG);
        assert_eq!(obj(eg, METER).status, MOVED);
        assert_eq!(obj(eg, METER).old_rect.map(|r| r[0]), Some(6720));
        let div = obj(eg, DIV);
        assert_eq!(div.status, CHANGED);
        assert_eq!(
            div.ports
                .iter()
                .find(|p| p.key == "Input2")
                .map(|p| p.status),
            Some(CHANGED)
        );
        let param = v.objects[DIV]
            .ports
            .iter()
            .find(|p| p.key == "Input2")
            .unwrap();
        assert_eq!(
            (param.old_def.as_deref(), param.def.as_deref()),
            (Some("1000"), Some("100"))
        );

        let note = v.changes.iter().find(|c| c.kind == "attr").unwrap();
        assert!(
            note.text.contains("note") && note.text.contains("+ Dritte Zeile"),
            "{}",
            note.text
        );

        // The removed wire still shows, marked, as a link on the page it left.
        let gone = eg
            .labels
            .iter()
            .find(|l| l.obj == AND && l.status == REMOVED);
        assert!(gone.is_some(), "removed cross-page link is drawn");
        assert_eq!(v.stats.wires_removed, 1);
    }

    #[test]
    fn removed_and_added_blocks_show_on_their_page() {
        let base = doc(HOUSE);
        let div_start = HOUSE.find("\t\t\t<C Type=\"Div\"").unwrap();
        let div_end = HOUSE[div_start..].find("</C>\n").unwrap() + div_start + "</C>\n".len();
        let added = "\t\t\t<C Type=\"Or\" V=\"178\" U=\"1000000a-0000-00a0-ffff504f94000001\" Title=\"Neu\" Px=\"960\" Py=\"2880\" Px2=\"2304\" Py2=\"3576\" Nio=\"3\" WF=\"16384\">\n\t\t\t\t<Co K=\"I1\" Nc=\"1\" U=\"1000000a-0000-00a1-00ff504f94000001\">\n\t\t\t\t\t<In Input=\"10000003-0000-0032-01ff504f94000001\"/>\n\t\t\t\t</Co>\n\t\t\t\t<Co K=\"I2\" U=\"1000000a-0000-00a2-01ff504f94000001\"/>\n\t\t\t\t<Co K=\"Q\" U=\"1000000a-0000-00a3-02ff504f94000001\"/>\n\t\t\t</C>\n";
        let src = format!("{}{}{}", &HOUSE[..div_start], added, &HOUSE[div_end..]);
        let v = build(&doc(&src), Some(&base), "");
        let eg = page(&v, PAGE_EG);

        let ghost = eg
            .objects
            .iter()
            .find(|o| o.uuid == DIV)
            .expect("removed block is drawn");
        assert_eq!(ghost.status, REMOVED);
        assert_eq!(v.objects[DIV].status, REMOVED);
        let new = obj(eg, "1000000a-0000-00a0-ffff504f94000001");
        assert_eq!(new.status, ADDED);
        assert!(
            eg.wires
                .iter()
                .any(|w| w.status == ADDED && w.to[0] == new.uuid)
        );
        assert!(
            eg.wires
                .iter()
                .any(|w| w.status == REMOVED && w.to[0] == DIV)
        );
        assert_eq!((v.stats.added, v.stats.removed), (1, 1));
    }

    #[test]
    fn block_moved_to_another_page_leaves_a_pointer_behind() {
        let base = doc(HOUSE);
        let div_start = HOUSE.find("\t\t\t<C Type=\"Div\"").unwrap();
        let div_end = HOUSE[div_start..].find("</C>\n").unwrap() + div_start + "</C>\n".len();
        let div = &HOUSE[div_start..div_end];
        let without = format!("{}{}", &HOUSE[..div_start], &HOUSE[div_end..]);
        let og_at = without.find("\t\t\t<C Type=\"Or\"").unwrap();
        let src = format!("{}{}{}", &without[..og_at], div, &without[og_at..]);
        let v = build(&doc(&src), Some(&base), "");
        let ghost = page(&v, PAGE_EG)
            .objects
            .iter()
            .find(|o| o.uuid == DIV)
            .expect("ghost");
        assert_eq!(ghost.status, MOVED);
        assert_eq!(
            ghost.moved_to.as_ref().and_then(|t| t.page.as_deref()),
            Some(PAGE_OG)
        );
        assert_eq!(
            obj(page(&v, PAGE_OG), DIV).old_rect,
            None,
            "no ghost box on the new page"
        );
        let moved = v.changes.iter().find(|c| c.kind == "moved").unwrap();
        assert!(
            moved.text.contains("Erdgeschoss → Obergeschoss"),
            "{}",
            moved.text
        );
    }

    #[test]
    fn identical_revisions_have_no_changes() {
        let v = build(&doc(HOUSE), Some(&doc(HOUSE)), "");
        assert!(v.compared);
        assert!(v.changes.is_empty());
        assert!(
            v.pages
                .iter()
                .all(|p| p.changes == 0 && p.status.is_empty())
        );
    }

    #[test]
    fn attr_change_text_shows_changed_lines() {
        let t = attr_change_text("Notiz", "Text", "Text", Some("a\nb"), Some("a\nb\nc"));
        assert_eq!(t, "Notiz · note: + c");
        let t = attr_change_text("X", "Switch", "Cl", Some("1,2,3"), Some("4,5,6"));
        assert_eq!(t, "X · Cl: 1,2,3 → 4,5,6");
        let t = attr_change_text("X", "Text", "Text", Some("a\r\nb"), Some("a\nb"));
        assert_eq!(t, "X · note: whitespace changed");
    }
}
