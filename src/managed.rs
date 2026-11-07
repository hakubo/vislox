//! The blocks the project's Lox Logic module manages, and where its source
//! declares each one (#64).
//!
//! A block is managed when the lockfile pins it (`objects`: slug → UUID).
//! Externs (`externals`) are not: Loxone Config owns them. Where each slug
//! comes from is read off the module's `.loxl` files with lox's own
//! declaration scan (the one its language server uses for symbols), at the
//! top level of each file only:
//!
//! - a block, a named signal or a template instance declared by name is at
//!   its declaration's line;
//! - a block lox made for an expression (`sink_port__and1`, see lox's
//!   `desugar.rs`) is at the line of the expression: the `<-` statement, the
//!   argument of the block it feeds, or the named signal (`signal__or1`);
//! - a block a template instance made (`instance_block`) is at the
//!   instance's line, never inside the template's body (which declares the
//!   block once for every instance). Pointing into the body is #83's.
//!
//! A name declared twice at the top level (a module lox would refuse) is
//! `ambiguous`, with no line, rather than a guess. A slug none of these
//! finds (a module that does not parse as lox reads it, a lock from another
//! module) keeps its slug, with no file.

use loxone_config::lsp::analysis::scan;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Where one managed block comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub slug: String,
    /// The `.loxl` file, relative to the project (`pages/licht.loxl`), or to
    /// the module when it lies outside the project; never absolute.
    pub file: Option<String>,
    /// 1-based.
    pub line: Option<usize>,
    /// `declared`, `expression`, `template` or `ambiguous`; `None` when not
    /// found.
    pub how: Option<&'static str>,
}

/// The module's `.loxl` files as last read: each one's path as shown, blob
/// id and text. Kept between refreshes, so a file is read and hashed again
/// only when its size or modification time changed, or while that time is
/// too recent to tell a second write in the same tick from the first (as
/// git's racy-clean check does).
#[derive(Default)]
pub struct ModuleFiles {
    files: BTreeMap<PathBuf, Entry>,
    /// The lockfile, by the same rule: its path, stamp and blob id.
    lock: Option<(PathBuf, (SystemTime, u64), String)>,
}

/// A file's modification time and size, and whether that time is too
/// recent to trust ([`RACY`]).
fn stamp_of(f: &Path, now: SystemTime) -> Option<((SystemTime, u64), bool)> {
    let meta = std::fs::metadata(f).ok()?;
    let stamp = (
        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        meta.len(),
    );
    let racy = now.duration_since(stamp.0).map_or(true, |d| d < RACY);
    Some((stamp, racy))
}

struct Entry {
    stamp: (SystemTime, u64),
    rel: String,
    blob: String,
    text: Arc<str>,
}

/// A modification time this close to now may still be shared by a later
/// write of the same size.
const RACY: Duration = Duration::from_secs(3);

