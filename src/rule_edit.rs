//! Editing Automatic Designer rules (#87): the `rule`, `new_rule` and
//! `restore_rule` ops of `POST /api/edit`, and the `rule` declarations they
//! write into vislox's file of claims (`<module>/vislox/edits.loxl`).
//!
//! Lox Logic states what a rule does with a declaration (lox's
//! `language.md`, *`rule`*):
//!
//! ```text
//! extern vx_40000002_… = AutopilotRule(uuid: "40000002-…")
//! extern vx_40000001_… = PushButton(uuid: "40000001-…")
//!
//! rule vx_40000002_…
//!     active
//!     when weekday weekdays
//!     when time 09:00
//!     then vx_40000001_… on
//! end
//!
//! rule vx_rule_1 "Pumpe Mittag"     # created by the compile
//!     inactive
//! end
//!
//! removed vx_rule_2                 # deletes a rule created here
//! ```
//!
//! - A declaration on an extern owns the rule's conditions, actions and
//!   `active`; its name stays the app's. Dropping it leaves the rule as last
//!   written (lox keeps no lock state for it), so the op with no value drops
//!   vislox's claim.
//! - A named declaration (`rule <slug> "Name"`) creates the rule when the
//!   config lacks it (inactive and empty until its body is written, below the
//!   config's last rule): the `new_rule` op. The lock pins its UUID (`rules`);
//!   the name is the source's. `removed <slug>` deletes it (`remove_block`);
//!   one not in the base yet is simply dropped. A config without any rule
//!   cannot get one: the app decides where rules live.
//! - lox checks the rest when it compiles: a block's states and commands
//!   against the app's capability table (and the commands the config's rules
//!   already send), a date against the Miniserver's time zone. Its refusal is
//!   the edit's.

use crate::edit::{self, Applied, Ctx, Files, Lock, Op};
use crate::rules::{self, Line, Read, RuleVal};
use crate::set_claim;
use loxone_config::LoxoneDoc;
use loxone_config::logic::{Item, ModeName, Module, RuleStmtKind};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The prefix of a rule created here: `vx_rule_<n>`.
pub const NEW_PREFIX: &str = "vx_rule_";

/// The most lines one rule takes (a bound on a request).
pub const MAX_LINES: usize = 100;

/// One declaration of vislox's file: the rule's name when it is a named
/// one (created here), and its body as lox's parser read it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RuleClaim {
    pub title: Option<String>,
    pub body: Vec<RuleStmtKind>,
}

/// The rules of vislox's file: declarations by slug, and the `removed`
/// lines of rules created here.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rules {
    pub claims: BTreeMap<String, RuleClaim>,
    pub removed: BTreeSet<String>,
}

impl Rules {
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty() && self.removed.is_empty()
    }
}

/// The comment above the rules in the file.
pub const HEADER: &str = "# Automatic Designer rules edited in vislox. A declaration states what the rule does;\n\
                          # deleting it leaves the rule as last written.\n";

fn decl_text(slug: &str, c: &RuleClaim) -> String {
    let mut out = match &c.title {
        Some(t) => format!("rule {slug} {}\n", rules::quote(t)),
        None => format!("rule {slug}\n"),
    };
    for st in &c.body {
        out.push_str(&format!("\t{st}\n"));
    }
    out.push_str("end\n");
    out
}

/// The rules as the file has them (empty when there are none).
pub fn render(r: &Rules) -> String {
    let mut out = String::new();
    if r.is_empty() {
        return out;
    }
    out.push('\n');
    out.push_str(HEADER);
    for (slug, c) in &r.claims {
        out.push('\n');
        out.push_str(&decl_text(slug, c));
    }
    if !r.removed.is_empty() {
        out.push_str(
            "\n# Rules created in vislox and deleted: lox takes them out of the config.\n",
        );
        for s in &r.removed {
            out.push_str(&format!("removed {s}\n"));
        }
    }
    out
}

/// The slug vislox's file declares the rule `uuid` by: its extern
/// (`vx_<uuid>`), or a rule created here, by the lock's pin or (not compiled
/// yet) by its slug given as `uuid`.
pub fn decl_slug(files: &Files, lock: &Lock, uuid: &str) -> String {
    let r = &files.claims.9;
    if r.claims.get(uuid).is_some_and(|c| c.title.is_some()) {
        return uuid.to_string();
    }
    if let Some((s, _)) = lock
        .rules
        .iter()
        .find(|(s, u)| u.eq_ignore_ascii_case(uuid) && r.claims.contains_key(*s))
    {
        return s.clone();
    }
    edit::slug(&edit::extern_key(files, uuid))
}

/// The slug of the rule created here that `uuid` names (its pin, or the slug
/// itself), if it is one.
pub fn created_slug(files: &Files, lock: &Lock, uuid: &str) -> Option<String> {
    let s = decl_slug(files, lock, uuid);
    files
        .claims
        .9
        .claims
        .get(&s)
        .filter(|c| c.title.is_some())
        .map(|_| s)
}

