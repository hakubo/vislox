//! Editing the tiles of a Touch Pure Flex (#78): the `tiles` op of
//! `POST /api/edit`, and the `tiles … end` sets and `.tiles` claims it writes
//! into vislox's file of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic programs a Touch Pure Flex's tiles with a named set and a claim
//! (lox's `language.md`, *tiles*):
//!
//! ```text
//! extern vx_20000007_… = TreeDevice(uuid: "20000007-…")
//! extern vx_50000010_… = AutoJalousie(uuid: "50000010-…")
//!
//! tiles vx_flex_kuche_tiles
//!     tile 2()
//!     tile 3(click: "SET(As;Tg;pulse)", text: "=ECHO(As)", to: vx_50000010_…)
//! end
//!
//! vx_20000007_….tiles = vx_flex_kuche_tiles
//! ```
//!
//! - The device is an extern `TreeDevice` pinned by UUID, with a tile layout
//!   (`<btns>`); the layout (which tiles exist, where, their icons) stays with
//!   Loxone Config, so only the indices it has are written.
//! - Each tile listed gets its three functions (click, arrow up, arrow down):
//!   the command, the status text and the blocks it addresses (`to:`, written
//!   as `ctrls=`). `tile <i>()` clears a tile; a tile the set leaves out keeps
//!   its programs.
//! - A target must be API-linked to every device on the set
//!   ([`crate::tiles::Links`]); a command lox knows the abbreviation of (`As`,
//!   `Lico`) refuses a target of another type.
//! - Several devices may claim one set; one device claims one set. Dropping
//!   the claim leaves the programs last written (lox keeps no lock state).
//!
//! An op names targets by UUID: an extern of vislox's file is written for each
//! block Loxone Config drew (a block vislox added is named by its slug).
//! Everything shared with the other set kinds is [`crate::set_claim`]'s.

use crate::edit::{self, Applied, Claims, Ctx, Files, Lock};
use crate::set_claim::{self, SetItem};
use crate::tiles::{self, FUNCTIONS, Links};
use loxone_config::LoxoneDoc;
use loxone_config::logic::{
    ArgItem, Binding, BindingKind, Item, Module, TileDecl, TilesDecl, TilesItem, Value,
};
use loxone_config::xml::Element;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The most tiles one set takes, targets one function takes, and characters a
/// command or text takes (only bounds a request).
const MAX_TILES: usize = 64;
const MAX_TARGETS: usize = 32;
const MAX_TEXT: usize = 500;

/// One function of a tile as an op carries it: its command, the status text
/// and the blocks it addresses (by UUID; a block vislox added and lox has not
/// pinned yet by its slug).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct FnVal {
    pub api: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub to: Vec<String>,
}

/// One tile as an op carries it: its index and the functions it has (a
/// function left out does nothing).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct TileVal {
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click: Option<FnVal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<FnVal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<FnVal>,
}

impl TileVal {
    fn fns(&self) -> [Option<&FnVal>; 3] {
        [self.click.as_ref(), self.up.as_ref(), self.down.as_ref()]
    }
}

/// What a `tiles` op gives a Touch Pure Flex, as the other set kinds' values:
///
/// - `tiles` alone: these programs. They go into the set the device claims
///   when no other device claims it, else into a new set.
/// - `set` and `tiles`: write vislox's set `set` (for every device that
///   claims it) and claim it.
/// - `set` alone: claim vislox's set `set` as it is (*Same tiles as …*).
/// - neither: drop the device's claim, after `restore`.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct TilesVal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles: Option<Vec<TileVal>>,
    /// Set name → its tiles, written back before the rest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub restore: BTreeMap<String, Vec<TileVal>>,
}

/// One function of a `tile` line, as lox reads it: targets by slug.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnLine {
    pub api: String,
    pub text: Option<String>,
    pub to: Vec<String>,
}

/// A `tile <i>(…)` line of a set: the tile's three functions (click, up,
/// down), `None` where it does nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileLine {
    pub index: u32,
    pub fns: [Option<FnLine>; 3],
}

