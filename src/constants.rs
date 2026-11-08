//! `let` constants and `defaults` statements (#81): which ports of which
//! blocks take their value from one, read off the module's `.loxl` files.
//!
//! lox substitutes a constant before it writes `Def=`, and applies a type's
//! `defaults` to every managed block of the type that does not bind the port
//! itself, so the compiled config keeps no trace of either. vislox reads each
//! file with lox's parser, at the top level only (a template's body is made
//! once per instance, #83), and pins each binding to a block:
//!
//! - `slug = Type(Port: name)`, a managed block (the lockfile's `objects`);
//! - `x.Port = name` and `extern x = Type(…, Port: name)`, an extern (its
//!   `uuid: "…"`, else the lockfile's `externals`);
//! - `defaults Type(Port: value)` on every managed block of the type that
//!   does not bind `Port` ([`loxone_config::logic::defaults_taken`]); a
//!   default that names a constant counts for that constant too.
//!
//! A file that does not parse is left out: `lox logic check` says why (#65).
//! An alias (`let x = y.Q`) names a port, never a value, and is left out.

use crate::managed::{Declarations, bindings, code, template_bodies};
use crate::module_edit::{call_end, defaults_line, source_of};
use loxone_config::logic::{
    ArgItem, BindingKind, BlockDecl, DefaultsDecl, Item, MatchSpec, Module, Value, defaults_taken,
};
use loxone_config::port_names;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A `let` constant of the module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Let {
    pub name: String,
    /// As written: `28`, `30min`, `19:00`, `on`, `"Text"`.
    pub value: String,
    /// Shown as [`crate::managed::Source::file`]; 1-based line.
    pub file: String,
    pub line: Option<usize>,
    /// The ports bound to it, through a `defaults` value too.
    pub uses: usize,
    /// Declared in vislox's files, so vislox changes it (the `let` op).
    pub ours: bool,
    /// The statement's line as the file has it: the `let` op on a constant
    /// of the module's own files names it (`source`), and is refused when the
    /// file no longer has it.
    pub source: String,
    /// Why vislox does not change it (a text constant), or `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

/// A `defaults <Type>(…)` statement of the module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Defaults {
    #[serde(rename = "type")]
    pub ty: String,
    pub file: String,
    pub line: Option<usize>,
    /// Port (its Lox Logic name) → value as written.
    pub params: BTreeMap<String, String>,
    /// The ports that take one of its values.
    pub uses: usize,
    /// In vislox's files: made there, by the `defaults` op.
    pub ours: bool,
    /// The statement's lines as the file has them (`source` of the
    /// `defaults` op on the module's own statement).
    pub source: String,
    /// Port (its Lox Logic name) → the managed blocks that take its value:
    /// what a change of it changes, and what the op's `blocks` must name.
    pub reach: BTreeMap<String, Vec<Reached>>,
}

/// A managed block a default reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reached {
    /// Its lockfile key (an instance's block: the expanded slug).
    pub slug: String,
    /// Its UUID, when the lockfile has it.
    pub uuid: Option<String>,
}

/// Where one port of one block takes its value from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Use {
    /// `let` or `defaults`.
    pub kind: &'static str,
    /// The constant, or the type of the `defaults` statement.
    pub name: String,
    /// The constant a `defaults` value names (`defaults T(Port: name)`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constant: Option<String>,
    /// The value as the statement writes it (the constant's, for a default
    /// that names one).
    pub value: String,
    /// The statement: the `let` (the constant's, for a default that names
    /// one) or the `defaults`.
    pub file: String,
    pub line: Option<usize>,
    /// Where the block is bound or declared: the argument or `x.Port = …`
    /// line of a `let`, the block's declaration for a default.
    pub at_file: String,
    pub at_line: Option<usize>,
    /// The constant (or `defaults` statement) is in vislox's files.
    pub ours: bool,
    /// The binding or block is in vislox's files.
    pub at_ours: bool,
    /// Why this port cannot take a value of its own from vislox (*Only this
    /// block*): the module's own file binds it (`x.Port = name` on an
    /// extern), and a claim of vislox's would claim it twice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_refusal: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Constants {
    pub lets: Vec<Let>,
    pub defaults: Vec<Defaults>,
    /// Object UUID (lowercase) → connector key (or setting name) → its source.
    pub uses: BTreeMap<String, BTreeMap<String, Use>>,
}