/// The UUID a slug of vislox's files names a block by: an extern of the
/// file, a block of vislox's files, else the slug as it is (a block not
/// pinned yet).
fn block_of(files: &Files, lock: &Lock, slug: &str) -> String {
    set_claim::member_of(files, lock, slug)
        .map(|u| u.to_ascii_lowercase())
        .unwrap_or_else(|| slug.to_string())
}

/// The UUID a mode name of a declaration names: a slug of the lockfile's, or
/// a title of `doc`'s modes; else as written.
fn mode_of(lock: &Lock, doc: Option<&LoxoneDoc>, m: &ModeName) -> String {
    match m {
        ModeName::Slug(s) => lock
            .slugs
            .get(s)
            .or_else(|| lock.externals.get(s))
            .map(|u| u.to_ascii_lowercase())
            .unwrap_or_else(|| s.clone()),
        ModeName::Title(t) => doc
            .and_then(|d| {
                let modes = rules::modes(d);
                let mut f = modes.iter().filter(|(_, x)| x == t);
                let first = f.next()?.0.clone();
                f.next().is_none().then_some(first)
            })
            .unwrap_or_else(|| rules::quote(t)),
    }
}

/// What vislox's file says the rule declared as `slug` does, its blocks and
/// modes by UUID (modes named by title resolved in `doc`), or `None` when the
/// file declares no such rule.
pub fn current(files: &Files, lock: &Lock, doc: Option<&LoxoneDoc>, slug: &str) -> Option<RuleVal> {
    let c = files.claims.9.claims.get(slug)?;
    Some(val_of(files, lock, doc, c))
}

fn val_of(files: &Files, lock: &Lock, doc: Option<&LoxoneDoc>, c: &RuleClaim) -> RuleVal {
    let decl = loxone_config::logic::RuleDecl {
        slug: String::new(),
        title: None,
        body: c
            .body
            .iter()
            .map(|k| {
                loxone_config::logic::RuleItem::Stmt(loxone_config::logic::RuleStmt {
                    kind: k.clone(),
                    comment: None,
                })
            })
            .collect(),
        comment: None,
        end_comment: None,
    };
    rules::lines_of(&decl, &|s| block_of(files, lock, s), &|m| {
        mode_of(lock, doc, m)
    })
}

/// The shortest weekday words for the days `days` names, as lox lifts them:
/// a whole group where the set is one, single days in week order otherwise.
fn canonical_days(days: &[String]) -> Vec<String> {
    const WEEK: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let mut set = [false; 7];
    for d in days {
        match d.as_str() {
            "weekdays" => set[..5].iter_mut().for_each(|x| *x = true),
            "weekend" => set[5..].iter_mut().for_each(|x| *x = true),
            "daily" => set.iter_mut().for_each(|x| *x = true),
            w => {
                if let Some(i) = WEEK.iter().position(|x| *x == w) {
                    set[i] = true;
                }
            }
        }
    }
    let pick: Vec<usize> = (0..7).filter(|i| set[*i]).collect();
    match pick.as_slice() {
        [0, 1, 2, 3, 4, 5, 6] => vec!["daily".into()],
        [0, 1, 2, 3, 4] => vec!["weekdays".into()],
        [5, 6] => vec!["weekend".into()],
        p => p.iter().map(|i| WEEK[*i].to_string()).collect(),
    }
}

/// A value as compared: `on`/`off` as 1/0, a number as a number.
fn norm_value(v: &str) -> String {
    match v.trim() {
        "on" => "1".into(),
        "off" => "0".into(),
        t => t
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f.to_string())
            .unwrap_or_else(|| t.to_string()),
    }
}

/// `v` as compared: UUIDs lowercase, weekdays in their shortest words,
/// values as numbers, a time of day in two-digit form.
pub fn normalized(v: &RuleVal) -> RuleVal {
    let mut v = v.clone();
    for l in &mut v.lines {
        match l {
            Line::State { block, value, .. } => {
                *block = block.to_ascii_lowercase();
                *value = norm_value(value);
            }
            Line::Then { block, .. } => *block = block.to_ascii_lowercase(),
            Line::Mode { mode, .. } | Line::ThenMode { mode, .. } if rules::is_uuid(mode) => {
                *mode = mode.to_ascii_lowercase()
            }
            Line::Weather { value, .. } => *value = norm_value(value),
            Line::Weekday { days } => *days = canonical_days(days),
            Line::Time { at } => {
                if let Some((h, m)) = at.split_once(':')
                    && let (Ok(h), Ok(m)) = (h.trim().parse::<u32>(), m.trim().parse::<u32>())
                {
                    *at = format!("{h:02}:{m:02}");
                }
            }
            _ => {}
        }
    }
    v
}

fn title_of(doc: &LoxoneDoc, uuid: &str) -> String {
    crate::adopt::name_in(doc, uuid)
}

/// The rules of `doc` titled `t`, case aside.
fn titled(doc: &LoxoneDoc, t: &str) -> usize {
    doc.objects()
        .iter()
        .filter(|o| {
            o.block_type == rules::RULE
                && o.title
                    .as_deref()
                    .is_some_and(|x| x.trim().eq_ignore_ascii_case(t.trim()))
        })
        .count()
}