impl TileLine {
    /// The line as lox's parser reads it.
    fn of(d: &TileDecl) -> Result<TileLine, String> {
        let p = loxone_config::logic::tile_program(d)
            .map_err(|e| format!("`tile {}`: {e}", d.index))?;
        let fns = p.functions.map(|f| {
            f.api.map(|api| FnLine {
                api,
                text: f.text,
                to: f.to,
            })
        });
        Ok(TileLine {
            index: p.index,
            fns,
        })
    }

    /// The `tile` statement: the commands, then `text:`/`to:` once when every
    /// function with a command shares them, else per function.
    fn decl(&self) -> TileDecl {
        let mut args = Vec::new();
        let mut bind = |port: String, v: Value| {
            args.push(ArgItem::Binding(Binding {
                port,
                kind: BindingKind::Param(v),
                comment: None,
            }))
        };
        let have: Vec<&FnLine> = self.fns.iter().flatten().collect();
        let shared = |f: &dyn Fn(&FnLine) -> bool| have.len() > 1 && have.iter().all(|x| f(x));
        let text_all = have
            .first()
            .and_then(|f| f.text.clone())
            .filter(|t| shared(&|x| x.text.as_deref() == Some(t)));
        let to_all = have
            .first()
            .map(|f| f.to.clone())
            .filter(|t| !t.is_empty() && shared(&|x| x.to == *t));
        for ((key, _), f) in FUNCTIONS.iter().zip(&self.fns) {
            if let Some(f) = f {
                bind(key.to_string(), Value::Str(f.api.clone()));
            }
        }
        if let Some(t) = &text_all {
            bind("text".into(), Value::Str(t.clone()));
        }
        for t in to_all.iter().flatten() {
            bind("to".into(), Value::Ref(t.clone()));
        }
        for ((key, _), f) in FUNCTIONS.iter().zip(&self.fns) {
            let Some(f) = f else { continue };
            if text_all.is_none()
                && let Some(t) = &f.text
            {
                bind(format!("{key}_text"), Value::Str(t.clone()));
            }
            if to_all.is_none() {
                for t in &f.to {
                    bind(format!("{key}_to"), Value::Ref(t.clone()));
                }
            }
        }
        TileDecl {
            index: self.index,
            args,
            comment: None,
            close_comment: None,
        }
    }
}

/// Written as lox's formatter writes it in a set (one tab in: the set's
/// renderer adds the first), so that `lox logic fmt --check` finds nothing.
impl fmt::Display for TileLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = Module {
            items: vec![Item::Tiles(TilesDecl {
                name: "x".into(),
                body: vec![TilesItem::Tile(self.decl())],
                comment: None,
                end_comment: None,
            })],
        };
        let text = m.to_text();
        let lines: Vec<&str> = text.lines().collect();
        let body: Vec<&str> = lines[1..lines.len().saturating_sub(1)]
            .iter()
            .map(|l| l.strip_prefix('\t').unwrap_or(l))
            .collect();
        write!(f, "{}", body.join("\n\t"))
    }
}

/// The tile sets vislox's file writes: its `tiles` sets and claims.
pub type TileSets = set_claim::SetStore<TilesKind>;

/// Tiles as a kind of set claim ([`set_claim`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TilesKind;

/// What reading and checking tile sets against one config needs.
pub struct Index<'a> {
    doc: &'a LoxoneDoc,
    lock: &'a Lock,
    links: Links,
}

impl Index<'_> {
    /// The block a target's slug names: an extern of vislox's file (its UUID
    /// as the config writes it), or a block of vislox's files.
    fn uuid_of(&self, slug: &str) -> Option<String> {
        if let Some(u) = edit::slug_uuid(slug) {
            return Some(
                tiles::find_ci(&self.doc.xml.root, &u)
                    .and_then(|e| e.attr("U"))
                    .map_or(u, String::from),
            );
        }
        self.lock.uuid_of(slug).map(String::from)
    }
}

