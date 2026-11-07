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

/// The names an expression (as written) reads from: the slugs of its port
/// operands, and the bare names of vislox's that are no constant of
/// `consts` (a bare block name reads its default output). Empty when it
/// does not read.
pub fn operand_names(text: &str, consts: &BTreeMap<String, Value>) -> Vec<String> {
    match parse_feed(text) {
        Ok(Feed::Port(p)) => vec![p.slug],
        Ok(Feed::Expr(e)) => names_of(&e, consts),
        Err(_) => Vec::new(),
    }
}

fn names_of(e: &Expr, consts: &BTreeMap<String, Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for o in e.operands() {
        let n = match o {
            Operand::Port(p) => p.slug.clone(),
            Operand::Value(Value::Ref(n))
                if n.starts_with(blocks::SLUG_PREFIX) && !consts.contains_key(n) =>
            {
                n.clone()
            }
            _ => continue,
        };
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// The output a signal's expression gives: `AQ` for a `Formula`, else `Q`.
pub fn signal_output(expr: &str) -> &'static str {
    match parse_feed(expr) {
        Ok(Feed::Expr(e)) if e.top_block_type() == Some("Formula") => "AQ",
        _ => "Q",
    }
}

/// The externs an expression's operands need (key in the claims, type),
/// each checked: a block of vislox's files by its slug, one Loxone Config
/// drew as `vx_<uuid>` (in the base, not managed), or a constant.
fn operands(files: &Files, ctx: &Ctx, names: &[String]) -> Result<Vec<(String, String)>, String> {
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
fn fed(files: &Files, slug_: &str, uuid: &str, port: &str) -> Option<String> {
    let expr = match files.block(slug_) {
        Some((_, b)) => b
            .exprs
            .iter()
            .find(|(k, _)| k == port)
            .map(|(_, e)| e.clone()),
        None => files
            .claims
            .0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(uuid))
            .and_then(|(_, c)| c.exprs.get(port).cloned()),
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

fn add_externs(files: &mut Files, externs: Vec<(String, String)>) {
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
    let Some(text) = value.map(str::trim) else {
        unfeed(files, &sink, &dst.uuid, &port);
        files.repin();
        return Ok(previous);
    };
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
            // A port of vislox's: a wire (what *Name this expression* leaves).
            let from_port = if p.port.is_empty() {
                match files.signal(&p.slug) {
                    Some((_, x)) => signal_output(&x.expr).to_string(),
                    None => {
                        return Err(format!(
                            "`{}` is a bare name: say which output (`{}.Q`), or write an \
                             expression",
                            p.slug, p.slug
                        ));
                    }
                }
            } else {
                p.port.clone()
            };
            if p.slug == sink {
                return Err(format!(
                    "{label}: a block's own output does not feed its input here"
                ));
            }
            let externs = operands(files, ctx, std::slice::from_ref(&p.slug))?;
            unfeed(files, &sink, &dst.uuid, &port);
            add_externs(files, externs);
            files.claims.1.insert(Wire {
                to: (sink.clone(), port.clone()),
                from: (p.slug, from_port),
            });
        }
        Feed::Expr(e) => {
            let externs = operands(files, ctx, &names_of(&e, &files.claims.8))?;
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
            let text = e.to_string();
            unfeed(files, &sink, &dst.uuid, &port);
            add_externs(files, externs);
            if let Some(b) = files.block_mut(&sink) {
                b.exprs.push((port.clone(), text));
            } else {
                let key = extern_key(files, &dst.uuid);
                let c = files.claims.0.entry(key).or_default();
                if c.ty.is_empty() {
                    c.ty = dst.ty.clone();
                }
                c.exprs.insert(port.clone(), text);
            }
        }
    }
    files.repin();
    Ok(previous)
}

/// The page UUID of vislox's page file `file`.
fn page_of(files: &Files, ctx: &Ctx, file: &str) -> String {
    let pf = &files.pages[file];
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
    for (u, c) in &files.claims.0 {
        for (k, e) in &c.exprs {
            if operand_names(e, c8).iter().any(|n| n == s) {
                return Some(format!("the expression on {}.{k}", slug(u)));
            }
        }
    }
    for p in files.pages.values() {
        for b in p.blocks.iter().filter(|b| b.slug != s) {
            for (k, e) in &b.exprs {
                if operand_names(e, c8).iter().any(|n| n == s) {
                    return Some(format!("the expression on {}.{k}", b.slug));
                }
            }
        }
        for x in p.signals.iter().filter(|x| x.slug != s) {
            if operand_names(&x.expr, c8).iter().any(|n| n == s) {
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
        value: existing.as_ref().map(|(_, x)| x.expr.clone()),
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
        // lox refuses a managed block the source lost without it.
        if ctx.lock.slugs.contains_key(&s) {
            p.removed.insert(s.clone());
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
    let names = names_of(&e, &files.claims.8);
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
    let text = e.to_string();
    let added = match existing {
        Some((file, _)) => {
            let p = files.pages.get_mut(&file).expect("found");
            let x = p.signals.iter_mut().find(|x| x.slug == s).expect("found");
            x.expr = text;
            x.title = title;
            None
        }
        None => {
            // Dropped earlier and not compiled away yet: its `removed` line
            // goes, and it comes back.
            let again = files.pages.values().any(|p| p.removed.contains(&s))
                && !files.names_besides_removed(&s);
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
                     rename one in Loxone Config to add a signal here",
                    same.len()
                ));
            }
            for p in files.pages.values_mut() {
                p.removed.remove(&s);
            }
            let file = match files
                .pages
                .iter()
                .find(|(_, p)| p.uuid.as_deref() == Some(page.as_str()))
            {
                Some((n, _)) => n.clone(),
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
                expr: text,
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
        assert_eq!(
            operand_names("vx_a.Q and vx_b.AQ >= vx_schwelle and not vx_c", &consts),
            ["vx_a", "vx_b", "vx_c"]
        );
        assert_eq!(operand_names("vx_t.AQ / 1000 + vx_t.AQ", &consts), ["vx_t"]);
        assert_eq!(operand_names("vx_w.Q", &consts), ["vx_w"]);
        assert!(operand_names("(((", &consts).is_empty());
        assert_eq!(signal_output("vx_t.AQ / 1000"), "AQ");
        assert_eq!(signal_output("vx_t.AQ >= 1"), "Q");
    }
}