impl ModuleFiles {
    /// Look at the module's files again: `module` is a file or a directory
    /// of them. Returns each file's shown path and blob id, in order.
    pub fn refresh(&mut self, root: &Path, module: &Path) -> Vec<(String, String)> {
        let mut paths = Vec::new();
        if module.is_dir() {
            crate::edit::module_files(module, &mut |f, _| {
                if f.extension().is_some_and(|x| x == "loxl") {
                    paths.push(f.to_path_buf());
                }
            });
        } else {
            paths.push(module.to_path_buf());
        }
        paths.sort();
        let now = SystemTime::now();
        let mut kept = BTreeMap::new();
        for f in paths {
            let Some((stamp, racy)) = stamp_of(&f, now) else {
                continue;
            };
            if let Some(e) = self.files.remove(&f)
                && e.stamp == stamp
                && !racy
            {
                kept.insert(f, e);
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let e = Entry {
                stamp,
                rel: shown(root, module, &f),
                blob: crate::history::blob_id(text.as_bytes()),
                text: text.into(),
            };
            kept.insert(f, e);
        }
        self.files = kept;
        self.files
            .values()
            .map(|e| (e.rel.clone(), e.blob.clone()))
            .collect()
    }

    /// The lockfile's blob id, read and hashed again only when it changed
    /// (by the same rule as the module's files); `None` without one.
    pub fn lock_blob(&mut self, path: &Path) -> Option<String> {
        let now = SystemTime::now();
        let Some((stamp, racy)) = stamp_of(path, now) else {
            self.lock = None;
            return None;
        };
        if let Some((p, s, blob)) = &self.lock
            && p == path
            && *s == stamp
            && !racy
        {
            return Some(blob.clone());
        }
        let blob = crate::history::blob_id(&std::fs::read(path).ok()?);
        self.lock = Some((path.to_path_buf(), stamp, blob.clone()));
        Some(blob)
    }

    /// The module's files and the lockfile as one blob id: the files part
    /// of the edit and check keys. `None` when there is neither.
    pub fn key(&mut self, root: &Path, module: &Path, lock: &Path) -> Option<String> {
        let mut all: Vec<String> = self
            .refresh(root, module)
            .into_iter()
            .map(|(n, b)| format!("{n}={b}"))
            .collect();
        if let Some(b) = self.lock_blob(lock) {
            all.push(format!("lock={b}"));
        }
        (!all.is_empty()).then(|| crate::history::blob_id(all.join("\n").as_bytes()))
    }

    /// Each file's shown path and text, as last refreshed.
    pub fn texts(&self) -> Vec<(String, Arc<str>)> {
        self.files
            .values()
            .map(|e| (e.rel.clone(), e.text.clone()))
            .collect()
    }
}

/// `file` as the browser shows it: relative to the project root, else to
/// the module's directory (a module linked in from elsewhere), compared as
/// canonical paths; else its name alone. Never absolute: it is shown, and
/// pasted into prompts.
pub fn shown(root: &Path, module: &Path, file: &Path) -> String {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let f = canon(file);
    let module = canon(module);
    let module_dir = if module.is_dir() {
        module.as_path()
    } else {
        module.parent().unwrap_or(&module)
    };
    // As written first (a module linked into the project keeps the link's
    // path), then as the file system resolves it.
    let rel = file
        .strip_prefix(root)
        .or_else(|_| f.strip_prefix(canon(root)))
        .or_else(|_| f.strip_prefix(module_dir))
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| f.file_name().map(PathBuf::from).unwrap_or_default());
    rel.to_string_lossy().replace('\\', "/")
}

/// Every slug of `slugs` (slug → UUID) located in `files`: UUID → source.
pub fn sources<T: AsRef<str>>(
    files: &[(String, T)],
    slugs: &BTreeMap<String, String>,
) -> BTreeMap<String, Source> {
    let ix = Index::new(files);
    slugs
        .iter()
        .map(|(slug, uuid)| {
            let (file, line, how) = match ix.locate(slug) {
                Found::At(f, l, how) => (Some(f.to_string()), Some(l), Some(how)),
                Found::Ambiguous => (None, None, Some("ambiguous")),
                Found::No => (None, None, None),
            };
            (
                uuid.clone(),
                Source {
                    slug: slug.clone(),
                    file,
                    line,
                    how,
                },
            )
        })
        .collect()
}

/// Where the module declares each top-level name (any kind: a block, an
/// extern, a `let`, a template, …), for what lox names by it (#66).
pub struct Declarations<'a>(Index<'a>);

impl<'a> Declarations<'a> {
    pub fn new<T: AsRef<str>>(files: &'a [(String, T)]) -> Self {
        Declarations(Index::new(files))
    }

    /// `name`'s one top-level declaration (in `file`, when given): its file
    /// and 1-based line. `None` when there is none, or more than one.
    pub fn at(&self, name: &str, file: Option<&str>) -> Option<(String, usize)> {
        let d = self.0.decls.get(name)?;
        let at: Vec<&At> =
            d.at.iter()
                .filter(|(f, _)| file.is_none_or(|x| x == *f))
                .collect();
        match at.as_slice() {
            [(f, l)] => Some((f.to_string(), *l)),
            _ => None,
        }
    }
}

type At<'a> = (&'a str, usize);

enum Found<'a> {
    At(&'a str, usize, &'static str),
    Ambiguous,
    No,
}

/// A name's top-level declarations: one, or more (ambiguous).
#[derive(Default)]
struct Decl<'a> {
    at: Vec<At<'a>>,
    /// Its block type (externs and blocks), for the port keys lox names
    /// expression blocks after.
    ty: Option<String>,
    extern_: bool,
}

#[derive(Default)]
struct Index<'a> {
    /// Top-level names, externs among them.
    decls: BTreeMap<String, Decl<'a>>,
    /// The instances of templates: their slugs.
    instances: BTreeSet<String>,
    /// `<sink>_<port key>` of every expression: where it is written (the
    /// first one, when several feed one port), or ambiguous (`None`) when
    /// the sink is.
    exprs: BTreeMap<String, Option<At<'a>>>,
    /// Sinks declared more than once: the port key of their expressions,
    /// so their slugs, cannot be told.
    ambiguous_sinks: BTreeSet<String>,
}

