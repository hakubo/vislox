//! Editing template instances (#83) in vislox's files.
//!
//! An instance of vislox's is a call in its page file, under the file's
//! *Template instances* comment: `vx_ost = Fassade("Ost", sonne: vx_….AQ)`.
//! lox expands it into the template's blocks (`vx_ost_b`, …) on the call's
//! page. The `instance` op makes, changes or drops one; an instance of the
//! module's own files is changed there (vislox writes only its own files,
//! as for `let` constants, #81, and expressions, #82).
//!
//! The template is one of the module's, or of lox's library: vislox copies a
//! library template into its directory (`templates.loxl`) for the first
//! instance that calls it and drops it with the last, so the module keeps
//! compiling. The arguments are checked against the template's parameters
//! the way `lox logic check` checks a call (an unknown or missing parameter,
//! a value of the wrong kind, a label the template takes none of), and name
//! only what vislox's files may name (an object of Loxone Config's as
//! `vx_<uuid>`, a block, signal or instance of vislox's, a constant); lox
//! checks the rest when the module is checked and compiled.

use crate::blocks::{self, InstanceDef, PageFile};
use crate::edit::{Applied, Ctx, Files, Op, find, op_value, own_constant, pages_titled, slug_uuid};
use crate::templates::{expanded, library_template, owns, text_of};
use loxone_config::logic::{
    self, BindingKind, Item, Module, Operand, TemplateDecl, TemplateParam, Value, ValueKind,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// An instance's call: the template, the label and each argument as typed
/// (a value as Lox Logic writes it, a name, a port or an expression; a text
/// parameter's value unquoted).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct Call {
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

/// The header of [`crate::edit::TEMPLATES`].
const HEADER: &str = "# Templates of lox's library that vislox's instances call. vislox rewrites\n\
                      # this file on every edit; see docs/editing.md in vislox.\n";

/// Read vislox's file of library templates: templates and comments only.
pub fn parse_templates(text: &str) -> Result<BTreeMap<String, TemplateDecl>, String> {
    let m = Module::parse_fragment(text).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for item in m.items {
        match item {
            Item::Template(t) => {
                if out.insert(t.name.clone(), t).is_some() {
                    return Err("it declares a template twice".into());
                }
            }
            Item::Comment(_) | Item::Gap | Item::FragmentEnd => {}
            _ => {
                return Err(
                    "it holds a statement vislox did not write; move it to another file of the \
                     module"
                        .into(),
                );
            }
        }
    }
    Ok(out)
}

pub fn render_templates(t: &BTreeMap<String, TemplateDecl>) -> String {
    let mut out = String::from(HEADER);
    for x in t.values() {
        out.push('\n');
        out.push_str(&text_of(x));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Where a template comes from.
enum From {
    /// vislox's copy of a library template.
    Ours,
    /// The module's own files.
    Module,
    /// lox's library: copied in for this instance.
    Library,
}

/// The template `name` an instance of vislox's may call.
fn template(files: &Files, ctx: &Ctx, name: &str) -> Option<(TemplateDecl, From)> {
    if let Some(t) = ctx.templates.get(name) {
        return Some((t.clone(), From::Module));
    }
    if let Some(t) = files.templates.get(name) {
        return Some((t.clone(), From::Ours));
    }
    library_template(name).map(|(_, t)| (t, From::Library))
}

/// The template `name` to expand an instance with (any source).
fn lookup<'a>(files: &'a Files, ctx: &'a Ctx<'a>) -> impl Fn(&str) -> Option<TemplateDecl> + 'a {
    move |n: &str| template(files, ctx, n).map(|(t, _)| t)
}

/// What an instance of vislox's calls, as the `instance` op writes it.
pub fn call_of(x: &InstanceDef) -> Call {
    Call {
        template: x.template.clone(),
        label: x.label.clone(),
        args: x
            .args
            .iter()
            .map(|(k, v)| {
                // A quoted text is typed unquoted.
                let shown = match Module::parse_fragment(&format!("vx_x = T(p: {v})"))
                    .ok()
                    .and_then(|m| m.items.into_iter().next())
                {
                    Some(Item::Block(b)) => match b.bindings().next().map(|b| &b.kind) {
                        Some(BindingKind::Param(Value::Str(s))) => s.clone(),
                        _ => v.clone(),
                    },
                    _ => v.clone(),
                };
                (k.clone(), shown)
            })
            .collect(),
    }
}

/// The page UUID of vislox's page file `file`.
fn page_of(files: &Files, ctx: &Ctx, file: &str) -> String {
    let Some(pf) = files.pages.get(file) else {
        return String::new();
    };
    match &pf.uuid {
        Some(u) => u.clone(),
        None => {
            let mut same = Vec::new();
            pages_titled(&ctx.base.xml.root, &pf.page, &mut same);
            same.first()
                .and_then(|e| e.attr("U"))
                .unwrap_or_default()
                .to_string()
        }
    }
}

/// The lockfile keys an instance `slug` of template `t` owns now.
fn owned(files: &Files, ctx: &Ctx, slug: &str, t: &TemplateDecl) -> Vec<String> {
    let exp = expanded(t, slug, &lookup(files, ctx));
    ctx.lock
        .slugs
        .keys()
        .filter(|k| owns(&exp, k))
        .cloned()
        .collect()
}

/// The argument of `param` as written into the call, checked against the
/// parameter's kind, with the names of vislox's it reads.
fn argument(
    files: &Files,
    ctx: &Ctx,
    slug: &str,
    t: &TemplateDecl,
    param: &TemplateParam,
    text: &str,
) -> Result<String, String> {
    let name = param.name();
    let consts = &files.claims.8;
    let is_const = |n: &str| consts.contains_key(n) || ctx.lets.contains(n);
    // A text parameter's value is typed unquoted; a constant's name stays a
    // name.
    let text = match param {
        TemplateParam::Value {
            kind: ValueKind::Str,
            ..
        } if !is_const(text) => Value::Str(text.to_string()).to_string(),
        _ => text.to_string(),
    };
    if text.contains(['\n', '\r']) {
        return Err(format!("{name}: an argument is one line"));
    }
    let m = Module::parse_fragment(&format!("vx_x = {}({name}: {text})", t.name)).map_err(|e| {
        format!(
            "{name}: {}",
            e.to_string().replace(&format!("vx_x = {}(", t.name), "")
        )
    })?;
    let b = match m.items.into_iter().find(|i| matches!(i, Item::Block(_))) {
        Some(Item::Block(b)) => b,
        _ => return Err(format!("{name}: `{text}` is not one argument")),
    };
    let kind = match b.bindings().next() {
        Some(x) if b.bindings().count() == 1 => x.kind.clone(),
        _ => return Err(format!("{name}: `{text}` is not one argument")),
    };
    match (param, &kind) {
        (TemplateParam::Value { kind: want, .. }, BindingKind::Param(v)) => match v {
            Value::Ref(n) if is_const(n) => {}
            Value::Ref(n) => {
                return Err(format!(
                    "{name}: `{n}` is no constant vislox's files or the module declare; give {}",
                    want.example()
                ));
            }
            v => {
                if ValueKind::of(v) != Some(*want) {
                    return Err(format!(
                        "{name} is a {} parameter: give {}, not `{v}`",
                        want.keyword(),
                        want.example()
                    ));
                }
            }
        },
        (TemplateParam::Value { kind: want, .. }, _) => {
            return Err(format!(
                "{name} is a {} parameter: give {}",
                want.keyword(),
                want.example()
            ));
        }
        (TemplateParam::Object { .. }, BindingKind::Param(Value::Ref(n))) => {
            object(files, ctx, slug, name, n)?;
        }
        // A block of an instance of vislox's: `vx_ost.b`.
        (TemplateParam::Object { .. }, BindingKind::Wire(p))
            if files.instance(&p.slug).is_some() && p.slug != slug => {}
        (TemplateParam::Object { .. }, _) => {
            return Err(format!(
                "{name} takes an object by its name: one Loxone Config drew as vx_ and its UUID \
                 (`-` as `_`), or a block, signal or instance's block of vislox's"
            ));
        }
        (TemplateParam::Set { kind: want, .. }, BindingKind::Param(Value::Ref(n))) => {
            let ok = crate::set_claim::KINDS.iter().any(|k| {
                k.keyword() == want.keyword() && k.set_names(&files.claims).contains(&n.as_str())
            });
            if !ok {
                return Err(format!(
                    "{name} takes a {} set of vislox's file by its name; `{n}` is none",
                    want.keyword()
                ));
            }
        }
        (TemplateParam::Set { kind: want, .. }, _) => {
            return Err(format!(
                "{name} takes a {} set of vislox's file by its name",
                want.keyword()
            ));
        }
        (TemplateParam::Input { .. }, k) => {
            let mut names: Vec<String> = Vec::new();
            match k {
                BindingKind::Param(Value::Ref(n)) if !is_const(n) => names.push(n.clone()),
                BindingKind::Param(_) => {}
                BindingKind::Wire(p) => names.push(p.slug.clone()),
                BindingKind::Expr(e) => {
                    for o in e.operands() {
                        match o {
                            Operand::Port(p) => names.push(p.slug.clone()),
                            Operand::Value(Value::Ref(n)) if !is_const(n) => names.push(n.clone()),
                            _ => {}
                        }
                    }
                }
            }
            for n in names {
                object(files, ctx, slug, name, &n)?;
            }
        }
    }
    Ok(kind.to_string())
}

/// Check one name an argument gives: vislox's block, signal or instance, or
/// an object of Loxone Config's as `vx_<uuid>`.
fn object(files: &Files, ctx: &Ctx, slug: &str, param: &str, n: &str) -> Result<(), String> {
    if n == slug {
        return Err(format!("{param}: an instance does not read itself"));
    }
    if files.instance(n).is_some() {
        return Ok(());
    }
    crate::expr_edit::operands(files, ctx, &[n.to_string()])
        .map(|_| ())
        .map_err(|e| format!("{param}: {e}"))
}

/// The `instance` op: make, change or drop a template instance of vislox's
/// files. Its `previous` is the `instance` op that puts back what was.
pub fn set_instance(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::Instance { slug, page, value } = op else {
        unreachable!()
    };
    let s = slug.trim().to_string();
    let existing = files.instance(&s).map(|(f, x)| (f.to_string(), x.clone()));
    let previous = Op::Instance {
        slug: s.clone(),
        page: existing
            .as_ref()
            .map(|(f, _)| page_of(files, ctx, f))
            .unwrap_or_else(|| page.clone()),
        value: existing.as_ref().map(|(_, x)| call_of(x)),
    };
    let done = || Applied {
        previous: op_value(&previous),
        added: None,
        warnings: Vec::new(),
    };
    let Some(call) = value else {
        let Some((file, x)) = existing else {
            return Err(format!("vislox's files have no instance `{s}`"));
        };
        if let Some(w) = files.claims.1.iter().find(|w| w.from.0 == s) {
            return Err(format!(
                "{s}: the wire into {}.{} reads it; take that out first",
                w.to.0, w.to.1
            ));
        }
        if let Some(what) = crate::expr_edit::read_by_expression(files, &s) {
            return Err(format!("{s}: {what} reads it; take that out first"));
        }
        let mine = match template(files, ctx, &x.template) {
            Some((t, _)) => owned(files, ctx, &s, &t),
            // The template is gone: what the lock has under the instance.
            None => ctx
                .lock
                .slugs
                .keys()
                .filter(|k| {
                    k.strip_prefix(s.as_str())
                        .is_some_and(|r| r.starts_with('_'))
                })
                .cloned()
                .collect(),
        };
        if let Some(d) = ctx.deployed
            && mine.iter().any(|k| {
                ctx.lock
                    .uuid_of(k)
                    .is_some_and(|u| crate::tiles::find_ci(&d.xml.root, u).is_some())
            })
        {
            return Err(format!(
                "{s} is deployed: deleting a deployed instance is not offered yet; change its \
                 arguments instead"
            ));
        }
        let p = files.pages.get_mut(&file).expect("found");
        p.instances.retain(|i| i.slug != s);
        // lox refuses a managed block the source lost without it; marked as a
        // dropped instance's, which only making it again takes back.
        for k in mine {
            p.removed.insert(k.clone());
            p.unstamped.insert(k);
        }
        files.repin();
        prune(files, ctx);
        return Ok(done());
    };
    let tname = call.template.trim();
    if let Some((_, x)) = &existing
        && x.template != tname
    {
        return Err(format!(
            "{s} is an instance of {}: lox keeps its blocks' types. Make a new instance of \
             {tname}, then drop {s}",
            x.template
        ));
    }
    let (t, from) = template(files, ctx, tname).ok_or_else(|| {
        format!(
            "`{tname}` is no template of the module or of lox's library (`lox logic templates`)"
        )
    })?;
    if existing.is_none() {
        logic::valid_slug(&s).map_err(|e| e.to_string())?;
        if !own_constant(&s) || slug_uuid(&s).is_some() {
            return Err(format!(
                "an instance of vislox's is named {} and then lowercase letters, digits and _",
                blocks::SLUG_PREFIX
            ));
        }
        if s.contains("__") {
            return Err("`__` is lox's for the blocks an expression makes".into());
        }
    }
    // The label binds the `label` parameter.
    let label_param = t.params.iter().find(|p| p.name() == "label");
    let label = call
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty());
    match (label, label_param) {
        (Some(_), None) => {
            return Err(format!(
                "{tname} takes no label: it has no `label: string` parameter"
            ));
        }
        (Some(l), Some(_)) if l.contains(['\n', '\r']) => {
            return Err("a label is one line".into());
        }
        (None, Some(TemplateParam::Value { default: None, .. })) => {
            return Err(format!("{tname} needs a label"));
        }
        _ => {}
    }
    for k in call.args.keys() {
        if k == "label" {
            return Err("the label is given as the label, not as an argument".into());
        }
        if !t.params.iter().any(|p| p.name() == k) {
            let names: Vec<&str> = t
                .params
                .iter()
                .map(TemplateParam::name)
                .filter(|n| *n != "label")
                .collect();
            return Err(format!(
                "{tname} has no parameter `{k}`; it has {}",
                if names.is_empty() {
                    "none".to_string()
                } else {
                    names.join(", ")
                }
            ));
        }
    }
    let mut args = Vec::new();
    let mut missing = Vec::new();
    for p in t.params.iter().filter(|p| p.name() != "label") {
        let given = call
            .args
            .get(p.name())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        match given {
            Some(v) => args.push((p.name().to_string(), argument(files, ctx, &s, &t, p, v)?)),
            None => {
                let required = match p {
                    TemplateParam::Object { .. } => true,
                    TemplateParam::Value { default, .. } => default.is_none(),
                    TemplateParam::Set { default, .. } => default.is_none(),
                    TemplateParam::Input { .. } => false,
                };
                if required {
                    missing.push(p.name());
                }
            }
        }
    }
    if !missing.is_empty() {
        return Err(format!("{tname} needs {}", missing.join(", ")));
    }
    // Read back as lox parses the whole call: what is written is what it reads.
    let consts: BTreeSet<String> = files.claims.8.keys().cloned().collect();
    let def = {
        let mut x = InstanceDef {
            slug: s.clone(),
            template: tname.to_string(),
            label: label.map(String::from),
            args,
            names: Vec::new(),
        };
        let m = Module::parse_fragment(&x.render()).map_err(|e| e.to_string())?;
        match m.items.into_iter().find(|i| matches!(i, Item::Block(_))) {
            Some(Item::Block(b)) => x = InstanceDef::of(&b, &consts),
            _ => return Err(format!("`{}` is not one call", x.render())),
        }
        x
    };
    // Externs for the objects of Loxone Config's it names.
    let externs = crate::expr_edit::operands(
        files,
        ctx,
        &def.names
            .iter()
            .filter(|n| files.instance(n).is_none())
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    match existing {
        Some((file, _)) => {
            let p = files.pages.get_mut(&file).expect("found");
            if let Some(x) = p.instances.iter_mut().find(|x| x.slug == s) {
                *x = def;
            }
        }
        None => {
            // An instance dropped here and not compiled away yet: its blocks'
            // `removed` lines go, and it comes back.
            let exp = expanded(&t, &s, &lookup(files, ctx));
            let again: BTreeSet<String> = files
                .pages
                .values()
                .flat_map(|p| p.unstamped.iter())
                .filter(|k| owns(&exp, k))
                .cloned()
                .collect();
            let prefixed = |k: &str| {
                k.strip_prefix(s.as_str())
                    .is_some_and(|r| r.starts_with('_'))
                    && !again.contains(k)
            };
            if ctx.names.contains(&s)
                || ctx.lock.slugs.contains_key(&s)
                || ctx.lock.tombstoned(&s)
                || files.names(&s)
            {
                return Err(format!(
                    "`{s}` is taken: the module or its lockfile has that name"
                ));
            }
            // Its blocks are `<slug>_<name>`: no other name may start so.
            if let Some(k) = ctx
                .lock
                .slugs
                .keys()
                .chain(ctx.names.iter())
                .find(|k| prefixed(k))
                .cloned()
                .or_else(|| {
                    files
                        .pages
                        .values()
                        .flat_map(|p| {
                            p.blocks
                                .iter()
                                .map(|b| b.slug.clone())
                                .chain(p.signals.iter().map(|x| x.slug.clone()))
                                .chain(p.instances.iter().map(|x| x.slug.clone()))
                                .chain(p.removed.iter().cloned())
                        })
                        .find(|k| prefixed(k))
                })
            {
                return Err(format!(
                    "`{s}` would name its blocks `{s}_…`, and `{k}` already starts so; pick \
                     another name"
                ));
            }
            let pg = find(&ctx.base.xml.root, page)
                .filter(|e| e.attr("Type") == Some("Page"))
                .ok_or_else(|| format!("no page {page} in the project's base config"))?;
            let page_title = pg.attr_decoded("Title").unwrap_or_default().into_owned();
            if page_title.trim().is_empty() {
                return Err("the page has no title; name it in Loxone Config first".into());
            }
            let mut same = Vec::new();
            pages_titled(&ctx.base.xml.root, &page_title, &mut same);
            if same.first().and_then(|e| e.attr("U")) != Some(page.as_str()) {
                return Err(format!(
                    "{} pages are titled \"{page_title}\", and lox puts new blocks on the first; \
                     rename one in Loxone Config to add an instance here",
                    same.len()
                ));
            }
            for p in files.pages.values_mut() {
                for k in &again {
                    p.removed.remove(k);
                    p.unstamped.remove(k);
                }
            }
            let file = match files
                .pages
                .iter()
                .find(|(_, p)| p.uuid.as_deref() == Some(page.as_str()))
            {
                Some((n, _)) => n.clone(),
                None => blocks::file_name(&page_title, &|n| {
                    n == crate::edit::CLAIMS
                        || n == crate::edit::TEMPLATES
                        || files.pages.contains_key(n)
                }),
            };
            let f = files.pages.entry(file).or_insert_with(|| PageFile {
                uuid: Some(page.clone()),
                ..PageFile::new(&page_title)
            });
            f.page = page_title;
            f.instances.push(def);
        }
    }
    if matches!(from, From::Library) {
        files.templates.insert(t.name.clone(), t);
    }
    crate::expr_edit::add_externs(files, externs);
    files.repin();
    prune(files, ctx);
    Ok(done())
}

/// Drop the library templates no instance of vislox's calls any more, and
/// those the module now declares itself (lox refuses a template twice).
pub fn prune(files: &mut Files, ctx: &Ctx) {
    let called: BTreeSet<String> = files
        .pages
        .values()
        .flat_map(|p| p.instances.iter().map(|x| x.template.clone()))
        .collect();
    files
        .templates
        .retain(|n, _| called.contains(n) && !ctx.templates.contains_key(n));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_templates_file_reads_back() {
        let (_, t) = library_template("Treppenlicht").unwrap();
        let mut m = BTreeMap::new();
        m.insert(t.name.clone(), t);
        let text = render_templates(&m);
        assert!(text.starts_with("# Templates of lox's library"));
        assert_eq!(parse_templates(&text).unwrap(), m);
        assert!(parse_templates("vx_a = And()\n").is_err());
    }
}
