//! Personal data before a git push (#71). A real config holds the
//! installation's address, its users' names, mail addresses, phone numbers
//! and access codes, credentials and hardware serials. When a repository's
//! `.gitignore` misses one (a downloaded `live.Loxone`, a backup outside
//! `backups/`), a commit takes it along, and `git push` would send it to the
//! remote. So before every push vislox makes (Apply, Roll back, Sync, and
//! *Push to git*), [`before_push`] asks `lox logic scan` about what the push
//! would send:
//!
//! - **What the push sends**: every commit `HEAD` has that the push remote's
//!   tracking branches do not (`git rev-list HEAD --not --remotes=<remote>`;
//!   every remote's when the branch has no push remote), and in each, every file
//!   it adds or changes that may hold a config: a `.Loxone`, a `.LoxCC` and a
//!   Miniserver backup `.zip` (its `sps0.LoxCC`). Each version, not only the
//!   last: a file added in one commit and deleted in the next is still pushed.
//!   The whole repository, not only the project: the push sends all of it.
//! - **lox decides** (`lox logic scan --json`): it lists where personal data
//!   is, the element and attribute and the value's length, never the value.
//!   vislox adds the line it finds that element and attribute on, and never
//!   reads or keeps a value: nothing it logs, sends to the browser or says in
//!   an error can carry one.
//! - **Fail closed**: a file lox cannot read, a lox without `logic scan`, or an
//!   answer vislox cannot read block the push as findings do. Only a push with
//!   no config file in it goes ahead without lox.

use super::removals::scratch_dir;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// `lox logic scan` reads every config of a push in one run.
const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

/// Where a push would send personal data: lox's finding, and where vislox
/// found its element. Never its value.
#[derive(Clone, Serialize, PartialEq, Debug)]
pub struct Finding {
    /// The file, from the repository's top.
    pub file: String,
    /// The program inside it, for a backup `.zip`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// The line of the element lox names, when vislox finds it.
    pub line: Option<usize>,
    /// lox's rule (`person`, `mail`, `access-code`, `location`, `credential`,
    /// `remote-access`, `network`, `hardware-serial`, …), or `unreadable`
    /// when lox could not read the file.
    pub kind: String,
    /// The element and attribute, as lox names them (`User #1 FiN`).
    pub what: String,
    /// The length of the value, as lox gives it.
    pub len: Option<usize>,
    /// The newest commit not pushed yet that has this version of the file.
    pub commit: String,
    /// Still the file as `HEAD` has it; else only in an earlier commit.
    pub now: bool,
}

impl Finding {
    /// `live.Loxone:12`, `backup.zip › sps0.LoxCC:12`.
    pub fn at(&self) -> String {
        let mut s = self.file.clone();
        if let Some(e) = &self.entry {
            s.push_str(&format!(" › {e}"));
        }
        if let Some(l) = self.line {
            s.push_str(&format!(":{l}"));
        }
        s
    }

    /// One line: where, what, and how long the value is.
    pub fn text(&self) -> String {
        let len = self
            .len
            .map(|n| format!(" ({n} char{})", if n == 1 { "" } else { "s" }))
            .unwrap_or_default();
        let when = if self.now {
            String::new()
        } else {
            format!(" [only in commit {}, deleted since]", self.commit)
        };
        format!("{}: {} — {}{len}{when}", self.at(), self.kind, self.what)
    }
}

/// What [`before_push`] says of a push.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Nothing personal found (or no config in the push): how much was read.
    Clean(String),
    /// Personal data, or a file lox could not read.
    Found(Vec<Finding>),
    /// vislox could not find out: why, and how to go on.
    CannotScan(String),
}

/// A config file version the push sends.
struct Candidate {
    path: String,
    blob: String,
    commit: String,
}

/// What `git push` would send that may hold a config, scanned by lox.
pub async fn before_push(dir: &Path, lox: &Path) -> Verdict {
    match scan_push(dir, lox).await {
        Ok(v) => v,
        Err(why) => Verdict::CannotScan(why),
    }
}