impl<'a> Index<'a> {
    fn new<T: AsRef<str>>(files: &'a [(String, T)]) -> Self {
        let mut ix = Index::default();
        let mut templates = BTreeSet::new();
        // (sink, port, where) of `<-` statements and argument bindings.
        let mut sinks: Vec<(String, String, At<'a>)> = Vec::new();
        let mut calls: Vec<(String, String)> = Vec::new();
        for (file, text) in files {
            let text = text.as_ref();
            let lines: Vec<&str> = text.split('\n').collect();
            let body = template_bodies(&lines);
            for d in scan(text, Path::new(file)) {
                if body.contains(&d.span.line) {
                    continue;
                }
                let at = (file.as_str(), d.span.line + 1);
                match d.kind {
                    "template" => {
                        templates.insert(d.slug.clone());
                    }
                    "instance" => {
                        ix.instances.insert(d.slug.clone());
                    }
                    "block" => {
                        if let Some(ty) = &d.ty {
                            calls.push((d.slug.clone(), ty.clone()));
                        }
                        for (port, line) in bindings(&lines, d.span.line) {
                            sinks.push((d.slug.clone(), port, (file.as_str(), line + 1)));
                        }
                    }
                    _ => {}
                }
                let e = ix.decls.entry(d.slug).or_default();
                // One rule for the line and the type: both from the one
                // declaration; a second makes the name ambiguous.
                if e.at.is_empty() {
                    e.ty = d.ty;
                    e.extern_ = d.kind == "extern";
                }
                e.at.push(at);
            }
            for (n, line) in lines.iter().enumerate() {
                if body.contains(&n) {
                    continue;
                }
                if let Some((sink, port)) = wire_sink(line) {
                    sinks.push((sink, port, (file.as_str(), n + 1)));
                }
            }
        }
        for (slug, ty) in calls {
            if templates.contains(&ty) {
                ix.instances.insert(slug);
            }
        }
        for (sink, port, at) in sinks {
            let d = ix.decls.get(&sink);
            let ty = d.and_then(|d| d.ty.as_deref()).unwrap_or("");
            let key = loxone_config::port_names::key_or_raw(ty, &port).to_lowercase();
            if d.is_some_and(|d| d.at.len() > 1) {
                ix.ambiguous_sinks.insert(sink);
                continue;
            }
            ix.exprs.entry(format!("{sink}_{key}")).or_insert(Some(at));
        }
        ix
    }

    /// The one top-level declaration of `name` that is not an extern.
    fn decl(&self, name: &str) -> Found<'a> {
        match self.decls.get(name) {
            Some(d) if d.at.len() > 1 => Found::Ambiguous,
            Some(d) if !d.extern_ => Found::At(d.at[0].0, d.at[0].1, "declared"),
            _ => Found::No,
        }
    }

    fn locate(&self, slug: &str) -> Found<'a> {
        let own = self.decl(slug);
        if !matches!(own, Found::No) {
            return own;
        }
        // `<sink>_<port>__<op><n>`, or `<signal>__<op><n>`.
        if let Some(i) = slug.rfind("__") {
            let prefix = &slug[..i];
            match self.exprs.get(prefix) {
                Some(Some((f, l))) => return Found::At(f, *l, "expression"),
                Some(None) => return Found::Ambiguous,
                None => match self.decl(prefix) {
                    Found::At(f, l, _) => return Found::At(f, l, "expression"),
                    Found::Ambiguous => return Found::Ambiguous,
                    Found::No => {
                        if self.ambiguous_sinks.iter().any(|s| {
                            prefix
                                .strip_prefix(s.as_str())
                                .is_some_and(|r| r.starts_with('_'))
                        }) {
                            return Found::Ambiguous;
                        }
                    }
                },
            }
        }
        // `<instance>_<block>`: the longest instance it starts with.
        let inst = self
            .instances
            .iter()
            .filter(|i| {
                slug.len() > i.len()
                    && slug.starts_with(i.as_str())
                    && slug[i.len()..].starts_with('_')
            })
            .max_by_key(|i| i.len());
        match inst.map(|i| self.decl(i)) {
            Some(Found::At(f, l, _)) => Found::At(f, l, "template"),
            Some(Found::Ambiguous) => Found::Ambiguous,
            _ => Found::No,
        }
    }
}

