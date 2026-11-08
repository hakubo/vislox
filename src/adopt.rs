//! Adopting a block Loxone Config drew into the Lox Logic module (#72). The
//! design is in `docs/editing.md` (*Adopting blocks*).
//!
//! `lox logic adopt <base> --uuid <uuid> --as <slug> --module … --lock …`
//! writes a block's declaration (with externs for its wired neighbours and
//! `<-` wires onto them) into the module's file for its page, and pins its
//! UUIDs in the lockfile, once it has checked that compiling the pair
//! rebuilds the config unchanged. vislox writes only its own files, so it
//! runs lox on a scratch copy of the module and lock, and moves what lox
//! wrote into them ([`extract`]):
//!
//! - the declaration goes into vislox's file of the page, under its
//!   *adopted* comment, with the wires out of it ([`crate::blocks`]);
//! - each extern lox pinned becomes vislox's extern of that object
//!   (`vx_<uuid>`, pinned by UUID, in `edits.loxl`), and every reference is
//!   renamed to it, so that vislox's files never name the module's own slugs;
//! - the lockfile gains the block's entry (its UUIDs, layout and page) and the
//!   wires lox tracks onto the externs ([`LockEdits`]).
//!
//! Then the project is compiled like any edit, and [`verify`] checks that the
//! pending config is what it was, the block exactly as the base has it: same
//! UUIDs, connectors, wires, parameters and place. Anything else is refused
//! and put back. Adopting changes nothing the Miniserver runs: only who owns
//! the block. Nothing here talks to the Miniserver.

use crate::blocks::{FileWire, NewBlock};
use crate::edit::{find_ci as find, slug as vx};
use loxone_config::LoxoneDoc;
use loxone_config::connectors;
use loxone_config::logic::{ArgItem, BindingKind, Item, MatchSpec, Module, PortRef, Value};
use loxone_config::xml::{Element, Node};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};

/// One block of an `adopt` op: its UUID, and the slug to give it (else
/// vislox picks one from its name).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Pick {
    pub uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

/// A block adopted on the scratch copy, as vislox writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Adoption {
    pub uuid: String,
    pub slug: String,
    /// The page it is drawn on: UUID and title.
    pub page: String,
    pub page_title: String,
    /// Its declaration, renamed to vislox's externs (`adopted`).
    pub block: NewBlock,
    /// The wires out of it onto externs, `vx_<uuid>.Port <- slug`.
    pub wires: Vec<FileWire>,
    /// The externs it names: object UUID → its type.
    pub externs: BTreeMap<String, String>,
    /// Its lockfile entry (`objects.<slug>`).
    pub object: Json,
    /// The `extern_wires` entries lox recorded for its wires.
    pub extern_wires: Vec<Json>,
    /// It was vislox's extern (`vx_<uuid>`) of blocks adopted earlier: lox
    /// moved their wires onto it into its declaration.
    pub promoted: bool,
    /// The `extern_wires` entries into it lox dropped, and its `externals`
    /// entries: it is managed now.
    pub unwires: Vec<Json>,
    pub unexternals: Vec<String>,
}

/// What an adoption (or its undo) changes in the lockfile, written before
/// the compile like a move ([`write_lock`]).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct LockEdits {
    /// Slug → the entry to put under `objects`.
    pub add: BTreeMap<String, Json>,
    /// `extern_wires` entries to add.
    pub wires: Vec<Json>,
    /// Slugs to take out of `objects`, with every `extern_wires` entry from
    /// one of their ports: without the source line, lox would take those
    /// wires out of the config (they were Loxone Config's). The entries into
    /// them stay: the blocks that stay adopted draw those onto their extern.
    pub drop: BTreeSet<String>,
    /// `extern_wires` entries and `externals` slugs to take out: those lox
    /// dropped when it adopted a block that was vislox's extern.
    pub unwires: Vec<Json>,
    pub unexternals: BTreeSet<String>,
    /// Removal tombstones to take out, by object UUID: a deleted block
    /// restored (#80) is the lock's again, with its UUIDs.
    pub untomb: BTreeSet<String>,
    /// Tree devices created here and restored (#85): slug → (UUID, title).
    /// Each gets its `externals` entry (`matched_by: created`) and its
    /// `hardware` entry back, as lox wrote them.
    pub created: BTreeMap<String, (String, Option<String>)>,
    /// Named Automatic Designer rules restored (#87): slug → rule UUID, the
    /// lock's `rules` pin back, as lox wrote it.
    pub rules: BTreeMap<String, String>,
}

impl LockEdits {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty()
            && self.wires.is_empty()
            && self.drop.is_empty()
            && self.unwires.is_empty()
            && self.unexternals.is_empty()
            && self.untomb.is_empty()
            && self.created.is_empty()
            && self.rules.is_empty()
    }
}

