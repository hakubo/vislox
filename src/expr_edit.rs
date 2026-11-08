//! Editing expressions and named signals (#82) in vislox's files.
//!
//! Lox Logic writes logic as expressions: `x.Port <- a.Q and b.AQ >= 28` on
//! an extern, `Port: …` in a managed block's declaration, or a named signal
//! `warm = b.AQ >= 28`. lox desugars each into `And`, `Or`, `Not`, comparator
//! and `Formula` blocks, named `<sink>_<port>__<op><n>` (a signal's top block
//! is the signal itself), which it re-derives on every compile. So vislox
//! never edits those blocks: it edits the expression, in its own files only
//! (an expression of the module's own files is changed there, as its `let`
//! constants are, #81).
//!
//! - `expr` (a value op): what vislox's files feed into one input. An
//!   expression, written `vx_<uuid>.Port <- …` in `edits.loxl` for a block
//!   Loxone Config drew and `Port: …` in the declaration of a block of
//!   vislox's files; or a port of vislox's (`vx_warm.Q`), written as a wire,
//!   which is what *Name this expression* leaves.
//! - `signal`: a named signal of vislox's page file, `vx_warm = …`, with its
//!   label (`vx_warm.title = "…"`).
//!
//! Operands name what vislox's files may name: a block of theirs by its
//! slug, one Loxone Config drew as `vx_<uuid>` (an extern of `edits.loxl`,
//! added when the expression names it and dropped when nothing does), and a
//! constant (vislox's or the module's). lox checks the rest when the module
//! is checked and compiled: anything it refuses changes nothing.

use crate::blocks::{self, PageFile, SignalDef};
use crate::edit::{
    Applied, Ctx, Files, Geo, Op, Signal, Wire, block_title, end, extern_key, find, op_value,
    own_constant, pages_titled, slug, slug_uuid,
};
use loxone_config::logic::{self, Expr, Item, Module, Operand, PortRef, Value};
use std::collections::BTreeMap;

/// What a typed expression is: an expression lox desugars into blocks, or a
/// bare port, which is a plain wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Feed {
    Expr(Expr),
    Port(PortRef),
}

/// Read a typed expression with lox's parser, as the right side of `<-`.
pub fn parse_feed(text: &str) -> Result<Feed, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("the expression is empty".into());
    }
    if t.contains(['\n', '\r']) {
        return Err("an expression is one line".into());
    }
    let m = Module::parse_fragment(&format!("vx_expression.In <- {t}")).map_err(|e| {
        e.to_string()
            .replace("vx_expression.In <- ", "")
            .replace("vx_expression.In", "the input")
    })?;
    let mut items = m
        .items
        .into_iter()
        .filter(|i| !matches!(i, Item::Gap | Item::FragmentEnd | Item::Comment(_)));
    match (items.next(), items.next()) {
        (Some(Item::ExprWire(w)), None) => Ok(Feed::Expr(w.expr)),
        (Some(Item::Wire(w)), None) => Ok(Feed::Port(w.from)),
        _ => Err(format!("`{t}` is not one expression")),
    }
}

/// An expression of vislox's files as written (`text`, as lox prints it),
/// with what it reads and the type of its top block, read once when it is
/// parsed rather than on every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprSrc {
    pub text: String,
    /// The slugs of its port operands and its bare names of vislox's (a
    /// bare block name reads its default output, unless it is a constant).
    names: Vec<String>,
    top: Option<&'static str>,
}

impl std::fmt::Display for ExprSrc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl ExprSrc {
    pub fn of(e: &Expr) -> ExprSrc {
        let mut names: Vec<String> = Vec::new();
        for o in e.operands() {
            let n = match o {
                Operand::Port(p) => p.slug.clone(),
                Operand::Value(Value::Ref(n)) if n.starts_with(blocks::SLUG_PREFIX) => n.clone(),
                _ => continue,
            };
            if !names.contains(&n) {
                names.push(n);
            }
        }
        ExprSrc {
            text: e.to_string(),
            names,
            top: e.top_block_type(),
        }
    }