/// The lines (0-based) inside `template … end`, the `end` among them: what
/// they declare is the template's, made once per instance.
fn template_bodies(lines: &[&str]) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut inside = false;
    for (n, line) in lines.iter().enumerate() {
        let c = code(line).trim();
        if inside {
            out.insert(n);
            if c == "end" {
                inside = false;
            }
        } else if c
            .strip_prefix("template")
            .is_some_and(|r| r.starts_with(char::is_whitespace))
        {
            inside = true;
        }
    }
    out
}

/// The text of a line before its `#` comment (not one inside a string).
fn code(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn is_name(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `sink.Port <- …`: the sink and its port.
fn wire_sink(line: &str) -> Option<(String, String)> {
    let (lhs, _) = code(line).split_once("<-")?;
    let (sink, port) = lhs.trim().split_once('.')?;
    (is_name(sink) && is_name(port)).then(|| (sink.to_string(), port.to_string()))
}

/// The keyword arguments of the call that starts on line `start`, with the
/// line each is on (0-based): `Port:` at the call's own depth.
fn bindings(lines: &[&str], start: usize) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut opened = false;
    // Whether a name here would start an argument: after `(` or `,`, or at
    // the start of a line inside the call.
    let mut at_arg = false;
    for (n, line) in lines.iter().enumerate().skip(start) {
        let text = code(line);
        let bytes = text.as_bytes();
        // A string does not span lines.
        let mut quoted = false;
        let mut escaped = false;
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if quoted {
                match c {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => quoted = false,
                    _ => {}
                }
                i += 1;
                continue;
            }
            match c {
                '"' => {
                    quoted = true;
                    at_arg = false;
                }
                '(' => {
                    depth += 1;
                    opened = true;
                    at_arg = depth == 1;
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    if opened && depth == 0 {
                        return out;
                    }
                }
                ',' => at_arg = depth == 1,
                c if c.is_whitespace() => {}
                c if (c.is_ascii_alphabetic() || c == '_') && at_arg => {
                    let s = i;
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    let rest = text[i..].trim_start();
                    if rest.starts_with(':') && !rest.starts_with("::") {
                        out.push((text[s..i].to_string(), n));
                    }
                    at_arg = false;
                    continue;
                }
                _ => at_arg = false,
            }
            i += 1;
        }
        // A new line inside the call may start an argument.
        at_arg = opened && depth == 1;
    }
    out
}

/// A claim of the module on a block's whole set of rights or switching
/// times (#68): `haustuer.access = bewohner`, `frost.recipients = familie`,
/// `uhr.schedule = abendlicht`. Whoever an `access` or `recipients` set
/// leaves out loses the right on the next push; a `schedule` replaces the
/// Daytimer's switching times, edits made in the app among them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetClaim {
    /// `access`, `recipients`, `schedule` or `moods` (#74).
    pub kind: &'static str,
    /// The block, as the module names it.
    pub slug: String,
    /// The set it claims.
    pub set: String,
    /// The claim's line (file shown as [`Source::file`]; 1-based).
    pub file: String,
    pub line: usize,
    /// Where the set is declared (`access bewohner` … `end`): its one
    /// top-level declaration of that kind, when there is one.
    pub set_file: Option<String>,
    pub set_line: Option<usize>,
    /// The block the slug names: the lockfile's managed block, the object an
    /// extern resolved to (`externals`, or its `uuid: "…"`), or the UUID
    /// vislox names an extern after. Empty when none is known (an extern lox
    /// has not resolved yet).
    pub blocks: Vec<String>,
}

/// The claim keywords on a block, and the set kinds they name.
const SET_CLAIMS: &[&str] = &["access", "recipients", "schedule", "moods"];

