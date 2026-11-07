//! Whether the project's Lox Logic module compiles (#65).
//!
//! vislox shows the compiled `out.Loxone` as the pending revision. When the
//! module no longer compiles, `out` stays at the last good compile, and the
//! review would show an older change than the one in the source. So vislox
//! runs `lox logic check --json` on the module whenever it changes, and says
//! so: each error with its file, line and message, and the blocks that line
//! names. Apply is refused while there are errors.
//!
//! `lox logic fmt --check` runs too, as a hint only: the files that are not
//! formatted the way `lox logic fmt --write` would write them.
//!
//! This module only reads lox's output; `actions.rs` runs lox and works out
//! whether the pending config is from before the latest change.

use crate::edit::Lock;
use crate::managed::Source;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One error of `lox logic check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Error {
    /// The `.loxl` file, shown as `GET /api/edits`' `sources` show it:
    /// relative to the project (to the module when it lies outside it).
    pub file: Option<String>,
    /// 1-based.
    pub line: Option<usize>,
    pub message: String,
    /// The blocks that line makes or names, by UUID: a managed block declared
    /// there, a lockfile slug or an extern's UUID written on it.
    pub blocks: Vec<String>,
}

/// What `lox logic check --json` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub errors: Vec<Error>,
    /// The errors lox left out (it reports at most 50).
    pub truncated: usize,
}

/// `lox logic check --json`'s standard output; `None` when it is not
/// (an older lox, a stand-in, a crash): then nothing is known, and nothing
/// is refused. `file` turns lox's path of a file into the shown one.
pub fn parse(stdout: &str, file: impl Fn(&str) -> String) -> Option<Checked> {
    let v: serde_json::Value = stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .find_map(|l| serde_json::from_str(l).ok())?;
    let ok = v.get("ok")?.as_bool()?;
    let errors = if ok {
        Vec::new()
    } else {
        v.get("errors")?
            .as_array()?
            .iter()
            .map(|e| Error {
                file: e.get("path").and_then(|p| p.as_str()).map(&file),
                line: e
                    .get("line")
                    .and_then(|l| l.as_u64())
                    .map(|l| l as usize)
                    .filter(|l| *l > 0),
                message: message(e.get("message").and_then(|m| m.as_str()).unwrap_or("")),
                blocks: Vec::new(),
            })
            .collect()
    };
    if !ok && errors.is_empty() {
        // `ok: false` with no error to show: not lox's format.
        return None;
    }
    let truncated = v.get("truncated").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
    Some(Checked { errors, truncated })
}

/// lox prefixes every message with what failed; the banner says that.
fn message(m: &str) -> String {
    let m = m.trim();
    m.strip_prefix("compile error: ").unwrap_or(m).to_string()
}

/// The files `lox logic fmt --check` names as not formatted, from its
/// standard output and error, each read line by line on its own
/// (`pages/a.loxl: not canonical (run …)`), shown by `file`.
pub fn unformatted(stdout: &str, stderr: &str, file: impl Fn(&str) -> String) -> Vec<String> {
    let mut v: Vec<String> = stdout
        .lines()
        .chain(stderr.lines())
        .filter_map(|l| l.split_once(": not canonical"))
        .map(|(f, _)| file(f.trim()))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Fill in each error's `blocks`: what its line declares (`sources`, UUID →
/// where the module makes it) and the managed slugs and extern UUIDs written
/// on it. `texts` are the module's files by shown path.
pub fn locate(
    errors: &mut [Error],
    texts: &[(String, impl AsRef<str>)],
    lock: &Lock,
    sources: &BTreeMap<String, Source>,
) {
    for e in errors {
        if let (Some(file), Some(line)) = (&e.file, e.line) {
            e.blocks = blocks_at(file, line, texts, lock, sources);
        }
    }
}

/// The blocks line `line` of `file` makes or names: what it declares
/// (`sources`) and the managed slugs and extern UUIDs written on it.
fn blocks_at(
    file: &str,
    line: usize,
    texts: &[(String, impl AsRef<str>)],
    lock: &Lock,
    sources: &BTreeMap<String, Source>,
) -> Vec<String> {
    let mut blocks: Vec<String> = sources
        .iter()
        .filter(|(_, s)| s.file.as_deref() == Some(file) && s.line == Some(line))
        .map(|(u, _)| u.clone())
        .collect();
    let text = texts
        .iter()
        .find(|(f, _)| f == file)
        .and_then(|(_, t)| t.as_ref().lines().nth(line - 1));
    if let Some(text) = text {
        for u in named(text, lock) {
            if !blocks.contains(&u) {
                blocks.push(u);
            }
        }
    }
    blocks
}

/// One finding of `lox logic lint` (#66): never an error, and never a
/// reason to refuse Apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// lox's kind (`unused-extern`, `dead-outputs`, `no-access-right`, …).
    pub kind: String,
    /// What it is about, as lox names it: a slug, a constant, a template, a
    /// block type (`unused-default`), a user (`no-access-right`).
    pub name: String,
    pub message: String,
    /// The `.loxl` file that declares `name`, shown as [`Error::file`] is.
    pub file: Option<String>,
    /// 1-based: the declaration's line.
    pub line: Option<usize>,
    /// The blocks it is about, by UUID: the managed block or extern `name`
    /// is, and what the declaration's line names.
    pub blocks: Vec<String>,
}

/// `lox logic lint`'s standard output, read: each kind (`unused-let (2)`),
/// its findings (`` pages/a.loxl: `slug` — detail ``, indented by two, the
/// file only in a directory module), and the count it ends with (`lint: 3
/// findings …` or `lint: clean`). Any other line after a finding goes on
/// with that finding's detail, up to the next kind. `None` without the
/// count: lox failed (it exits 1 for findings and for errors alike) or is
/// not one vislox can read. lox has no JSON for lint yet, so this reads the
/// text it prints.
pub fn parse_lint(stdout: &str, file: impl Fn(&str) -> String) -> Option<Vec<Finding>> {
    let mut out: Vec<Finding> = Vec::new();
    let mut kind: Option<String> = None;
    // Whether the last finding is of the current kind: a line that goes on
    // goes on with it.
    let mut open = false;
    for l in stdout.lines() {
        if l == "lint: clean" || (l.starts_with("lint: ") && l.contains(" finding")) {
            return Some(out);
        }
        if let Some(k) = lint_kind(l) {
            kind = Some(k.to_string());
            open = false;
            continue;
        }
        if let Some(k) = &kind
            && let Some((f, name, message)) = lint_finding(l)
        {
            out.push(Finding {
                kind: k.clone(),
                name: name.to_string(),
                message: message.trim().to_string(),
                file: f.map(&file),
                line: None,
                blocks: Vec::new(),
            });
            open = true;
            continue;
        }
        if open
            && !l.trim().is_empty()
            && let Some(last) = out.last_mut()
        {
            last.message.push('\n');
            last.message.push_str(l.trim());
        }
    }
    None
}

