//! The module's own `let` and `defaults` lines, changed from vislox (#81),
//! and the `defaults` statements vislox makes.
//!
//! vislox writes its own files under `<module>/vislox/`. These two are the
//! exception the product decision on #81 makes: a `let` constant or a
//! `defaults` value of the module's own files is changed where it is
//! declared, by a surgical rewrite of that one value on its line. The rest
//! of the line and of the file stays byte for byte as it was (comments,
//! spacing, the other arguments). The op names the statement's text as the
//! browser saw it (`source`): when the file no longer has exactly that, it is
//! refused, so a hand edit is never overwritten. Its `previous` is the op
//! that writes the old text back, so undo restores it exactly.
//!
//! A `defaults` statement vislox makes goes into vislox's file of claims
//! ([`crate::edit::FILE`]): lox reads a `defaults` statement from any file of
//! the module, at the top level (docs/language.md, *One statement per type*).
//! A default changes every managed block of the type that does not bind the
//! port itself, the module's own blocks among them, so the op names exactly
//! those blocks (`blocks`), as Apply's consent to removals does (#70).

use crate::constants::Reached;
use crate::edit::{Ctx, Files};
use crate::managed::{code, template_bodies};
use loxone_config::logic::{ArgItem, BindingKind, Item, Module, Value};
use loxone_config::port_names;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// The module's own `.loxl` files, outside vislox's directory, as the
/// request began: by their path from the project root (as `GET /api/edits`'
/// `constants` names them), with vislox's directory shown the same way.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ModuleSrc {
    /// vislox's directory, from the project root (`module/vislox`).
    pub dir: String,
    pub texts: BTreeMap<String, String>,
}

/// No module files: a [`Ctx`] of a request that changes none.
#[cfg(test)]
pub static NO_MODULE: ModuleSrc = ModuleSrc {
    dir: String::new(),
    texts: BTreeMap::new(),
};

/// The module as the request has changed it so far: its own files (with
/// the rewrites of earlier ops) and vislox's, by path from the project root.
pub fn module_now(files: &Files, ctx: &Ctx) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = ctx
        .module
        .texts
        .iter()
        .map(|(p, t)| (p.clone(), files.module.get(p).unwrap_or(t).clone()))
        .collect();
    for (name, t) in files.render() {
        if name.ends_with(".loxl") {
            out.push((format!("{}/{name}", ctx.module.dir), t));
        }
    }
    out
}

/// The text of the module's own file `path` as the request has it now.
fn text_now<'a>(files: &'a Files, ctx: &'a Ctx, path: &str) -> &'a str {
    files
        .module
        .get(path)
        .or_else(|| ctx.module.texts.get(path))
        .map(String::as_str)
        .unwrap_or_default()
}

/// Each of the module's own files parsed, as the request has it now; lox's
/// error, naming the file, when one does not read.
fn parsed(files: &Files, ctx: &Ctx) -> Result<Vec<(String, Module)>, String> {
    ctx.module
        .texts
        .keys()
        .map(|p| {
            Module::parse_fragment(text_now(files, ctx, p))
                .map(|m| (p.clone(), m))
                .map_err(|e| format!("{p}: {e}"))
        })
        .collect()
}

/// The line (0-based) of `let <name> = …` at the top level of `lines`.
pub fn let_line(lines: &[&str], name: &str) -> Option<usize> {
    let body = template_bodies(lines);
    let found: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(n, l)| {
            !body.contains(n)
                && code(l)
                    .trim_start()
                    .strip_prefix("let")
                    .filter(|r| r.starts_with(char::is_whitespace))
                    .and_then(|r| r.trim_start().strip_prefix(name))
                    .is_some_and(|r| r.trim_start().starts_with('='))
        })
        .map(|(n, _)| n)
        .collect();
    match found.as_slice() {
        [n] => Some(*n),
        _ => None,
    }
}

/// The line (0-based) of `defaults <ty>` at the top level of `lines`.
pub fn defaults_line(lines: &[&str], ty: &str) -> Option<usize> {
    let body = template_bodies(lines);
    lines.iter().enumerate().find_map(|(n, l)| {
        let rest = code(l).trim_start().strip_prefix("defaults")?;
        if !rest.starts_with(char::is_whitespace) || body.contains(&n) {
            return None;
        }
        let name = rest
            .trim_start()
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()?;
        (name == ty).then_some(n)
    })
}

