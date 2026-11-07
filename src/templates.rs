//! The module's templates and their instances (#83), and the blocks each
//! instance made.
//!
//! A `template` is a block type made of blocks; each instance
//! (`sued = Fassade(sonne: sonne_sued)`) stamps out a copy whose lockfile
//! keys are `<instance>_<body name>` (`sued_hoch`, `sued_hitze_alarm` under a
//! nested instance `hitze`), which source spells `sued.hoch` and
//! `sued.hitze.alarm` (lox's `template.rs`). This reads them with lox's
//! parser: every template with its parameters and outputs, every instance
//! (nested ones too, by their path) with its arguments as written, and the
//! lockfile entries each one made, so the canvas can group them, Properties
//! can name a block `sued.hoch`, and the Changes list can say whether a
//! change came from a template's body or from one call.
//!
//! What the deployed revision had is read off the module's files at its
//! commit ([`compare`]): a template whose body (as lox prints it) differs,
//! an instance that is new or whose call differs.

use crate::edit::Lock;
use crate::managed::template_bodies;
use loxone_config::logic::{Item, Module, PortRef, TemplateDecl, TemplateParam, Value};
use loxone_config::lsp::analysis::scan;
use loxone_config::port_names;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One parameter of a template, for Properties and the form that adds an
/// instance (lox's `loxl types <Template>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Param {
    pub name: String,
    /// `object`, `input`, `value` or `set`.
    pub kind: &'static str,
    /// The block type of an object (`any` for any), the value's kind
    /// (`number`, `duration`, `string`, `bool`) or the set's kind (`moods`,
    /// …); empty for an input.
    #[serde(rename = "type")]
    pub ty: String,
    /// The default as written, when it has one.
    pub default: Option<String>,
    /// Whether a call must give it: an object, and a value or set without a
    /// default. An input not given leaves its port open.
    pub required: bool,
}

/// One output of a template: `Q: beschatten.Q`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Output {
    pub name: String,
    /// The body port it reads, as written.
    pub port: String,
}

/// One template of the module.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Template {
    pub name: String,
    /// Shown as [`crate::managed::Source::file`]; 1-based line.
    pub file: String,
    pub line: Option<usize>,
    /// In vislox's directory: a template of lox's library vislox copied in
    /// for an instance of its own.
    pub ours: bool,
    /// The library file it is (by name and text), when it is one.
    pub library: Option<&'static str>,
    pub params: Vec<Param>,
    pub outputs: Vec<Output>,
    /// The comment opening its body, as lox's library lists it.
    pub description: String,
    /// Its instances, nested ones too.
    pub instances: usize,
    /// Whether the deployed revision's files had it ([`compare`]).
    pub deployed: Option<bool>,
    /// Whether its body (as lox prints it) differs from the deployed
    /// revision's: every instance's blocks changed because of it.
    pub changed: bool,
    /// As lox prints it ([`text_of`]).
    #[serde(skip)]
    pub text: String,
}

/// What a block of an instance is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Block {
    pub uuid: String,
    /// Its lockfile key: `sued_hoch`.
    pub slug: String,
    /// How source spells it: `sued.hoch`; a block lox made for an expression
    /// of the body, the body block or signal it belongs to.
    pub name: String,
    /// Made by lox for an expression of the body.
    pub expression: bool,
}

/// One output of an instance, resolved to a block of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceOutput {
    pub name: String,
    /// The block's UUID and the connector's key, when the lockfile has it.
    pub uuid: Option<String>,
    pub key: Option<String>,
}