/// The lockfile `lock` with `edits` made. Every other field is kept as lox
/// wrote it.
pub fn write_lock(lock: &[u8], edits: &LockEdits) -> Result<Vec<u8>, String> {
    let mut v: Json =
        serde_json::from_slice(lock).map_err(|e| format!("cannot read the lock: {e}"))?;
    let root = v
        .as_object_mut()
        .ok_or_else(|| "the lock is not a JSON object".to_string())?;
    let objects = root
        .entry("objects")
        .or_insert_with(|| Json::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| "the lock's objects are not a JSON object".to_string())?;
    let mut gone_ports: BTreeSet<String> = BTreeSet::new();
    for slug in &edits.drop {
        if let Some(o) = objects.remove(slug) {
            gone_ports.extend(ports_of(&o));
        }
    }
    for (slug, o) in &edits.add {
        objects.insert(slug.clone(), o.clone());
    }
    if let Some(t) = root.get_mut("removed").and_then(|x| x.as_object_mut()) {
        t.retain(|u, _| !edits.untomb.iter().any(|x| x.eq_ignore_ascii_case(u)));
    }
    if root
        .get("removed")
        .and_then(|x| x.as_object())
        .is_some_and(|t| t.is_empty())
    {
        root.remove("removed");
    }
    // A wire written again is no withdrawn one any more.
    if let Some(r) = root.get_mut("removed_wires").and_then(|x| x.as_array_mut()) {
        r.retain(|w| !edits.wires.contains(w));
    }
    if let Some(x) = root.get_mut("externals").and_then(|x| x.as_object_mut()) {
        for s in &edits.unexternals {
            x.remove(s);
        }
    }
    for (slug, (uuid, title)) in &edits.created {
        let x = root
            .entry("externals")
            .or_insert_with(|| Json::Object(Default::default()))
            .as_object_mut()
            .ok_or_else(|| "the lock's externals are not a JSON object".to_string())?;
        x.insert(
            slug.clone(),
            serde_json::json!({ "uuid": uuid, "matched_by": "created", "title_at_match": title,
                "iname_at_match": null, "owned": true }),
        );
        let h = root
            .entry("hardware")
            .or_insert_with(|| Json::Object(Default::default()))
            .as_object_mut()
            .ok_or_else(|| "the lock's hardware is not a JSON object".to_string())?;
        h.insert(slug.clone(), Json::String(uuid.clone()));
    }
    for (slug, uuid) in &edits.rules {
        let r = root
            .entry("rules")
            .or_insert_with(|| Json::Object(Default::default()))
            .as_object_mut()
            .ok_or_else(|| "the lock's rules are not a JSON object".to_string())?;
        r.insert(slug.clone(), Json::String(uuid.clone()));
    }
    let wires = root
        .entry("extern_wires")
        .or_insert_with(|| Json::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "the lock's extern_wires are not a list".to_string())?;
    let end = |w: &Json, k: &str| w.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    // Out of a block handed back: Loxone Config's again. A wire into it from
    // a block that stays adopted is that block's wire onto its extern now.
    wires.retain(|w| !gone_ports.contains(&end(w, "from")) && !edits.unwires.contains(w));
    for w in &edits.wires {
        if !wires.contains(w) {
            wires.push(w.clone());
        }
    }
    let mut out = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
    out.push(b'\n');
    Ok(out)
}

// ── what can be adopted ─────────────────────────────────────────────────

/// A name for messages: the title, else the UUID.
pub fn title_or(title: Option<&str>, uuid: &str) -> String {
    title
        .filter(|t| !t.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| uuid.to_string())
}

/// [`title_or`] of the object `uuid` of `doc`.
pub fn name_in(doc: &LoxoneDoc, uuid: &str) -> String {
    let t = find(&doc.xml.root, uuid).and_then(|e| e.attr_decoded("Title"));
    title_or(t.as_deref(), uuid)
}

/// One object of an [`Index`].
#[derive(Clone, Debug)]
pub struct Entry {
    pub uuid: String,
    pub ancestors: Vec<String>,
    pub ordinal: usize,
    pub ty: String,
    pub title: String,
    /// The logic page it is drawn on (its nearest `Page` ancestor).
    pub page: Option<String>,
    /// Its connectors' UUIDs.
    pub ports: Vec<String>,
    pub read_only: Option<String>,
}

/// A document and its [`Index`].
pub type Indexed = (LoxoneDoc, Index);

/// Every object of a document by UUID, from one walk: what deciding whether
/// blocks can be adopted needs, without walking the tree per block.
#[derive(Clone, Debug, Default)]
pub struct Index(std::collections::HashMap<String, Entry>);

impl Index {
    pub fn of(doc: &LoxoneDoc) -> Index {
        fn walk(
            el: &Element,
            page: Option<&str>,
            ancestors: &mut Vec<String>,
            out: &mut std::collections::HashMap<String, Entry>,
        ) {
            let mut here = page;
            if let (Some(u), Some(ty)) = (el.attr("U"), el.attr("Type")) {
                out.insert(
                    u.to_ascii_lowercase(),
                    Entry {
                        uuid: u.to_string(),
                        ancestors: ancestors.clone(),
                        ordinal: out.len(),
                        ty: ty.to_string(),
                        read_only: if ty == "Mode"
                            && el
                                .attr("Mode")
                                .and_then(|v| v.parse::<i64>().ok())
                                .is_none_or(|n| n >= 0)
                        {
                            Some("Built-in operating modes stay read-only".into())
                        } else if ty == "CalendarEntry"
                            && crate::calendar_edit::date_of(el).is_none()
                        {
                            Some(crate::calendar_edit::UNWRITABLE_DATE.into())
                        } else {
                            None
                        },
                        title: el.attr_decoded("Title").unwrap_or_default().into_owned(),
                        page: page.map(String::from),
                        ports: el
                            .child_elements()
                            .filter(|c| c.name == "Co")
                            .filter_map(|c| c.attr("U").map(String::from))
                            .collect(),
                    },
                );
                if ty == "Page" {
                    here = Some(u);
                }
            }
            let has_id = el.attr("U").filter(|_| el.attr("Type").is_some());
            if let Some(u) = has_id {
                ancestors.push(u.to_ascii_lowercase());
            }
            for c in el.child_elements() {
                walk(c, here, ancestors, out);
            }
            if has_id.is_some() {
                ancestors.pop();
            }
        }
        let mut out = std::collections::HashMap::new();
        walk(&doc.xml.root, None, &mut Vec::new(), &mut out);
        Index(out)
    }

    pub fn get(&self, uuid: &str) -> Option<&Entry> {
        self.0.get(&uuid.to_ascii_lowercase())
    }

    /// Canonical UUIDs in document order, including the root itself.
    pub fn subtree(&self, uuid: &str) -> Vec<&Entry> {
        let key = uuid.to_ascii_lowercase();
        let mut entries: Vec<_> = self
            .0
            .values()
            .filter(|e| e.uuid.eq_ignore_ascii_case(uuid) || e.ancestors.contains(&key))
            .collect();
        entries.sort_by_key(|e| e.ordinal);
        entries
    }