async fn scan_push(dir: &Path, lox: &Path) -> Result<Verdict, String> {
    let top = git(dir, &["rev-parse", "--show-toplevel"]).await?;
    let top = PathBuf::from(top.trim());
    let cands = candidates(&top).await?;
    if cands.is_empty() {
        return Ok(Verdict::Clean(
            "no config file in the commits to push".into(),
        ));
    }
    let now = head_blobs(&top, &cands).await;
    let blobs: Vec<String> = cands.iter().map(|c| c.blob.clone()).collect();
    let bytes = git_objects(&top, &blobs).await?;
    let scratch = scratch_dir("vislox-scan")?;
    let _gone = RemoveDir(scratch.clone());
    private(&scratch);
    let mut found = Vec::new();
    // Each config to scan: its name in the scratch directory, its candidate,
    // the zip entry, and its text (to find lines in).
    let mut files: Vec<(String, usize, Option<String>, Vec<u8>)> = Vec::new();
    for (i, (c, b)) in cands.iter().zip(bytes).enumerate() {
        let finding = |entry: Option<String>, what: &str| Finding {
            file: c.path.clone(),
            entry,
            line: None,
            kind: "unreadable".into(),
            what: what.into(),
            len: None,
            commit: c.commit.clone(),
            now: now.get(&c.path) == Some(&c.blob),
        };
        let Some(b) = b else {
            found.push(finding(None, "git has no such object"));
            continue;
        };
        let lower = c.path.to_lowercase();
        let (entry, xml) = if lower.ends_with(".zip") {
            if !loxone_config::archive::is_zip(&b) {
                found.push(finding(None, "not a zip archive: lox cannot read it"));
                continue;
            }
            match loxone_config::archive::extract_and_decompress(&b) {
                Ok(x) => (Some("sps0.LoxCC".to_string()), x),
                // Not a Miniserver backup: no program in it.
                Err(e) if e.to_string().contains("does not contain sps0.LoxCC") => continue,
                Err(_) => {
                    found.push(finding(None, "a backup whose program cannot be read"));
                    continue;
                }
            }
        } else if lower.ends_with(".loxcc") {
            match loxone_config::archive::decompress_loxcc(&b) {
                Ok(x) => (None, x),
                Err(_) => {
                    found.push(finding(None, "a program file that cannot be read"));
                    continue;
                }
            }
        } else {
            (None, b)
        };
        let name = format!("c{i}.Loxone");
        write_private(&scratch.join(&name), &xml)?;
        files.push((name, i, entry, xml));
    }
    if !files.is_empty() {
        let mut args = vec!["logic", "scan", "--json"];
        args.extend(files.iter().map(|f| f.0.as_str()));
        let (code, stdout, stderr) = run_status(lox, &args, &scratch, SCAN_TIMEOUT).await?;
        let report = first_json(&stdout);
        let Some(report) = report.as_ref().and_then(Value::as_object) else {
            return Err(if check::unsupported(code, &stdout, &stderr) {
                format!(
                    "this lox ({}) has no `lox logic scan`, so vislox cannot check the push for \
                     personal data. Update lox (0.83 or later), then push again.",
                    lox.display()
                )
            } else {
                format!(
                    "lox logic scan gave no answer vislox can read ({}).",
                    match code {
                        Some(c) => format!("exit {c}"),
                        None => "no exit code".into(),
                    }
                )
            });
        };
        for (name, i, entry, xml) in &files {
            let c = &cands[*i];
            let base = Finding {
                file: c.path.clone(),
                entry: entry.clone(),
                line: None,
                kind: String::new(),
                what: String::new(),
                len: None,
                commit: c.commit.clone(),
                now: now.get(&c.path) == Some(&c.blob),
            };
            let Some(list) = report.get(name).and_then(Value::as_array) else {
                // lox says why on standard error, with the file's name; vislox
                // does not repeat it (a parse error may quote the file).
                found.push(Finding {
                    kind: "unreadable".into(),
                    what: "lox could not read it as a config".into(),
                    ..base
                });
                continue;
            };
            let mut of_file = Vec::new();
            for x in list {
                let (Some(rule), Some(object), Some(attr)) =
                    (x["rule"].as_str(), x["object"].as_str(), x["attr"].as_str())
                else {
                    return Err("lox logic scan gave a finding vislox cannot read.".into());
                };
                of_file.push(Finding {
                    kind: rule.into(),
                    what: format!("{object} {attr}"),
                    len: x["len"].as_u64().map(|n| n as usize),
                    ..base.clone()
                });
            }
            locate(&String::from_utf8_lossy(xml), &mut of_file, list);
            found.extend(of_file);
        }
        if found.is_empty() && code != Some(0) {
            return Err(format!(
                "lox logic scan failed ({}) without naming a finding.",
                code.map_or("no exit code".into(), |c| format!("exit {c}"))
            ));
        }
    }
    if found.is_empty() {
        let n = files.len();
        return Ok(Verdict::Clean(format!(
            "{n} config{} checked (lox logic scan)",
            if n == 1 { "" } else { "s" }
        )));
    }
    found.sort_by(|a, b| {
        (&a.file, &a.entry, a.line, &a.commit).cmp(&(&b.file, &b.entry, b.line, &b.commit))
    });
    Ok(Verdict::Found(found))
}