    /// Read `text`; a bare port is no expression.
    #[cfg(test)]
    pub fn parse(text: &str) -> Result<ExprSrc, String> {
        match parse_feed(text)? {
            Feed::Expr(e) => Ok(ExprSrc::of(&e)),
            Feed::Port(p) => Err(format!("`{p}` is a port, not an expression")),
        }
    }

    /// The names it reads, but for the constants of `consts`.
    pub fn reads<'a>(
        &'a self,
        consts: &'a BTreeMap<String, Value>,
    ) -> impl Iterator<Item = &'a String> {
        self.names.iter().filter(move |n| !consts.contains_key(*n))
    }

    /// The output it gives as a signal: `AQ` for a `Formula`, else `Q`.
    pub fn output(&self) -> &'static str {
        if self.top == Some("Formula") {
            "AQ"
        } else {
            "Q"
        }
    }
}

/// The externs an expression's operands need (key in the claims, type),
/// each checked: a block of vislox's files by its slug, one Loxone Config
/// drew as `vx_<uuid>` (in the base, not managed), or a constant.
pub(crate) fn operands(
    files: &Files,
    ctx: &Ctx,
    names: &[String],
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for n in names {
        if files.block(n).is_some() || files.signal(n).is_some() {
            continue;
        }
        if files.claims.8.contains_key(n) || ctx.lets.contains(n) {
            continue;
        }
        let Some(u) = slug_uuid(n) else {
            if ctx.names.contains(n) || ctx.lock.slugs.contains_key(n) {
                return Err(format!(
                    "`{n}` is the module's own: vislox's files name only their own blocks (by \
                     slug) and Loxone Config's (vx_ and the UUID, `-` as `_`)"
                ));
            }
            return Err(format!(
                "`{n}` names nothing vislox's files know: a block of theirs by its slug, one \
                 Loxone Config drew as vx_ and its UUID (`-` as `_`), or a constant"
            ));
        };
        let el = crate::tiles::find_ci(&ctx.doc.xml.root, &u)
            .ok_or_else(|| format!("`{n}` names no object of the pending config"))?;
        if let Some(s) = ctx.lock.slug_of(&u) {
            return Err(if files.block(s).is_some() || files.signal(s).is_some() {
                format!("`{n}` is vislox's `{s}`: name it by its slug")
            } else {
                format!(
                    "`{n}` is `{s}`, a block the module's own files manage; vislox's files do not \
                     name it"
                )
            });
        }
        if crate::tiles::find_ci(&ctx.base.xml.root, &u).is_none() {
            return Err(format!("`{n}` is not in the project's base config"));
        }
        let ty = el.attr("Type").unwrap_or_default().to_string();
        let key = extern_key(files, el.attr("U").unwrap_or(&u));
        if let Some(c) = files.claims.0.get(&key)
            && !c.ty.is_empty()
            && c.ty != ty
        {
            return Err(format!("vislox's file names {u} as a {}, not a {ty}", c.ty));
        }
        out.push((key, ty));
    }
    Ok(out)
}

/// What vislox's files feed into `port` (a Lox Logic name) of the block
/// `slug` (vislox's name for it) with UUID `uuid`: its expression, else the
/// wire into it, as `slug.Port`.
pub(crate) fn fed(files: &Files, slug_: &str, uuid: &str, port: &str) -> Option<String> {
    let expr = match files.block(slug_) {
        Some((_, b)) => b
            .exprs
            .iter()
            .find(|(k, _)| k == port)
            .map(|(_, e)| e.text.clone()),
        None => files
            .claims
            .0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(uuid))
            .and_then(|(_, c)| c.exprs.get(port).map(|e| e.text.clone())),
    };
    expr.or_else(|| {
        files
            .claims
            .1
            .iter()
            .find(|w| w.to.0 == slug_ && w.to.1 == port)
            .map(|w| format!("{}.{}", w.from.0, w.from.1))
    })
}

/// Take out what vislox's files feed into `port` of `slug_`.
fn unfeed(files: &mut Files, slug_: &str, uuid: &str, port: &str) {
    if let Some(b) = files.block_mut(slug_) {
        b.exprs.retain(|(k, _)| k != port);
    } else if let Some((_, c)) = files
        .claims
        .0
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case(uuid))
    {
        c.exprs.remove(port);
    }
    files
        .claims
        .1
        .retain(|w| !(w.to.0 == slug_ && w.to.1 == port));
}