impl Constants {
    /// Where the module's own files bind port `key` of `uuid` to a constant
    /// (`x.Port = name` on an extern): its `file:line`. vislox does not claim
    /// such a port too.
    pub fn module_binding(&self, uuid: &str, key: &str) -> Option<String> {
        let u = self.uses.get(&uuid.to_ascii_lowercase())?.get(key)?;
        (u.kind == "let" && !u.at_ours).then(|| match u.at_line {
            Some(l) => format!("{}:{l}", u.at_file),
            None => u.at_file.clone(),
        })
    }
}

/// One file parsed, with its lines.
struct Parsed<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    /// The lines inside a template's body: none of its statements is the
    /// module's own.
    body: BTreeSet<usize>,
    items: Vec<Item>,
}

/// The line (1-based) of `slug.port = …` at the top level of `p`.
fn set_line(p: &Parsed, slug: &str, port: &str) -> Option<usize> {
    p.lines.iter().enumerate().find_map(|(n, l)| {
        if p.body.contains(&n) {
            return None;
        }
        let (lhs, rhs) = code(l).split_once('=')?;
        let (s, q) = lhs.trim().split_once('.')?;
        (s.trim() == slug && q.trim() == port && !rhs.starts_with('=')).then_some(n + 1)
    })
}

/// Every `let` and `defaults` of the module's files (`files`: the path as
/// shown, the text), and the ports that take a value from one. `ours` tells
/// a file of vislox's directory.
pub fn constants<T: AsRef<str>>(
    files: &[(String, T)],
    lock: &crate::edit::Lock,
    ours: impl Fn(&str) -> bool,
) -> Constants {
    let decls = Declarations::new(files);
    let parsed: Vec<Parsed> = files
        .iter()
        .filter_map(|(f, t)| {
            let t = t.as_ref();
            let lines: Vec<&str> = t.split('\n').collect();
            Some(Parsed {
                file: f.as_str(),
                body: template_bodies(&lines),
                lines,
                items: Module::parse_fragment(t).ok()?.items,
            })
        })
        .collect();
    let mut out = Constants::default();
    // name → index in `out.lets`; type → (index in `out.defaults`, decl).
    let mut lets: BTreeMap<String, usize> = BTreeMap::new();
    let mut defaults: BTreeMap<String, (usize, &DefaultsDecl)> = BTreeMap::new();
    // An extern's slug → (type, UUID when known).
    let mut externs: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    for p in &parsed {
        for item in &p.items {
            match item {
                Item::Let(l) if !matches!(l.value, Value::Port(_)) => {
                    if lets.contains_key(&l.name) {
                        continue;
                    }
                    lets.insert(l.name.clone(), out.lets.len());
                    let line = decls.at(&l.name, Some(p.file)).map(|(_, l)| l);
                    out.lets.push(Let {
                        name: l.name.clone(),
                        value: l.value.to_string(),
                        file: p.file.to_string(),
                        source: line
                            .map(|n| source_of(&p.lines, n - 1, n - 1))
                            .unwrap_or_default(),
                        refusal: matches!(l.value, Value::Str(_)).then(|| {
                            format!(
                                "a text constant: vislox does not change it; change it in {}",
                                p.file
                            )
                        }),
                        line,
                        uses: 0,
                        ours: ours(p.file),
                    });
                }
                Item::Defaults(d) => {
                    if defaults.contains_key(&d.block_type) {
                        continue;
                    }
                    defaults.insert(d.block_type.clone(), (out.defaults.len(), d));
                    let line = defaults_line(&p.lines, &d.block_type);
                    out.defaults.push(Defaults {
                        ty: d.block_type.clone(),
                        file: p.file.to_string(),
                        line: line.map(|n| n + 1),
                        source: line
                            .map(|n| source_of(&p.lines, n, call_end(&p.lines, n)))
                            .unwrap_or_default(),
                        params: d
                            .params()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                        uses: 0,
                        ours: ours(p.file),
                        reach: BTreeMap::new(),
                    });
                }
                Item::Extern(e) => {
                    let uuid = match &e.match_spec {
                        MatchSpec::Uuid(u) => Some(u.clone()),
                        _ => lock.externals.get(&e.slug).cloned(),
                    };
                    externs
                        .entry(e.slug.clone())
                        .or_insert((e.block_type.clone(), uuid));
                }
                _ => {}
            }
        }
    }
    let mut uses: Vec<(Option<String>, String, Use)> = Vec::new();
    let let_use = |name: &str, at_file: &str, at_line: Option<usize>| -> Option<Use> {
        let l = &out.lets[*lets.get(name)?];
        Some(Use {
            kind: "let",
            name: name.to_string(),
            constant: None,
            value: l.value.clone(),
            file: l.file.clone(),
            line: l.line,
            at_file: at_file.to_string(),
            at_line,
            ours: l.ours,
            at_ours: ours(at_file),
            block_refusal: None,
        })
    };
    for p in &parsed {
        let body_line = |b: &BlockDecl| decls.at(&b.slug, Some(p.file)).map(|(_, l)| l);
        for item in &p.items {
            match item {
                Item::Block(b) => {
                    let uuid = lock.slugs.get(&b.slug).cloned();
                    let decl = body_line(b);
                    let lines: BTreeMap<String, usize> = decl
                        .map(|l| bindings(&p.lines, l - 1))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(k, n)| (k, n + 1))
                        .collect();
                    for x in b.bindings() {
                        if let BindingKind::Param(Value::Ref(n)) = &x.kind
                            && let Some(u) = let_use(n, p.file, lines.get(&x.port).copied())
                        {
                            uses.push((uuid.clone(), key(&b.block_type, &x.port), u));
                        }
                    }
                    let Some((i, d)) = defaults.get(&b.block_type) else {
                        continue;
                    };
                    let dd = &out.defaults[*i];
                    for (port, v) in defaults_taken(d, b) {
                        let named = match v {
                            Value::Ref(n) => lets.get(n).map(|&i| &out.lets[i]),
                            _ => None,
                        };
                        uses.push((
                            uuid.clone(),
                            key(&b.block_type, port),
                            Use {
                                kind: "defaults",
                                name: b.block_type.clone(),
                                constant: named.map(|l| l.name.clone()),
                                value: named.map_or_else(|| v.to_string(), |l| l.value.clone()),
                                file: dd.file.clone(),
                                line: dd.line,
                                at_file: p.file.to_string(),
                                at_line: decl,
                                ours: ours(&dd.file),
                                at_ours: ours(p.file),
                                block_refusal: None,
                            },
                        ));
                    }
                }
                Item::Extern(e) => {
                    let uuid = externs.get(&e.slug).and_then(|x| x.1.clone());
                    // Each argument at its own line, as on a block.
                    let lines: BTreeMap<String, usize> = decls
                        .at(&e.slug, Some(p.file))
                        .map(|(_, l)| bindings(&p.lines, l - 1))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(k, n)| (k, n + 1))
                        .collect();
                    for a in &e.args {
                        if let ArgItem::Binding(x) = a
                            && let BindingKind::Param(Value::Ref(n)) = &x.kind
                            && let Some(mut u) = let_use(n, p.file, lines.get(&x.port).copied())
                        {
                            u.block_refusal = refusal_of(&u);
                            uses.push((uuid.clone(), key(&e.block_type, &x.port), u));
                        }
                    }
                }
                Item::Set(s) => {
                    let Value::Ref(n) = &s.value else { continue };
                    let Some((ty, uuid)) = externs.get(&s.target.slug) else {
                        continue;
                    };
                    let line = set_line(p, &s.target.slug, &s.target.port);
                    if let Some(mut u) = let_use(n, p.file, line) {
                        u.block_refusal = refusal_of(&u);
                        uses.push((uuid.clone(), key(ty, &s.target.port), u));
                    }
                }
                _ => {}
            }
        }
    }
    for (uuid, key, u) in uses {
        if let Some(&i) = lets.get(u.constant.as_deref().unwrap_or(&u.name))
            && (u.kind == "let" || u.constant.is_some())
        {
            out.lets[i].uses += 1;
        }
        if u.kind == "defaults"
            && let Some((i, _)) = defaults.get(&u.name)
        {
            out.defaults[*i].uses += 1;
        }
        if let Some(uuid) = uuid {
            out.uses
                .entry(uuid.to_ascii_lowercase())
                .or_default()
                .entry(key)
                .or_insert(u);
        }
    }
    // What each default reaches: worked out on the module expanded, as lox
    // applies defaults (an instance's blocks too). A module that does not
    // read reaches nothing known.
    for d in &mut out.defaults {
        if let Ok(blocks) = typed_blocks(files, &d.ty) {
            for port in d.params.keys() {
                d.reach.insert(port.clone(), reached(&blocks, lock, port));
            }
        }
    }
    out
}