/// The body for `value`, checked and read back with lox's parser, and the
/// externs its lines need (UUID → type). Nothing is changed.
fn body_of(
    files: &Files,
    ctx: &Ctx,
    name: &str,
    value: &RuleVal,
) -> Result<(Vec<RuleStmtKind>, BTreeMap<String, String>), String> {
    if value.lines.len() > MAX_LINES {
        return Err(format!("{name}: more than {MAX_LINES} lines in one rule"));
    }
    let mut value = value.clone();
    for l in &mut value.lines {
        if let Line::Weekday { days } = l {
            for d in days.iter() {
                if !rules::DAYS.contains(&d.as_str()) {
                    return Err(format!(
                        "{name}: `{d}` is no day: {}",
                        rules::DAYS.join(" ")
                    ));
                }
            }
            *days = canonical_days(days);
        }
    }
    rules::check_shape(&value).map_err(|e| format!("{name}: {e}"))?;
    if value.lines.iter().any(|l| matches!(l, Line::Date { .. })) && timezone(ctx.base).is_none() {
        return Err(format!(
            "{name}: `when date` needs the Miniserver's time zone in the config (Loxone Config \
             writes it with the Miniserver's settings); Lox Logic converts dates in the Central \
             European time zones only"
        ));
    }
    let externs = std::cell::RefCell::new(BTreeMap::<String, String>::new());
    let block = |u: &str| -> Result<String, String> {
        let u = u.trim();
        // A block of vislox's files lox has not pinned yet: by its slug.
        if files.block(u).is_some() && ctx.lock.uuid_of(u).is_none() {
            return Ok(u.to_string());
        }
        let el = edit::find_ci(&ctx.doc.xml.root, u)
            .ok_or_else(|| format!("no object {u} in the pending config"))?;
        let t = title_of(ctx.doc, u);
        match ctx.lock.slug_of(u) {
            Some(s) if files.block(s).is_some() => Ok(s.to_string()),
            Some(_) => Err(format!(
                "{t} is managed by the module, and vislox's files name only their own blocks and \
                 Loxone Config's; write this rule in the module's own file"
            )),
            None => {
                let ty = el.attr("Type").unwrap_or_default();
                if ty == rules::RULE || ty.is_empty() {
                    return Err(format!("{t} is no block a rule watches or drives"));
                }
                let key = edit::extern_key(files, u);
                externs.borrow_mut().insert(key.clone(), ty.to_string());
                Ok(edit::slug(&key))
            }
        }
    };
    let modes = rules::modes(ctx.doc);
    let mode = |m: &str| -> Result<String, String> {
        let m = m.trim();
        if rules::is_uuid(m) {
            let u = m.to_ascii_lowercase();
            let (_, title) = modes
                .iter()
                .find(|(x, _)| *x == u)
                .ok_or_else(|| format!("the config has no operating mode {m}"))?;
            // A mode the module declares or externs is named by its slug.
            if let Some((s, _)) = ctx
                .lock
                .slugs
                .iter()
                .chain(&ctx.lock.externals)
                .find(|(_, x)| x.eq_ignore_ascii_case(&u))
            {
                return Ok(s.clone());
            }
            let same = modes.iter().filter(|(_, t)| t == title).count();
            if same > 1 {
                return Err(format!(
                    "{same} operating modes are titled \"{title}\", and a rule names a mode by its \
                     title: rename one in Loxone Config"
                ));
            }
            return Ok(rules::quote(title));
        }
        // As written (a mode no longer in the config): a slug, or a quoted
        // title that is one string.
        let quoted = m.len() >= 2
            && m.starts_with('"')
            && m.ends_with('"')
            && !m.contains('\n')
            && !m[1..m.len() - 1].contains('"');
        let slug = !m.is_empty()
            && m.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if quoted || slug {
            Ok(m.to_string())
        } else {
            Err(format!("`{m}` names no operating mode"))
        }
    };
    let names = rules::Names {
        block: &block,
        mode: &mode,
    };
    let mut src = String::from("rule vx_check\n");
    src.push_str(if value.active {
        "\tactive\n"
    } else {
        "\tinactive\n"
    });
    for l in &value.lines {
        let t = rules::text(l, &names).map_err(|e| format!("{name}: {e}"))?;
        src.push_str(&format!("\t{t}\n"));
    }
    src.push_str("end\n");
    let m = Module::parse_fragment(&src).map_err(|e| format!("{name}: {e}"))?;
    let decl = m
        .rule_decls()
        .next()
        .ok_or_else(|| format!("{name}: the rule does not read"))?;
    let body: Vec<RuleStmtKind> = decl.stmts().map(|s| s.kind.clone()).collect();
    if body.len() != value.lines.len() + 1
        || m.items
            .iter()
            .filter(|i| !matches!(i, Item::Gap | Item::FragmentEnd))
            .count()
            != 1
    {
        return Err(format!("{name}: a line reads as more than one statement"));
    }
    Ok((body, externs.into_inner()))
}

/// The config's time zone (`Timezone=` on the Miniserver), if it states one.
pub fn timezone(doc: &LoxoneDoc) -> Option<String> {
    fn walk(el: &loxone_config::xml::Element) -> Option<String> {
        if let Some(tz) = el.attr("Timezone") {
            return Some(tz.to_string());
        }
        el.child_elements().find_map(walk)
    }
    walk(&doc.xml.root).filter(|t| !t.trim().is_empty())
}