pub(crate) fn add_externs(files: &mut Files, externs: Vec<(String, String)>) {
    for (key, ty) in externs {
        let c = files.claims.0.entry(key).or_default();
        if c.ty.is_empty() {
            c.ty = ty;
        }
    }
}

/// The `expr` op: what vislox's files feed into input `key` of `uuid`.
/// Returns what fed it before (`None`: nothing of vislox's).
pub fn set_expr(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    key: &str,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    // Dropping what vislox's files feed is always let through, even where
    // the block is gone or the module manages it now.
    if value.is_none() {
        let (sink, ty) = match ctx.lock.slug_of(uuid).filter(|s| files.block(s).is_some()) {
            Some(s) => (
                s.to_string(),
                files
                    .block(s)
                    .map(|(_, b)| b.ty.clone())
                    .unwrap_or_default(),
            ),
            None => (
                slug(uuid),
                files
                    .claims
                    .0
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(uuid))
                    .map(|(_, c)| c.ty.clone())
                    .unwrap_or_default(),
            ),
        };
        let port = loxone_config::port_names::name_of(&ty, key).to_string();
        let previous = fed(files, &sink, uuid, &port);
        unfeed(files, &sink, uuid, &port);
        files.repin();
        return Ok(previous);
    }
    let mut geo = Geo::new();
    let dst = end(files, ctx, &[uuid.to_string(), key.to_string()], &mut geo)?;
    let Some(sink) = dst.slug.clone() else {
        return Err(format!(
            "{}: the module manages this block; change its expressions in its .loxl file",
            dst.name
        ));
    };
    let port = dst.port.clone();
    let previous = fed(files, &sink, &dst.uuid, &port);
    let text = value.unwrap_or_default().trim();
    let label = format!("{}.{}", dst.name, dst.port);
    if dst.input != Some(true) {
        return Err(format!(
            "{label} is not an input: an expression feeds an input"
        ));
    }
    if !dst.added && find(&ctx.base.xml.root, &dst.uuid).is_none() {
        return Err(format!("{} is not in the project's base config", dst.name));
    }
    if dst.co.is_some_and(|c| c.attr("Inv").is_some()) {
        return Err(format!(
            "{label} is inverted in Loxone Config, which owns that input; lox does not wire it"
        ));
    }
    if dst.signal == Signal::Text {
        return Err(format!(
            "{label} takes text: an expression gives on/off or a number"
        ));
    }
    // An input takes one source here: Loxone Config's wire, the module's,
    // or what vislox's files feed it.
    if previous.is_none()
        && dst
            .co
            .is_some_and(|c| c.child_elements().any(|i| i.name == "In"))
    {
        return Err(format!(
            "{label} is wired already; take that wire out first (in Loxone Config, or here when \
             vislox drew it)"
        ));
    }
    let constant = match files.block(&sink) {
        Some((_, b)) => b.get(&port).is_some(),
        None => files
            .claims
            .0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&dst.uuid))
            .is_some_and(|(_, c)| c.sets.contains_key(&port)),
    };
    if constant {
        return Err(format!(
            "{label} has a constant edited here; clear it first, then give it an expression"
        ));
    }
    match parse_feed(text)? {
        Feed::Port(p) => {
            // A port of vislox's: a wire (what *Name this expression* leaves),
            // checked as the `wire` op checks one, on a copy until it passes.
            let mut next = files.clone();
            unfeed(&mut next, &sink, &dst.uuid, &port);
            let signal = files.signal(&p.slug).map(|(_, x)| x.expr.output());
            let compiled = ctx
                .lock
                .uuid_of(&p.slug)
                .filter(|u| crate::tiles::find_ci(&ctx.doc.xml.root, u).is_some());
            match (signal, compiled) {
                // A signal not compiled yet (made by this request): its output.
                (Some(out), None) => {
                    if !p.port.is_empty() && p.port != out {
                        return Err(format!("{}: a signal's output is {}.{out}", p.slug, p.slug));
                    }
                    if p.slug == sink {
                        return Err(format!(
                            "{label}: a block's own output does not feed its input here"
                        ));
                    }
                    // The sink's extern, as the wire op declares it.
                    if !dst.added {
                        let key = extern_key(&next, &dst.uuid);
                        let c = next.claims.0.entry(key).or_default();
                        if c.ty.is_empty() {
                            c.ty = dst.ty.clone();
                        }
                    }
                    next.claims.1.insert(Wire {
                        to: (sink.clone(), port.clone()),
                        from: (p.slug, out.to_string()),
                    });
                }
                _ => {
                    let from_port = match (signal, p.port.is_empty()) {
                        (Some(out), true) => out.to_string(),
                        (None, true) => {
                            return Err(format!(
                                "`{}` is a bare name: say which output (`{}.Q`), or write an \
                                 expression",
                                p.slug, p.slug
                            ));
                        }
                        _ => p.port.clone(),
                    };
                    let uuid = match compiled {
                        Some(u) => u.to_string(),
                        None if files.block(&p.slug).is_some() => {
                            return Err(format!("{} is not compiled yet: compile first", p.slug));
                        }
                        None => {
                            operands(files, ctx, std::slice::from_ref(&p.slug))?;
                            slug_uuid(&p.slug).expect("checked")
                        }
                    };
                    let el = crate::tiles::find_ci(&ctx.doc.xml.root, &uuid).ok_or_else(|| {
                        format!("`{}` names no object of the pending config", p.slug)
                    })?;
                    let ty = el.attr("Type").unwrap_or_default();
                    let at = [
                        el.attr("U").unwrap_or(&uuid).to_string(),
                        loxone_config::port_names::key_or_raw(ty, &from_port).to_string(),
                    ];
                    let src = end(&next, ctx, &at, &mut geo)?;
                    crate::edit::connect(&mut next, ctx, &src, &dst)?;
                }
            }
            next.repin();
            *files = next;
            return Ok(previous);
        }
        Feed::Expr(e) => {
            let src = ExprSrc::of(&e);
            let externs = operands(
                files,
                ctx,
                &src.reads(&files.claims.8).cloned().collect::<Vec<_>>(),
            )?;
            if files.block(&sink).is_none()
                && let Some((_, c)) = files
                    .claims
                    .0
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&dst.uuid))
                && !c.ty.is_empty()
                && c.ty != dst.ty
            {
                return Err(format!(
                    "vislox's file names {} as a {}, not a {}",
                    dst.uuid, c.ty, dst.ty
                ));
            }
            unfeed(files, &sink, &dst.uuid, &port);
            add_externs(files, externs);
            if let Some(b) = files.block_mut(&sink) {
                b.exprs.push((port.clone(), src));
            } else {
                let key = extern_key(files, &dst.uuid);
                let c = files.claims.0.entry(key).or_default();
                if c.ty.is_empty() {
                    c.ty = dst.ty.clone();
                }
                c.exprs.insert(port.clone(), src);
            }
        }
    }
    files.repin();
    Ok(previous)
}