/// Why *Only this block* is off on a use: the module's own file binds the
/// port (`x.Port = name` on an extern).
fn refusal_of(u: &Use) -> Option<String> {
    (!u.at_ours).then(|| {
        let at = match u.at_line {
            Some(l) => format!("{}:{l}", u.at_file),
            None => u.at_file.clone(),
        };
        format!(
            "the module's own file binds it ({at}): a value of this block's own would claim it              twice; change the constant, or the binding there"
        )
    })
}

/// The managed blocks of type `ty` after the module (`files`, every file of
/// it by path) is expanded without its `defaults <ty>` statement: what a
/// default of the type may reach. lox's error, naming the file, when the
/// module does not read.
fn typed_blocks<T: AsRef<str>>(files: &[(String, T)], ty: &str) -> Result<Vec<BlockDecl>, String> {
    let mut fragments = Vec::new();
    for (f, t) in files {
        let mut m = Module::parse_fragment(t.as_ref()).map_err(|e| format!("{f}: {e}"))?;
        m.items
            .retain(|i| !matches!(i, Item::Defaults(d) if d.block_type == ty));
        fragments.push(m);
    }
    let m = Module::from_fragments(fragments)
        .expand()
        .map_err(|e| e.to_string())?;
    Ok(m.items
        .into_iter()
        .filter_map(|i| match i {
            Item::Block(b) if b.block_type == ty => Some(b),
            _ => None,
        })
        .collect())
}