    /// [`title_or`] of `uuid`.
    pub fn name(&self, uuid: &str) -> String {
        title_or(self.get(uuid).map(|e| e.title.as_str()), uuid)
    }

    /// Page blocks and object-tree kinds supported by vislox, by UUID.
    pub fn candidates(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.0
            .values()
            .filter(|e| {
                (e.page.is_some() && e.ty != "Page" && connectors::builtin(&e.ty).is_some())
                    || crate::periphery::of(&e.ty).is_some()
            })
            .map(|e| (&e.uuid, e))
    }
}

/// The page `uuid` is drawn on (its nearest `Page` ancestor): UUID, title.
pub fn page_of(doc: &LoxoneDoc, uuid: &str) -> Option<(String, String)> {
    fn walk(el: &Element, uuid: &str, page: Option<&Element>) -> Option<(String, String)> {
        let page = if el.attr("Type") == Some("Page") {
            Some(el)
        } else {
            page
        };
        for c in el.child_elements() {
            if c.attr("U").is_some_and(|u| u.eq_ignore_ascii_case(uuid)) {
                let p = page?;
                return Some((
                    p.attr("U")?.to_string(),
                    p.attr_decoded("Title").unwrap_or_default().into_owned(),
                ));
            }
            if let Some(r) = walk(c, uuid, page) {
                return Some(r);
            }
        }
        None
    }
    walk(&doc.xml.root, uuid, None)
}

/// The blocks drawn right on page `page` that lox has a type for (its
/// builtin port table: lox refuses any other type): UUID, type, title.
pub fn on_page(doc: &LoxoneDoc, page: &str) -> Vec<(String, String, String)> {
    let Some(pg) = find(&doc.xml.root, page).filter(|e| e.attr("Type") == Some("Page")) else {
        return Vec::new();
    };
    pg.child_elements()
        .filter(|c| c.name == "C")
        .filter_map(|c| {
            let ty = c.attr("Type")?;
            connectors::builtin(ty)?;
            Some((
                c.attr("U")?.to_string(),
                ty.to_string(),
                c.attr_decoded("Title").unwrap_or_default().into_owned(),
            ))
        })
        .collect()
}

/// Whether `s` is a slug vislox may give an adopted block: `vx_` and then
/// lowercase letters, digits and `_`, as lox's slugs are, and not the name
/// vislox gives an extern (`vx_<uuid>`).
pub fn check_slug(s: &str) -> Result<(), String> {
    let rest = s.strip_prefix(crate::blocks::SLUG_PREFIX).unwrap_or("");
    if rest.is_empty()
        || !rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(format!(
            "`{s}`: a slug of vislox's files is vx_ and then lowercase letters, digits and _"
        ));
    }
    let parts: Vec<&str> = rest.split('_').collect();
    let uuid_like = parts.iter().map(|p| p.len()).collect::<Vec<_>>() == [8, 4, 4, 16]
        && parts
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()));
    if uuid_like {
        return Err(format!(
            "`{s}` is how vislox names an extern; pick another slug"
        ));
    }
    Ok(())
}

/// A slug for the block `title` of `ty`: `vx_` and its name in plain
/// letters (`Licht Küche` → `vx_licht_kueche`), else its type; with `_2`,
/// `_3`, … when `taken`.
pub fn default_slug(title: &str, ty: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let word = |t: &str| {
        let mut s = String::new();
        for c in t.chars().flat_map(char::to_lowercase) {
            match c {
                'ä' => s.push_str("ae"),
                'ö' => s.push_str("oe"),
                'ü' => s.push_str("ue"),
                'ß' => s.push_str("ss"),
                c if c.is_ascii_alphanumeric() => s.push(c),
                _ if !s.ends_with('_') => s.push('_'),
                _ => {}
            }
        }
        let s: String = s.trim_matches('_').chars().take(32).collect();
        s.trim_end_matches('_').to_string()
    };
    let mut stem = word(title);
    if stem.is_empty() {
        stem = word(ty);
    }
    let first = format!("{}{stem}", crate::blocks::SLUG_PREFIX);
    let ok = |s: &str| !taken(s) && check_slug(s).is_ok();
    if ok(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{first}_{n}"))
        .find(|s| ok(s))
        .expect("an unused slug")
}

// ── what lox wrote ──────────────────────────────────────────────────────

/// The words lox reads a bare name as a day (or day set) by, before an
/// object (`loxone_config::logic::schedule`, not public): `Days: mon`.
const DAY_WORDS: &[&str] = &[
    "holiday", "vacation", "dayoff", "mon", "tue", "wed", "thu", "fri", "sat", "sun", "weekdays",
    "weekend", "daily", "heating", "cooling",
];

/// The statements of a file, without blank lines and comments.
fn statements(text: &str) -> Result<Vec<Item>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(Module::parse_fragment(text)
        .map_err(|e| e.to_string())?
        .items
        .into_iter()
        .filter(|i| !matches!(i, Item::Gap | Item::Comment(_) | Item::FragmentEnd))
        .collect())
}