fn add_externs(files: &mut Files, externs: BTreeMap<String, String>) {
    for (key, ty) in externs {
        let c = files.claims.0.entry(key).or_default();
        // Never retype another claim's extern.
        if c.ty.is_empty() {
            c.ty = ty;
        }
    }
}

/// Why `ctx`'s rule `uuid` cannot get a declaration of vislox's now, if it
/// cannot: no rule, the module states it, Lox Logic cannot write it.
pub fn refusal(
    files: &Files,
    ctx: &Ctx,
    read: &BTreeMap<String, Read>,
    uuid: &str,
) -> Option<String> {
    let Some(el) = edit::find_ci(&ctx.doc.xml.root, uuid) else {
        return Some(format!("no object {uuid} in the pending config"));
    };
    let name = title_of(ctx.doc, uuid);
    if el.attr("Type") != Some(rules::RULE) {
        return Some(format!("{name} is no Automatic Designer rule"));
    }
    if let Some(at) = ctx.claimed("rule").get(&uuid.to_ascii_lowercase()) {
        return Some(format!("{name}: {at} states this rule; change it there"));
    }
    if let Some((s, _)) = ctx
        .lock
        .rules
        .iter()
        .find(|(s, u)| u.eq_ignore_ascii_case(uuid) && !files.claims.9.claims.contains_key(*s))
    {
        return Some(format!(
            "{name}: the module declares this rule by its name (`rule {s}`); change it there"
        ));
    }
    let ours = files
        .claims
        .9
        .claims
        .contains_key(&decl_slug(files, ctx.lock, uuid));
    match read.get(&uuid.to_ascii_lowercase()) {
        Some(Read::Not(why)) if !ours => Some(format!("{name}: {why}")),
        _ => None,
    }
}

/// Apply a `rule` op: make `value` what the rule `uuid` does, or drop
/// vislox's declaration (`None`) on a rule of Loxone Config's. Its
/// `previous` is what vislox's file stated before (`null`: nothing).
pub fn apply(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    value: Option<&RuleVal>,
) -> Result<Applied, String> {
    let slug = decl_slug(files, ctx.lock, uuid);
    let previous = current(files, ctx.lock, Some(ctx.doc), &slug);
    let done = |p: Option<RuleVal>| Applied {
        previous: json!(p),
        added: None,
        warnings: Vec::new(),
    };
    let named = files
        .claims
        .9
        .claims
        .get(&slug)
        .and_then(|c| c.title.clone());
    // Dropping vislox's declaration is always let through, whatever the rule
    // has become (deleted, stated by the module too): one left behind would
    // break every compile.
    let Some(value) = value else {
        if named.is_some() {
            return Err(format!(
                "{}: a rule created here is deleted, not handed back (remove_block)",
                title_of(ctx.doc, uuid)
            ));
        }
        if !files.claims.9.claims.contains_key(&slug)
            && edit::find_ci(&ctx.doc.xml.root, uuid).is_none()
        {
            return Err(format!("no object {uuid} in the pending config"));
        }
        files.claims.9.claims.remove(&slug);
        files.claims.prune();
        return Ok(done(previous));
    };
    let name = named.clone().unwrap_or_else(|| title_of(ctx.doc, uuid));
    // A rule created here and not compiled yet is not in the config.
    if named.is_none() || edit::find_ci(&ctx.doc.xml.root, uuid).is_some() {
        let read = rules::read(ctx.doc);
        if let Some(why) = refusal(files, ctx, &read, uuid) {
            return Err(why);
        }
    }
    let (body, externs) = body_of(files, ctx, &name, value)?;
    add_externs(files, externs);
    if named.is_none() {
        let key = edit::extern_key(files, uuid);
        let c = files.claims.0.entry(key).or_default();
        if c.ty.is_empty() {
            c.ty = rules::RULE.into();
        }
    }
    files
        .claims
        .9
        .claims
        .insert(slug, RuleClaim { title: named, body });
    files.claims.prune();
    Ok(done(previous))
}

/// A name for a new rule: one line, not empty, no other rule's (case
/// aside), so that lox creates it rather than takes another.
fn check_title(
    files: &Files,
    ctx: &Ctx,
    title: &str,
    except: Option<&str>,
) -> Result<String, String> {
    let t = title.trim();
    if t.is_empty() {
        return Err("a new rule needs a name".into());
    }
    if t.contains(['\n', '\r']) || t.chars().count() > 120 {
        return Err("a rule's name is one line of at most 120 characters".into());
    }
    if titled(ctx.doc, t) > 0 || titled(ctx.base, t) > 0 {
        return Err(format!(
            "the config has a rule named \"{t}\" already: lox would take that rule instead of \
             creating one"
        ));
    }
    if files.claims.9.claims.iter().any(|(s, c)| {
        Some(s.as_str()) != except
            && c.title
                .as_deref()
                .is_some_and(|x| x.trim().eq_ignore_ascii_case(t))
    }) {
        return Err(format!("a rule created here is named \"{t}\" already"));
    }
    Ok(t.to_string())
}