/// The last line (0-based) of the call that opens on line `start`: the one
/// with its closing `)`, or `start` when it has no parentheses.
pub fn call_end(lines: &[&str], start: usize) -> usize {
    let mut depth = 0usize;
    let mut opened = false;
    for (n, line) in lines.iter().enumerate().skip(start) {
        let mut quoted = false;
        let mut escaped = false;
        for c in code(line).chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                '(' if !quoted => {
                    depth += 1;
                    opened = true;
                }
                ')' if !quoted => {
                    depth = depth.saturating_sub(1);
                    if opened && depth == 0 {
                        return n;
                    }
                }
                _ => {}
            }
        }
        if !opened {
            return start;
        }
    }
    start
}

/// The statement's text, lines `start..=end` (0-based) as the file has them:
/// what the browser names in `source`, and what is compared with it.
pub fn source_of(lines: &[&str], start: usize, end: usize) -> String {
    lines[start..=end.min(lines.len() - 1)]
        .iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where the value of `let <name> = <value>` is on `line`: from the first
/// character after `=` and its spaces to the last before the comment.
fn let_value_span(line: &str) -> Option<Range<usize>> {
    let c = code(line);
    let eq = c.find('=')?;
    value_span(c, eq + 1)
}

/// The span of a value that starts at or after `from` in `c` (code, no
/// comment), up to the first `,` or `)` at its own depth outside a string,
/// or the end; spaces on either side left out.
fn value_span(c: &str, from: usize) -> Option<Range<usize>> {
    let start = from + (c.len() - from - c[from..].trim_start().len());
    let mut end = c.len();
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for (i, ch) in c[start..].char_indices() {
        match ch {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted && depth > 0 => depth -= 1,
            ',' | ')' if !quoted => {
                end = start + i;
                break;
            }
            _ => {}
        }
    }
    let end = start + c[start..end].trim_end().len();
    (end > start).then_some(start..end)
}

/// Where the value of argument `port` (`Port: value`) is on `line`.
fn arg_value_span(line: &str, port: &str) -> Option<Range<usize>> {
    let c = code(line);
    let bytes = c.as_bytes();
    let mut quoted = false;
    let mut escaped = false;
    for (i, ch) in c.char_indices() {
        match ch {
            _ if escaped => {
                escaped = false;
                continue;
            }
            '\\' if quoted => {
                escaped = true;
                continue;
            }
            '"' => {
                quoted = !quoted;
                continue;
            }
            _ if quoted => continue,
            _ => {}
        }
        if !c[i..].starts_with(port) {
            continue;
        }
        // A whole name, at an argument's start: after `(`, `,` or the line's
        // indentation.
        let before = c[..i].trim_end();
        let starts = before.is_empty() || before.ends_with('(') || before.ends_with(',');
        let after = i + port.len();
        let whole = bytes
            .get(after)
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
        let rest = c[after..].trim_start();
        if starts && whole && rest.starts_with(':') {
            let colon = after + (c[after..].len() - rest.len());
            return value_span(c, colon + 1);
        }
    }
    None
}

/// The value as the browser sent it, checked and trimmed: what is written.
fn written(v: &str) -> Result<String, String> {
    let t = v.trim();
    if t.is_empty() || t.contains(['\n', '\r', '#']) {
        return Err(format!("`{t}`: one value, on one line"));
    }
    Ok(t.to_string())
}

/// Replace `span` of line `n` (0-based) of `text` with `with`; every other
/// byte stays.
fn splice(text: &str, n: usize, span: Range<usize>, with: &str) -> String {
    let mut at = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i == n {
            let s = at + span.start;
            let e = at + span.end;
            return format!("{}{with}{}", &text[..s], &text[e..]);
        }
        at += l.len() + 1;
    }
    text.to_string()
}

/// The source as the browser saw it, compared with the file's.
fn same_source(sent: Option<&str>, now: &str, what: &str, at: &str) -> Result<(), String> {
    let Some(sent) = sent else {
        return Err(format!(
            "{what} is in {at}, the module's own file: send the statement as this page showed \
             it (`source`)"
        ));
    };
    let norm = |s: &str| s.replace("\r\n", "\n");
    if norm(sent) != norm(now) {
        return Err(format!(
            "{what} in {at} changed since this page showed it; look at it again, then change it"
        ));
    }
    Ok(())
}