/// The lock's slug of `uuid`, case aside.
fn lock_slug<'a>(lock: &'a Lock, uuid: &str) -> Option<&'a str> {
    lock.slugs
        .iter()
        .find(|(_, u)| u.eq_ignore_ascii_case(uuid))
        .map(|(s, _)| s.as_str())
}

fn title_of(el: &Element) -> String {
    el.attr_decoded("Title")
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.into_owned())
        .unwrap_or_else(|| el.attr("Type").unwrap_or_default().to_string())
}

/// A command or text as one line of the app's command language.
fn one_line(what: &str, v: &str) -> Result<(), String> {
    if v.contains(['\n', '\r']) {
        return Err(format!("{what} is one line"));
    }
    if v.chars().count() > MAX_TEXT {
        return Err(format!("{what} is longer than {MAX_TEXT} characters"));
    }
    Ok(())
}

impl set_claim::SetKind for TilesKind {
    type Line = TileLine;
    type Content = Vec<TileVal>;
    type Val = TilesVal;
    const KEYWORD: &'static str = "tiles";
    const BLOCK: &'static str = tiles::DEVICE;
    const WHAT: &'static str = "tiles";
    const STEM: &'static str = "tiles";
    const HEADER: &'static str = "# Touch Pure Flex tiles edited in vislox. A claim writes each tile the set lists (its click, up\n\
         # and down commands, texts and targets); the others stay. Deleting it leaves the tiles as last written.\n";
    const CONTENT_KEY: &'static str = "tiles";
    const MEMBERS_KEY: &'static str = "devices";
    const ENTRIES_KEY: &'static str = "tiles";
    const SETS_KEY: &'static str = "tile_sets";
    type Index<'a> = Index<'a>;