/// A kind's header, `dead-outputs (2)`: its kind.
fn lint_kind(l: &str) -> Option<&str> {
    let (k, n) = l.strip_suffix(')')?.split_once(" (")?;
    let ok = k.starts_with(|c: char| c.is_ascii_lowercase())
        && k.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit());
    ok.then_some(k)
}

/// A finding's line: two spaces, the file and `: ` when lox names one,
/// then `` `name` — `` and the detail. The file and the name hold no
/// backtick.
fn lint_finding(l: &str) -> Option<(Option<&str>, &str, &str)> {
    let rest = l.strip_prefix("  ")?;
    let (file, rest) = match rest.strip_prefix('`') {
        Some(r) => (None, r),
        None => {
            let (f, r) = rest.split_once(": `")?;
            if f.is_empty() || f.contains('`') || f.starts_with(' ') {
                return None;
            }
            (Some(f), r)
        }
    };
    let (name, message) = rest.split_once("` — ")?;
    if name.is_empty() || name.contains('`') {
        return None;
    }
    Some((file, name, message))
}

/// Whether a `lox logic lint` that gave no count is a lox without it:
/// lox's usage error (exit 2, `unknown command`), not a lint that failed.
pub fn lint_unsupported(code: Option<i32>, stdout: &str, stderr: &str) -> bool {
    let said = |t: &str| t.contains("unknown command") || t.contains("unrecognized subcommand");
    code == Some(2) || said(stdout) || said(stderr)
}