/// Of `blocks`, those that do not bind `port` themselves: a default of it
/// reaches them.
fn reached(blocks: &[BlockDecl], lock: &crate::edit::Lock, port: &str) -> Vec<Reached> {
    let mut out: Vec<Reached> = blocks
        .iter()
        .filter(|b| !b.bindings().any(|x| x.port == port))
        .map(|b| Reached {
            slug: b.slug.clone(),
            uuid: lock.slugs.get(&b.slug).cloned(),
        })
        .collect();
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out.dedup();
    out
}

/// The managed blocks a default of `ty`'s `port` (its Lox Logic name)
/// reaches in the module `files` (every file, by path), with the module's
/// own statement for the type or without: those of the type that do not
/// bind the port themselves.
pub fn reach<T: AsRef<str>>(
    files: &[(String, T)],
    lock: &crate::edit::Lock,
    ty: &str,
    port: &str,
) -> Result<Vec<Reached>, String> {
    Ok(reached(&typed_blocks(files, ty)?, lock, port))
}

/// A value of the module's own `let` or of a `defaults` statement that is
/// not what the deployed revision's files give it (#81): compared by value,
/// not by how it is written. vislox's own constants are `GET /api/edits`'
/// `claims`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pending {
    /// `let` or `defaults`.
    pub kind: &'static str,
    /// The constant, or the statement's type.
    pub name: String,
    /// A default's port: its Lox Logic name, and its connector key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Now, and in the deployed revision's files (`None`: none).
    pub value: Option<String>,
    pub deployed: Option<String>,
    pub file: String,
    pub line: Option<usize>,
    /// The statement is in vislox's files.
    pub ours: bool,
    /// The ports a constant gives its value to.
    pub uses: usize,
    /// The managed blocks a default reaches: what it changes.
    pub blocks: Vec<Reached>,
    /// The op that puts the deployed value back, or `None` when vislox cannot
    /// (a value the module's own file did not have then).
    pub revert: Option<crate::edit::Op>,
}