/// The slug → entry map of a lockfile's `field` (`objects`, `externals`).
fn entries<'a>(lock: &'a Json, field: &str) -> BTreeMap<&'a str, &'a Json> {
    lock.get(field)
        .and_then(|o| o.as_object())
        .map(|o| o.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .unwrap_or_default()
}

fn uuid_in(entry: &Json) -> Option<&str> {
    entry.get("uuid").and_then(|u| u.as_str())
}

/// What `lox logic adopt` wrote into the scratch copy, as vislox writes it.
///
/// `before`/`after` are the module's files (path → text) before the first
/// adoption and after the last, and the lockfiles before and after.
/// `picked` is the slug vislox chose for each UUID lox adopted (lox may
/// have used another: a block an earlier adoption of the same request
/// pinned as an extern keeps that extern's name). `own` says whether a slug
/// is a block of vislox's files, which an adopted block may name.
pub fn extract(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    (lock_before, lock_after): (&Json, &Json),
    picked: &BTreeMap<String, String>,
    own: &dyn Fn(&str) -> bool,
    lets: &BTreeSet<String>,
    base: &LoxoneDoc,
) -> Result<Vec<Adoption>, String> {
    // The new statements: lox appends to a page's file, and a new file
    // opens with its `page` line. The rest of each file must be as it was.
    let mut new: Vec<Item> = Vec::new();
    // A block an earlier edit adopted wires onto through vislox's extern of
    // it (`vx_<uuid>`, an extern of the lock): lox adopts it under that name
    // and moves the lines on the extern into its declaration, out of
    // vislox's files. Only those lines may leave them.
    let externals_before = entries(lock_before, "externals");
    let promoted: BTreeSet<String> = picked
        .keys()
        .map(|u| vx(u))
        .filter(|x| externals_before.contains_key(x.as_str()))
        .collect();
    for (path, text) in after {
        let was = before.get(path).map(String::as_str).unwrap_or("");
        if was == text {
            continue;
        }
        let (old, now) = (
            statements(was).map_err(|e| format!("{path}: {e}"))?,
            statements(text).map_err(|e| format!("{path}: {e}"))?,
        );
        if path.starts_with(&format!("{}/", crate::edit::DIR)) {
            let mut rest = now.iter().peekable();
            for o in &old {
                if rest.peek() == Some(&o) {
                    rest.next();
                    continue;
                }
                let moves = match o {
                    Item::Extern(e) => promoted.contains(&e.slug),
                    Item::Wire(w) => promoted.contains(&w.to.slug),
                    _ => false,
                };
                if !moves {
                    return Err(format!(
                        "lox logic adopt changed {path} beyond moving the lines on a block it adopted"
                    ));
                }
            }
            if rest.next().is_some() {
                return Err(format!(
                    "lox logic adopt added to {path}, which is vislox's"
                ));
            }
            continue;
        }
        if now.len() < old.len() || now[..old.len()] != old[..] {
            return Err(format!(
                "lox logic adopt changed {path} beyond adding the block; vislox writes only its own files"
            ));
        }
        let mut added = now[old.len()..].to_vec();
        if old.is_empty() && matches!(added.first(), Some(Item::Page(_))) {
            added.remove(0);
        }
        new.extend(added);
    }
    if let Some(gone) = before.keys().find(|p| !after.contains_key(*p)) {
        return Err(format!("lox logic adopt removed {gone}"));
    }
    let (objects_before, objects_after) = (
        entries(lock_before, "objects"),
        entries(lock_after, "objects"),
    );
    let externals = entries(lock_after, "externals");
    // lox's slug of each adopted block → vislox's.
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    for (s, o) in &objects_after {
        if objects_before.contains_key(s) {
            continue;
        }
        let u = uuid_in(o).ok_or_else(|| format!("the lock has no UUID for {s}"))?;
        let mine = picked
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(u))
            .map(|(_, v)| v)
            .ok_or_else(|| format!("lox logic adopt pinned {s} ({u}), which was not asked for"))?;
        rename.insert(s.to_string(), mine.clone());
    }
    // An extern's object, and its type.
    let mut externs: BTreeMap<String, String> = BTreeMap::new();
    for i in &new {
        if let Item::Extern(e) = i {
            if !e.args.is_empty() {
                return Err(format!(
                    "lox logic adopt wrote `extern {}` with statements vislox does not write",
                    e.slug
                ));
            }
            let u = externals
                .get(e.slug.as_str())
                .and_then(|x| uuid_in(x))
                .or(match &e.match_spec {
                    MatchSpec::Uuid(u) => Some(u.as_str()),
                    _ => None,
                })
                .ok_or_else(|| {
                    format!("the lock does not say which object extern {} is", e.slug)
                })?;
            externs.insert(u.to_string(), e.block_type.clone());
        }
    }
    // A name an adopted block's statements use, as vislox's files name it.
    let mut needs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut name = |owner: &str, s: &str, value: bool| -> Result<String, String> {
        if let Some(m) = rename.get(s) {
            return Ok(m.clone());
        }
        if let Some(u) = externals.get(s).and_then(|x| uuid_in(x)) {
            if objects_after.values().any(|o| uuid_in(o) == Some(u)) {
                return Err(format!("{s} is managed and an extern at once"));
            }
            let ty = match externs.get(u) {
                Some(t) => t.clone(),
                None => find(&base.xml.root, u)
                    .and_then(|e| e.attr("Type"))
                    .ok_or_else(|| format!("{s} ({u}) is not in the project's base config"))?
                    .to_string(),
            };
            needs
                .entry(owner.to_string())
                .or_default()
                .insert(u.to_string(), ty);
            return Ok(vx(u));
        }
        if objects_after.contains_key(s) && own(s) {
            return Ok(s.to_string());
        }
        if objects_after.contains_key(s) {
            if value {
                return Err(format!(
                    "its value references `{s}`, which the module declares in its own .loxl file; adopt this object with lox or change that source file"
                ));
            }
            return Err(format!(
                "it is wired to `{s}`, which the module manages in its own .loxl file; vislox's files name only their own blocks and Loxone Config's (adopt it with lox, or wire it in that file)"
            ));
        }
        Err(format!(
            "its declaration names `{s}`, which vislox cannot pin"
        ))
    };
    let mut out: BTreeMap<String, Adoption> = BTreeMap::new();
    let mut wires: Vec<(String, FileWire)> = Vec::new();
    for i in &new {
        match i {
            Item::Extern(_) => {}
            // A central block's members (#75): lox lifts them as a claim.
            // Left out, the block keeps the list Loxone Config wrote (a
            // claim dropped leaves the members as last written), so adopting
            // changes nothing; editing them later claims them in vislox's
            // file of claims, by the block's slug.
            Item::MembersClaim(_) => {}
            // An NFC Code Touch's access rights (#76): lox lifts them as a
            // set and a claim. Left out for the same reason: a dropped claim
            // leaves the rights as last written, so the door opens for the
            // same people; editing them later claims them in vislox's file.
            Item::Access(_) | Item::AccessClaim(_) => {}
            Item::Block(b) => {
                let slug = rename.get(&b.slug).cloned().ok_or_else(|| {
                    format!(
                        "lox logic adopt declared `{}`, which it did not pin",
                        b.slug
                    )
                })?;
                let entry = objects_after[b.slug.as_str()];
                let uuid = uuid_in(entry).unwrap_or_default().to_string();
                let mut nb = NewBlock {
                    slug: slug.clone(),
                    ty: b.block_type.clone(),
                    title: b.title.clone(),
                    adopted: true,
                    ..Default::default()
                };
                for a in &b.args {
                    let ArgItem::Binding(x) = a else { continue };
                    match &x.kind {
                        // `mirrors:` names the object a ref is (a push
                        // notification's `OutputRefLM`, #77): an extern of
                        // vislox's, as a value, never a wire.
                        BindingKind::Param(Value::Ref(s))
                            if crate::blocks::value_ref_arg(&x.port) =>
                        {
                            nb.args.push((
                                x.port.clone(),
                                Value::Ref(
                                    if DAY_WORDS.contains(&s.as_str())
                                        && x.port == crate::calendar_edit::SWITCHES
                                    {
                                        s.clone()
                                    } else {
                                        name(&slug, s, true)?
                                    },
                                ),
                            ))
                        }
                        // A bare name is a value when it names a `let`
                        // constant or a day: lox reads it so first.
                        BindingKind::Param(v @ Value::Ref(s))
                            if lets.contains(s) || DAY_WORDS.contains(&s.as_str()) =>
                        {
                            nb.args.push((x.port.clone(), v.clone()))
                        }
                        BindingKind::Param(Value::Ref(s)) => nb.wires.push((
                            x.port.clone(),
                            PortRef {
                                slug: name(&slug, s, false)?,
                                port: String::new(),
                            },
                        )),
                        BindingKind::Param(Value::File { .. }) => {
                            return Err(format!(
                                "{slug}: lox put its {} into a file of its own, which vislox does not \
                                 move across when adopting; adopt it without, then edit the program \
                                 here (vislox keeps it in a file next to the block's)",
                                x.port
                            ));
                        }
                        BindingKind::Param(v) => nb.args.push((x.port.clone(), v.clone())),
                        BindingKind::Wire(r) => nb.wires.push((
                            x.port.clone(),
                            PortRef {
                                slug: name(&slug, &r.slug, false)?,
                                port: r.port.clone(),
                            },
                        )),
                        BindingKind::Expr(_) => {
                            return Err(format!(
                                "{slug}: lox wrote an expression into {}; vislox does not write those",
                                x.port
                            ));
                        }
                    }
                }
                let (page, page_title) = if crate::periphery::of(&nb.ty).is_some() {
                    (String::new(), String::new())
                } else {
                    page_of(base, &uuid).ok_or_else(|| {
                        format!("{slug} is not drawn on a page of the base config")
                    })?
                };
                // Keyed by its slug in the lock; nothing in the entry names it.
                let object = entry.clone();
                let is_promoted = promoted.contains(&vx(&uuid));
                out.insert(
                    uuid.clone(),
                    Adoption {
                        uuid,
                        slug,
                        page,
                        page_title,
                        block: nb,
                        wires: Vec::new(),
                        externs: BTreeMap::new(),
                        promoted: is_promoted,
                        object,
                        extern_wires: Vec::new(),
                        unwires: Vec::new(),
                        unexternals: Vec::new(),
                    },
                );
            }
            Item::Wire(w) => {
                let from = rename.get(&w.from.slug).cloned().ok_or_else(|| {
                    format!(
                        "lox logic adopt wrote the wire `{} <- {}`, which leaves no block it adopted",
                        w.to, w.from
                    )
                })?;
                let to = name(&from, &w.to.slug, false)?;
                wires.push((
                    from.clone(),
                    FileWire {
                        to: PortRef {
                            slug: to,
                            port: w.to.port.clone(),
                        },
                        from: PortRef {
                            slug: from,
                            port: w.from.port.clone(),
                        },
                    },
                ));
            }
            other => {
                return Err(format!(
                    "lox logic adopt wrote a statement vislox does not write: {}",
                    Module {
                        items: vec![other.clone()]
                    }
                    .to_text()
                    .trim()
                ));
            }
        }
    }
    for (owner, w) in wires {
        let a = out
            .values_mut()
            .find(|a| a.slug == owner)
            .ok_or_else(|| format!("no declaration of {owner}"))?;
        a.wires.push(w);
    }
    for a in out.values_mut() {
        a.externs = needs.remove(&a.slug).unwrap_or_default();
    }
    // The wires lox tracks onto externs, by the adopted block whose port is
    // their source.
    let wires_before: BTreeSet<String> = lock_before
        .get("extern_wires")
        .and_then(|w| w.as_array())
        .into_iter()
        .flatten()
        .map(|w| w.to_string())
        .collect();
    for w in lock_after
        .get("extern_wires")
        .and_then(|w| w.as_array())
        .into_iter()
        .flatten()
        .filter(|w| !wires_before.contains(&w.to_string()))
    {
        let from = w.get("from").and_then(|f| f.as_str()).unwrap_or("");
        let a = out
            .values_mut()
            .find(|a| ports_of(&a.object).contains(from))
            .ok_or_else(|| {
                format!("lox logic adopt tracks a wire from {from}, which no adopted block has")
            })?;
        a.extern_wires.push(w.clone());
    }
    // What lox dropped from the lock for a block it promoted.
    let wires_after: BTreeSet<String> = lock_after
        .get("extern_wires")
        .and_then(|w| w.as_array())
        .into_iter()
        .flatten()
        .map(|w| w.to_string())
        .collect();
    for w in lock_before
        .get("extern_wires")
        .and_then(|w| w.as_array())
        .into_iter()
        .flatten()
        .filter(|w| !wires_after.contains(&w.to_string()))
    {
        let to = w.get("to").and_then(|f| f.as_str()).unwrap_or("");
        if let Some(a) = out.values_mut().find(|a| ports_of(&a.object).contains(to)) {
            a.unwires.push(w.clone());
        }
    }
    for (x, e) in &externals_before {
        if externals.contains_key(x) {
            continue;
        }
        if let Some(a) = uuid_in(e).and_then(|u| out.get_mut(u)) {
            a.unexternals.push(x.to_string());
        }
    }
    for u in picked.keys() {
        if !out.contains_key(u) {
            return Err(format!("lox logic adopt wrote no declaration for {u}"));
        }
    }
    Ok(out.into_values().collect())
}