/// Every `<slug>.access|recipients|schedule = <set>` at the top level of the
/// module's files, outside template bodies (what a template claims is made
/// once per instance), with the block each one is on.
pub fn set_claims<T: AsRef<str>>(files: &[(String, T)], lock: &crate::edit::Lock) -> Vec<SetClaim> {
    let decls = Declarations::new(files);
    // (kind, set) → its declarations.
    let mut sets: BTreeMap<(&str, String), Vec<(String, usize)>> = BTreeMap::new();
    let mut claims = Vec::new();
    for (file, text) in files {
        let lines: Vec<&str> = text.as_ref().split('\n').collect();
        let body = template_bodies(&lines);
        for (n, line) in lines.iter().enumerate() {
            if body.contains(&n) {
                continue;
            }
            let c = code(line).trim();
            // `access bewohner`: a set's header.
            if let Some((kw, name)) = c.split_once(char::is_whitespace)
                && let Some(kind) = SET_CLAIMS.iter().find(|k| **k == kw)
                && is_name(name.trim())
            {
                sets.entry((kind, name.trim().to_string()))
                    .or_default()
                    .push((file.clone(), n + 1));
                continue;
            }
            let Some((lhs, set)) = c.split_once('=') else {
                continue;
            };
            let (set, lhs) = (set.trim(), lhs.trim());
            let Some((slug, kw)) = lhs.split_once('.') else {
                continue;
            };
            let (slug, kw) = (slug.trim(), kw.trim());
            let Some(kind) = SET_CLAIMS.iter().find(|k| **k == kw) else {
                continue;
            };
            if !is_name(slug) || !is_name(set) || set.starts_with('=') {
                continue;
            }
            claims.push(SetClaim {
                kind,
                slug: slug.to_string(),
                set: set.to_string(),
                file: file.clone(),
                line: n + 1,
                set_file: None,
                set_line: None,
                blocks: Vec::new(),
            });
        }
    }
    for c in &mut claims {
        if let Some([(f, l)]) = sets.get(&(c.kind, c.set.clone())).map(Vec::as_slice) {
            c.set_file = Some(f.clone());
            c.set_line = Some(*l);
        }
        let (f, l) = decls
            .at(&c.slug, None)
            .map_or((None, None), |(f, l)| (Some(f), Some(l)));
        let mut blocks = crate::check::name_blocks(&c.slug, f.as_deref(), l, files, lock);
        if blocks.is_empty()
            && let Some(u) = lock.externals.get(&c.slug)
        {
            blocks.push(u.to_ascii_lowercase());
        }
        c.blocks = blocks;
    }
    claims
}