/// One instance of a template, a nested one too.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Instance {
    /// How source names it: `sued`, `sued.hitze` for a nested one.
    pub path: String,
    /// Its blocks' lockfile prefix: `sued`, `sued_hitze`.
    pub prefix: String,
    pub template: String,
    /// The instance it is nested in.
    pub parent: Option<String>,
    /// The label (`sued = Fassade("Süd", …)`), as written.
    pub label: Option<String>,
    /// The arguments as written (`sonne: sonne_sued.Q`), but the label.
    pub args: Vec<[String; 2]>,
    /// Its own call's file and line; a nested one's is its outermost
    /// instance's.
    pub file: String,
    pub line: Option<usize>,
    /// The line of its call inside the parent template's body (nested).
    pub body_line: Option<usize>,
    /// Declared in vislox's files: edited there (the `instance` op).
    pub ours: bool,
    /// The page its blocks are on (the lockfile's `page_uuid`).
    pub page: Option<String>,
    pub outputs: Vec<InstanceOutput>,
    /// Its own blocks (not a nested instance's), in lockfile order.
    pub blocks: Vec<Block>,
    /// Against the deployed revision's files ([`compare`]): `new` (not
    /// there), `call` (its call differs), or none. Only for a top-level one:
    /// a nested call is part of its parent's template.
    pub change: Option<&'static str>,
    /// Its call as lox prints it.
    #[serde(skip)]
    pub call: String,
}

/// A template of lox's library, which an instance of vislox's may call
/// (vislox copies it into its directory, `templates.loxl`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryTemplate {
    pub file: &'static str,
    pub name: String,
    pub params: Vec<Param>,
    pub outputs: Vec<Output>,
    pub description: String,
}

/// Everything [`read`] finds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Templates {
    pub templates: Vec<Template>,
    pub instances: Vec<Instance>,
}

/// A template's parameters as Properties lists them.
pub fn params(t: &TemplateDecl) -> Vec<Param> {
    t.params
        .iter()
        .map(|p| match p {
            TemplateParam::Object { name, block_type } => Param {
                name: name.clone(),
                kind: "object",
                ty: block_type.clone(),
                default: None,
                required: true,
            },
            TemplateParam::Input { name } => Param {
                name: name.clone(),
                kind: "input",
                ty: String::new(),
                default: None,
                required: false,
            },
            TemplateParam::Value {
                name,
                kind,
                default,
                ..
            } => Param {
                name: name.clone(),
                kind: "value",
                ty: kind.keyword().to_string(),
                default: default.as_ref().map(Value::to_string),
                required: default.is_none(),
            },
            TemplateParam::Set {
                name,
                kind,
                default,
            } => Param {
                name: name.clone(),
                kind: "set",
                ty: kind.keyword().to_string(),
                default: default.clone(),
                required: default.is_none(),
            },
        })
        .collect()
}

fn outputs(t: &TemplateDecl) -> Vec<Output> {
    t.outputs
        .iter()
        .map(|o| Output {
            name: o.name.clone(),
            port: o.port.to_string(),
        })
        .collect()
}