/// Apply a `new_rule` op: a rule named `title`, created by the compile
/// (lox creates it inactive and empty, then writes `value`). Its `previous` is null; it is undone
/// by `remove_block` of the UUID lox gives it.
pub fn new_rule(
    files: &mut Files,
    ctx: &Ctx,
    title: &str,
    value: Option<&RuleVal>,
) -> Result<Applied, String> {
    // lox places a new rule below the config's last one, where the app keeps
    // them: a config without any has nowhere.
    if !has_rules(ctx.base) {
        return Err(
            "the config has no Automatic Designer rule yet: create one in the app once (the app \
             decides where rules live), then add rules here"
                .into(),
        );
    }
    let t = check_title(files, ctx, title, None)?;
    // lox's checker wants a body: a condition and an action at least.
    let value = value.ok_or_else(|| format!("{t}: a new rule needs its conditions and actions"))?;
    let (body, externs) = body_of(files, ctx, &t, value)?;
    let slug = (1u32..)
        .map(|n| format!("{NEW_PREFIX}{n}"))
        .find(|s| {
            !set_claim::taken(files, ctx, s)
                && !ctx.lock.rules.contains_key(s)
                && !files.claims.9.removed.contains(s)
        })
        .expect("a free slug");
    add_externs(files, externs);
    files.claims.9.claims.insert(
        slug.clone(),
        RuleClaim {
            title: Some(t),
            body,
        },
    );
    files.claims.prune();
    Ok(Applied {
        previous: Value::Null,
        added: Some(slug),
        warnings: Vec::new(),
    })
}

/// Whether `doc` has an Automatic Designer rule.
pub fn has_rules(doc: &LoxoneDoc) -> bool {
    doc.objects().iter().any(|o| o.block_type == rules::RULE)
}

/// Rename a rule created here (its `title` op): the source owns its name.
pub fn rename(
    files: &mut Files,
    ctx: &Ctx,
    slug: &str,
    title: Option<&str>,
) -> Result<Applied, String> {
    let before = files.claims.9.claims[slug].title.clone();
    let Some(title) = title else {
        return Err("a rule created here keeps a name".into());
    };
    let same = before.as_deref().is_some_and(|b| b.trim() == title.trim());
    let t = if same {
        title.trim().to_string()
    } else {
        check_title(files, ctx, title, Some(slug))?
    };
    files.claims.9.claims.get_mut(slug).expect("checked").title = Some(t);
    Ok(Applied {
        previous: json!(before),
        added: None,
        warnings: Vec::new(),
    })
}

/// Delete the rule created here that `uuid` names (`remove_block`): one the
/// base has goes with `removed <slug>`, and its `previous` is the
/// `restore_rule` that brings it back under its UUID; one the base lacks is
/// dropped, and its `previous` is the `new_rule` that creates it again.
pub fn remove(files: &mut Files, ctx: &Ctx, slug: &str, uuid: &str) -> Result<Applied, String> {
    // lox refuses to delete a rule whose outputs feed a block: say so first,
    // from the pending config (what would deploy).
    if let Some(fed) = drives(ctx.doc, uuid) {
        return Err(format!(
            "{}: the rule drives {fed}; take that wire out in Loxone Config first",
            title_of(ctx.doc, uuid)
        ));
    }
    let c = files.claims.9.claims[slug].clone();
    let title = c.title.clone().unwrap_or_default();
    let value = val_of(files, ctx.lock, Some(ctx.doc), &c);
    let pinned = ctx.lock.rules.get(slug).cloned();
    let in_base = pinned
        .as_deref()
        .is_some_and(|u| edit::find_ci(&ctx.base.xml.root, u).is_some());
    files.claims.9.claims.remove(slug);
    let previous = if let (true, Some(u)) = (in_base, pinned) {
        files.claims.9.removed.insert(slug.to_string());
        json!(Op::RestoreRule {
            uuid: u,
            slug: slug.to_string(),
            title,
            value,
        })
    } else {
        json!(Op::NewRule {
            title,
            value: Some(value),
            restores: Some(uuid.to_string()),
        })
    };
    files.claims.prune();
    Ok(Applied {
        previous,
        added: None,
        warnings: Vec::new(),
    })
}

/// The block a wire from one of the rule `uuid`'s connectors feeds in `doc`,
/// by name, if any.
fn drives(doc: &LoxoneDoc, uuid: &str) -> Option<String> {
    let el = edit::find_ci(&doc.xml.root, uuid)?;
    let ports: BTreeSet<String> = el
        .child_elements()
        .filter(|c| c.name == "Co")
        .filter_map(|c| c.attr("U").map(|u| u.to_ascii_lowercase()))
        .collect();
    fn walk(
        el: &loxone_config::xml::Element,
        owner: Option<&loxone_config::xml::Element>,
        ports: &BTreeSet<String>,
    ) -> Option<String> {
        let owner = if el.name == "C" { Some(el) } else { owner };
        if el.name == "In"
            && el
                .attr("Input")
                .is_some_and(|i| ports.contains(&i.to_ascii_lowercase()))
        {
            return owner.map(|o| {
                o.attr_decoded("Title")
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|| o.attr("Type").unwrap_or_default().to_string())
            });
        }
        el.child_elements().find_map(|c| walk(c, owner, ports))
    }
    if ports.is_empty() {
        return None;
    }
    walk(&doc.xml.root, None, &ports)
}