    fn store(c: &Claims) -> &TileSets {
        &c.7
    }
    fn store_mut(c: &mut Claims) -> &mut TileSets {
        &mut c.7
    }
    fn wrong_type(name: &str) -> String {
        format!(
            "{name}: only a Touch Pure Flex's tiles are written in Lox Logic (a Tree device with a \
             tile layout)"
        )
    }
    fn parts(v: &TilesVal) -> set_claim::Parts<'_, Self> {
        (v.set.as_ref(), v.tiles.as_ref(), &v.restore)
    }
    fn make(
        set: Option<String>,
        tiles: Option<Vec<TileVal>>,
        restore: BTreeMap<String, Vec<TileVal>>,
    ) -> TilesVal {
        TilesVal {
            set,
            tiles,
            restore,
        }
    }
    fn index<'a>(doc: &'a LoxoneDoc, lock: &'a Lock) -> Index<'a> {
        Index {
            doc,
            lock,
            links: Links::read(doc),
        }
    }
    fn content_of(lines: &[TileLine], idx: &Index<'_>) -> Option<Vec<TileVal>> {
        lines
            .iter()
            .map(|l| {
                let f = |i: usize| -> Option<Option<FnVal>> {
                    let Some(f) = &l.fns[i] else {
                        return Some(None);
                    };
                    Some(Some(FnVal {
                        api: f.api.clone(),
                        text: f.text.clone(),
                        to: f.to.iter().map(|s| idx.uuid_of(s)).collect::<Option<_>>()?,
                    }))
                };
                Some(TileVal {
                    index: l.index,
                    click: f(0)?,
                    up: f(1)?,
                    down: f(2)?,
                })
            })
            .collect()
    }
    fn read_item(item: &Item) -> Option<SetItem<TileLine>> {
        let vx = |n: &str| n.starts_with(crate::blocks::SLUG_PREFIX);
        match item {
            Item::Tiles(t) if vx(&t.name) => Some(
                match t.tiles().map(TileLine::of).collect::<Result<Vec<_>, _>>() {
                    Ok(lines) => SetItem::Set(t.name.clone(), lines),
                    Err(e) => SetItem::Bad(format!("`tiles {}`: {e}", t.name)),
                },
            ),
            Item::TilesClaim(c) if vx(&c.slug) => {
                Some(SetItem::Claim(c.slug.clone(), c.tiles.clone()))
            }
            _ => None,
        }
    }
    fn op(op: &crate::edit::Op) -> Option<(&str, Option<&Self::Val>)> {
        match op {
            crate::edit::Op::Tiles { uuid, value } => Some((uuid, value.as_ref())),
            _ => None,
        }
    }
    fn missed(
        files: &Files,
        out: &LoxoneDoc,
        lock: &Lock,
        uuids: &[String],
        restored: &[String],
    ) -> Vec<String> {
        missed(files, out, lock, uuids, restored)
    }
    fn differ<'a>(a: &'a LoxoneDoc, b: &'a LoxoneDoc) -> Box<dyn Fn(&str) -> bool + 'a> {
        Box::new(move |u| tiles::differ(a, b, u))
    }
    fn lines_of(
        _: &Vec<TileVal>,
        _: &Ctx,
        _: &Index<'_>,
        _: &[(String, &Element)],
    ) -> Result<Vec<TileLine>, String> {
        Err("tiles name their targets by vislox's files (lines_with)".into())
    }
    /// The op's tiles as lines: each index once, each command and text one
    /// line, each target a block of the pending config named as vislox's
    /// files name it (a new extern for a block Loxone Config drew).
    fn lines_with(
        list: &Vec<TileVal>,
        files: &Files,
        ctx: &Ctx,
        _: &Index<'_>,
        _: &[(String, &Element)],
    ) -> Result<(Vec<TileLine>, Vec<(String, String)>), String> {
        if list.len() > MAX_TILES {
            return Err(format!("at most {MAX_TILES} tiles in one set"));
        }
        let mut seen = BTreeSet::new();
        let mut externs: Vec<(String, String)> = Vec::new();
        let mut lines = Vec::with_capacity(list.len());
        for t in list {
            if !seen.insert(t.index) {
                return Err(format!(
                    "tile {} is listed twice; a tile is in a set once",
                    t.index
                ));
            }
            let mut fns: [Option<FnLine>; 3] = Default::default();
            for (((key, _), f), slot) in FUNCTIONS.iter().zip(t.fns()).zip(fns.iter_mut()) {
                let Some(f) = f else { continue };
                let at = format!("tile {} {key}", t.index);
                if f.api.trim().is_empty() {
                    return Err(format!(
                        "{at}: give a command, or leave the function out to clear it"
                    ));
                }
                one_line(&format!("{at}: the command"), &f.api)?;
                let text = f.text.clone().filter(|x| !x.is_empty());
                if let Some(x) = &text {
                    one_line(&format!("{at}: the status text"), x)?;
                }
                if f.to.len() > MAX_TARGETS {
                    return Err(format!("{at}: at most {MAX_TARGETS} targets"));
                }
                let mut to = Vec::with_capacity(f.to.len());
                for u in &f.to {
                    let u = u.trim();
                    // A block of vislox's files lox has not pinned yet: its slug.
                    let slug = if files.block(u).is_some() && ctx.lock.uuid_of(u).is_none() {
                        u.to_string()
                    } else {
                        let el = tiles::find_ci(&ctx.doc.xml.root, u)
                            .ok_or_else(|| format!("{at}: no object {u} in the pending config"))?;
                        let uu = el.attr("U").unwrap_or(u);
                        let ty = el.attr("Type").unwrap_or_default();
                        match lock_slug(ctx.lock, uu) {
                            Some(s) if files.block(s).is_some() => s.to_string(),
                            Some(_) => {
                                return Err(format!(
                                    "{at}: {} is managed by the module, and vislox's files name \
                                     only their own blocks and Loxone Config's; program the tile in \
                                     the module's own file, or pick another target",
                                    title_of(el)
                                ));
                            }
                            None => {
                                if let Some((_, c)) = files
                                    .claims
                                    .0
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(uu))
                                    && !c.ty.is_empty()
                                    && c.ty != ty
                                {
                                    return Err(format!(
                                        "{at}: vislox's file names {uu} as a {}, not a {ty}",
                                        c.ty
                                    ));
                                }
                                externs.push((uu.to_string(), ty.to_string()));
                                edit::slug(uu)
                            }
                        }
                    };
                    if to.contains(&slug) {
                        return Err(format!("{at}: a target is listed twice"));
                    }
                    to.push(slug);
                }
                *slot = Some(FnLine {
                    api: f.api.clone(),
                    text,
                    to,
                });
            }
            lines.push(TileLine {
                index: t.index,
                fns,
            });
        }
        // In the layout's index order, whatever order the op gives.
        lines.sort_by_key(|l: &TileLine| l.index);
        Ok((lines, externs))
    }
    fn named(lines: &[TileLine]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in lines {
            for t in l.fns.iter().flatten().flat_map(|f| &f.to) {
                if !out.contains(t) {
                    out.push(t.clone());
                }
            }
        }
        out
    }
    /// For every device on the set: a Touch Pure Flex that has each tile the
    /// set lists, and each target a block API-linked to it, of the type a
    /// command lox knows addresses.
    fn check(
        lines: &[TileLine],
        _: &Ctx,
        idx: &Index<'_>,
        set: &str,
        members: &[(String, &Element)],
    ) -> Result<(), String> {
        for (_, dev) in members {
            let name = title_of(dev);
            if !tiles::is_flex(dev) {
                return Err(format!(
                    "{name} has no Touch Pure Flex tile layout (no `<btns>`); lay out its tiles in \
                     Loxone Config first"
                ));
            }
            let du = dev.attr("U").unwrap_or_default();
            let have = tiles::indices(dev);
            for l in lines {
                if !have.contains(&l.index) {
                    let list: Vec<String> = have.iter().map(u32::to_string).collect();
                    return Err(format!(
                        "`{set}`: {name} has no tile {} (it has {}); add the tile in Loxone Config \
                         first",
                        l.index,
                        list.join(", ")
                    ));
                }
                for ((key, _), f) in FUNCTIONS.iter().zip(&l.fns) {
                    let Some(f) = f else { continue };
                    let types = tiles::addressed_types(&f.api);
                    for t in &f.to {
                        let at = format!("`{set}`, tile {} {key}", l.index);
                        let u = idx
                            .uuid_of(t)
                            .ok_or_else(|| format!("{at}: `{t}` names no block"))?;
                        let target = tiles::find_ci(&idx.doc.xml.root, &u)
                            .map(title_of)
                            .unwrap_or_else(|| u.clone());
                        let Some(ty) = idx.links.linked(du, &u) else {
                            return Err(format!(
                                "{at}: {target} is not API-linked to {name}; wire its OutputAPI \
                                 to the device's API connector (in Loxone Config, or through an \
                                 OutputRef of it) first"
                            ));
                        };
                        if let Some(types) = &types
                            && !types.contains(&ty)
                        {
                            return Err(format!(
                                "{at}: {target} is a {ty}, but `{}` addresses {}",
                                f.api,
                                types.join("/")
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Apply a `tiles` op: give the Touch Pure Flex `uuid` the tile programs
/// `value`, or drop vislox's claim on them (`None`). Its `previous` undoes it
/// whole.
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&TilesVal>,
) -> Result<Applied, String> {
    set_claim::apply::<TilesKind>(files, ctx, uuid, value)
}

/// After the compile: the devices among `uuids`, those sharing a set with
/// them, and those on the sets `restored`, whose tiles in `out` are not as
/// their set programs them. lox writes a claim exactly; this catches one it
/// did not.
pub fn missed(
    files: &Files,
    out: &LoxoneDoc,
    lock: &Lock,
    uuids: &[String],
    restored: &[String],
) -> Vec<String> {
    let s = &files.claims.7;
    let sets = set_claim::sets_written::<TilesKind>(files, lock, uuids, restored);
    let mut bad = Vec::new();
    if sets.is_empty() {
        return bad;
    }
    let idx = <TilesKind as set_claim::SetKind>::index(out, lock);
    for set in sets {
        let Some(lines) = s.sets.get(set) else {
            continue;
        };
        for t in s.claimants(set) {
            let Some(u) = set_claim::member_of(files, lock, t) else {
                continue;
            };
            let Some(el) = tiles::find_ci(&out.xml.root, &u) else {
                continue;
            };
            let have = tiles::tiles_of(el).unwrap_or_default();
            let ok = lines.iter().all(|l| {
                let Some(tile) = have.iter().find(|x| x.index == l.index) else {
                    return false;
                };
                tile.functions
                    .iter()
                    .zip(&l.fns)
                    .all(|(got, want)| match want {
                        None => *got == tiles::Function::default(),
                        Some(w) => {
                            let to: Option<Vec<String>> =
                                w.to.iter()
                                    .map(|x| idx.uuid_of(x).map(|u| u.to_ascii_lowercase()))
                                    .collect();
                            got.api.as_deref() == Some(w.api.as_str())
                                && got.text == w.text
                                && to.is_some_and(|to| to == got.ctrls_lower())
                        }
                    })
            });
            if !ok {
                bad.push(title_of(el));
            }
        }
    }
    bad
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set_claim::SetKind;

    fn f(api: &str, text: Option<&str>, to: &[&str]) -> Option<FnLine> {
        Some(FnLine {
            api: api.into(),
            text: text.map(String::from),
            to: to.iter().map(|x| x.to_string()).collect(),
        })
    }

    #[test]
    fn a_line_is_written_as_lox_formats_it_and_read_back() {
        let lines = [
            TileLine {
                index: 2,
                fns: [None, None, None],
            },
            TileLine {
                index: 3,
                fns: [
                    f("SET(As;Tg;pulse)", Some("=ECHO(As)"), &["vx_a"]),
                    f("SET(As;Co;pulse)", Some("=ECHO(As)"), &["vx_a"]),
                    f("SET(As;Cc;pulse)", Some("=ECHO(As)"), &["vx_a"]),
                ],
            },
            TileLine {
                index: 5,
                fns: [
                    f("SET(Lico;M+;pulse)&WAIT(500)", Some("a \"b\""), &[]),
                    f("SET(As;Co;pulse)", None, &["vx_a", "vx_b"]),
                    None,
                ],
            },
        ];
        let mut s = TileSets::default();
        s.sets.insert("vx_t".into(), lines.to_vec());
        s.claims.insert("vx_d".into(), "vx_t".into());
        let text = s.render();
        assert!(
            text.contains("tiles vx_t\n\ttile 2()\n\ttile 3(\n"),
            "{text}"
        );
        assert!(
            text.contains("\t\ttext: \"=ECHO(As)\",\n\t\tto: vx_a,\n"),
            "{text}"
        );
        assert!(
            text.contains("\t\tup_to: vx_a,\n\t\tup_to: vx_b,\n"),
            "{text}"
        );
        assert!(text.contains("vx_d.tiles = vx_t\n"), "{text}");
        // What lox's formatter writes is what vislox writes.
        let m = Module::parse_fragment(&text).unwrap();
        assert_eq!(m.to_text().trim(), text.trim());
        let item = m
            .items
            .iter()
            .find(|i| matches!(i, Item::Tiles(_)))
            .unwrap();
        match TilesKind::read_item(item) {
            Some(SetItem::Set(name, back)) => {
                assert_eq!(name, "vx_t");
                assert_eq!(back, lines);
            }
            _ => panic!("not read"),
        }
        assert_eq!(TilesKind::named(&lines), ["vx_a", "vx_b"]);
        // A tile lox would refuse is no line vislox reads.
        let bad = Module::parse_fragment("tiles vx_t\n\ttile 2(text: \"x\")\nend\n").unwrap();
        assert!(matches!(
            TilesKind::read_item(&bad.items[0]),
            Some(SetItem::Bad(e)) if e.contains("without a command")
        ));
    }
}