/// The `let` op on a constant the module's own files declare: its value
/// rewritten on its line. Returns the op that writes the old text back.
pub fn set_let(
    files: &mut Files,
    ctx: &Ctx,
    name: &str,
    value: Option<&str>,
    source: Option<&str>,
) -> Result<serde_json::Value, String> {
    let Some(value) = value else {
        return Err(format!(
            "`{name}` is a constant of the module's own files: vislox changes its value, it \
             never drops it; take it out in its file"
        ));
    };
    let mods = parsed(files, ctx)?;
    let found: Vec<(&String, usize, &loxone_config::logic::LetDecl)> = mods
        .iter()
        .flat_map(|(p, m)| {
            m.items
                .iter()
                .enumerate()
                .filter_map(move |(i, it)| match it {
                    Item::Let(l) if l.name == name && !matches!(l.value, Value::Port(_)) => {
                        Some((p, i, l))
                    }
                    _ => None,
                })
        })
        .collect();
    let [(path, index, decl)] = found.as_slice() else {
        return Err(format!(
            "`{name}`: the module's own files declare it {} times at their top level; vislox \
             changes one",
            found.len()
        ));
    };
    if matches!(decl.value, Value::Str(_)) {
        return Err(format!(
            "`{name}` is a text constant: change it in its file ({path})"
        ));
    }
    let text = text_now(files, ctx, path).to_string();
    let lines: Vec<&str> = text.split('\n').collect();
    let n = let_line(&lines, name)
        .ok_or_else(|| format!("`{name}`: vislox cannot find its line in {path}"))?;
    let at = format!("{path}:{}", n + 1);
    same_source(
        source,
        &source_of(&lines, n, n),
        &format!("`let {name}`"),
        &at,
    )?;
    let new = written(value)?;
    crate::edit::constant_value(&new)?;
    let span = let_value_span(lines[n]).ok_or_else(|| format!("`{name}`: no value on {at}"))?;
    let old = lines[n][span.clone()].to_string();
    let out = splice(&text, n, span, &new);
    // The rewrite is that one value: the file reads as before but for it,
    // and it reads as the value does alone.
    let want = Module::parse_fragment(&format!("let {name} = {new}\n"))
        .ok()
        .and_then(|m| match m.items.into_iter().next() {
            Some(Item::Let(l)) => Some(l.value),
            _ => None,
        })
        .ok_or_else(|| format!("`{new}` is no value lox reads"))?;
    let m = Module::parse_fragment(&out).map_err(|e| format!("{at}: {e}"))?;
    let before = &mods.iter().find(|(p, _)| p == *path).expect("found").1;
    let ok = m.items.len() == before.items.len()
        && m.items.iter().zip(&before.items).enumerate().all(|(i, (a, b))| {
            if i != *index {
                return a == b;
            }
            matches!(a, Item::Let(l) if l.name == name && l.value == want && l.comment == decl.comment)
        });
    if !ok {
        return Err(format!(
            "`{name}`: writing {new} on {at} would change more than its value; change it in \
             the file"
        ));
    }
    let after: Vec<&str> = out.split('\n').collect();
    let previous = serde_json::json!({
        "op": "let",
        "name": name,
        "value": old,
        "source": source_of(&after, n, n),
    });
    // The same text written again changes nothing.
    if out != text {
        files.module.insert((*path).clone(), out);
    }
    Ok(previous)
}

/// The module's own `defaults <ty>` statement: its file and index among the
/// file's items.
fn module_defaults<'a>(
    mods: &'a [(String, Module)],
    ty: &str,
) -> Option<(&'a String, usize, &'a loxone_config::logic::DefaultsDecl)> {
    mods.iter().find_map(|(p, m)| {
        m.items.iter().enumerate().find_map(|(i, it)| match it {
            Item::Defaults(d) if d.block_type == ty => Some((p, i, d)),
            _ => None,
        })
    })
}

/// A default's value as the browser sent it, checked: a constant by its
/// name (vislox's or the module's), else a value the port takes.
fn default_value(ctx: &Ctx, files: &Files, ty: &str, key: &str, v: &str) -> Result<(), String> {
    let t = v.trim();
    if crate::edit::bare_name(t) {
        if files.claims.8.contains_key(t) || ctx.lets.contains(t) {
            return Ok(());
        }
        return Err(format!("`{t}` is no constant of the module"));
    }
    if loxone_config::connectors::builtin(ty)
        .is_some_and(|specs| specs.iter().any(|s| s.key == key))
    {
        crate::units::port_value(ty, key, t).map(|_| ())
    } else {
        crate::edit::constant_value(t).map(|_| ())
    }
}