/// Pin each finding to where its name is declared (the file lox named, or
/// the only file that declares it) and to the blocks that name is: the
/// managed block or device the lockfile pins by it, or the object an extern
/// of that name pins (`uuid: "…"`, or the UUID vislox names it after). Not
/// the blocks the declaration only refers to.
pub fn locate_findings<T: AsRef<str>>(
    findings: &mut [Finding],
    texts: &[(String, T)],
    lock: &Lock,
) {
    let decls = crate::managed::Declarations::new(texts);
    for f in findings {
        if let Some((file, line)) = decls.at(&f.name, f.file.as_deref()) {
            f.file = Some(file);
            f.line = Some(line);
        }
        let mut blocks: Vec<String> = lock
            .uuid_of(&f.name)
            .map(String::from)
            .into_iter()
            .collect();
        blocks.extend(as_uuid(f.name.strip_prefix("vx_").unwrap_or(&f.name)));
        let text = match (&f.file, f.line) {
            (Some(file), Some(line)) => texts
                .iter()
                .find(|(t, _)| t == file)
                .and_then(|(_, t)| t.as_ref().lines().nth(line - 1)),
            _ => None,
        };
        if let Some(text) = text
            && let [Tok::Ident(kw), Tok::Ident(n), ..] = tokens(text).as_slice()
            && kw == "extern"
            && *n == f.name
        {
            blocks.extend(names(text).1.iter().filter_map(|u| as_uuid(u)));
        }
        let mut seen = std::collections::BTreeSet::new();
        blocks.retain(|u| seen.insert(u.clone()));
        f.blocks = blocks;
    }
}

/// The UUIDs a line of Lox Logic names ([`names`]): a name by the
/// lockfile's slugs, or by the UUID vislox names an extern after
/// (`vx_10000006_0000_0060_ffff504f94000001`); an extern's `uuid: "…"`.
fn named(line: &str, lock: &Lock) -> Vec<String> {
    let (idents, uuids) = names(line);
    let mut out = Vec::new();
    for id in idents {
        let u = lock
            .uuid_of(&id)
            .map(String::from)
            .or_else(|| as_uuid(id.strip_prefix("vx_").unwrap_or(&id)));
        out.extend(u);
    }
    out.extend(uuids.iter().filter_map(|u| as_uuid(u)));
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|u| seen.insert(u.clone()));
    out
}

/// A token of a line of Lox Logic.
#[derive(Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    /// A number, a time (`07:05`) or anything else of digits.
    Num,
    Punct(char),
}

/// A line of Lox Logic as tokens, up to its comment: a `#` outside a
/// string starts one.
fn tokens(line: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '#' => break,
            c if c.is_whitespace() => {}
            '"' => {
                let mut s = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => s.extend(it.next()),
                        c => s.push(c),
                    }
                }
                out.push(Tok::Str(s));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = String::from(c);
                while let Some(&c) = it
                    .peek()
                    .filter(|c| c.is_ascii_alphanumeric() || **c == '_')
                {
                    s.push(c);
                    it.next();
                }
                out.push(Tok::Ident(s));
            }
            c if c.is_ascii_digit() => {
                while it
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '_'))
                {
                    it.next();
                }
                out.push(Tok::Num);
            }
            c => out.push(Tok::Punct(c)),
        }
    }
    out
}

/// What a line of Lox Logic names: the name it declares (`slug = …`,
/// `extern slug = …`, `removed slug`, `moved slug …`) and the heads of its
/// dotted references (`slug.Port`); and the UUIDs an extern pins
/// (`uuid: "…"`). Not types, keywords, port names, argument keys, or
/// anything in a string or a comment.
fn names(line: &str) -> (Vec<String>, Vec<String>) {
    let t = tokens(line);
    let ident = |i: usize| match t.get(i) {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    };
    let punct = |i: usize, c: char| t.get(i) == Some(&Tok::Punct(c));
    let mut idents = Vec::new();
    let mut uuids = Vec::new();
    for i in 0..t.len() {
        let Some(id) = ident(i) else {
            continue;
        };
        let lead = matches!(ident(0), Some("extern" | "removed" | "moved")) as usize;
        let declared = i == lead && (punct(i + 1, '=') && !punct(i + 2, '=') || lead == 1);
        let head = punct(i + 1, '.') && !(i > 0 && punct(i - 1, '.'));
        if (declared || head) && !(lead == 1 && i == 0) {
            idents.push(id.to_string());
        }
        if id == "uuid"
            && punct(i + 1, ':')
            && let Some(Tok::Str(u)) = t.get(i + 2)
        {
            uuids.push(u.clone());
        }
    }
    (idents, uuids)
}