/// Bring back a rule created here and deleted (`restore_rule`, the
/// `previous` of its `remove_block`): while lox keeps its tombstone, the
/// lock's pin goes back and the declaration with it, so it keeps its UUID.
/// Routed by the lockfile's tombstone, never by the request's word. Its
/// `previous` is the `remove_block`.
pub fn restore(
    files: &mut Files,
    ctx: &Ctx,
    uuid: &str,
    slug: &str,
    title: &str,
    value: &RuleVal,
) -> Result<Applied, String> {
    if !slug.starts_with(NEW_PREFIX) || loxone_config::logic::valid_slug(slug).is_err() {
        return Err(format!("`{slug}` is no rule created here"));
    }
    if files.claims.9.claims.contains_key(slug) {
        return Err(format!("vislox's file declares `{slug}` already"));
    }
    // Deleted in this request, before any compile: the line goes again.
    let pending = files.claims.9.removed.contains(slug)
        && ctx
            .lock
            .rules
            .get(slug)
            .is_some_and(|u| u.eq_ignore_ascii_case(uuid));
    if !pending {
        match ctx.lock.tomb(uuid) {
            Some((u, s, ty)) if s == slug && ty == rules::RULE => {
                files.lock_edits.untomb.insert(u.to_string());
                files
                    .lock_edits
                    .rules
                    .insert(slug.to_string(), u.to_string());
            }
            _ => {
                return Err(format!(
                    "lox keeps no deleted rule {uuid} as `{slug}` any more: create it again \
                     instead"
                ));
            }
        }
    }
    let t = title.trim();
    if t.is_empty() || t.contains(['\n', '\r']) {
        return Err("a rule created here keeps a name of one line".into());
    }
    let (body, externs) = body_of(files, ctx, t, value)?;
    add_externs(files, externs);
    files.claims.9.removed.remove(slug);
    files.claims.9.claims.insert(
        slug.to_string(),
        RuleClaim {
            title: Some(t.to_string()),
            body,
        },
    );
    files.claims.prune();
    Ok(Applied {
        previous: json!(Op::RemoveBlock {
            uuid: uuid.to_string()
        }),
        added: None,
        warnings: Vec::new(),
    })
}

/// Drop the `removed` lines lox needs no more: those whose rule the lock
/// neither pins nor keeps a tombstone of.
pub fn tidy(r: &mut Rules, lock: &Lock) {
    r.removed
        .retain(|s| lock.rules.contains_key(s) || lock.tombstoned(s));
}

/// The lines [`tidy`] would drop.
pub fn stale_lines(r: &Rules, lock: &Lock) -> Vec<String> {
    r.removed
        .iter()
        .filter(|s| !lock.rules.contains_key(*s) && !lock.tombstoned(s))
        .map(|s| format!("removed {s}"))
        .collect()
}

/// The UUID of the rule `slug` of vislox's file: its extern's, or the lock's
/// pin of a rule created here.
pub fn rule_uuid(files: &Files, lock: &Lock, slug: &str) -> Option<String> {
    match files.claims.9.claims.get(slug)?.title {
        Some(_) => lock.rules.get(slug).map(|u| u.to_ascii_lowercase()),
        None => edit::slug_uuid(slug),
    }
}

/// After the compile: the rules among `uuids` the config does not have as
/// vislox's file states them.
pub fn missed(files: &Files, out: &LoxoneDoc, lock: &Lock, uuids: &[String]) -> Vec<String> {
    let read = rules::read(out);
    let mut bad = Vec::new();
    for u in uuids {
        let slug = decl_slug(files, lock, u);
        let Some(want) = current(files, lock, Some(out), &slug) else {
            continue;
        };
        let Some(uuid) = rule_uuid(files, lock, &slug) else {
            continue;
        };
        let got = match read.get(&uuid) {
            Some(Read::Rule(v)) => Some(normalized(v)),
            _ => None,
        };
        if got != Some(normalized(&want)) {
            bad.push(title_of(out, &uuid));
        }
    }
    bad
}

/// What `doc` says rule `uuid` does, normalized; `None` when it has no such
/// rule or Lox Logic cannot write it.
fn read_norm(read: &BTreeMap<String, Read>, uuid: &str) -> Option<RuleVal> {
    match read.get(&uuid.to_ascii_lowercase()) {
        Some(Read::Rule(v)) => Some(normalized(v)),
        _ => None,
    }
}