/// Each version of a file that may hold a config, in the commits `HEAD` has
/// and no remote-tracking branch has: added, changed or copied there. The
/// newest commit first; a version in several is named by the newest.
async fn candidates(top: &Path) -> Result<Vec<Candidate>, String> {
    // What the push remote has, as far as this repository knows: its
    // remote-tracking branches; with no push remote known, every remote's.
    let branch = git(top, &["symbolic-ref", "-q", "HEAD"])
        .await
        .unwrap_or_default();
    let remote = git(
        top,
        &["for-each-ref", "--format=%(push:remotename)", branch.trim()],
    )
    .await
    .unwrap_or_default();
    let remote = remote.trim();
    let has = if remote.is_empty() || remote.contains(['*', '?', '[']) {
        "--remotes".to_string()
    } else {
        format!("--remotes={remote}")
    };
    let list = git(top, &["rev-list", "--parents", "HEAD", "--not", &has]).await?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in list.lines() {
        let mut ids = line.split_whitespace();
        let Some(commit) = ids.next() else { continue };
        let first_parent = ids.next();
        // Against the first parent: a merge's own changes too (and what it
        // brings in from a side branch, which is scanned once more).
        let diff = match first_parent {
            Some(p) => {
                git_raw(
                    top,
                    &[
                        "diff-tree",
                        "-r",
                        "-z",
                        "--no-renames",
                        "--no-abbrev",
                        p,
                        commit,
                    ],
                )
                .await?
            }
            None => {
                git_raw(
                    top,
                    &[
                        "diff-tree",
                        "-r",
                        "-z",
                        "--no-renames",
                        "--no-abbrev",
                        "--no-commit-id",
                        "--root",
                        commit,
                    ],
                )
                .await?
            }
        };
        for (blob, path) in changed_files(&diff) {
            if is_config_name(&path) && seen.insert((path.clone(), blob.clone())) {
                out.push(Candidate {
                    path,
                    blob,
                    commit: commit[..10.min(commit.len())].to_string(),
                });
            }
        }
    }
    Ok(out)
}

/// A name that may hold a config: `.Loxone`, `.LoxCC`, or a `.zip` backup.
fn is_config_name(path: &str) -> bool {
    let l = path.to_lowercase();
    l.ends_with(".loxone") || l.ends_with(".loxcc") || l.ends_with(".zip")
}

