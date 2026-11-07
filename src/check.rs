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
/// output (`pages/a.loxl: not canonical (run …)`), shown by `file`.
pub fn unformatted(output: &str, file: impl Fn(&str) -> String) -> Vec<String> {
    let mut v: Vec<String> = output
        .lines()
        .filter_map(|l| l.split_once(": not canonical"))
        .map(|(f, _)| file(f.trim()))
        .collect();
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
        let (Some(file), Some(line)) = (&e.file, e.line) else {
            continue;
        };
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
            // A comment names nothing.
            let code = text.split('#').next().unwrap_or("");
            for u in named(code, lock) {
                if !blocks.contains(&u) {
                    blocks.push(u);
                }
            }
        }
        e.blocks = blocks;
    }
}

/// The UUIDs a line of Lox Logic names: managed slugs (by the lockfile),
/// and UUIDs written out, as an extern pins one (`uuid: "…"`) or as vislox
/// names an extern after it (`vx_10000006_0000_0060_ffff504f94000001`).
fn named(code: &str, lock: &Lock) -> Vec<String> {
    let mut out = Vec::new();
    for word in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        if let Some(u) = lock.uuid_of(word) {
            out.push(u.to_string());
        }
        let w = word.strip_prefix("vx_").unwrap_or(word);
        if let Some(u) = as_uuid(w) {
            out.push(u);
        }
    }
    out.dedup();
    out
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
            unformatted(out, str::to_string),
            vec!["pages/a.loxl", "pages/vislox/c.loxl"]
        );
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
                "page \"OG\"\nflur = And(\"Flur\")\nflur.In1 <- nope.Q # not teiler\n".to_string(),
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
                vec![DIV.to_string()],
                vec![]
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
