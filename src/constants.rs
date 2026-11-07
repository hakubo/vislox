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
use loxone_config::logic::{
    ArgItem, BindingKind, BlockDecl, DefaultsDecl, Item, MatchSpec, Module, Value, defaults_taken,
};
use loxone_config::port_names;
use serde::Serialize;
use std::collections::BTreeMap;

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Constants {
    pub lets: Vec<Let>,
    pub defaults: Vec<Defaults>,
    /// Object UUID (lowercase) → connector key (or setting name) → its source.
    pub uses: BTreeMap<String, BTreeMap<String, Use>>,
}

/// One file parsed, with its lines.
struct Parsed<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    items: Vec<Item>,
}

/// The line (1-based) of `slug.port = …` at the top level of `p`.
fn set_line(p: &Parsed, slug: &str, port: &str) -> Option<usize> {
    let body = template_bodies(&p.lines);
    p.lines.iter().enumerate().find_map(|(n, l)| {
        if body.contains(&n) {
            return None;
        }
        let (lhs, rhs) = code(l).split_once('=')?;
        let (s, q) = lhs.trim().split_once('.')?;
        (s.trim() == slug && q.trim() == port && !rhs.starts_with('=')).then_some(n + 1)
    })
}

/// The line (1-based) of `defaults <ty>` at the top level of `p`.
fn defaults_line(p: &Parsed, ty: &str) -> Option<usize> {
    let body = template_bodies(&p.lines);
    p.lines.iter().enumerate().find_map(|(n, l)| {
        let c = code(l).trim_start();
        let rest = c.strip_prefix("defaults")?;
        if !rest.starts_with(char::is_whitespace) || body.contains(&n) {
            return None;
        }
        let name = rest
            .trim_start()
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()?;
        (name == ty).then_some(n + 1)
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
            Some(Parsed {
                file: f.as_str(),
                lines: t.split('\n').collect(),
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
                    out.lets.push(Let {
                        name: l.name.clone(),
                        value: l.value.to_string(),
                        file: p.file.to_string(),
                        line: decls.at(&l.name, Some(p.file)).map(|(_, l)| l),
                        uses: 0,
                        ours: ours(p.file),
                    });
                }
                Item::Defaults(d) => {
                    if defaults.contains_key(&d.block_type) {
                        continue;
                    }
                    defaults.insert(d.block_type.clone(), (out.defaults.len(), d));
                    out.defaults.push(Defaults {
                        ty: d.block_type.clone(),
                        file: p.file.to_string(),
                        line: defaults_line(p, &d.block_type),
                        params: d
                            .params()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                        uses: 0,
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
                            },
                        ));
                    }
                }
                Item::Extern(e) => {
                    let uuid = externs.get(&e.slug).and_then(|x| x.1.clone());
                    let line = decls.at(&e.slug, Some(p.file)).map(|(_, l)| l);
                    for a in &e.args {
                        if let ArgItem::Binding(x) = a
                            && let BindingKind::Param(Value::Ref(n)) = &x.kind
                            && let Some(u) = let_use(n, p.file, line)
                        {
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
                    if let Some(u) = let_use(n, p.file, line) {
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
    fn a_binding_on_its_own_line_is_found_there() {
        let files = vec![(
            "m.loxl".to_string(),
            "let t = 3\nflur = Monoflop(\n\t\"Flur\",\n\tTime: t,\n)\n".to_string(),
        )];
        let c = constants(&files, &lock(), |_| false);
        let u = &c.uses[FLUR]["Time"];
        assert_eq!((u.at_line, u.line), (Some(4), Some(1)));
    }
}