/// The `defaults` op: the default of port `key` (a connector key) for every
/// managed block of `ty` that does not bind it. On the module's own
/// statement it rewrites that value; else vislox's statement in its file of
/// claims takes it (made when there is none), and `None` drops it. `blocks`
/// must name exactly the blocks it changes. Returns the op that undoes it.
pub fn set_defaults(
    files: &mut Files,
    ctx: &Ctx,
    ty: &str,
    key: &str,
    value: Option<&str>,
    source: Option<&str>,
    blocks: Option<&[String]>,
) -> Result<serde_json::Value, String> {
    if !crate::managed::is_name(ty) {
        return Err(format!("`{ty}` is no block type"));
    }
    let port = port_names::name_of(ty, key).to_string();
    if !crate::managed::is_name(&port) {
        return Err(format!("`{key}` is no port of {ty}"));
    }
    if let Some(v) = value {
        written(v)?;
        default_value(ctx, files, ty, key, v)?;
    }
    let mods = parsed(files, ctx)?;
    // Every check first: the blocks it changes, as the module is now.
    let reach =
        crate::constants::reach(&module_now(files, ctx), ctx.lock, ty, &port).map_err(|e| {
            format!("the module does not read, so the blocks a default changes are not known: {e}")
        })?;
    consent(ty, &port, &reach, blocks)?;
    let slugs: Vec<String> = reach.iter().map(|r| r.slug.clone()).collect();
    let undo = |value: Option<String>, source: Option<String>| {
        let mut op = serde_json::json!({
            "op": "defaults", "type": ty, "port": key, "value": value, "blocks": slugs,
        });
        if let Some(s) = source {
            op["source"] = serde_json::json!(s);
        }
        op
    };
    if let Some((path, index, decl)) = module_defaults(&mods, ty) {
        let Some(value) = value else {
            return Err(format!(
                "`defaults {ty}` is in the module's own file ({path}): vislox changes one of its \
                 values, it never drops one; take it out there"
            ));
        };
        if !decl.params().any(|(p, _)| p == port) {
            return Err(format!(
                "`defaults {ty}` in {path} has no {port}: vislox changes a value it has, it never \
                 adds one; add it there"
            ));
        }
        let text = text_now(files, ctx, path).to_string();
        let lines: Vec<&str> = text.split('\n').collect();
        let start = defaults_line(&lines, ty)
            .ok_or_else(|| format!("`defaults {ty}`: vislox cannot find it in {path}"))?;
        let end = call_end(&lines, start);
        let at = format!("{path}:{}", start + 1);
        same_source(
            source,
            &source_of(&lines, start, end),
            &format!("`defaults {ty}`"),
            &at,
        )?;
        let new = written(value)?;
        let (n, span) = (start..=end)
            .find_map(|n| arg_value_span(lines[n], &port).map(|s| (n, s)))
            .ok_or_else(|| format!("`defaults {ty}`: vislox cannot find {port} on {at}"))?;
        let old = lines[n][span.clone()].to_string();
        let out = splice(&text, n, span, &new);
        let want = Module::parse_fragment(&format!("defaults {ty}({port}: {new})\n"))
            .ok()
            .and_then(|m| match m.items.into_iter().next() {
                Some(Item::Defaults(d)) => d.params().next().map(|(_, v)| v.clone()),
                _ => None,
            })
            .ok_or_else(|| format!("`{new}` is no value lox reads in a default"))?;
        let m = Module::parse_fragment(&out).map_err(|e| format!("{at}: {e}"))?;
        let before = &mods.iter().find(|(p, _)| p == path).expect("found").1;
        let same_but = |a: &Item| -> bool {
            let Item::Defaults(d) = a else { return false };
            d.block_type == decl.block_type
                && d.comment == decl.comment
                && d.close_comment == decl.close_comment
                && d.args.len() == decl.args.len()
                && d.args.iter().zip(&decl.args).all(|(x, y)| match (x, y) {
                    (ArgItem::Binding(x), ArgItem::Binding(y)) if x.port == port => {
                        x.comment == y.comment && x.kind == BindingKind::Param(want.clone())
                    }
                    _ => x == y,
                })
        };
        let ok = m.items.len() == before.items.len()
            && m.items
                .iter()
                .zip(&before.items)
                .enumerate()
                .all(|(i, (a, b))| if i == index { same_but(a) } else { a == b });
        if !ok {
            return Err(format!(
                "`defaults {ty}`: writing {new} on {at} would change more than its {port}; change \
                 it in the file"
            ));
        }
        let after: Vec<&str> = out.split('\n').collect();
        let previous = undo(
            Some(old),
            Some(source_of(&after, start, call_end(&after, start))),
        );
        if out != text {
            files.module.insert(path.clone(), out);
        }
        return Ok(previous);
    }
    // vislox's own statement, in its file of claims.
    let ours = files.claims.10.entry(ty.to_string()).or_default();
    let previous = ours.get(&port).map(|v| v.to_string());
    match value {
        Some(v) => {
            let t = v.trim();
            let parsed = Module::parse_fragment(&format!("defaults {ty}({port}: {t})\n"))
                .ok()
                .and_then(|m| match m.items.into_iter().next() {
                    Some(Item::Defaults(d)) => d.params().next().map(|(_, v)| v.clone()),
                    _ => None,
                })
                .ok_or_else(|| format!("`{t}` is no value lox reads in a default"))?;
            ours.insert(port.clone(), parsed);
        }
        None if previous.is_none() => {
            files.claims.10.retain(|_, m| !m.is_empty());
            return Err(format!("vislox's files have no default for {ty} {port}"));
        }
        None => {
            ours.remove(&port);
        }
    }
    files.claims.10.retain(|_, m| !m.is_empty());
    Ok(undo(previous, None))
}