/// The UUIDs a lockfile entry pins: the block's and its ports'.
pub fn ports_of(entry: &Json) -> BTreeSet<String> {
    let mut s: BTreeSet<String> = entry
        .get("ports")
        .and_then(|p| p.as_object())
        .into_iter()
        .flat_map(|p| p.values())
        .filter_map(|u| u.as_str().map(String::from))
        .collect();
    if let Some(u) = uuid_in(entry) {
        s.insert(u.to_string());
    }
    s
}

// ── checking the compile ────────────────────────────────────────────────

/// `el` without the whitespace between its elements, attributes in order:
/// what two saves of the same block share.
fn normal(el: &Element) -> Element {
    let mut attrs = el.attrs.clone();
    attrs.sort_by(|a, b| a.name.cmp(&b.name));
    Element {
        name: el.name.clone(),
        attrs,
        children: el
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Element(e) => Some(Node::Element(normal(e))),
                Node::Text(t) if t.trim().is_empty() => None,
                Node::Text(t) => Some(Node::Text(t.clone())),
            })
            .collect(),
        self_closing: el.self_closing,
    }
}

/// Whether `a` and `b` have the object `uuid` alike: its element (UUIDs,
/// connectors, wires into it, parameters, place) and the wires out of it.
pub fn same_block(a: &LoxoneDoc, b: &LoxoneDoc, uuid: &str) -> bool {
    let (Some(x), Some(y)) = (find(&a.xml.root, uuid), find(&b.xml.root, uuid)) else {
        return false;
    };
    if normal(x) != normal(y) {
        return false;
    }
    let ports: BTreeSet<&str> = x
        .child_elements()
        .filter(|c| c.name == "Co")
        .filter_map(|c| c.attr("U"))
        .collect();
    let out = |d: &LoxoneDoc| -> BTreeSet<(String, String)> {
        d.wires()
            .into_iter()
            .filter(|w| ports.contains(w.from_port.as_str()))
            .map(|w| (w.from_port, w.to_port))
            .collect()
    };
    out(a) == out(b)
}