/// The files a `git diff-tree -r -z --no-renames` adds or changes (not
/// deletes; not links or submodules), with their new blob id.
fn changed_files(raw: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut parts = raw.split(|&b| b == 0);
    while let Some(head) = parts.next() {
        let head = String::from_utf8_lossy(head);
        let Some(meta) = head.trim_start_matches(['\n', '\r']).strip_prefix(':') else {
            continue;
        };
        let Some(path) = parts.next() else { break };
        let f: Vec<&str> = meta.split(' ').collect();
        let [_, mode, _, blob, status] = f[..] else {
            continue;
        };
        if status.starts_with('D') || mode == "120000" || mode == "160000" {
            continue;
        }
        out.push((blob.to_string(), String::from_utf8_lossy(path).into_owned()));
    }
    out
}

/// The blob `HEAD` has for each candidate's path.
async fn head_blobs(top: &Path, cands: &[Candidate]) -> BTreeMap<String, String> {
    let paths: BTreeSet<&str> = cands.iter().map(|c| c.path.as_str()).collect();
    let mut m = BTreeMap::new();
    let Ok(raw) = git_raw(top, &["ls-tree", "-r", "-z", "--full-tree", "HEAD"]).await else {
        return m;
    };
    for rec in raw.split(|&b| b == 0) {
        let rec = String::from_utf8_lossy(rec);
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        if paths.contains(path)
            && let Some(blob) = meta.split(' ').nth(2)
        {
            m.insert(path.to_string(), blob.to_string());
        }
    }
    m
}

/// Find the line of each finding's element in the config's text: lox lists
/// them in the document's order (each element's attributes in order), so the
/// next line, from the last one found, that is an element of that type and
/// sets that attribute. Not found: no line.
fn locate(text: &str, found: &mut [Finding], raw: &[Value]) {
    let lines: Vec<&str> = text.lines().collect();
    let mut from = 0;
    for (f, x) in found.iter_mut().zip(raw) {
        let object = x["object"].as_str().unwrap_or_default();
        let attr = x["attr"].as_str().unwrap_or_default();
        // `User #2`, `LoxLIVE "Miniserver"`, `Document`.
        let ty = object.split([' ', '"']).next().unwrap_or_default();
        let set = format!(" {attr}=\"");
        let typed = format!("Type=\"{ty}\"");
        let tag = format!("<{ty} ");
        let at = (from..lines.len()).find(|&i| {
            let l = lines[i];
            l.contains(&set) && (l.contains(&typed) || l.trim_start().starts_with(&tag))
        });
        if let Some(i) = at {
            f.line = Some(i + 1);
            from = i;
        }
    }
}

/// What to do about findings, for the message and the browser.
pub fn advice(found: &[Finding]) -> String {
    let now: BTreeSet<&str> = found
        .iter()
        .filter(|f| f.now)
        .map(|f| f.file.as_str())
        .collect();
    let earlier = found.iter().any(|f| !f.now);
    let mut s = String::new();
    if !now.is_empty() {
        s.push_str(
            "Keep the file out of git, then push again (in the repository's top directory):\n",
        );
        for f in &now {
            s.push_str(&format!("  git rm --cached -- {}\n", shell_quote(f)));
        }
        s.push_str("  add each to .gitignore:");
        for f in &now {
            s.push_str(&format!(" /{f}"));
        }
        s.push('\n');
        s.push_str(
            "  git add .gitignore && git commit --amend --no-edit   (if the last commit added it; \
             for an earlier one, change the commits not pushed yet: git rebase -i)\n",
        );
    }
    if earlier {
        s.push_str(
            "A file only in an earlier commit (deleted since) is still pushed with it: change the \
             commits not pushed yet (git rebase -i) so that none has it.\n",
        );
    }
    s.push_str(
        "lox lists where the data is, never its value. It has no allowlist: a file that must be in \
         git must not hold personal data (lox logic scan --redact writes a copy without it).",
    );
    s
}

fn shell_quote(s: &str) -> String {
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The first JSON value in lox's standard output.
fn first_json(out: &str) -> Option<Value> {
    let i = out.find('{')?;
    serde_json::Deserializer::from_str(&out[i..])
        .into_iter::<Value>()
        .next()?
        .ok()
}

/// A git command's raw standard output (for `-z`).
async fn git_raw(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("git {} took too long", args[0]))?
    .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "git {}: {}",
            args[0],
            last_lines(&String::from_utf8_lossy(&out.stderr), 2)
        ))
    }
}