/// The blocks a default changes must be exactly the ones the browser showed
/// and the user confirmed.
fn consent(
    ty: &str,
    port: &str,
    reach: &[Reached],
    blocks: Option<&[String]>,
) -> Result<(), String> {
    let want: BTreeSet<&str> = reach.iter().map(|r| r.slug.as_str()).collect();
    let got: Option<BTreeSet<&str>> = blocks.map(|b| b.iter().map(String::as_str).collect());
    if got.as_ref() == Some(&want) {
        return Ok(());
    }
    let list = if want.is_empty() {
        "none".to_string()
    } else {
        want.iter().copied().collect::<Vec<_>>().join(", ")
    };
    Err(format!(
        "the default of {ty} {port} changes {} block{}: {list}. Confirm exactly these (`blocks`)",
        want.len(),
        if want.len() == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_found_between_its_neighbours() {
        let l = "\tTime: 30min, # Nachlauf";
        assert_eq!(&l[arg_value_span(l, "Time").unwrap()], "30min");
        let l = "defaults Div(In1: 2, In2: 4)";
        assert_eq!(&l[arg_value_span(l, "In2").unwrap()], "4");
        assert_eq!(&l[arg_value_span(l, "In1").unwrap()], "2");
        // Not inside another name, nor a string.
        let l = "defaults X(Label: \"In2: 9\", MyIn2: 3, In2:7 )";
        assert_eq!(&l[arg_value_span(l, "In2").unwrap()], "7");
        let l = "let nachlauf  =  30min   # bis aus";
        assert_eq!(&l[let_value_span(l).unwrap()], "30min");
    }

    #[test]
    fn a_splice_keeps_every_other_byte() {
        let t = "a\r\nlet x = 1 # eins\r\nb";
        let lines: Vec<&str> = t.split('\n').collect();
        let s = let_value_span(lines[1]).unwrap();
        assert_eq!(splice(t, 1, s, "22"), "a\r\nlet x = 22 # eins\r\nb");
    }

    #[test]
    fn a_statement_s_lines_are_found() {
        let t = "page \"P\"\ndefaults Monoflop(\n\tTime: 1s, # x\n)\nlet a = 1\ntemplate t()\n\tlet a = 2\nend\n";
        let lines: Vec<&str> = t.split('\n').collect();
        assert_eq!(defaults_line(&lines, "Monoflop"), Some(1));
        assert_eq!(call_end(&lines, 1), 3);
        assert_eq!(let_line(&lines, "a"), Some(4));
        assert_eq!(
            source_of(&lines, 1, 3),
            "defaults Monoflop(\n\tTime: 1s, # x\n)"
        );
    }
}