/// What [`Constants`] of the module now (`files`, every file by path) gives
/// differently from `was`, the deployed revision's files'.
pub fn pending<T: AsRef<str>>(
    now: &Constants,
    was: &Constants,
    files: &[(String, T)],
    lock: &crate::edit::Lock,
) -> Vec<Pending> {
    let mut out = Vec::new();
    for l in now.lets.iter().filter(|l| !l.ours) {
        let w = was.lets.iter().find(|w| w.name == l.name);
        if w.is_some_and(|w| w.value == l.value) {
            continue;
        }
        out.push(Pending {
            kind: "let",
            name: l.name.clone(),
            port: None,
            key: None,
            value: Some(l.value.clone()),
            deployed: w.map(|w| w.value.clone()),
            file: l.file.clone(),
            line: l.line,
            ours: false,
            uses: l.uses,
            blocks: Vec::new(),
            revert: w.map(|w| crate::edit::Op::Let {
                name: l.name.clone(),
                value: Some(w.value.clone()),
                source: Some(l.source.clone()),
            }),
        });
    }
    let types: BTreeSet<&str> = now
        .defaults
        .iter()
        .chain(&was.defaults)
        .map(|d| d.ty.as_str())
        .collect();
    for ty in types {
        let n = now.defaults.iter().find(|d| d.ty == ty);
        let w = was.defaults.iter().find(|d| d.ty == ty);
        // A statement of the module's own that is gone was taken out by hand:
        // not vislox's to put back.
        let Some(stmt) = n.or(w.filter(|w| w.ours)) else {
            continue;
        };
        let ports: BTreeSet<&String> = n
            .iter()
            .chain(w.iter())
            .flat_map(|d| d.params.keys())
            .collect();
        for port in ports {
            let now_v = n.and_then(|d| d.params.get(port)).cloned();
            let was_v = w.and_then(|d| d.params.get(port)).cloned();
            if now_v == was_v {
                continue;
            }
            let blocks = match n.and_then(|d| d.reach.get(port)) {
                Some(r) => r.clone(),
                None => reach(files, lock, ty, port).unwrap_or_default(),
            };
            let ours = n.is_none_or(|d| d.ours);
            let key = key(ty, port);
            let revert = (ours || was_v.is_some()).then(|| crate::edit::Op::Defaults {
                ty: ty.to_string(),
                port: key.clone(),
                value: was_v.clone(),
                source: (!ours).then(|| stmt.source.clone()),
                blocks: Some(blocks.iter().map(|b| b.slug.clone()).collect()),
            });
            out.push(Pending {
                kind: "defaults",
                name: ty.to_string(),
                port: Some(port.clone()),
                key: Some(key),
                value: now_v,
                deployed: was_v,
                file: stmt.file.clone(),
                line: stmt.line,
                ours,
                uses: blocks.len(),
                blocks,
                revert,
            });
        }
    }
    out
}