/// After the compile of an adoption (or its undo): the pending config is
/// what it was (`old`), and each block of `uuids` is in `new` exactly as
/// the base has it. `Err` says what differs.
pub fn verify(
    base: &LoxoneDoc,
    old: &LoxoneDoc,
    new: &LoxoneDoc,
    uuids: &[String],
) -> Result<(), String> {
    let name = |u: &str| name_in(base, u);
    for u in uuids {
        if !same_block(base, new, u) {
            return Err(format!(
                "lox logic compile does not rebuild {} exactly as the base has it",
                name(u)
            ));
        }
    }
    let d = loxone_config::diff::diff(old, new);
    if !d.logic_is_empty() {
        return Err(format!(
            "lox logic compile changed what the Miniserver would run ({} added, {} removed, {} \
             renamed, {} values, {} moved, {} wires added, {} wires removed)",
            d.added.len(),
            d.removed.len(),
            d.renamed.len(),
            d.param_changes.len(),
            d.placements.len(),
            d.wires_added.len(),
            d.wires_removed.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> LoxoneDoc {
        let xml = std::fs::read_to_string("tests/fixtures/house.Loxone").unwrap();
        LoxoneDoc::parse(xml.as_bytes()).unwrap()
    }

    const GATE: &str = "10000004-0000-0040-ffff504f94000001";

    /// What lox v0.83.0 writes for the fixture's AND gate (checked by hand
    /// with `lox logic adopt tests/fixtures/house.Loxone --uuid … --as gate`).
    fn lox_wrote() -> (BTreeMap<String, String>, Json, Json) {
        let mut after = BTreeMap::new();
        after.insert(
            "erdgeschoss.loxl".to_string(),
            "page \"Erdgeschoss\"\n\nextern temperatur = VirtualIn(iname: \"VI1\")\n\
             extern zaehler = MeterAbsUni(title: \"Zähler\")\nextern licht_og = Or(title: \"Licht OG\")\n\n\
             gate = And(\"O1001\", In1: temperatur)\n\nzaehler.Power <- gate\nlicht_og.In1 <- gate\n"
                .to_string(),
        );
        let lock_before =
            json!({ "lockfile_version": 8, "objects": {}, "externals": {}, "extern_wires": [] });
        let lock_after = json!({
            "lockfile_version": 8,
            "objects": { "gate": { "uuid": GATE, "type": "And",
                "ports": { "I1": "10000004-0000-0041-00ff504f94000001", "I2": "10000004-0000-0042-01ff504f94000001", "Q": "10000004-0000-0043-02ff504f94000001" },
                "layout": { "px": 4032, "py": 768, "px2": 5376, "py2": 1464 },
                "page_uuid": "10000002-0000-0020-ffff504f94000001" } },
            "externals": {
                "licht_og": { "uuid": "10000009-0000-0090-ffff504f94000001", "matched_by": "title" },
                "temperatur": { "uuid": "10000001-0000-0010-ffff504f94000001", "matched_by": "iname" },
                "zaehler": { "uuid": "10000005-0000-0050-ffff504f94000001", "matched_by": "title" } },
            "extern_wires": [
                { "from": "10000004-0000-0043-02ff504f94000001", "to": "10000005-0000-0051-00ff504f94000001" },
                { "from": "10000004-0000-0043-02ff504f94000001", "to": "10000009-0000-0091-00ff504f94000001" } ]
        });
        (after, lock_before, lock_after)
    }

    #[test]
    fn what_lox_adopts_is_moved_into_vislox_s_names() {
        let (after, l0, l1) = lox_wrote();
        let picked = BTreeMap::from([(GATE.to_string(), "gate".to_string())]);
        let a = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &BTreeSet::new(),
            &base(),
        )
        .unwrap();
        assert_eq!(a.len(), 1);
        let a = &a[0];
        assert_eq!(
            (a.slug.as_str(), a.page_title.as_str()),
            ("gate", "Erdgeschoss")
        );
        assert_eq!(a.block.title.as_deref(), Some("O1001"));
        assert_eq!(
            a.block.wires,
            vec![(
                "In1".to_string(),
                PortRef {
                    slug: "vx_10000001_0000_0010_ffff504f94000001".into(),
                    port: String::new()
                }
            )]
        );
        assert_eq!(a.wires.len(), 2);
        assert_eq!(a.wires[0].to.slug, "vx_10000005_0000_0050_ffff504f94000001");
        assert_eq!(a.wires[0].to.port, "Power");
        // Externs pinned by UUID, with the types lox declared.
        assert_eq!(a.externs.len(), 3);
        assert_eq!(
            a.externs["10000005-0000-0050-ffff504f94000001"],
            "MeterAbsUni"
        );
        assert_eq!(a.extern_wires.len(), 2);
        assert_eq!(a.object["uuid"], GATE);
    }

    #[test]
    fn a_bare_name_of_a_let_or_a_day_is_a_value_not_a_wire() {
        let (mut after, l0, l1) = lox_wrote();
        after.insert(
            "erdgeschoss.loxl".into(),
            "page \"Erdgeschoss\"\n\nextern temperatur = VirtualIn(iname: \"VI1\")\n\n\
             gate = And(\"O1001\", In1: temperatur, Days: mon, In2: dauer)\n"
                .into(),
        );
        let mut l1 = l1;
        l1["extern_wires"] = json!([]);
        let picked = BTreeMap::from([(GATE.to_string(), "vx_gate".to_string())]);
        let lets = BTreeSet::from(["dauer".to_string()]);
        let a = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &lets,
            &base(),
        )
        .unwrap();
        let b = &a[0].block;
        assert_eq!(
            b.args,
            vec![
                ("Days".to_string(), Value::Ref("mon".into())),
                ("In2".to_string(), Value::Ref("dauer".into()))
            ]
        );
        assert_eq!(b.wires.len(), 1, "{:?}", b.wires);
        assert_eq!(a[0].externs.len(), 1);
        // Written and read back as values.
        let mut f = crate::blocks::PageFile::new("Erdgeschoss");
        f.blocks.push(b.clone());
        let text = f.render();
        assert!(
            text.contains("Days: mon, In2: dauer, In1: vx_10000001"),
            "{text}"
        );
        assert_eq!(crate::blocks::PageFile::parse(&text).unwrap(), f);
        // Not a let: a name lox would wire, which vislox cannot pin.
        let e = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &BTreeSet::new(),
            &base(),
        )
        .unwrap_err();
        assert!(e.contains("`dauer`"), "{e}");
    }

    #[test]
    fn an_alias_of_the_module_stays_a_bare_name_on_an_adopted_block() {
        // `let dauer = licht.Q` names a port: lox reads `In2: dauer` as it, so
        // adopting keeps the bare name rather than pinning an object (#81).
        let tmp = tempfile::tempdir().unwrap();
        let module = tmp.path().join("pages");
        std::fs::create_dir_all(module.join(crate::edit::DIR)).unwrap();
        std::fs::write(
            module.join("main.loxl"),
            "licht = Monoflop(\"Licht\")\nlet dauer = licht.Q\n",
        )
        .unwrap();
        let decls = crate::edit::module_decls(&module).unwrap();
        assert!(decls.aliases.contains("dauer") && !decls.lets.contains("dauer"));
        let (mut after, l0, l1) = lox_wrote();
        after.insert(
            "erdgeschoss.loxl".into(),
            "page \"Erdgeschoss\"\n\ngate = And(\"O1001\", In2: dauer)\n".into(),
        );
        let mut l1 = l1;
        l1["extern_wires"] = json!([]);
        let picked = BTreeMap::from([(GATE.to_string(), "vx_gate".to_string())]);
        let a = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &decls.bare(),
            &base(),
        )
        .unwrap();
        assert_eq!(
            a[0].block.args,
            vec![("In2".to_string(), Value::Ref("dauer".into()))]
        );
    }

    #[test]
    fn a_block_lox_pinned_under_another_name_gets_vislox_s() {
        let (after, l0, l1) = lox_wrote();
        let picked = BTreeMap::from([(GATE.to_string(), "vx_tor".to_string())]);
        let a = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &BTreeSet::new(),
            &base(),
        )
        .unwrap();
        assert_eq!(a[0].slug, "vx_tor");
        assert_eq!(a[0].wires[1].from.slug, "vx_tor");
    }

    #[test]
    fn what_vislox_does_not_write_is_refused() {
        let (mut after, l0, l1) = lox_wrote();
        let picked = BTreeMap::from([(GATE.to_string(), "gate".to_string())]);
        // A file of the module's own, changed.
        let before =
            BTreeMap::from([("a.loxl".to_string(), "page \"A\"\nx = And()\n".to_string())]);
        after.insert("a.loxl".into(), "page \"A\"\ny = And()\n".into());
        let e = extract(
            &before,
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &BTreeSet::new(),
            &base(),
        )
        .unwrap_err();
        assert!(e.contains("beyond adding"), "{e}");
        // A wire from a block the rest of the module manages.
        let (mut after, l0, mut l1) = lox_wrote();
        after.insert(
            "erdgeschoss.loxl".into(),
            "page \"Erdgeschoss\"\ngate = And(\"O1001\", In1: flur)\n".into(),
        );
        l1["objects"]["flur"] = json!({ "uuid": "1000000b-0000-00b0-ffff504f94000001" });
        let l0 = {
            let mut l = l0;
            l["objects"]["flur"] = json!({ "uuid": "1000000b-0000-00b0-ffff504f94000001" });
            l
        };
        l1["extern_wires"] = json!([]);
        let e = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|_| false,
            &BTreeSet::new(),
            &base(),
        )
        .unwrap_err();
        assert!(e.contains("manages in its own .loxl"), "{e}");
        // vislox's own block is fine.
        let a = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &picked,
            &|s| s == "flur",
            &BTreeSet::new(),
            &base(),
        )
        .unwrap();
        assert_eq!(a[0].block.wires[0].1.slug, "flur");
    }

    #[test]
    fn a_module_mode_value_is_refused_as_a_value_not_a_wire() {
        let (mut after, mut l0, mut l1) = lox_wrote();
        after.insert(
            "erdgeschoss.loxl".into(),
            "page \"Erdgeschoss\"\ngate = And(\"O1001\", switches: party)\n".into(),
        );
        l0["objects"]["party"] = json!({"uuid":"10000011-0000-0110-ffff504f94000001"});
        l1["objects"]["party"] = l0["objects"]["party"].clone();
        l1["extern_wires"] = json!([]);
        let e = extract(
            &BTreeMap::new(),
            &after,
            (&l0, &l1),
            &BTreeMap::from([(GATE.into(), "vx_gate".into())]),
            &|_| false,
            &BTreeSet::new(),
            &base(),
        )
        .unwrap_err();
        assert!(
            e.contains("value references `party`") && !e.contains("wired"),
            "{e}"
        );
    }

    #[test]
    fn the_lock_gains_and_loses_the_block_and_its_wires() {
        let (_, l0, l1) = lox_wrote();
        let bytes = serde_json::to_vec(&json!({ "lockfile_version": 8, "objects": {}, "extern_wires": [{ "from": "a", "to": "b" }] })).unwrap();
        let mut e = LockEdits::default();
        e.add
            .insert("vx_gate".into(), l1["objects"]["gate"].clone());
        e.wires
            .extend(l1["extern_wires"].as_array().unwrap().iter().cloned());
        let added: Json = serde_json::from_slice(&write_lock(&bytes, &e).unwrap()).unwrap();
        assert_eq!(added["objects"]["vx_gate"]["uuid"], GATE);
        assert_eq!(added["extern_wires"].as_array().unwrap().len(), 3);
        let mut back = LockEdits::default();
        back.drop.insert("vx_gate".into());
        let gone: Json = serde_json::from_slice(
            &write_lock(&serde_json::to_vec(&added).unwrap(), &back).unwrap(),
        )
        .unwrap();
        assert!(gone["objects"].as_object().unwrap().is_empty());
        assert_eq!(gone["extern_wires"], json!([{ "from": "a", "to": "b" }]));
        let _ = l0;
    }

    #[test]
    fn slugs_are_vislox_s_and_plain() {
        let none = |_: &str| false;
        assert_eq!(default_slug("Licht Küche", "And", &none), "vx_licht_kueche");
        assert_eq!(default_slug("  ", "MeterAbsUni", &none), "vx_meterabsuni");
        let taken = |s: &str| s == "vx_o1001";
        assert_eq!(default_slug("O1001", "And", &taken), "vx_o1001_2");
        assert!(check_slug("vx_tor").is_ok());
        assert!(check_slug("tor").is_err());
        assert!(check_slug("vx_Tor").is_err());
        assert!(check_slug(&vx(GATE)).is_err());
    }

    #[test]
    fn a_block_rebuilt_as_the_base_has_it_passes() {
        let b = base();
        assert!(same_block(&b, &b, GATE));
        assert!(verify(&b, &b, &b, &[GATE.to_string()]).is_ok());
        // Another title is another block.
        let xml = std::fs::read_to_string("tests/fixtures/house.Loxone")
            .unwrap()
            .replace("Title=\"O1001\"", "Title=\"O1003\"");
        let other = LoxoneDoc::parse(xml.as_bytes()).unwrap();
        assert!(!same_block(&b, &other, GATE));
        let e = verify(&b, &b, &other, &[GATE.to_string()]).unwrap_err();
        assert!(e.contains("exactly as the base"), "{e}");
        // A wire out of it taken away, too.
        let xml = std::fs::read_to_string("tests/fixtures/house.Loxone").unwrap();
        let q = "10000004-0000-0043-02ff504f94000001";
        assert!(xml.contains(q));
        let cut = xml.replacen(&format!("<In Input=\"{q}\"/>"), "", 1);
        assert_ne!(cut, xml, "the gate feeds something");
        let cut = LoxoneDoc::parse(cut.as_bytes()).unwrap();
        assert!(!same_block(&b, &cut, GATE));
        assert_eq!(page_of(&b, GATE).unwrap().1, "Erdgeschoss");
        assert!(
            on_page(&b, "10000002-0000-0020-ffff504f94000001")
                .iter()
                .any(|(u, ..)| u == GATE)
        );
    }
}