/// Run a command: its exit code, standard output and standard error.
async fn run_status(
    cmd: &Path,
    args: &[&str],
    dir: &Path,
    limit: Duration,
) -> Result<(Option<i32>, String, String), String> {
    let out = tokio::time::timeout(
        limit,
        tokio::process::Command::new(cmd)
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        format!(
            "lox logic scan took longer than {} s, so vislox cannot tell what the push would send.",
            limit.as_secs()
        )
    })?
    .map_err(|e| format!("cannot run {}: {e}", cmd.display()))?;
    Ok((
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// Only this user may read the scratch directory: it holds configs.
fn private(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, cmd: &str) {
        let ok = std::process::Command::new("sh")
            .args(["-c", cmd])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            ok.status.success(),
            "{cmd}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }

    /// A repository with a bare remote it has pushed to, and a stand-in lox
    /// whose `logic scan --json` answers as lox does: a `mail` finding for a
    /// file with an `Ema="…"`, `hardware-serial` for a `Serial="…"`, nothing
    /// for a file it cannot read (`BROKEN`, said on standard error), exit 1
    /// when it found or missed anything. `nosan` in the directory: a lox
    /// without scan; `garbage`: an answer that is no JSON.
    #[cfg(unix)]
    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("repo");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("README"), "x").unwrap();
        sh(tmp.path(), "git init -q --bare remote.git");
        sh(
            &d,
            "git init -q && git config user.email t@t && git config user.name t \
             && git add -A && git commit -qm init && git remote add origin ../remote.git \
             && git push -q -u origin HEAD",
        );
        let lox = tmp.path().join("lox");
        let flags = tmp.path().display().to_string();
        std::fs::write(
            &lox,
            format!(
                r#"#!/bin/sh
echo "$*" >> {flags}/calls.log
[ -e {flags}/noscan ] && {{ echo 'unknown command `scan`. Run `lox logic help`.' >&2; exit 2; }}
[ -e {flags}/garbage ] && {{ echo 'all fine'; exit 0; }}
[ "$1 $2 $3" = "logic scan --json" ] || {{ echo "unexpected: $*" >&2; exit 2; }}
shift 3
code=0; sep=; printf '{{'
for f in "$@"; do
  if grep -q BROKEN "$f"; then echo "$f: not scanned: bad XML" >&2; code=1; continue; fi
  printf '%s"%s": [' "$sep" "$f"; sep=,; s=
  if grep -q 'Ema="[^"]' "$f"; then printf '{{"rule":"mail","object":"User #1","attr":"Ema","len":17}}'; s=,; code=1; fi
  if grep -q 'Serial="[^"]' "$f"; then printf '%s{{"rule":"hardware-serial","object":"LoxLIVE \\"Miniserver\\"","attr":"Serial","len":12}}' "$s"; code=1; fi
  printf ']'
done
echo '}}'
exit $code
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
        (tmp, d, lox)
    }

    const USERS: &str = "<?xml version=\"1.0\"?>\n<C Type=\"Document\" Title=\"x\">\n\t<C Type=\"User\" U=\"1\" Title=\"alice\" Ema=\"alice@example.com\"/>\n\t<C Type=\"LoxLIVE\" Title=\"Miniserver\" Serial=\"504F94000001\"/>\n</C>\n";

    fn calls(tmp: &tempfile::TempDir) -> String {
        std::fs::read_to_string(tmp.path().join("calls.log")).unwrap_or_default()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_push_without_a_config_needs_no_lox() {
        let (tmp, d, lox) = repo();
        sh(
            &d,
            "echo hi > notes.txt && git add -A && git commit -qm notes",
        );
        let v = before_push(&d, &lox).await;
        assert_eq!(
            v,
            Verdict::Clean("no config file in the commits to push".into())
        );
        assert_eq!(calls(&tmp), "");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_config_with_personal_data_is_found_by_line_without_its_value() {
        let (tmp, d, lox) = repo();
        std::fs::create_dir_all(d.join("sub dir")).unwrap();
        std::fs::write(d.join("sub dir/live.Loxone"), USERS).unwrap();
        sh(&d, "git add -A && git commit -qm live");
        let v = before_push(&d.join("sub dir"), &lox).await;
        let Verdict::Found(f) = v else {
            panic!("not found: {v:?}");
        };
        assert_eq!(f.len(), 2, "{f:?}");
        assert_eq!(f[0].file, "sub dir/live.Loxone");
        assert_eq!(
            (f[0].kind.as_str(), f[0].what.as_str(), f[0].line, f[0].len),
            ("mail", "User #1 Ema", Some(3), Some(17))
        );
        assert_eq!(
            (f[1].kind.as_str(), f[1].what.as_str(), f[1].line),
            ("hardware-serial", "LoxLIVE \"Miniserver\" Serial", Some(4))
        );
        assert!(f[0].now);
        assert_eq!(
            f[0].text(),
            "sub dir/live.Loxone:3: mail — User #1 Ema (17 chars)"
        );
        // Never the value: not in what the browser gets, nor in the advice.
        let shown = serde_json::to_string(&f).unwrap() + &advice(&f);
        assert!(
            !shown.contains("alice@example.com") && !shown.contains("504F94000001"),
            "{shown}"
        );
        assert!(
            advice(&f).contains("git rm --cached -- 'sub dir/live.Loxone'"),
            "{}",
            advice(&f)
        );
        // lox reads a copy in a scratch directory, by a name of vislox's.
        assert!(
            calls(&tmp).starts_with("logic scan --json c0.Loxone"),
            "{}",
            calls(&tmp)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn every_version_the_push_sends_is_scanned_not_what_is_pushed_already() {
        let (_tmp, d, lox) = repo();
        // Pushed before: not sent again.
        std::fs::write(d.join("old.Loxone"), USERS).unwrap();
        sh(&d, "git add -A && git commit -qm old && git push -q");
        assert_eq!(
            before_push(&d, &lox).await,
            Verdict::Clean("no config file in the commits to push".into())
        );
        // Added in one commit and deleted in the next: still in the push.
        std::fs::write(d.join("live.Loxone"), USERS).unwrap();
        sh(
            &d,
            "git add -A && git commit -qm add && git rm -q live.Loxone && git commit -qm rm",
        );
        let added = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "--short=10", "HEAD~1"])
                .current_dir(&d)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let Verdict::Found(f) = before_push(&d, &lox).await else {
            panic!("not found");
        };
        assert_eq!(f.len(), 2);
        assert!(!f[0].now);
        assert_eq!(f[0].commit, added.trim());
        assert!(
            f[0].text()
                .ends_with(&format!("[only in commit {}, deleted since]", added.trim()))
        );
        assert!(advice(&f).contains("git rebase -i"));
        // A config without personal data passes.
        sh(&d, "git reset -q --hard HEAD~2");
        std::fs::write(d.join("clean.Loxone"), "<C Type=\"Document\"/>\n").unwrap();
        sh(&d, "git add -A && git commit -qm clean");
        assert_eq!(
            before_push(&d, &lox).await,
            Verdict::Clean("1 config checked (lox logic scan)".into())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn what_another_remote_has_is_still_sent_to_the_push_remote() {
        let (tmp, d, lox) = repo();
        std::fs::write(d.join("live.Loxone"), USERS).unwrap();
        sh(tmp.path(), "git init -q --bare other.git");
        sh(
            &d,
            "git add -A && git commit -qm live && git remote add other ../other.git \
             && git push -q other HEAD",
        );
        assert!(matches!(before_push(&d, &lox).await, Verdict::Found(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn it_fails_closed() {
        let (tmp, d, lox) = repo();
        std::fs::write(d.join("a.Loxone"), "BROKEN").unwrap();
        sh(&d, "git add -A && git commit -qm a");
        // A file lox cannot read is a finding, without lox's words.
        let Verdict::Found(f) = before_push(&d, &lox).await else {
            panic!("not found");
        };
        assert_eq!(
            (f[0].kind.as_str(), f[0].what.as_str(), f[0].line),
            ("unreadable", "lox could not read it as a config", None)
        );
        // A lox without scan, and an answer that is no JSON: cannot tell.
        std::fs::write(tmp.path().join("noscan"), "").unwrap();
        let Verdict::CannotScan(why) = before_push(&d, &lox).await else {
            panic!("scanned");
        };
        assert!(why.contains("has no `lox logic scan`"), "{why}");
        std::fs::remove_file(tmp.path().join("noscan")).unwrap();
        std::fs::write(tmp.path().join("garbage"), "").unwrap();
        let Verdict::CannotScan(why) = before_push(&d, &lox).await else {
            panic!("scanned");
        };
        assert!(why.contains("no answer vislox can read (exit 0)"), "{why}");
        // No lox at all.
        let Verdict::CannotScan(why) = before_push(&d, &tmp.path().join("none")).await else {
            panic!("scanned");
        };
        assert!(why.contains("cannot run"), "{why}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_backup_zip_is_scanned_by_its_program() {
        use std::io::Write;
        let (_tmp, d, lox) = repo();
        let backup = |xml: &str| {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            w.start_file("sps0.LoxCC", zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(&loxone_config::archive::compress_loxcc(xml.as_bytes()).unwrap())
                .unwrap();
            w.finish().unwrap().into_inner()
        };
        std::fs::write(d.join("sps_1.zip"), backup(USERS)).unwrap();
        // Not a Miniserver backup: not a config.
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"hi").unwrap();
        std::fs::write(d.join("docs.zip"), w.finish().unwrap().into_inner()).unwrap();
        // A .zip that is none: cannot be read.
        std::fs::write(d.join("odd.zip"), "not a zip").unwrap();
        sh(&d, "git add -A && git commit -qm backups");
        let Verdict::Found(f) = before_push(&d, &lox).await else {
            panic!("not found");
        };
        let at: Vec<String> = f.iter().map(|f| format!("{} {}", f.at(), f.kind)).collect();
        assert_eq!(
            at,
            [
                "odd.zip unreadable",
                "sps_1.zip › sps0.LoxCC:3 mail",
                "sps_1.zip › sps0.LoxCC:4 hardware-serial"
            ]
        );
    }

    /// The scan of the real lox, when it is installed: its JSON is what
    /// vislox reads.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_real_lox_scan_is_read() {
        let lox = Path::new("lox");
        let has_scan = std::process::Command::new(lox)
            .args(["logic", "scan", "--help"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--json"));
        if !has_scan {
            return;
        }
        let (_tmp, d, _) = repo();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hardware.Loxone");
        std::fs::copy(&fixture, d.join("hardware.Loxone")).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/house.Loxone"),
            d.join("house.Loxone"),
        )
        .unwrap();
        sh(&d, "git add -A && git commit -qm fixtures");
        let Verdict::Found(f) = before_push(&d, lox).await else {
            panic!("not found");
        };
        assert_eq!(f.len(), 6, "{f:?}");
        assert!(f.iter().all(|f| f.file == "hardware.Loxone"
            && f.kind == "hardware-serial"
            && f.line.is_some()));
        let text = std::fs::read_to_string(&fixture).unwrap();
        let l = text.lines().nth(f[0].line.unwrap() - 1).unwrap();
        assert!(
            l.contains("Type=\"LoxLIVE\"") && l.contains(" Serial=\""),
            "{l}"
        );
    }

    #[test]
    fn diff_tree_records() {
        let raw = b":000000 100644 0000 aaaa A\0new.Loxone\0:100644 000000 bbbb 0000 D\0gone.Loxone\0:120000 120000 c d M\0link.Loxone\0:100644 100644 e ffff M\0a b.zip\0";
        assert_eq!(
            changed_files(raw),
            [
                ("aaaa".to_string(), "new.Loxone".to_string()),
                ("ffff".to_string(), "a b.zip".to_string())
            ]
        );
    }
}