/// The page UUID of vislox's page file `file`.
pub(crate) fn page_of(files: &Files, ctx: &Ctx, file: &str) -> String {
    let Some(pf) = files.pages.get(file) else {
        return String::new();
    };
    match pf
        .uuid
        .clone()
        .or_else(|| crate::page_edit::created_uuid(pf, ctx.lock))
    {
        Some(u) => u,
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

/// Whether anything of vislox's files reads the signal `s`: a wire out of
/// it, or an expression naming it.
fn reads(files: &Files, s: &str) -> Option<String> {
    if let Some(w) = files.claims.1.iter().find(|w| w.from.0 == s) {
        return Some(format!("the wire into {}.{}", w.to.0, w.to.1));
    }
    read_by_expression(files, s)
}

/// The expression of vislox's files that names `s`, but for `s`'s own (a
/// block's declaration, a signal), which go with it.
pub(crate) fn read_by_expression(files: &Files, s: &str) -> Option<String> {
    let c8 = &files.claims.8;
    // An argument of a template instance (#83).
    for p in files.pages.values() {
        for x in p.instances.iter().filter(|x| x.slug != s) {
            if x.names.iter().any(|n| n == s) {
                return Some(format!("the instance {}", x.slug));
            }
        }
    }
    for (u, c) in &files.claims.0 {
        for (k, e) in &c.exprs {
            if e.reads(c8).any(|n| n == s) {
                return Some(format!("the expression on {}.{k}", slug(u)));
            }
        }
    }
    for p in files.pages.values() {
        for b in p.blocks.iter().filter(|b| b.slug != s) {
            for (k, e) in &b.exprs {
                if e.reads(c8).any(|n| n == s) {
                    return Some(format!("the expression on {}.{k}", b.slug));
                }
            }
        }
        for x in p.signals.iter().filter(|x| x.slug != s) {
            if x.expr.reads(c8).any(|n| n == s) {
                return Some(format!("the signal {}", x.slug));
            }
        }
    }
    None
}

/// The `signal` op: make, change or drop a named signal of vislox's files.
/// Its `previous` is the `signal` op that puts back what was.
pub fn set_signal(files: &mut Files, ctx: &Ctx, op: &Op) -> Result<Applied, String> {
    let Op::Signal {
        slug: s,
        page,
        value,
        title,
    } = op
    else {
        unreachable!()
    };
    let s = s.trim().to_string();
    let existing = files.signal(&s).map(|(f, x)| (f.to_string(), x.clone()));
    let previous = Op::Signal {
        slug: s.clone(),
        page: existing
            .as_ref()
            .map(|(f, _)| page_of(files, ctx, f))
            .unwrap_or_else(|| page.clone()),
        value: existing.as_ref().map(|(_, x)| x.expr.text.clone()),
        title: existing.as_ref().and_then(|(_, x)| x.title.clone()),
    };
    let done = |added: Option<String>| Applied {
        previous: op_value(&previous),
        added,
        warnings: Vec::new(),
    };
    let lock_ty = ctx
        .lock
        .entry(&s)
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .map(String::from);
    let Some(text) = value.as_deref().map(str::trim) else {
        let Some((file, _)) = existing else {
            return Err(format!("vislox's files have no signal `{s}`"));
        };
        if let Some(what) = reads(files, &s) {
            return Err(format!("{s}: {what} reads it; take that out first"));
        }
        if let Some(u) = ctx.lock.uuid_of(&s)
            && ctx
                .deployed
                .is_some_and(|d| crate::tiles::find_ci(&d.xml.root, u).is_some())
        {
            return Err(format!(
                "{s} is deployed: deleting a deployed signal is not offered yet; change its \
                 expression instead"
            ));
        }
        let p = files.pages.get_mut(&file).expect("found");
        p.signals.retain(|x| x.slug != s);
        // lox refuses a managed block the source lost without it; marked as
        // a dropped signal's, which only making it again takes back.
        if ctx.lock.slugs.contains_key(&s) {
            p.removed.insert(s.clone());
            p.dropped.insert(s.clone());
        }
        files.repin();
        return Ok(done(None));
    };
    if existing.is_none() {
        logic::valid_slug(&s).map_err(|e| e.to_string())?;
        if !own_constant(&s) {
            return Err(format!(
                "a signal of vislox's is named {} and then lowercase letters, digits and _",
                blocks::SLUG_PREFIX
            ));
        }
        if s.contains("__") {
            return Err("`__` is lox's for the blocks an expression makes".into());
        }
    }
    let e = match parse_feed(text)? {
        Feed::Expr(e) => e,
        Feed::Port(p) => {
            return Err(format!(
                "`{s} = {p}` names a port: an alias is not a block; write `{p}` where `{s}.Q` \
                 would go"
            ));
        }
    };
    let top = e
        .top_block_type()
        .ok_or_else(|| format!("`{text}` makes no block"))?;
    let title = match title.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => Some(block_title(top, t)?),
        _ => None,
    };
    let src = ExprSrc::of(&e);
    let names: Vec<String> = src.reads(&files.claims.8).cloned().collect();
    if names.contains(&s) {
        return Err(format!("{s}: a signal does not read itself"));
    }
    let externs = operands(files, ctx, &names)?;
    // lox keeps a pinned block's type: `and` → `or` is a new block.
    if let Some(old) = &lock_ty
        && old != top
    {
        return Err(format!(
            "{s} is a{} {old} block, and `{text}` makes a{} {top}: lox keeps a block's type. \
             Make a new signal for it, then drop {s}",
            if old.starts_with(['A', 'E', 'I', 'O', 'U']) {
                "n"
            } else {
                ""
            },
            if top.starts_with(['A', 'E', 'I', 'O', 'U']) {
                "n"
            } else {
                ""
            },
        ));
    }
    let added = match existing {
        Some((file, _)) => {
            let p = files.pages.get_mut(&file).expect("found");
            let x = p.signals.iter_mut().find(|x| x.slug == s).expect("found");
            x.expr = src;
            x.title = title;
            None
        }
        None => {
            // A signal dropped here and not compiled away yet: its `removed`
            // line goes, and it comes back. Only a dropped signal's line: any
            // other `removed` line (a deleted block) is the name taken.
            let again = files
                .pages
                .values()
                .any(|p| p.removed.contains(&s) && p.dropped.contains(&s))
                && !files.names_with(&s, false)
                && !ctx.lock.tombstoned(&s);
            if !again
                && (ctx.names.contains(&s)
                    || ctx.lock.slugs.contains_key(&s)
                    || ctx.lock.tombstoned(&s)
                    || files.names(&s))
            {
                return Err(format!(
                    "`{s}` is taken: the module or its lockfile has that name"
                ));
            }
            if again && ctx.names.contains(&s) {
                return Err(format!("`{s}` is taken: the module has that name"));
            }
            // A page of the base, or one created here (#88).
            let (page_title, page_file) = crate::page_edit::target(files, ctx, page, "a signal")?;
            for p in files.pages.values_mut() {
                p.removed.remove(&s);
                p.dropped.remove(&s);
            }
            let file = match page_file {
                Some(n) => n,
                None => blocks::file_name(&page_title, &|n| {
                    n == crate::edit::CLAIMS || files.pages.contains_key(n)
                }),
            };
            let f = files.pages.entry(file).or_insert_with(|| PageFile {
                uuid: Some(page.clone()),
                ..PageFile::new(&page_title)
            });
            f.page = page_title;
            f.signals.push(SignalDef {
                slug: s.clone(),
                expr: src,
                title,
            });
            Some(s.clone())
        }
    };
    add_externs(files, externs);
    files.repin();
    Ok(done(added))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feed_is_an_expression_or_a_port() {
        let Feed::Expr(e) = parse_feed("vx_a.Q and vx_b.AQ >= 28").unwrap() else {
            panic!()
        };
        assert_eq!(e.to_string(), "vx_a.Q and vx_b.AQ >= 28");
        assert_eq!(e.top_block_type(), Some("And"));
        assert_eq!(
            parse_feed(" vx_warm.Q ").unwrap(),
            Feed::Port(PortRef {
                slug: "vx_warm".into(),
                port: "Q".into()
            })
        );
        assert!(parse_feed("").is_err());
        assert!(parse_feed("a.Q and\nb.Q").is_err());
        let e = parse_feed("5").unwrap_err();
        assert!(!e.contains("vx_expression"), "{e}");
        assert!(parse_feed("a.Q and (").is_err());
    }

    #[test]
    fn operands_are_names_not_constants() {
        let consts: BTreeMap<String, Value> =
            [("vx_schwelle".to_string(), Value::Number("28".into()))].into();
        let names = |t: &str| {
            ExprSrc::parse(t)
                .unwrap()
                .reads(&consts)
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("vx_a.Q and vx_b.AQ >= vx_schwelle and not vx_c"),
            ["vx_a", "vx_b", "vx_c"]
        );
        assert_eq!(names("vx_t.AQ / 1000 + vx_t.AQ"), ["vx_t"]);
        assert!(ExprSrc::parse("vx_w.Q").is_err());
        assert!(ExprSrc::parse("(((").is_err());
        assert_eq!(ExprSrc::parse("vx_t.AQ / 1000").unwrap().output(), "AQ");
        assert_eq!(ExprSrc::parse("vx_t.AQ >= 1").unwrap().output(), "Q");
    }
}