/// A Loxone UUID (`10000006-0000-0060-ffff504f94000001`, 8-4-4-16 hex
/// digits), written with dashes or underscores.
fn as_uuid(w: &str) -> Option<String> {
    let parts: Vec<&str> = w.split(['-', '_']).collect();
    let ok = parts.len() == 4
        && parts
            .iter()
            .zip([8, 4, 4, 16])
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_hexdigit()));
    ok.then(|| parts.join("-").to_ascii_lowercase())
}

/// The module's path as lox is given it: relative to the project root when
/// it lies inside it, so lox's error paths are short; else as it is.
pub fn module_arg(root: &Path, module: &Path) -> String {
    module
        .strip_prefix(root)
        .unwrap_or(module)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIV: &str = "10000006-0000-0060-ffff504f94000001";
    const FLUR: &str = "1000000b-0000-00b0-ffff504f94000001";

    #[test]
    fn errors_are_read_with_their_file_and_line() {
        let out = r#"{"errors":[{"line":5,"message":"compile error: reference to undeclared slug `bar` (in `bar.In1`)","path":"pages/a.loxl"},{"line":null,"message":"compile error: no page","path":"pages/b.loxl"}],"ok":false,"path":"pages/a.loxl","truncated":3}"#;
        let c = parse(out, |f| format!("./{f}")).unwrap();
        assert_eq!(c.truncated, 3);
        assert_eq!(c.errors.len(), 2);
        assert_eq!(c.errors[0].file.as_deref(), Some("./pages/a.loxl"));
        assert_eq!(c.errors[0].line, Some(5));
        assert_eq!(
            c.errors[0].message,
            "reference to undeclared slug `bar` (in `bar.In1`)"
        );
        assert_eq!(c.errors[1].line, None);
    }

    #[test]
    fn a_module_that_compiles_has_no_errors_and_anything_else_is_unknown() {
        let ok = r#"{"counts":{"blocks":1},"ok":true,"path":"pages"}"#;
        assert_eq!(parse(ok, str::to_string).unwrap().errors, vec![]);
        assert_eq!(parse("unexpected: logic check", str::to_string), None);
        assert_eq!(parse("", str::to_string), None);
        assert_eq!(parse(r#"{"ok":false,"errors":[]}"#, str::to_string), None);
    }

    #[test]
    fn unformatted_files_are_named() {
        let out = "pages/a.loxl: not canonical (run `lox logic fmt --write pages`)\n\
                   pages/b.loxl: canonical\n\
                   pages/vislox/c.loxl: not canonical (run `lox logic fmt --write pages`)\n";
        assert_eq!(
            unformatted(out, "", str::to_string),
            vec!["pages/a.loxl", "pages/vislox/c.loxl"]
        );
        // Standard output without a last newline, then standard error: two
        // lines, never one.
        assert_eq!(
            unformatted(
                "pages/a.loxl: not canonical (run x)",
                "pages/b.loxl: not canonical (run x)\n",
                str::to_string
            ),
            vec!["pages/a.loxl", "pages/b.loxl"]
        );
    }

    #[test]
    fn a_line_names_declarations_and_references_only() {
        let n = |l: &str| names(l);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(n("flur = And(\"Flur\")"), (s(&["flur"]), s(&[])));
        assert_eq!(
            n("flur.In1 <- nope.Q # teiler.Q"),
            (s(&["flur", "nope"]), s(&[]))
        );
        // Types, keywords, argument keys and port names are no blocks;
        // neither is a word in a string, nor after a # inside one.
        assert_eq!(
            n("vx_and_1 = And(\"teiler # flur.Q\", In1: teiler.Q, Time: 30, At: 07:05)"),
            (s(&["vx_and_1", "teiler"]), s(&[]))
        );
        assert_eq!(
            n(&format!("extern vx_x = Div(uuid: \"{DIV}\")")),
            (s(&["vx_x"]), s(&[DIV]))
        );
        assert_eq!(n("removed vx_and_1"), (s(&["vx_and_1"]), s(&[])));
        assert_eq!(n("page \"Obergeschoss\""), (s(&[]), s(&[])));
        assert_eq!(n("if a == b"), (s(&[]), s(&[])));
    }

    #[test]
    fn an_error_points_at_the_blocks_its_line_names() {
        let lock = Lock {
            slugs: [("flur".to_string(), FLUR.to_string())].into(),
            ..Lock::default()
        };
        let texts = vec![
            (
                "pages/a.loxl".to_string(),
                "page \"OG\"\nflur = And(\"Flur\")\nflur.In1 <- nope.Q # not teiler\nx = And(\"flur\", In1: q.flur)\n".to_string(),
            ),
            (
                "pages/vislox/edits.loxl".to_string(),
                format!("extern vx_10000006_0000_0060_ffff504f94000001 = Div(uuid: \"{DIV}\")\n"),
            ),
        ];
        let sources: BTreeMap<String, Source> = [(
            FLUR.to_string(),
            Source {
                slug: "flur".into(),
                file: Some("pages/a.loxl".into()),
                line: Some(2),
                how: Some("declared"),
            },
        )]
        .into();
        let err = |file: &str, line| Error {
            file: Some(file.into()),
            line: Some(line),
            message: String::new(),
            blocks: vec![],
        };
        let mut errors = vec![
            err("pages/a.loxl", 2),
            err("pages/a.loxl", 3),
            err("pages/a.loxl", 1),
            err("pages/a.loxl", 4),
            err("pages/vislox/edits.loxl", 1),
            err("pages/gone.loxl", 1),
        ];
        locate(&mut errors, &texts, &lock, &sources);
        let blocks: Vec<_> = errors.iter().map(|e| e.blocks.clone()).collect();
        assert_eq!(
            blocks,
            vec![
                vec![FLUR.to_string()],
                vec![FLUR.to_string()],
                vec![],
                vec![],
                vec![DIV.to_string()],
                vec![]
            ]
        );
    }

    #[test]
    fn lint_findings_are_read_by_kind() {
        let out = "unit-suffix (1)\n  pages/a.loxl: `m` — `Time: 10800` reads better with its unit.\n\
                   unused-let (2)\n  pages/a.loxl: `c` — constant is never referenced\n  `d` — constant is never referenced\n\
                   no-access-right (1)\n  `Anna Muster` — user with an NFC tag has no access right on any NFC Code Touch.\n\
                   lint: 4 findings (1 fixable with `loxl lint --fix`)\n";
        let f = parse_lint(out, |f| format!("./{f}")).unwrap();
        let k: Vec<_> = f
            .iter()
            .map(|f| (f.kind.as_str(), f.name.as_str(), f.file.as_deref()))
            .collect();
        assert_eq!(
            k,
            vec![
                ("unit-suffix", "m", Some("./pages/a.loxl")),
                ("unused-let", "c", Some("./pages/a.loxl")),
                ("unused-let", "d", None),
                ("no-access-right", "Anna Muster", None),
            ]
        );
        assert_eq!(f[0].message, "`Time: 10800` reads better with its unit.");
        assert_eq!(parse_lint("lint: clean\n", str::to_string), Some(vec![]));
        // No count at the end: lox failed, or is not one vislox reads.
        assert_eq!(parse_lint("", str::to_string), None);
        assert_eq!(
            parse_lint("unused-let (1)\n  `c` — x\n", str::to_string),
            None
        );
    }

    /// lox 0.83's own output (`lox logic lint .` on a project of two files).
    const REAL: &str = "unused-template (1)
  pages/b.loxl: `Unbenutzt` — declared but never instantiated
unit-suffix (1)
  pages/a.loxl: `m` — `Time: 10800` reads better with its unit. Write `Time: 3h` instead of `Time: 10800`.
unused-let (1)
  pages/a.loxl: `unused_c` — constant is never referenced
unused-extern (1)
  pages/a.loxl: `e2` — extern Div is never referenced
dead-outputs (1)
  pages/a.loxl: `x` — no wire draws from any of its ports (compiled config, GUI wires included)
unused-allow (1)
  pages/b.loxl: `y` — `# lint: allow dead-outputs because why not` covers no finding. Remove it, or check that it names the right kind.
lint: 6 findings (1 fixable with `loxl lint --fix`)
";

    #[test]
    fn lox_s_own_output_is_read_finding_by_finding() {
        let f = parse_lint(REAL, str::to_string).unwrap();
        let k: Vec<_> = f
            .iter()
            .map(|f| (f.kind.as_str(), f.name.as_str(), f.file.as_deref().unwrap()))
            .collect();
        assert_eq!(
            k,
            vec![
                ("unused-template", "Unbenutzt", "pages/b.loxl"),
                ("unit-suffix", "m", "pages/a.loxl"),
                ("unused-let", "unused_c", "pages/a.loxl"),
                ("unused-extern", "e2", "pages/a.loxl"),
                ("dead-outputs", "x", "pages/a.loxl"),
                ("unused-allow", "y", "pages/b.loxl"),
            ]
        );
        assert_eq!(
            f[5].message,
            "`# lint: allow dead-outputs because why not` covers no finding. Remove it, or check that it names the right kind."
        );
    }

    #[test]
    fn a_detail_over_several_lines_stays_with_its_finding_and_its_kind() {
        // Wrapped lines, one with backticks and a dash, one indented like a
        // finding but not one; and none that goes on across a kind.
        let out = "dead-outputs (1)
  pages/a.loxl: `x` — no wire draws from any of its ports;
the `Q` — output feeds nothing
  and `x.Q` goes nowhere
unused-let (1)
stray line before any finding of this kind
  `c` — constant is never referenced
lint: 2 findings
";
        let f = parse_lint(out, str::to_string).unwrap();
        assert_eq!(f.len(), 2, "{f:?}");
        assert_eq!(
            f[0].message,
            "no wire draws from any of its ports;\nthe `Q` — output feeds nothing\nand `x.Q` goes nowhere"
        );
        assert_eq!(
            (f[1].kind.as_str(), f[1].name.as_str()),
            ("unused-let", "c")
        );
        assert_eq!(f[1].message, "constant is never referenced");
    }

    #[test]
    fn a_lox_without_lint_is_told_from_a_lint_that_failed() {
        assert!(lint_unsupported(
            Some(2),
            "",
            "unknown command `lint`. Run `lox logic help`."
        ));
        assert!(lint_unsupported(
            Some(2),
            "",
            "error: unrecognized subcommand 'logic'"
        ));
        assert!(!lint_unsupported(
            Some(1),
            "",
            "error: compile for dead-output analysis failed: boom"
        ));
        assert!(!lint_unsupported(None, "", ""));
    }

    #[test]
    fn a_finding_points_at_its_declaration_and_the_block_it_names() {
        let lock = Lock {
            slugs: [("flur".to_string(), FLUR.to_string())].into(),
            ..Lock::default()
        };
        let texts = vec![
            (
                "pages/a.loxl".to_string(),
                "page \"OG\"\nlet c = 5\nflur = And(\"Flur\", In1: teiler.Q)\n".to_string(),
            ),
            (
                "pages/b.loxl".to_string(),
                format!("extern e = Div(uuid: \"{DIV}\")\nlet c = 6\n"),
            ),
        ];
        let f = |name: &str, file: Option<&str>| Finding {
            kind: "k".into(),
            name: name.into(),
            message: String::new(),
            file: file.map(String::from),
            line: None,
            blocks: vec![],
        };
        let mut fs = vec![
            f("flur", None),
            f("e", Some("pages/b.loxl")),
            f("c", Some("pages/a.loxl")),
            f("c", None),
            f("Anna", None),
        ];
        locate_findings(&mut fs, &texts, &lock);
        let at: Vec<_> = fs
            .iter()
            .map(|f| (f.file.as_deref(), f.line, f.blocks.clone()))
            .collect();
        assert_eq!(
            at,
            vec![
                // Not `teiler`, which the line only refers to.
                (Some("pages/a.loxl"), Some(3), vec![FLUR.to_string()]),
                (Some("pages/b.loxl"), Some(1), vec![DIV.to_string()]),
                (Some("pages/a.loxl"), Some(2), vec![]),
                // Declared in two files, and lox named neither: no line.
                (None, None, vec![]),
                (None, None, vec![]),
            ]
        );
    }

    #[test]
    fn uuids_are_read_with_dashes_or_underscores() {
        assert_eq!(
            as_uuid("10000006_0000_0060_FFFF504F94000001").as_deref(),
            Some(DIV)
        );
        assert_eq!(as_uuid(DIV).as_deref(), Some(DIV));
        assert_eq!(as_uuid("vx_and_1"), None);
        assert_eq!(as_uuid("10000006-0000-0060"), None);
    }
}