/// Every `<central>.members = (a, b, …)` at the top level of the module's
/// files, outside template bodies (#75), as a [`SetClaim`] of kind `members`
/// whose `set` is the list as written (`(a, b)`) and whose `blocks` is the
/// central block it is on. A member list is no named set, so it is not in
/// [`set_claims`] (whose sets Apply's confirmation reviews).
pub fn members_claims<T: AsRef<str>>(
    files: &[(String, T)],
    lock: &crate::edit::Lock,
) -> Vec<SetClaim> {
    let decls = Declarations::new(files);
    let mut claims = Vec::new();
    for (file, text) in files {
        let lines: Vec<&str> = text.as_ref().split('\n').collect();
        let body = template_bodies(&lines);
        for (n, line) in lines.iter().enumerate() {
            if body.contains(&n) {
                continue;
            }
            let c = code(line).trim();
            let Some((lhs, list)) = c.split_once('=') else {
                continue;
            };
            let (lhs, list) = (lhs.trim(), list.trim());
            let Some((slug, kw)) = lhs.split_once('.') else {
                continue;
            };
            let slug = slug.trim();
            if kw.trim() != "members" || !is_name(slug) || !list.starts_with('(') {
                continue;
            }
            let (f, l) = decls
                .at(slug, None)
                .map_or((None, None), |(f, l)| (Some(f), Some(l)));
            let mut blocks = crate::check::name_blocks(slug, f.as_deref(), l, files, lock);
            if blocks.is_empty()
                && let Some(u) = lock.externals.get(slug)
            {
                blocks.push(u.to_ascii_lowercase());
            }
            claims.push(SetClaim {
                kind: "members",
                slug: slug.to_string(),
                set: list.to_string(),
                file: file.clone(),
                line: n + 1,
                set_file: None,
                set_line: None,
                blocks,
            });
        }
    }
    claims
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &Source) -> (Option<&str>, Option<usize>, Option<&str>) {
        (s.file.as_deref(), s.line, s.how)
    }

    #[test]
    fn each_managed_block_points_at_the_line_that_makes_it() {
        // Compiled with lox 0.83.0: its lockfile has exactly these slugs.
        let files = vec![
            (
                "pages/licht.loxl".to_string(),
                "# Licht\n\
                 page \"Obergeschoss\"\n\
                 extern temp = VirtualIn(uuid: \"10000001-0000-0010-ffff504f94000001\")\n\
                 hell = Less(\n\
                 \t\"Hell # kein Kommentar\",\n\
                 \tIn1: temp.Q,\n\
                 \tIn2: temp.Q and temp.Q,\n\
                 )\n\
                 tor = Or(In1: temp.Q or hell.Q)  # zwei\n\
                 tor.In2 <- temp.Q and hell.Q\n\
                 warm = temp.Q > 20 and hell.Q\n"
                    .to_string(),
            ),
            (
                "pages/vorlage.loxl".to_string(),
                "page \"Obergeschoss\"\n\
                 template Licht(t: Or) -> (Q: g.Q)\n\tg = Or(In1: t.Q)\nend\n\
                 bad = Licht(t: tor)\n"
                    .to_string(),
            ),
        ];
        let slugs: BTreeMap<String, String> = [
            "bad_g",
            "hell",
            "hell_input2__and1",
            "tor",
            "tor_i1__or1",
            "tor_i2__and1",
            "warm",
            "warm__gt1",
            "temp",
            "fremd",
        ]
        .iter()
        .map(|s| (s.to_string(), format!("u-{s}")))
        .collect();
        let got = sources(&files, &slugs);
        let of = |s: &str| at(&got[&format!("u-{s}")]);
        let licht = Some("pages/licht.loxl");
        assert_eq!(of("hell"), (licht, Some(4), Some("declared")));
        assert_eq!(of("tor"), (licht, Some(9), Some("declared")));
        // An argument's expression: the line of that argument, by the
        // port's key (a Less's `In2` is `Input2`, an Or's `I2`).
        assert_eq!(
            of("hell_input2__and1"),
            (licht, Some(7), Some("expression"))
        );
        assert_eq!(of("tor_i1__or1"), (licht, Some(9), Some("expression")));
        // `<-`: the statement.
        assert_eq!(of("tor_i2__and1"), (licht, Some(10), Some("expression")));
        // A named signal: its top block is declared, the rest are its expression.
        assert_eq!(of("warm"), (licht, Some(11), Some("declared")));
        assert_eq!(of("warm__gt1"), (licht, Some(11), Some("expression")));
        // A template's block: the instance, in another file than the template.
        assert_eq!(
            of("bad_g"),
            (Some("pages/vorlage.loxl"), Some(5), Some("template"))
        );
        // An extern is no declaration of a managed block; an unknown slug has none.
        assert_eq!(of("temp"), (None, None, None));
        assert_eq!(got["u-fremd"].slug, "fremd");
        assert_eq!(of("fremd"), (None, None, None));
    }

    #[test]
    fn a_wire_port_named_by_its_lox_logic_name_keys_like_lox() {
        // `In2` of an And is its `I2`, so lox names the block `x_i2__…`.
        let files = vec![(
            "m.loxl".to_string(),
            "x = And(In1: 1)\nx.In2 <- not x.Q\n".to_string(),
        )];
        let slugs = BTreeMap::from([("x_i2__not1".to_string(), "u".to_string())]);
        assert_eq!(
            at(&sources(&files, &slugs)["u"]),
            (Some("m.loxl"), Some(2), Some("expression"))
        );
    }

    fn module(root: &Path, module: &Path) -> Vec<(String, String)> {
        let mut m = ModuleFiles::default();
        m.refresh(root, module);
        m.texts()
            .into_iter()
            .map(|(f, t)| (f, t.to_string()))
            .collect()
    }

    #[test]
    fn a_module_of_one_file_or_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("pages/sub")).unwrap();
        std::fs::write(root.join("pages/b.loxl"), "b = Or()\n").unwrap();
        std::fs::write(root.join("pages/sub/a.loxl"), "a = Or()\n").unwrap();
        std::fs::write(root.join("pages/.hidden.loxl"), "h = Or()\n").unwrap();
        std::fs::write(root.join("pages/README"), "not lox logic\n").unwrap();
        let names: Vec<String> = module(root, &root.join("pages"))
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(names, ["pages/b.loxl", "pages/sub/a.loxl"]);
        let one = module(root, &root.join("pages/b.loxl"));
        assert_eq!(
            one,
            [("pages/b.loxl".to_string(), "b = Or()\n".to_string())]
        );
    }

    #[test]
    fn a_block_in_a_template_body_is_never_a_top_level_declaration() {
        // The template's file sorts first, and its body declares `g` and
        // wires `g.In2`; the module's own `g` and its expression are elsewhere.
        let files = vec![
            (
                "pages/a-vorlage.loxl".to_string(),
                "template Licht(t: Or) -> (Q: g.Q)\n\
                 \tg = Or(In1: t.Q and t.Q)\n\
                 \tg.In2 <- t.Q or t.Q\n\
                 end\n\
                 bad = Licht(t: g)\n"
                    .to_string(),
            ),
            (
                "pages/b.loxl".to_string(),
                "g = Or()\ng.In2 <- g.Q and g.Q\n".to_string(),
            ),
        ];
        let slugs: BTreeMap<String, String> = ["g", "g_i2__and1", "bad_g", "bad_g_i2__or1", "bad"]
            .iter()
            .map(|s| (s.to_string(), s.to_string()))
            .collect();
        let got = sources(&files, &slugs);
        let b = Some("pages/b.loxl");
        assert_eq!(at(&got["g"]), (b, Some(1), Some("declared")));
        assert_eq!(at(&got["g_i2__and1"]), (b, Some(2), Some("expression")));
        // The instance's blocks, its expressions among them: the instance's line.
        let v = Some("pages/a-vorlage.loxl");
        assert_eq!(at(&got["bad_g"]), (v, Some(5), Some("template")));
        assert_eq!(at(&got["bad_g_i2__or1"]), (v, Some(5), Some("template")));
        assert_eq!(at(&got["bad"]), (v, Some(5), Some("declared")));
        // Without the module's own `g`, the body's is no answer.
        let only = &files[..1];
        assert_eq!(at(&sources(only, &slugs)["g"]), (None, None, None));
    }

    #[test]
    fn a_name_declared_twice_is_ambiguous_not_guessed() {
        // `x` is a Less in one file and an Or in another: neither its line
        // nor the port key of its expressions (`input2` or `i2`) is known.
        let files = vec![
            ("a.loxl".to_string(), "x = Less(In2: 1)\n".to_string()),
            (
                "b.loxl".to_string(),
                "x = Or()\nx.In2 <- x.Q and x.Q\ny = x.Q or x.Q\n".to_string(),
            ),
        ];
        let slugs: BTreeMap<String, String> = ["x", "x_i2__and1", "x_input2__and1", "y", "y__or1"]
            .iter()
            .map(|s| (s.to_string(), s.to_string()))
            .collect();
        let got = sources(&files, &slugs);
        let amb = (None, None, Some("ambiguous"));
        assert_eq!(at(&got["x"]), amb);
        assert_eq!(at(&got["x_i2__and1"]), amb);
        assert_eq!(at(&got["x_input2__and1"]), amb);
        // A name declared once is still found.
        assert_eq!(at(&got["y"]), (Some("b.loxl"), Some(3), Some("declared")));
        assert_eq!(
            at(&got["y__or1"]),
            (Some("b.loxl"), Some(3), Some("expression"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_module_outside_the_project_is_shown_relative_never_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let elsewhere = tmp.path().join("shared/logic");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(elsewhere.join("sub")).unwrap();
        std::fs::write(elsewhere.join("sub/a.loxl"), "a = Or()\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("pages")).unwrap();
        // Linked in: the path through the link, relative to the project.
        let names: Vec<String> = module(&root, &root.join("pages"))
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(names, ["pages/sub/a.loxl"]);
        // A module that is not under the project at all: relative to itself.
        let names: Vec<String> = module(&root, &elsewhere)
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(names, ["sub/a.loxl"]);
        let one: Vec<String> = module(&root, &elsewhere.join("sub/a.loxl"))
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(one, ["a.loxl"]);
    }

    #[test]
    fn a_file_is_read_again_only_when_it_may_have_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("pages")).unwrap();
        let f = root.join("pages/a.loxl");
        std::fs::write(&f, "a = Or()\n").unwrap();
        let old = SystemTime::now() - Duration::from_secs(60);
        let set = |t: SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(&f)
                .unwrap()
                .set_modified(t)
                .unwrap()
        };
        set(old);
        let mut m = ModuleFiles::default();
        let first = m.refresh(root, &root.join("pages"));
        // Same size and time, long ago: the text kept is not read again.
        std::fs::write(&f, "b = Or()\n").unwrap();
        set(old);
        assert_eq!(m.refresh(root, &root.join("pages")), first);
        // A new time: read again.
        set(old + Duration::from_secs(1));
        let second = m.refresh(root, &root.join("pages"));
        assert_ne!(second, first);
        assert_eq!(&*m.texts()[0].1, "b = Or()\n");
        // A time too recent to trust is read again every time, even when
        // size and time stay the same.
        let now = SystemTime::now();
        std::fs::write(&f, "c = Or()\n").unwrap();
        set(now);
        m.refresh(root, &root.join("pages"));
        std::fs::write(&f, "d = Or()\n").unwrap();
        set(now);
        m.refresh(root, &root.join("pages"));
        assert_eq!(&*m.texts()[0].1, "d = Or()\n");
    }

    #[test]
    fn member_lists_point_at_their_line_and_their_central_block() {
        let files = vec![(
            "pages/licht.loxl".to_string(),
            "extern zentral = CentralLight(uuid: \"10000050-0000-0500-ffff504f94000001\")\n\
             l1 = LightController2(\"L1\")\n\
             zentral.members = (l1)  # die Lichter\n\
             # zentral.members = ()\n\
             template T(c: CentralLight, l: LightController2)\n\
             \tc.members = (l)\n\
             end\n\
             l1.moods = abend\n"
                .to_string(),
        )];
        let lock = crate::edit::Lock::default();
        let c = members_claims(&files, &lock);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(
            (c[0].kind, c[0].slug.as_str(), c[0].set.as_str()),
            ("members", "zentral", "(l1)")
        );
        assert_eq!((c[0].file.as_str(), c[0].line), ("pages/licht.loxl", 3));
        assert_eq!(c[0].blocks, ["10000050-0000-0500-ffff504f94000001"]);
        // Not a set claim: Apply's review of sets leaves it out.
        assert!(
            set_claims(&files, &lock)
                .iter()
                .all(|c| c.kind != "members")
        );
    }

    #[test]
    fn claims_on_rights_and_schedules_point_at_their_line_their_set_and_their_block() {
        let files = vec![
            (
                "pages/access.loxl".to_string(),
                "extern haustuer = NfcCodeTouch(uuid: \"40000001-0000-0000-ffff504f94000001\")\n\
                 extern frost = Notification(title: \"Frost\")\n\
                 \n\
                 access bewohner  # wer öffnen darf\n\
                 \tuser \"alice\": \"Always\"\n\
                 end\n\
                 recipients familie\n\
                 \tuser \"bob\"\n\
                 end\n\
                 haustuer.access = bewohner\n\
                 frost.recipients = familie # wer es bekommt\n\
                 # haustuer.access = alle\n\
                 template T(x: NfcCodeTouch)\n\
                 \tx.access = bewohner\n\
                 end\n"
                    .to_string(),
            ),
            (
                "pages/zeit.loxl".to_string(),
                "schedule abend\n\tweekdays 17:00-22:00\nend\n\
                 uhr = DayTimer(\"Uhr\")\n\
                 uhr.schedule = abend\n\
                 vx_10000030_0000_0300_ffff504f94000001.schedule = abend\n"
                    .to_string(),
            ),
        ];
        let lock = crate::edit::Lock {
            slugs: [(
                "uhr".to_string(),
                "10000031-0000-0310-ffff504f94000001".to_string(),
            )]
            .into(),
            externals: [(
                "frost".to_string(),
                "60000001-0000-0010-ffff504f94000001".to_string(),
            )]
            .into(),
            ..Default::default()
        };
        let c = set_claims(&files, &lock);
        let got: Vec<_> = c
            .iter()
            .map(|c| {
                (
                    c.kind,
                    c.slug.as_str(),
                    c.set.as_str(),
                    format!("{}:{}", c.file, c.line),
                    c.set_line,
                    c.blocks.join(","),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    "access",
                    "haustuer",
                    "bewohner",
                    "pages/access.loxl:10".to_string(),
                    Some(4),
                    "40000001-0000-0000-ffff504f94000001".to_string()
                ),
                (
                    "recipients",
                    "frost",
                    "familie",
                    "pages/access.loxl:11".to_string(),
                    Some(7),
                    "60000001-0000-0010-ffff504f94000001".to_string()
                ),
                (
                    "schedule",
                    "uhr",
                    "abend",
                    "pages/zeit.loxl:5".to_string(),
                    Some(1),
                    "10000031-0000-0310-ffff504f94000001".to_string()
                ),
                (
                    "schedule",
                    "vx_10000030_0000_0300_ffff504f94000001",
                    "abend",
                    "pages/zeit.loxl:6".to_string(),
                    Some(1),
                    "10000030-0000-0300-ffff504f94000001".to_string()
                ),
            ]
        );
        assert_eq!(c[2].set_file.as_deref(), Some("pages/zeit.loxl"));
    }
}