/// The config's key of port `port` (its Lox Logic name) of `ty`; a setting
/// keeps its name.
fn key(ty: &str, port: &str) -> String {
    port_names::key_or_raw(ty, port).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIV: &str = "10000006-0000-0060-FFFF504F94000001";
    const LICHT: &str = "a0000001-0000-0000-ffff504f94000001";
    const FLUR: &str = "a0000002-0000-0000-ffff504f94000001";
    const AND: &str = "10000004-0000-0040-ffff504f94000001";

    fn module() -> Vec<(String, String)> {
        vec![
            (
                "pages/heizung.loxl".into(),
                format!(
                    "page \"Heizung\"\n\
                     let temp_schwelle = 28 # °C\n\
                     let aussen = licht.Q\n\
                     let nachlauf = 30min\n\
                     defaults Monoflop(\n\
                     \tTime: nachlauf,\n\
                     )\n\
                     defaults Div(In2: 4)\n\
                     \n\
                     extern teiler = Div(uuid: \"{DIV}\")\n\
                     teiler.In2 = temp_schwelle\n\
                     licht = Monoflop(\"Licht\")\n\
                     flur = Monoflop(\n\
                     \t\"Flur\",\n\
                     \tTime: 10s,\n\
                     )\n\
                     template paar(t: number)\n\
                     \tm = Monoflop(Time: temp_schwelle)\n\
                     end\n"
                ),
            ),
            (
                "pages/vislox/edits.loxl".into(),
                format!("let vx_a = 5\n\nextern vx_x = And(uuid: \"{AND}\")\nvx_x.In1 = vx_a\n"),
            ),
            ("pages/broken.loxl".into(), "let = (\n".into()),
        ]
    }

    fn lock() -> crate::edit::Lock {
        crate::edit::Lock {
            slugs: [("licht", LICHT), ("flur", FLUR)]
                .into_iter()
                .map(|(s, u)| (s.to_string(), u.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn each_port_names_the_constant_or_defaults_it_takes() {
        let c = constants(&module(), &lock(), |f| f.starts_with("pages/vislox/"));
        let names: Vec<(&str, &str, Option<usize>, usize, bool)> = c
            .lets
            .iter()
            .map(|l| (l.name.as_str(), l.value.as_str(), l.line, l.uses, l.ours))
            .collect();
        // An alias names a port: no constant.
        assert_eq!(
            names,
            vec![
                ("temp_schwelle", "28", Some(2), 1, false),
                ("nachlauf", "30min", Some(4), 1, false),
                ("vx_a", "5", Some(1), 1, true),
            ]
        );
        let d: Vec<(&str, Option<usize>, usize)> = c
            .defaults
            .iter()
            .map(|d| (d.ty.as_str(), d.line, d.uses))
            .collect();
        // Div's default reaches no managed block: `teiler` is an extern.
        assert_eq!(d, vec![("Monoflop", Some(5), 1), ("Div", Some(8), 0)]);
        assert_eq!(c.defaults[0].params["Time"], "nachlauf");
        // The extern's claim, by its UUID lowercase and the port's key.
        let u = &c.uses[&DIV.to_ascii_lowercase()]["Input2"];
        assert_eq!(
            (u.kind, u.name.as_str(), u.value.as_str()),
            ("let", "temp_schwelle", "28")
        );
        assert_eq!((u.file.as_str(), u.line), ("pages/heizung.loxl", Some(2)));
        assert_eq!(
            (u.at_file.as_str(), u.at_line),
            ("pages/heizung.loxl", Some(11))
        );
        assert!(!u.ours && !u.at_ours);
        // A block that binds the port keeps its binding; one that does not
        // takes the default, and through it the constant.
        let t = &c.uses[LICHT]["Time"];
        assert_eq!(
            (
                t.kind,
                t.name.as_str(),
                t.constant.as_deref(),
                t.value.as_str()
            ),
            ("defaults", "Monoflop", Some("nachlauf"), "30min")
        );
        assert_eq!((t.line, t.at_line), (Some(5), Some(12)));
        assert!(!c.uses.contains_key(FLUR), "{:?}", c.uses);
        // vislox's own constant, on vislox's claim.
        let a = c.uses[AND].values().next().unwrap();
        assert_eq!(c.uses[AND].len(), 1, "{:?}", c.uses[AND]);
        assert_eq!((a.name.as_str(), a.ours, a.at_ours), ("vx_a", true, true));
        assert_eq!(a.at_line, Some(4));
    }

    #[test]
    fn an_extern_s_argument_is_found_on_its_own_line() {
        let files = vec![(
            "m.loxl".to_string(),
            format!("let t = 3\nextern teiler = Div(\n\tuuid: \"{DIV}\",\n\tIn2: t,\n)\n"),
        )];
        let c = constants(&files, &lock(), |_| false);
        let u = &c.uses[&DIV.to_ascii_lowercase()]["Input2"];
        assert_eq!((u.at_line, u.line), (Some(4), Some(1)));
        assert_eq!(c.module_binding(DIV, "Input2").as_deref(), Some("m.loxl:4"));
    }

    #[test]
    fn a_binding_on_its_own_line_is_found_there() {
        let files = vec![(
            "m.loxl".to_string(),
            "let t = 3\nflur = Monoflop(\n\t\"Flur\",\n\tTime: t,\n)\n".to_string(),
        )];
        let c = constants(&files, &lock(), |_| false);
        let u = &c.uses[FLUR]["Time"];
        assert_eq!((u.at_line, u.line), (Some(4), Some(1)));
    }

    #[test]
    fn a_default_reaches_the_blocks_that_do_not_bind_its_port_instances_too() {
        let files = vec![
            (
                "m/a.loxl".to_string(),
                "defaults Monoflop(Time: 5s)\nlicht = Monoflop(\"Licht\")\nflur = Monoflop(\"Flur\", Time: 1s)\ntemplate paar(t: number)\n\tm = Monoflop(Reset: t)\nend\nost = paar(t: 1)\n".to_string(),
            ),
            (
                "m/vislox/edits.loxl".to_string(),
                "defaults Div(In2: 4)\n".to_string(),
            ),
        ];
        let c = constants(&files, &lock(), |f| f.starts_with("m/vislox/"));
        let mono = c.defaults.iter().find(|d| d.ty == "Monoflop").unwrap();
        let slugs: Vec<(&str, Option<&str>)> = mono.reach["Time"]
            .iter()
            .map(|r| (r.slug.as_str(), r.uuid.as_deref()))
            .collect();
        assert_eq!(slugs, vec![("licht", Some(LICHT)), ("ost_m", None)]);
        assert_eq!(mono.source, "defaults Monoflop(Time: 5s)");
        assert!(!mono.ours);
        let div = c.defaults.iter().find(|d| d.ty == "Div").unwrap();
        assert!(div.ours && div.reach["In2"].is_empty());
        // A port the statement does not have yet reaches the same way.
        let r = reach(&files, &lock(), "Monoflop", "Reset").unwrap();
        assert_eq!(
            r.iter().map(|r| r.slug.as_str()).collect::<Vec<_>>(),
            ["flur", "licht"]
        );
        // A module that does not read: lox's error, naming the file.
        let broken = vec![("m/b.loxl".to_string(), "let = (\n".to_string())];
        assert!(
            reach(&broken, &lock(), "Monoflop", "Time")
                .unwrap_err()
                .starts_with("m/b.loxl: ")
        );
    }
}