/// `GET /api/edits`' `rules`: per rule vislox's file declares now or
/// declared in the deployed revision, what it states (`value`), what the
/// deployed revision stated (`deployed`: the value a Revert sends),
/// whether it is a rule created here (`created`: its name and slug), and
/// whether the pending config's rule differs from the deployed one's
/// (`pending`).
pub fn edits_section(
    now: &Files,
    was: &Files,
    lock: &Lock,
    pending: Option<&LoxoneDoc>,
    deployed: Option<&LoxoneDoc>,
) -> Value {
    let Some(doc) = pending.or(deployed) else {
        return json!([]);
    };
    let (rp, rd) = (pending.map(rules::read), deployed.map(rules::read));
    let mut by: BTreeMap<String, (Option<&str>, Option<&str>)> = BTreeMap::new();
    for (files, now_side) in [(now, true), (was, false)] {
        for s in files.claims.9.claims.keys() {
            let u = rule_uuid(files, lock, s).unwrap_or_else(|| s.clone());
            let slot = by.entry(u).or_default();
            if now_side {
                slot.0 = Some(s)
            } else {
                slot.1 = Some(s)
            }
        }
    }
    by.into_iter()
        .map(|(u, (sn, sw))| {
            let created = sn
                .and_then(|s| now.claims.9.claims.get(s).map(|c| (s, c)))
                .and_then(|(s, c)| c.title.as_ref().map(|t| json!({ "slug": s, "title": t })));
            json!({
                "uuid": u,
                "value": sn.and_then(|s| current(now, lock, pending, s)),
                "deployed": sw.and_then(|s| current(was, lock, deployed, s)),
                "created": created,
                // A rule created here: its name as deployed, which Revert gives back too.
                "deployed_title": sw.and_then(|s| was.claims.9.claims.get(s)).and_then(|c| c.title.clone()),
                "missing": edit::find_ci(&doc.xml.root, &u).is_none(),
                "pending": match (&rp, &rd) {
                    (Some(p), Some(d)) => read_norm(p, &u) != read_norm(d, &u)
                        || edit::find_ci(&pending.expect("read").xml.root, &u).is_some()
                            != edit::find_ci(&deployed.expect("read").xml.root, &u).is_some(),
                    (Some(_), None) => true,
                    (None, _) => false,
                },
            })
        })
        .collect()
}

/// `GET /api/edits`' `rule_edit`: per rule of `doc`, null when vislox may
/// declare it, else why not; and whether a rule can be created here
/// (`create`: null, or why not).
pub fn edit_section(files: &Files, ctx: &Ctx) -> Value {
    let read = rules::read(ctx.doc);
    let rules: BTreeMap<String, Option<String>> = read
        .keys()
        .map(|u| (u.clone(), refusal(files, ctx, &read, u)))
        .collect();
    json!({
        "rules": rules,
        "create": (!has_rules(ctx.base)).then_some(
            "The config has no Automatic Designer rule yet: create one in the app once (the app \
             decides where rules live), then add rules here."
        ),
        "date": timezone(ctx.base).is_none().then_some(
            "`when date` needs the Miniserver's time zone in the config; Lox Logic converts \
             dates in the Central European time zones only."
        ),
        "offers": offers(ctx.doc, &read),
    })
}

