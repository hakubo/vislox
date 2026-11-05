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
        };
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
            iname: self.el(uuid).and_then(|e| e.attr_decoded("IName").map(|t| t.into_owned())),
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
        Some(self.el(id).map(|_| self.title(id)).unwrap_or_else(|| id.to_string()))
    }
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
                el.attr_decoded(&a.name).map(|v| v.into_owned()).unwrap_or_default(),
            )
        })
        .collect()
}

/// The orthogonal route of a wire, as Loxone Config draws it: out of the
/// source, across at the midpoint, into the sink; a wire running right to
/// left loops around.
fn route(a: (i64, i64), b: (i64, i64), ra: &Layout, rb: &Layout, leaves_right: bool, enters_left: bool) -> Vec<[i64; 2]> {
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
    vec![[x1, y1], [out, y1], [out, ym], [into, ym], [into, y2], [x2, y2]]
}

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
    attrs: HashMap<String, Vec<(String, Option<String>, Option<String>)>>,
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
        x.wires_added.insert((w.from_port.clone(), w.to_port.clone()));
    }
    for w in &d.wires_removed {
        x.wires_removed.insert((w.from_port.clone(), w.to_port.clone()));
    }
    // Wires the semantic diff folds away still show on the page.
    let nw: HashSet<(String, String)> = new.wires.iter().map(|w| (w.from_port.clone(), w.to_port.clone())).collect();
    let ow: HashSet<(String, String)> = old.wires.iter().map(|w| (w.from_port.clone(), w.to_port.clone())).collect();
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
            let od = op.iter().find(|q| q.key == p.key).and_then(|q| q.def.clone());
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
        view.objects.insert(u.clone(), obj_info(&nc, oc.as_ref(), u, &d, false));
    }
    if let Some(oc) = &oc {
        for u in &oc.order {
            if !nc.els.contains_key(u) {
                view.objects.insert(u.clone(), obj_info(oc, None, u, &d, true));
            }
        }
    }

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
    let add_obj = |g: &GeoObject, ctx: &Ctx, status: &'static str, objects: &mut Vec<PObj>, port_pos: &mut HashMap<String, ((i64, i64), Side, Layout)>| {
        let el = ctx.el(&g.uuid);
        let defs: HashMap<String, PortView> = el
            .map(|e| ports(e).into_iter().map(|p| (p.uuid.clone(), p)).collect())
            .unwrap_or_default();
        let mut pp = Vec::new();
        for (i, p) in g.ports.iter().enumerate() {
            let (x, y) = g.port_point(i);
            port_pos.entry(p.uuid.clone()).or_insert(((x, y), p.side, g.rect));
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
        let target = el
            .and_then(|e| e.attr("Ref"))
            .map(|r| ctx.target(r, None));
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
    let mut emit = |w: &WireView, ctx: &Ctx, status: &'static str, wires: &mut Vec<PWire>, labels: &mut Vec<PLabel>| {
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
                    (pb, sb, rb, pa, sa, ra, [to.clone(), tk.clone()], [fo.clone(), fk.clone()])
                } else {
                    (pa, sa, ra, pb, sb, rb, [fo.clone(), fk.clone()], [to.clone(), tk.clone()])
                };
                wires.push(PWire {
                    points: route(pa, pb, &ra, &rb, sa == Side::Right, sb == Side::Left),
                    analog: analog_key(&from[1]),
                    from,
                    to: to_,
                    status,
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
                });
            }
            _ => {}
        }
    };
    for w in &c.wires {
        let status = if gone { REMOVED } else { d.wire(&w.from_port, &w.to_port) };
        emit(w, c, status, &mut wires, &mut labels);
    }
    if let Some(oc) = old {
        for w in &oc.wires {
            if d.wires_removed.contains(&(w.from_port.clone(), w.to_port.clone())) {
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
        status: if status.is_empty() && changes > 0 { CHANGED } else { status },
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
                    conns.push(Conn { dir: "in", target: t, status });
                }
            } else if w.from_port == p.uuid {
                if let Some(t) = ctx.port_target(&w.to_port) {
                    conns.push(Conn { dir: "out", target: t, status });
                }
            }
        };
        for w in &c.wires {
            let st = if gone { REMOVED } else { d.wire(&w.from_port, &w.to_port) };
            push(w, c, st);
        }
        if let Some(oc) = old {
            for w in &oc.wires {
                if d.wires_removed.contains(&(w.from_port.clone(), w.to_port.clone())) {
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
    fn node(c: &Ctx, uuid: &str, d: &Delta, kids: &HashMap<String, Vec<String>>, status: &'static str) -> TreeNode {
        let ty = c.ty(uuid);
        let children = kids
            .get(uuid)
            .map(|v| v.iter().map(|k| node(c, k, d, kids, if status == REMOVED { REMOVED } else { d.status(k) })).collect())
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
    let root: Vec<&String> = c.order.iter().filter(|u| !c.parent.contains_key(*u)).collect();
    let mut out: Vec<TreeNode> = root.iter().map(|u| node(c, u, d, &nk, d.status(u))).collect();
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
        let mut x: Vec<String> = set.iter().filter(|u| ctx.els.contains_key(*u)).cloned().collect();
        x.sort_by_key(|u| ctx.order.iter().position(|o| o == u));
        x
    };
    // Leave out ports' owners' children noise: only report top objects whose parent is not also added.
    for u in sorted(&d.added, nc) {
        if nc.parent.get(&u).is_some_and(|p| d.added.contains(p)) {
            continue;
        }
        view.stats.added += 1;
        v.push(Change { kind: ADDED, text: name(nc, &u), page: page_for(&u), obj: Some(u) });
    }
    for u in sorted(&d.removed, oc) {
        if oc.parent.get(&u).is_some_and(|p| d.removed.contains(p)) {
            continue;
        }
        view.stats.removed += 1;
        v.push(Change { kind: REMOVED, text: name(oc, &u), page: page_for(&u), obj: Some(u) });
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
                    text: format!(
                        "{} · {n}: {} → {}",
                        nc.title(&u),
                        short(o.as_deref()),
                        short(new.as_deref())
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
        owner(&w.1).and_then(|o| ctx.page_of.get(&o).cloned())
            .or_else(|| owner(&w.0).and_then(|o| ctx.page_of.get(&o).cloned()))
    };
    let wire_obj = |ctx: &Ctx, w: &(String, String)| ctx.port_owner.get(&w.1).map(|(o, _)| o.clone());
    let mut wa: Vec<_> = d.wires_added.iter().collect();
    wa.sort();
    for w in wa {
        view.stats.wires_added += 1;
        v.push(Change { kind: "wire+", text: wire_text(nc, w), page: wire_page(nc, w), obj: wire_obj(nc, w) });
    }
    let mut wr: Vec<_> = d.wires_removed.iter().collect();
    wr.sort();
    for w in wr {
        view.stats.wires_removed += 1;
        v.push(Change { kind: "wire-", text: wire_text(oc, w), page: wire_page(oc, w), obj: wire_obj(oc, w) });
    }
    let mut moved: Vec<_> = d.moved.keys().filter(|u| d.status(u) == MOVED).cloned().collect();
    moved.sort_by_key(|u| nc.order.iter().position(|o| o == u));
    for u in moved {
        view.stats.moved += 1;
        let (from, to) = (oc.page_of.get(&u), nc.page_of.get(&u));
        let text = match (from, to) {
            (Some(a), Some(b)) if a != b => format!("{} · page {} → {}", nc.title(&u), oc.title(a), nc.title(b)),
            _ => nc.title(&u),
        };
        v.push(Change { kind: MOVED, text, page: page_for(&u), obj: Some(u) });
    }
    view.changes = v;
}

fn short(s: Option<&str>) -> String {
    match s {
        None => "—".into(),
        Some(s) if s.chars().count() > 40 => format!("{}…", s.chars().take(40).collect::<String>()),
        Some(s) => s.to_string(),
    }
}