/// The lockfile keys of the blocks and signals an instance with blocks'
/// prefix `prefix` of template `t` makes, nested instances' too (`lookup`
/// finds a template by name).
pub fn expanded(
    t: &TemplateDecl,
    prefix: &str,
    lookup: &dyn Fn(&str) -> Option<TemplateDecl>,
) -> Vec<String> {
    fn walk(
        t: &TemplateDecl,
        prefix: &str,
        lookup: &dyn Fn(&str) -> Option<TemplateDecl>,
        depth: usize,
        out: &mut Vec<String>,
    ) {
        if depth > 8 {
            return;
        }
        let b = body(t);
        for n in &b.names {
            out.push(format!("{prefix}_{n}"));
        }
        for (n, call) in &b.nested {
            if let Some(inner) = lookup(&call.block_type) {
                walk(&inner, &format!("{prefix}_{n}"), lookup, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(t, prefix, lookup, 0, &mut out);
    out
}

/// Whether lockfile key `slug` is one of `expanded`'s blocks, or one lox made
/// for an expression of theirs (`<slug>__…`, `<slug>_<port>__…`).
pub fn owns(expanded: &[String], slug: &str) -> bool {
    let head = slug.find("__").map_or(slug, |k| &slug[..k]);
    expanded.iter().any(|e| {
        slug == e
            || (head.len() < slug.len()
                && (head == e
                    || head
                        .strip_prefix(e.as_str())
                        .is_some_and(|r| r.starts_with('_'))))
    })
}

/// The comment opening a template's body (lox's library describes a
/// template so).
fn description(t: &TemplateDecl) -> String {
    t.body
        .iter()
        .take_while(|i| matches!(i, Item::Comment(_) | Item::Gap))
        .filter_map(|i| match i {
            Item::Comment(c) => Some(c.trim()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A template as lox prints it: what tells a changed body from a moved or
/// reformatted one.
pub fn text_of(t: &TemplateDecl) -> String {
    Module {
        items: vec![Item::Template(t.clone())],
    }
    .to_text()
}

/// An instance's call as lox prints it.
fn call_text(b: &loxone_config::logic::BlockDecl) -> String {
    Module {
        items: vec![Item::Instance(b.clone())],
    }
    .to_text()
}

/// The templates of lox's library, each with its parameters.
pub fn library() -> Vec<LibraryTemplate> {
    let mut out = Vec::new();
    for (file, src) in loxone_config::library::FILES {
        let Ok(m) = Module::parse_fragment(src) else {
            continue;
        };
        for item in m.items {
            if let Item::Template(t) = item {
                out.push(LibraryTemplate {
                    file,
                    name: t.name.clone(),
                    params: params(&t),
                    outputs: outputs(&t),
                    description: description(&t),
                });
            }
        }
    }
    out
}

/// The library's template `name`, as lox's library file declares it.
pub fn library_template(name: &str) -> Option<(&'static str, TemplateDecl)> {
    for (file, src) in loxone_config::library::FILES {
        let Ok(m) = Module::parse_fragment(src) else {
            continue;
        };
        for item in m.items {
            if let Item::Template(t) = item
                && t.name == name
            {
                return Some((file, t));
            }
        }
    }
    None
}

/// The library file a template is, when lox's library has one of its name
/// with the same text.
fn library_of(t: &TemplateDecl) -> Option<&'static str> {
    let (file, lib) = library_template(&t.name)?;
    (text_of(&lib) == text_of(t)).then_some(file)
}

/// Where [`read`]'s walk is: the instance's path and prefix, its parent, its
/// call's file, line and line in the parent's body, and whether vislox's.
struct Walk {
    path: String,
    prefix: String,
    parent: Option<String>,
    at: (String, Option<usize>, Option<usize>),
    ours: bool,
    depth: usize,
}

/// One file parsed, its instances classified with every template's name.
struct Parsed<'a> {
    file: &'a str,
    items: Vec<Item>,
    /// Top-level declarations by name: line (1-based).
    lines: BTreeMap<String, usize>,
    /// Templates by name: line.
    template_lines: BTreeMap<String, usize>,
}

fn parse<'a, T: AsRef<str>>(files: &'a [(String, T)]) -> Vec<Parsed<'a>> {
    let mut parsed: Vec<Parsed> = files
        .iter()
        .filter(|(f, _)| f.ends_with(".loxl"))
        .filter_map(|(f, t)| {
            let t = t.as_ref();
            let items = Module::parse_fragment(t).ok()?.items;
            let lines: Vec<&str> = t.split('\n').collect();
            let body = template_bodies(&lines);
            let mut decl_lines = BTreeMap::new();
            let mut template_lines = BTreeMap::new();
            for d in scan(t, Path::new(f)) {
                if d.kind == "template" {
                    template_lines.entry(d.slug).or_insert(d.span.line + 1);
                } else if !body.contains(&d.span.line) {
                    decl_lines.entry(d.slug).or_insert(d.span.line + 1);
                }
            }
            Some(Parsed {
                file: f.as_str(),
                items,
                lines: decl_lines,
                template_lines,
            })
        })
        .collect();
    let names: BTreeSet<String> = parsed
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i {
            Item::Template(t) => Some(t.name.clone()),
            _ => None,
        })
        .collect();
    for p in &mut parsed {
        let mut m = Module {
            items: std::mem::take(&mut p.items),
        };
        m.classify_instances_of(&names);
        p.items = m.items;
    }
    parsed
}

/// What a template's body declares: its blocks and signals by name, and its
/// nested instances by name (→ template).
struct Body<'t> {
    names: BTreeSet<&'t str>,
    nested: BTreeMap<&'t str, &'t loxone_config::logic::BlockDecl>,
}

fn body(t: &TemplateDecl) -> Body<'_> {
    let mut b = Body {
        names: BTreeSet::new(),
        nested: BTreeMap::new(),
    };
    for item in &t.body {
        match item {
            Item::Block(x) => {
                b.names.insert(&x.slug);
            }
            Item::Signal(x) => {
                b.names.insert(&x.slug);
            }
            Item::Instance(x) => {
                b.nested.insert(&x.slug, x);
            }
            _ => {}
        }
    }
    b
}

/// The line (1-based) of the nested call `slug` in template `t`'s body, in
/// its file's `text`, counting from the template's line.
fn nested_line(text: &str, template_line: usize, slug: &str) -> Option<usize> {
    let lines: Vec<&str> = text.split('\n').collect();
    for (n, l) in lines.iter().enumerate().skip(template_line) {
        let c = crate::managed::code(l).trim();
        if c == "end" {
            return None;
        }
        if let Some(rest) = c.strip_prefix(slug)
            && rest.trim_start().starts_with('=')
        {
            return Some(n + 1);
        }
    }
    None
}

/// `{name}` of a string parameter replaced by its value; anything else as it is.
fn interpolate(text: &str, strings: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (k, v) in strings {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// The text parameters' values in an instance of `t` made by `call` (the
/// call's, else the defaults), interpolated with the parent's (`outer`).
fn texts_of(
    t: &TemplateDecl,
    call: &loxone_config::logic::BlockDecl,
    outer: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for p in &t.params {
        if let TemplateParam::Value {
            name,
            kind: loxone_config::logic::ValueKind::Str,
            default,
            ..
        } = p
        {
            let given = if name == "label" {
                call.title.clone()
            } else {
                None
            }
            .or_else(|| {
                call.bindings()
                    .find(|b| b.port == *name)
                    .and_then(|b| match &b.kind {
                        loxone_config::logic::BindingKind::Param(Value::Str(s)) => Some(s.clone()),
                        _ => None,
                    })
            })
            .or_else(|| match default {
                Some(Value::Str(s)) => Some(s.clone()),
                _ => None,
            });
            if let Some(v) = given {
                out.insert(name.clone(), interpolate(&v, outer));
            }
        }
    }
    out
}

/// The arguments of a call as written, the label apart.
fn args_of(call: &loxone_config::logic::BlockDecl) -> (Option<String>, Vec<[String; 2]>) {
    let args = call
        .bindings()
        .filter(|b| b.port != "label")
        .map(|b| [b.port.clone(), b.kind.to_string()])
        .collect();
    let label = call.title.clone().or_else(|| {
        call.bindings()
            .find(|b| b.port == "label")
            .map(|b| match &b.kind {
                loxone_config::logic::BindingKind::Param(Value::Str(s)) => s.clone(),
                k => k.to_string(),
            })
    });
    (label, args)
}

/// Every template and instance of the module's files (`files`: the path as
/// shown, the text), with the blocks each instance made in `lock`. `ours`
/// tells a file of vislox's directory. A file that does not parse is left
/// out (`lox logic check` says why, #65).
pub fn read<T: AsRef<str>>(
    files: &[(String, T)],
    lock: &Lock,
    ours: impl Fn(&str) -> bool,
) -> Templates {
    let parsed = parse(files);
    let texts: BTreeMap<&str, &str> = files
        .iter()
        .map(|(f, t)| (f.as_str(), t.as_ref()))
        .collect();
    // Every template, the first declaration of a name (lox refuses two).
    let mut decls: BTreeMap<&str, (&TemplateDecl, &str)> = BTreeMap::new();
    for p in &parsed {
        for item in &p.items {
            if let Item::Template(t) = item {
                decls.entry(t.name.as_str()).or_insert((t, p.file));
            }
        }
    }
    let mut out = Templates::default();
    // (instance, template) top-level first, then nested ones (depth first).
    fn walk<'t>(
        decls: &BTreeMap<&str, (&'t TemplateDecl, &str)>,
        texts: &BTreeMap<&str, &str>,
        call: &loxone_config::logic::BlockDecl,
        w: Walk,
        strings: &BTreeMap<String, String>,
        out: &mut Vec<(Instance, Option<&'t TemplateDecl>)>,
    ) {
        let Walk {
            path,
            prefix,
            parent,
            at,
            ours,
            depth,
        } = w;
        if depth > 8 {
            return;
        }
        let (label, args) = args_of(call);
        // A nested call's label may interpolate the parent's text parameters,
        // as lox's expansion does (`"{label} H"`).
        let label = label.map(|l| interpolate(&l, strings));
        let t = decls.get(call.block_type.as_str()).map(|(t, _)| *t);
        let mine = t.map(|t| texts_of(t, call, strings)).unwrap_or_default();
        out.push((
            Instance {
                path: path.clone(),
                prefix: prefix.clone(),
                template: call.block_type.clone(),
                parent,
                label,
                args,
                file: at.0.clone(),
                line: at.1,
                body_line: at.2,
                ours,
                page: None,
                outputs: Vec::new(),
                blocks: Vec::new(),
                change: None,
                call: call_text(call),
            },
            t,
        ));
        let Some(t) = t else { return };
        let tfile = decls
            .get(t.name.as_str())
            .map(|(_, f)| *f)
            .unwrap_or_default();
        for inner in body(t).nested.values() {
            let line = texts.get(tfile).and_then(|text| {
                let tl = scan(text, Path::new(tfile))
                    .into_iter()
                    .find(|d| d.kind == "template" && d.slug == t.name)?
                    .span
                    .line;
                nested_line(text, tl + 1, &inner.slug)
            });
            let w = Walk {
                path: format!("{path}.{}", inner.slug),
                prefix: format!("{prefix}_{}", inner.slug),
                parent: Some(path.clone()),
                at: (at.0.clone(), at.1, line),
                ours,
                depth: depth + 1,
            };
            walk(decls, texts, inner, w, &mine, out);
        }
    }
    let mut found: Vec<(Instance, Option<&TemplateDecl>)> = Vec::new();
    for p in &parsed {
        for item in &p.items {
            if let Item::Instance(call) = item {
                let w = Walk {
                    path: call.slug.clone(),
                    prefix: call.slug.clone(),
                    parent: None,
                    at: (p.file.to_string(), p.lines.get(&call.slug).copied(), None),
                    ours: ours(p.file),
                    depth: 0,
                };
                walk(&decls, &texts, call, w, &BTreeMap::new(), &mut found);
            }
        }
    }
    // Each lockfile entry belongs to the instance with the longest prefix it
    // starts with whose template's body names it: `<prefix>_<name>` (a block
    // or signal), or a block lox made for an expression there
    // (`<prefix>_<name>__…` of a signal, `<prefix>_<name>_<port>__…` of an
    // input).
    let mut order: Vec<usize> = (0..found.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(found[i].0.prefix.len()));
    let bodies: Vec<Option<Body>> = found.iter().map(|(_, t)| t.map(body)).collect();
    for (slug, uuid) in &lock.slugs {
        for &i in &order {
            let (inst, _) = &found[i];
            let Some(rest) = slug
                .strip_prefix(inst.prefix.as_str())
                .and_then(|r| r.strip_prefix('_'))
            else {
                continue;
            };
            let Some(b) = &bodies[i] else { continue };
            let hit = if b.names.contains(rest) {
                Some((rest, false))
            } else {
                rest.find("__").and_then(|k| {
                    let head = &rest[..k];
                    b.names
                        .iter()
                        .find(|n| {
                            head == **n
                                || head.strip_prefix(**n).is_some_and(|r| r.starts_with('_'))
                        })
                        .map(|n| (*n, true))
                })
            };
            let Some((name, expression)) = hit else {
                continue;
            };
            let name = format!("{}.{name}", inst.path);
            found[i].0.blocks.push(Block {
                uuid: uuid.clone(),
                slug: slug.clone(),
                name,
                expression,
            });
            break;
        }
    }
    // The page: where the lock places its blocks (lox puts an instance's
    // blocks on its call's page); a nested one's, its own or its parent's.
    for (inst, _) in found.iter_mut() {
        inst.page = inst
            .blocks
            .iter()
            .find_map(|b| lock.placed.get(&b.slug).cloned().flatten());
    }
    for i in 0..found.len() {
        if found[i].0.page.is_none()
            && let Some(p) = found[i].0.parent.clone()
            && let Some(pp) = found
                .iter()
                .find(|(x, _)| x.path == p)
                .and_then(|(x, _)| x.page.clone())
        {
            found[i].0.page = Some(pp);
        }
    }
    // The outputs, each to the block and connector it reads.
    let by_path: BTreeMap<String, usize> = found
        .iter()
        .enumerate()
        .map(|(i, (x, _))| (x.path.clone(), i))
        .collect();
    let outs: Vec<Vec<InstanceOutput>> = found
        .iter()
        .map(|(inst, t)| {
            let Some(t) = t else { return Vec::new() };
            t.outputs
                .iter()
                .map(|o| {
                    let (uuid, key) =
                        match resolve(&found, &by_path, &decls, &inst.path, &o.port, lock, 0) {
                            Some((u, k)) => (Some(u), Some(k)),
                            None => (None, None),
                        };
                    InstanceOutput {
                        name: o.name.clone(),
                        uuid,
                        key,
                    }
                })
                .collect()
        })
        .collect();
    for ((inst, _), o) in found.iter_mut().zip(outs) {
        inst.outputs = o;
    }
    for (name, (t, file)) in &decls {
        let count = found.iter().filter(|(x, _)| x.template == *name).count();
        let line = parsed
            .iter()
            .find(|p| p.file == *file)
            .and_then(|p| p.template_lines.get(*name).copied());
        out.templates.push(Template {
            name: name.to_string(),
            file: file.to_string(),
            line,
            ours: ours(file),
            library: library_of(t),
            params: params(t),
            outputs: outputs(t),
            description: description(t),
            instances: count,
            deployed: None,
            changed: false,
            text: text_of(t),
        });
    }
    out.instances = found.into_iter().map(|(x, _)| x).collect();
    out
}

/// The block and connector key that `port` (written in the template of
/// instance `path`) reads: a body block's port, or a nested instance's output.
fn resolve(
    found: &[(Instance, Option<&TemplateDecl>)],
    by_path: &BTreeMap<String, usize>,
    decls: &BTreeMap<&str, (&TemplateDecl, &str)>,
    path: &str,
    port: &PortRef,
    lock: &Lock,
    depth: usize,
) -> Option<(String, String)> {
    if depth > 8 {
        return None;
    }
    let (inst, t) = &found[*by_path.get(path)?];
    let t = (*t)?;
    let b = body(t);
    if b.names.contains(port.slug.as_str()) {
        let slug = format!("{}_{}", inst.prefix, port.slug);
        let uuid = lock.uuid_of(&slug)?.to_string();
        let ty = lock
            .entry(&slug)
            .and_then(|e| e.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        let key = port_names::key_or_raw(ty, &port.port).to_string();
        return Some((uuid, key));
    }
    let inner = b.nested.get(port.slug.as_str())?;
    let (it, _) = decls.get(inner.block_type.as_str())?;
    let o = it.outputs.iter().find(|o| o.name == port.port)?;
    resolve(
        found,
        by_path,
        decls,
        &format!("{path}.{}", inner.slug),
        &o.port,
        lock,
        depth + 1,
    )
}

/// Mark what differs from the deployed revision's files (`deployed`: the
/// module's files at its commit; `None`: no commit to compare with).
pub fn compare<T: AsRef<str>>(t: &mut Templates, deployed: Option<&[(String, T)]>) {
    let Some(files) = deployed else { return };
    let parsed = parse(files);
    let mut was_templates: BTreeMap<String, String> = BTreeMap::new();
    let mut was_calls: BTreeMap<String, String> = BTreeMap::new();
    for p in &parsed {
        for item in &p.items {
            match item {
                Item::Template(x) => {
                    was_templates
                        .entry(x.name.clone())
                        .or_insert_with(|| text_of(x));
                }
                Item::Instance(x) => {
                    was_calls
                        .entry(x.slug.clone())
                        .or_insert_with(|| call_text(x));
                }
                _ => {}
            }
        }
    }
    for x in &mut t.templates {
        let was = was_templates.get(&x.name);
        x.deployed = Some(was.is_some());
        x.changed = was != Some(&x.text);
    }
    for x in t.instances.iter_mut().filter(|x| x.parent.is_none()) {
        x.change = match was_calls.get(&x.path) {
            None => Some("new"),
            Some(w) if *w != x.call => Some("call"),
            Some(_) => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPL: &str = "\
template Waechter(label: string = \"W\", quelle: input, grenze: number = 28) -> (Q: alarm.Q)
\talarm = GreaterEqual(\"{label} Alarm\", In1: quelle, In2: grenze)
\tinv = Not(In: alarm.Q)
end

template Fassade(label: string, sonne: input, pos: number = 70) -> (Q: b.Q, Hitze: hitze.Q)
\t# Beschattung einer Fassade.
\tb = And(\"{label} B\", In1: sonne)
\twarm = sonne.AQ >= 28
\thitze = Waechter(\"{label} H\", quelle: sonne)
end
";

    const EG: &str = "\
page \"Erdgeschoss\"
extern teiler = Div(uuid: \"10000006-0000-0060-ffff504f94000001\")
sued = Fassade(\"Süd\", sonne: teiler.AQ)
west = Fassade(\"West\", sonne: teiler.AQ, pos: 55)
";

    fn lock(objects: &[(&str, &str, &str)]) -> Lock {
        let mut o = serde_json::Map::new();
        for (s, u, ty) in objects {
            o.insert(
                s.to_string(),
                serde_json::json!({ "uuid": u, "type": ty, "page_uuid": "p-eg",
                    "layout": { "px": 0, "py": 0, "px2": 1, "py2": 1 } }),
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("logic.lock.json");
        std::fs::write(&p, serde_json::json!({ "objects": o }).to_string()).unwrap();
        Lock::read(&p)
    }

    fn files() -> Vec<(String, String)> {
        vec![
            ("pages/eg.loxl".into(), EG.into()),
            ("pages/tpl.loxl".into(), TPL.into()),
        ]
    }

    fn the_lock() -> Lock {
        lock(&[
            ("sued_b", "u-sb", "And"),
            ("sued_warm", "u-sw", "GreaterEqual"),
            ("sued_hitze_alarm", "u-sha", "GreaterEqual"),
            ("sued_hitze_inv", "u-shi", "Not"),
            ("west_b", "u-wb", "And"),
            ("west_warm", "u-ww", "GreaterEqual"),
            ("west_hitze_alarm", "u-wha", "GreaterEqual"),
            ("west_hitze_inv", "u-whi", "Not"),
            // Not an instance's: a block whose slug only starts like one.
            ("sued_bx", "u-x", "And"),
        ])
    }

    #[test]
    fn instances_nested_ones_too_with_their_blocks() {
        let t = read(&files(), &the_lock(), |f| f.starts_with("pages/vislox/"));
        let paths: Vec<&str> = t.instances.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, ["sued", "sued.hitze", "west", "west.hitze"]);
        let sued = &t.instances[0];
        assert_eq!(sued.template, "Fassade");
        assert_eq!(sued.label.as_deref(), Some("Süd"));
        assert_eq!(sued.args, [["sonne".to_string(), "teiler.AQ".to_string()]]);
        assert_eq!((sued.file.as_str(), sued.line), ("pages/eg.loxl", Some(3)));
        assert_eq!(sued.page.as_deref(), Some("p-eg"));
        let names: Vec<(&str, &str)> = sued
            .blocks
            .iter()
            .map(|b| (b.uuid.as_str(), b.name.as_str()))
            .collect();
        assert_eq!(names, [("u-sb", "sued.b"), ("u-sw", "sued.warm")]);
        let inner = &t.instances[1];
        assert_eq!(inner.parent.as_deref(), Some("sued"));
        assert_eq!(inner.prefix, "sued_hitze");
        assert_eq!((inner.line, inner.body_line), (Some(3), Some(10)));
        // Its label as lox expands it: the parent's label interpolated.
        assert_eq!(inner.label.as_deref(), Some("Süd H"));
        assert_eq!(inner.blocks.len(), 2);
        assert_eq!(inner.blocks[0].name, "sued.hitze.alarm");
        // The outputs, a nested instance's through it.
        let o: Vec<(&str, Option<&str>, Option<&str>)> = sued
            .outputs
            .iter()
            .map(|o| (o.name.as_str(), o.uuid.as_deref(), o.key.as_deref()))
            .collect();
        assert_eq!(
            o,
            [
                ("Q", Some("u-sb"), Some("Q")),
                ("Hitze", Some("u-sha"), Some("Q"))
            ]
        );
        let f = t.templates.iter().find(|x| x.name == "Fassade").unwrap();
        assert_eq!((f.file.as_str(), f.line), ("pages/tpl.loxl", Some(6)));
        assert_eq!(f.instances, 2);
        assert_eq!(f.description, "Beschattung einer Fassade.");
        assert_eq!(f.params[0].kind, "value");
        assert!(f.params[0].required);
        assert_eq!(f.params[1].kind, "input");
        assert_eq!(f.params[2].default.as_deref(), Some("70"));
        assert!(!f.ours && f.library.is_none());
        assert_eq!(
            t.templates
                .iter()
                .find(|x| x.name == "Waechter")
                .unwrap()
                .instances,
            2
        );
    }

    #[test]
    fn a_body_expression_belongs_to_its_block() {
        let l = lock(&[
            ("sued_b", "u-sb", "And"),
            ("sued_b_i1__ge1", "u-e", "GreaterEqual"),
            ("sued_warm__and1", "u-e2", "And"),
        ]);
        let t = read(&files(), &l, |_| false);
        let b: Vec<(&str, bool)> = t.instances[0]
            .blocks
            .iter()
            .map(|b| (b.name.as_str(), b.expression))
            .collect();
        assert_eq!(
            b,
            [("sued.b", false), ("sued.b", true), ("sued.warm", true)]
        );
    }

    #[test]
    fn changes_against_the_deployed_files() {
        let mut t = read(&files(), &the_lock(), |_| false);
        // The same files reformatted: nothing changed.
        let same = vec![
            (
                "pages/eg.loxl".to_string(),
                EG.replace("sonne: teiler.AQ)", "sonne:  teiler.AQ)"),
            ),
            ("pages/tpl.loxl".to_string(), TPL.to_string()),
        ];
        compare(&mut t, Some(&same));
        assert!(
            t.templates
                .iter()
                .all(|x| !x.changed && x.deployed == Some(true))
        );
        assert!(t.instances.iter().all(|x| x.change.is_none()));
        // The body changed, one call too, and one instance is new.
        let was = vec![
            (
                "pages/eg.loxl".to_string(),
                EG.replace("pos: 55", "pos: 50")
                    .replace("west = ", "# west = "),
            ),
            (
                "pages/tpl.loxl".to_string(),
                TPL.replace("grenze: number = 28", "grenze: number = 30"),
            ),
        ];
        compare(&mut t, Some(&was));
        let w = t.templates.iter().find(|x| x.name == "Waechter").unwrap();
        assert!(w.changed);
        assert!(
            !t.templates
                .iter()
                .find(|x| x.name == "Fassade")
                .unwrap()
                .changed
        );
        assert_eq!(t.instances[0].change, None);
        assert_eq!(t.instances[2].change, Some("new"));
        // Nested ones are their parent's template's.
        assert_eq!(t.instances[1].change, None);
        let was = vec![
            (
                "pages/eg.loxl".to_string(),
                EG.replace("pos: 55", "pos: 50"),
            ),
            ("pages/tpl.loxl".to_string(), TPL.to_string()),
        ];
        compare(&mut t, Some(&was));
        assert_eq!(t.instances[2].change, Some("call"));
    }

    #[test]
    fn the_library_is_listed_with_its_parameters() {
        let lib = library();
        let b = lib.iter().find(|t| t.name == "Treppenlicht").unwrap();
        assert_eq!(b.file, "licht");
        assert!(!b.description.is_empty());
        assert_eq!(b.params[0].kind, "object");
        assert_eq!(b.params[0].ty, "any");
        assert!(library_template("Treppenlicht").is_some());
        assert!(library_template("Nope").is_none());
        // A copy of it in the module is the library's.
        let (_, t) = library_template("Treppenlicht").unwrap();
        let files = vec![("pages/vislox/templates.loxl".to_string(), text_of(&t))];
        let r = read(&files, &Lock::default(), |f| f.starts_with("pages/vislox/"));
        assert_eq!(r.templates[0].library, Some("licht"));
        assert!(r.templates[0].ours);
    }
}
