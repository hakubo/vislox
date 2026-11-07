//! The blocks the project's Lox Logic module manages, and where its source
//! declares each one (#64).
//!
//! A block is managed when the lockfile pins it (`objects`: slug → UUID).
//! Externs (`externals`) are not: Loxone Config owns them. Where each slug
//! comes from is read off the module's `.loxl` files with lox's own
//! declaration scan (the one its language server uses for symbols):
//!
//! - a block, a named signal or a template instance declared by name is at
//!   its declaration's line;
//! - a block lox made for an expression (`sink_port__and1`, see lox's
//!   `desugar.rs`) is at the line of the expression: the `<-` statement, the
//!   argument of the block it feeds, or the named signal (`signal__or1`);
//! - a block a template instance made (`instance_block`) is at the
//!   instance's line.
//!
//! A slug none of these finds (a module that does not parse as lox reads
//! it, a lock from another module) keeps its slug, with no file.

use loxone_config::lsp::analysis::scan;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Where one managed block comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub slug: String,
    /// The `.loxl` file, relative to the project (`pages/licht.loxl`).
    pub file: Option<String>,
    /// 1-based.
    pub line: Option<usize>,
    /// `declared`, `expression` or `template`; `None` when not found.
    pub how: Option<&'static str>,
}

/// The module's `.loxl` files, by their path relative to the project root,
/// in order. `module` is a file or a directory of them.
pub fn module_texts(root: &Path, module: &Path) -> Vec<(String, String)> {
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
    paths
        .into_iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(&f).ok()?;
            let rel = f
                .strip_prefix(root)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, text))
        })
        .collect()
}

/// Every slug of `slugs` (slug → UUID) located in `files`: UUID → source.
pub fn sources(
    files: &[(String, String)],
    slugs: &BTreeMap<String, String>,
) -> BTreeMap<String, Source> {
    let ix = Index::new(files);
    slugs
        .iter()
        .map(|(slug, uuid)| {
            let at = ix.locate(slug);
            (
                uuid.clone(),
                Source {
                    slug: slug.clone(),
                    file: at.map(|(f, _, _)| f.to_string()),
                    line: at.map(|(_, l, _)| l),
                    how: at.map(|(_, _, h)| h),
                },
            )
        })
        .collect()
}

type At<'a> = (&'a str, usize);

#[derive(Default)]
struct Index<'a> {
    /// Declared names (not externs): slug → where.
    decls: BTreeMap<String, At<'a>>,
    /// The instances of templates: their slugs.
    instances: BTreeSet<String>,
    /// `<sink>_<port key>` of every expression: where it is written.
    exprs: BTreeMap<String, At<'a>>,
}

impl<'a> Index<'a> {
    fn new(files: &'a [(String, String)]) -> Self {
        let mut ix = Index::default();
        // Block types by name, for the port keys lox names expression
        // blocks after; and the templates, whose calls are instances.
        let mut types: BTreeMap<String, String> = BTreeMap::new();
        let mut templates = BTreeSet::new();
        // (sink, port, where) of `<-` statements and argument bindings.
        let mut sinks: Vec<(String, String, At<'a>)> = Vec::new();
        let mut calls: Vec<(String, String)> = Vec::new();
        for (file, text) in files {
            let lines: Vec<&str> = text.split('\n').collect();
            for d in scan(text, Path::new(file)) {
                let at = (file.as_str(), d.span.line + 1);
                if let Some(ty) = &d.ty {
                    types.insert(d.slug.clone(), ty.clone());
                }
                match d.kind {
                    "extern" => continue,
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
                ix.decls.entry(d.slug).or_insert(at);
            }
            for (n, line) in lines.iter().enumerate() {
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
            let ty = types.get(&sink).map(String::as_str).unwrap_or("");
            let key = loxone_config::port_names::key_or_raw(ty, &port).to_lowercase();
            ix.exprs.entry(format!("{sink}_{key}")).or_insert(at);
        }
        ix
    }

    fn locate(&self, slug: &str) -> Option<(&'a str, usize, &'static str)> {
        if let Some(&(f, l)) = self.decls.get(slug) {
            return Some((f, l, "declared"));
        }
        // `<sink>_<port>__<op><n>`, or `<signal>__<op><n>`.
        if let Some(i) = slug.rfind("__") {
            let prefix = &slug[..i];
            if let Some(&(f, l)) = self.exprs.get(prefix).or_else(|| self.decls.get(prefix)) {
                return Some((f, l, "expression"));
            }
        }
        // `<instance>_<block>`: the longest instance it starts with.
        self.instances
            .iter()
            .filter(|i| {
                slug.len() > i.len()
                    && slug.starts_with(i.as_str())
                    && slug[i.len()..].starts_with('_')
            })
            .max_by_key(|i| i.len())
            .and_then(|i| self.decls.get(i))
            .map(|&(f, l)| (f, l, "template"))
    }
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

    #[test]
    fn a_module_of_one_file_or_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("pages/sub")).unwrap();
        std::fs::write(root.join("pages/b.loxl"), "b = Or()\n").unwrap();
        std::fs::write(root.join("pages/sub/a.loxl"), "a = Or()\n").unwrap();
        std::fs::write(root.join("pages/.hidden.loxl"), "h = Or()\n").unwrap();
        std::fs::write(root.join("pages/README"), "not lox logic\n").unwrap();
        let names: Vec<String> = module_texts(root, &root.join("pages"))
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        assert_eq!(names, ["pages/b.loxl", "pages/sub/a.loxl"]);
        let one = module_texts(root, &root.join("pages/b.loxl"));
        assert_eq!(
            one,
            [("pages/b.loxl".to_string(), "b = Or()\n".to_string())]
        );
    }
}