/// What the browser offers for a block's conditions and actions: per block
/// type, the states and commands the config's rules already use on blocks
/// of that type. lox checks every state and command against the app's
/// capability table when it compiles (and refuses with the list a block
/// offers); vislox cannot read that table (it is private to lox), so these
/// are suggestions, never a limit.
pub fn offers(doc: &LoxoneDoc, read: &BTreeMap<String, Read>) -> Value {
    let types: BTreeMap<String, String> = doc
        .objects()
        .into_iter()
        .map(|o| (o.uuid.to_ascii_lowercase(), o.block_type))
        .collect();
    let mut out: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for r in read.values() {
        let Read::Rule(v) = r else { continue };
        for l in &v.lines {
            match l {
                Line::State { block, state, .. } => {
                    if let Some(t) = types.get(&block.to_ascii_lowercase()) {
                        out.entry(t.clone()).or_default().0.insert(state.clone());
                    }
                }
                Line::Then { block, command } => {
                    if let Some(t) = types.get(&block.to_ascii_lowercase()) {
                        out.entry(t.clone()).or_default().1.insert(command.clone());
                    }
                }
                _ => {}
            }
        }
    }
    out.into_iter()
        .map(|(t, (s, c))| (t, json!({ "states": s, "commands": c })))
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

pub struct RuleKind;

impl set_claim::ClaimKind for RuleKind {
    fn keyword(&self) -> &'static str {
        "rule"
    }
    fn what(&self) -> &'static str {
        "rule"
    }
    fn makes_sets(&self) -> bool {
        false
    }
    fn op<'a>(&self, op: &'a Op) -> Option<(&'a str, bool)> {
        match op {
            Op::Rule { uuid, value } => Some((uuid, value.is_none())),
            _ => None,
        }
    }
    fn restored(&self, _: &Op) -> Vec<String> {
        Vec::new()
    }
    fn is_empty(&self, c: &edit::Claims) -> bool {
        c.9.is_empty()
    }
    fn render(&self, c: &edit::Claims) -> String {
        render(&c.9)
    }
    fn on(&self, c: &edit::Claims, slug: &str) -> Option<(String, &'static str)> {
        c.9.claims.iter().find_map(|(r, decl)| {
            if r == slug {
                Some((format!("what the rule does (rule {r})"), "it"))
            } else if decl.body.iter().any(|k| target(k) == Some(slug)) {
                Some((format!("it in a rule (rule {r})"), "it"))
            } else {
                None
            }
        })
    }
    fn mentions(&self, c: &edit::Claims) -> Vec<String> {
        let mut out = Vec::new();
        for (r, decl) in &c.9.claims {
            if decl.title.is_none() {
                out.push(r.clone());
            }
            out.extend(decl.body.iter().filter_map(target).map(String::from));
        }
        out
    }
    fn set_names<'a>(&self, c: &'a edit::Claims) -> Vec<&'a str> {
        c.9.claims
            .iter()
            .filter(|(_, d)| d.title.is_some())
            .map(|(s, _)| s.as_str())
            .chain(c.9.removed.iter().map(String::as_str))
            .collect()
    }
    fn read(&self, c: &mut edit::Claims, item: &Item) -> Option<Result<(), String>> {
        match item {
            Item::Rule(d) if d.slug.starts_with(crate::blocks::SLUG_PREFIX) => Some({
                let body: Vec<RuleStmtKind> = d.stmts().map(|s| s.kind.clone()).collect();
                if c.9.claims.contains_key(&d.slug) {
                    Err(format!("a second `rule {}`", d.slug))
                } else {
                    c.9.claims.insert(
                        d.slug.clone(),
                        RuleClaim {
                            title: d.title.clone(),
                            body,
                        },
                    );
                    Ok(())
                }
            }),
            Item::Removed(r) if r.slug.starts_with(NEW_PREFIX) => {
                c.9.removed.insert(r.slug.clone());
                Some(Ok(()))
            }
            _ => None,
        }
    }
    fn check_read(&self, c: &edit::Claims) -> Result<(), String> {
        for (slug, d) in &c.9.claims {
            match &d.title {
                Some(_) if !slug.starts_with(NEW_PREFIX) => {
                    return Err(format!("`rule {slug}` by its name"));
                }
                None if edit::slug_uuid(slug).is_none() => {
                    return Err(format!("`rule {slug}`"));
                }
                _ => {}
            }
            for k in &d.body {
                if let Some(t) = target(k)
                    && !t.starts_with(crate::blocks::SLUG_PREFIX)
                {
                    return Err(format!("`rule {slug}` naming `{t}`"));
                }
            }
        }
        for s in &c.9.removed {
            if c.9.claims.contains_key(s) {
                return Err(format!("`rule {s}` and `removed {s}`"));
            }
        }
        Ok(())
    }
    fn names(&self, c: &edit::Claims) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (r, d) in &c.9.claims {
            if d.title.is_none() {
                out.push((format!("rule {r}"), r.clone()));
            }
            for k in &d.body {
                if let Some(t) = target(k) {
                    out.push((format!("rule {r}"), t.to_string()));
                }
            }
        }
        out
    }
    fn missed(
        &self,
        files: &Files,
        out: &LoxoneDoc,
        lock: &Lock,
        uuids: &[String],
        _: &[String],
    ) -> Vec<String> {
        missed(files, out, lock, uuids)
    }
    fn edits_section(
        &self,
        now: &Files,
        was: &Files,
        lock: &Lock,
        pending: Option<&LoxoneDoc>,
        deployed: Option<&LoxoneDoc>,
    ) -> Vec<(&'static str, Value)> {
        vec![("rules", edits_section(now, was, lock, pending, deployed))]
    }
    fn keys(&self) -> Value {
        Value::Null
    }
}

/// The block a line watches or drives, by its slug.
fn target(k: &RuleStmtKind) -> Option<&str> {
    match k {
        RuleStmtKind::State { target, .. } | RuleStmtKind::Then { target, .. } => Some(target),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::fixture::*;

    #[test]
    fn a_rule_that_feeds_a_block_is_named_by_what_it_drives() {
        let plain = LoxoneDoc::parse(house().as_bytes()).unwrap();
        assert_eq!(drives(&plain, RULE_ON), None);
        let wired = house().replace(
            "<Co K=\"Reset\" U=\"40000003-0000-0032-00ffaaaaaaaa0003\"/>",
            "<Co K=\"Reset\" U=\"40000003-0000-0032-00ffaaaaaaaa0003\"><In Input=\"40000002-0000-0021-00FFAAAAAAAA0002\"/></Co>",
        );
        let wired = LoxoneDoc::parse(wired.as_bytes()).unwrap();
        assert_eq!(
            drives(&wired, &RULE_ON.to_uppercase()).as_deref(),
            Some("Abdeckung")
        );
    }

    #[test]
    fn weekdays_take_their_shortest_words_and_values_compare_as_numbers() {
        let d = |v: &[&str]| canonical_days(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(d(&["fri", "mon", "tue", "wed", "thu"]), ["weekdays"]);
        assert_eq!(d(&["weekend", "weekdays"]), ["daily"]);
        assert_eq!(d(&["sun", "mon"]), ["mon", "sun"]);
        assert_eq!(norm_value("on"), "1");
        assert_eq!(norm_value("1.0"), "1");
        assert_eq!(norm_value("off"), norm_value("0"));
    }
}
