//! Personal data before a git push (#71). A real config holds the
//! installation's address, its users' names, mail addresses, phone numbers
//! and access codes, credentials and hardware serials. When a repository's
//! `.gitignore` misses one (a downloaded `live.Loxone`, a backup outside
//! `backups/`), a commit takes it along, and `git push` would send it to the
//! remote. So every push vislox makes (Apply, Roll back, Sync, and *Push to
//! git*) goes through `Actions::push_git`, which pushes exactly what
//! [`before_push`] read, and only when lox found nothing:
//!
//! - **Where a push goes** ([`target`]): the branch's upstream, its remote and
//!   branch there. Without one vislox refuses: it never guesses a remote.
//!   What the remote has is what its remote-tracking branches have, and a
//!   commit is not pushed while the upstream's tracking branch lacks it
//!   ([`unpushed`]): the same definition for the scan, *Push to git* and the
//!   check on start.
//! - **What a push sends**: exactly the objects of the scanned commit that no
//!   remote-tracking branch of that remote has (`git rev-list --objects <sha>
//!   --not <its tracking branches>`), as `git push` packs them. Of those, each
//!   file that may hold a config: a `.Loxone`, a `.LoxCC` and a Miniserver
//!   backup `.zip` (its `sps0.LoxCC`). Every version, not only the last: a file
//!   added in one commit and deleted in the next is still sent. A version the
//!   remote has already (a merge of what it has) is not sent, so not read.
//!   Then `git push --no-follow-tags <remote> <sha>:<branch>`: that commit,
//!   no other branch or tag, whatever `push.default`, a push refspec or
//!   `push.followTags` say, and no commit made after the scan.
//! - **lox decides** (`lox logic scan --json`): it lists where personal data
//!   is, the element and attribute and the value's length, never the value.
//!   vislox adds the line it finds that element and attribute on, and never
//!   reads or keeps a value: nothing it logs, sends to the browser or says in
//!   an error can carry one. lox reads copies in a scratch directory only this
//!   user can open (made so, not changed to it), removed afterwards.
//! - **Fail closed**: a file lox cannot read, a lox without `logic scan`, an
//!   answer vislox cannot read, no upstream, or a scan over [`SCAN_LIMIT`]
//!   block the push as findings do. Only a push with no config file in it goes
//!   ahead without lox.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// `lox logic scan` reads every config of a push in one run.
const LOX_TIMEOUT: Duration = Duration::from_secs(120);
/// The whole scan: git's reads, the copies and lox.
pub const SCAN_LIMIT: Duration = Duration::from_secs(300);

/// Where lox found personal data in what a push would send, and where
/// vislox found its element. Never its value.
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
    /// Still the file as the pushed commit has it; else only in an earlier
    /// commit of the push.
    pub now: bool,
    /// For a version deleted since: the newest commit of the push that
    /// added or changed it to this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
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
        let when = match (self.now, &self.commit) {
            (true, _) => String::new(),
            (false, Some(c)) => format!(" [only in commit {c}, deleted since]"),
            (false, None) => " [only in an earlier commit, deleted since]".into(),
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

/// Where the branch checked out is pushed: its upstream.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// The remote's name.
    pub remote: String,
    /// The branch there (`refs/heads/main`).
    pub remote_ref: String,
    /// Its remote-tracking branch (`refs/remotes/origin/main`).
    pub tracking: String,
}

/// The branch's upstream, or why vislox will not push.
pub async fn target(dir: &Path) -> Result<Target, String> {
    let branch = git_raw(dir, &["symbolic-ref", "-q", "HEAD"])
        .await
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .unwrap_or_default();
    if branch.is_empty() {
        return Err(
            "HEAD is on no branch, so vislox does not know where to push. Check the \
                    branch out, then push again."
                .into(),
        );
    }
    let raw = git_raw(
        dir,
        &[
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
            &branch,
        ],
    )
    .await?;
    let line = String::from_utf8_lossy(&raw);
    let f: Vec<&str> = line.trim_end_matches('\n').split('\0').collect();
    let short = branch.trim_start_matches("refs/heads/");
    match f[..] {
        [remote, remote_ref, tracking]
            if !remote.is_empty() && !remote_ref.is_empty() && !tracking.is_empty() =>
        {
            if remote.contains(['*', '?', '[', '\\']) {
                return Err(format!("vislox does not push to a remote named {remote}."));
            }
            Ok(Target {
                remote: remote.into(),
                remote_ref: remote_ref.into(),
                tracking: tracking.into(),
            })
        }
        _ => Err(format!(
            "the branch {short} has no upstream, so vislox does not know where to push. Set one \
             once (git branch --set-upstream-to=<remote>/<branch>, or git push -u after checking \
             what it sends), then push again."
        )),
    }
}

/// How many commits `HEAD` has that the upstream's tracking branch has not.
pub async fn unpushed(dir: &Path, t: &Target) -> Result<usize, String> {
    let has = git_raw(dir, &["rev-parse", "-q", "--verify", &t.tracking]).await;
    if has.is_err() {
        return Err(format!(
            "{} is not fetched, so vislox cannot tell what the remote has. Fetch it (git fetch {}), \
             then push again.",
            t.tracking, t.remote
        ));
    }
    let n = git_raw(dir, &["rev-list", "--count", "HEAD", "--not", &t.tracking]).await?;
    String::from_utf8_lossy(&n)
        .trim()
        .parse()
        .map_err(|_| "git rev-list gave no count".into())
}

/// The commit `HEAD` is now.
pub async fn head(dir: &Path) -> Result<String, String> {
    let h = git_raw(dir, &["rev-parse", "--verify", "HEAD"]).await?;
    Ok(String::from_utf8_lossy(&h).trim().to_string())
}

/// What `git push <t.remote> <sha>:<t.remote_ref>` would send that may hold
/// a config, scanned by lox, within [`SCAN_LIMIT`].
pub async fn before_push(dir: &Path, lox: &Path, t: &Target, sha: &str) -> Verdict {
    match tokio::time::timeout(SCAN_LIMIT, scan_push(dir, lox, t, sha)).await {
        Ok(Ok(v)) => v,
        Ok(Err(why)) => Verdict::CannotScan(why),
        Err(_) => Verdict::CannotScan(format!(
            "the scan took longer than {} minutes.",
            SCAN_LIMIT.as_secs() / 60
        )),
    }
}

/// A config file version the push sends.
struct Candidate {
    path: String,
    blob: String,
}

async fn scan_push(dir: &Path, lox: &Path, t: &Target, sha: &str) -> Result<Verdict, String> {
    let top = git_raw(dir, &["rev-parse", "--show-toplevel"]).await?;
    let top = PathBuf::from(String::from_utf8_lossy(&top).trim());
    // What the remote has: each of its remote-tracking branches.
    let refs = git_raw(
        &top,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            &format!("refs/remotes/{}/", t.remote),
        ],
    )
    .await?;
    let has: Vec<String> = String::from_utf8_lossy(&refs)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let cands = candidates(&top, sha, &has).await?;
    if cands.is_empty() {
        return Ok(Verdict::Clean(
            "no config file in what the push sends".into(),
        ));
    }
    let now = blobs_at(&top, sha).await?;
    let specs: Vec<String> = cands.iter().map(|c| c.blob.clone()).collect();
    let objects = git_typed_objects(&top, &specs).await?;
    let scratch = private_dir("vislox-scan")?;
    let _gone = RemoveDir(scratch.clone());
    let mut found = Vec::new();
    // Each config to scan: its name in the scratch directory, its candidate,
    // the zip entry, and its text (to find lines in).
    let mut files: Vec<(String, usize, Option<String>, Vec<u8>)> = Vec::new();
    for (i, (c, o)) in cands.iter().zip(objects).enumerate() {
        let finding = |what: &str| Finding {
            file: c.path.clone(),
            entry: None,
            line: None,
            kind: "unreadable".into(),
            what: what.into(),
            len: None,
            now: now.get(&c.path) == Some(&c.blob),
            commit: None,
        };
        let b = match o {
            Some((kind, b)) if kind == "blob" => b,
            // A directory named like a config: its files are listed apart.
            Some(_) => continue,
            None => {
                found.push(finding("git has no such object"));
                continue;
            }
        };
        let lower = c.path.to_lowercase();
        let (entry, xml) = if lower.ends_with(".zip") {
            if !loxone_config::archive::is_zip(&b) {
                found.push(finding("not a zip archive: lox cannot read it"));
                continue;
            }
            match loxone_config::archive::extract_and_decompress(&b) {
                Ok(x) => (Some("sps0.LoxCC".to_string()), x),
                // Not a Miniserver backup: no program in it.
                Err(e) if e.to_string().contains("does not contain sps0.LoxCC") => continue,
                Err(_) => {
                    found.push(finding("a backup whose program cannot be read"));
                    continue;
                }
            }
        } else if lower.ends_with(".loxcc") {
            match loxone_config::archive::decompress_loxcc(&b) {
                Ok(x) => (None, x),
                Err(_) => {
                    found.push(finding("a program file that cannot be read"));
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
        let (code, stdout, stderr) = run_status(lox, &args, &scratch, LOX_TIMEOUT)
            .await
            .map_err(|e| format!("lox logic scan: {e}"))?;
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
                    code.map_or("no exit code".into(), |c| format!("exit {c}"))
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
                now: now.get(&c.path) == Some(&c.blob),
                commit: None,
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
    // A version deleted since: the commit of the push that brought it in.
    let mut commits: BTreeMap<String, Option<String>> = BTreeMap::new();
    for f in found.iter().filter(|f| !f.now) {
        let Some(c) = cands.iter().find(|c| c.path == f.file) else {
            continue;
        };
        if commits.contains_key(&c.blob) {
            continue;
        }
        let find = format!("--find-object={}", c.blob);
        // The commit that brought it in, not the one that deleted it.
        let mut args = vec![
            "log",
            "-1",
            "--format=%h",
            "--diff-filter=d",
            &find,
            sha,
            "--not",
        ];
        args.extend(has.iter().map(String::as_str));
        let c_of = git_raw(&top, &args)
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o).trim().to_string())
            .filter(|s| !s.is_empty());
        commits.insert(c.blob.clone(), c_of);
    }
    for f in found.iter_mut().filter(|f| !f.now) {
        if let Some(c) = cands.iter().find(|c| c.path == f.file) {
            f.commit = commits.get(&c.blob).cloned().flatten();
        }
    }
    found.sort_by(|a, b| (&a.file, &a.entry, a.line).cmp(&(&b.file, &b.entry, b.line)));
    Ok(Verdict::Found(found))
}

/// Each object `git push` would send for `sha` that may be a config file:
/// `git rev-list --objects <sha> --not <has>`, by its path. One version
/// under two paths is read once.
async fn candidates(top: &Path, sha: &str, has: &[String]) -> Result<Vec<Candidate>, String> {
    let mut args = vec!["rev-list", "--objects", sha, "--not"];
    args.extend(has.iter().map(String::as_str));
    let raw = git_raw(top, &args).await?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in String::from_utf8_lossy(&raw).lines() {
        // `<id> <path>` for trees and files; a commit has no path.
        let Some((id, path)) = line.split_once(' ') else {
            continue;
        };
        if is_config_name(path) && seen.insert(id.to_string()) {
            out.push(Candidate {
                path: path.to_string(),
                blob: id.to_string(),
            });
        }
    }
    Ok(out)
}

/// A name that may hold a config: `.Loxone`, `.LoxCC`, or a `.zip` backup.
fn is_config_name(path: &str) -> bool {
    let l = path.to_lowercase();
    l.ends_with(".loxone") || l.ends_with(".loxcc") || l.ends_with(".zip")
}

/// The blob `sha` has at each path.
async fn blobs_at(top: &Path, sha: &str) -> Result<BTreeMap<String, String>, String> {
    let raw = git_raw(top, &["ls-tree", "-r", "-z", "--full-tree", sha]).await?;
    let mut m = BTreeMap::new();
    for rec in raw.split(|&b| b == 0) {
        let rec = String::from_utf8_lossy(rec);
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        if is_config_name(path)
            && let Some(blob) = meta.split(' ').nth(2)
        {
            m.insert(path.to_string(), blob.to_string());
        }
    }
    Ok(m)
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

/// A git command's standard output only (git's warnings on standard error
/// are never read as its answer).
pub async fn git_raw(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
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

/// A new scratch directory only this user can open: made with that mode
/// (the umask can only narrow it), never one that exists. Else an error.
fn private_dir(prefix: &str) -> Result<PathBuf, String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}-{n}",
        std::process::id(),
        now_nanos()
    ));
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(&d)
        .map_err(|e| format!("cannot create a private scratch directory: {e}"))?;
    Ok(d)
}

/// A new file only this user can read, made so; never an existing one.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o
        .open(path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    f.write_all(bytes)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a push of `HEAD` to its upstream would send, scanned.
    async fn scan(d: &Path, lox: &Path) -> Verdict {
        let t = target(d).await.unwrap();
        let sha = head(d).await.unwrap();
        before_push(d, lox, &t, &sha).await
    }

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
        let v = scan(&d, &lox).await;
        assert_eq!(
            v,
            Verdict::Clean("no config file in what the push sends".into())
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
        let v = scan(&d.join("sub dir"), &lox).await;
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
            scan(&d, &lox).await,
            Verdict::Clean("no config file in what the push sends".into())
        );
        // Added in one commit and deleted in the next: still in the push (other
        // bytes: the remote has old.Loxone's already, so a copy of it is not sent).
        std::fs::write(d.join("live.Loxone"), format!("{USERS}<!-- live -->\n")).unwrap();
        sh(
            &d,
            "git add -A && git commit -qm add && git rm -q live.Loxone && git commit -qm rm",
        );
        let added = String::from_utf8(
            std::process::Command::new("git")
                .args(["log", "-1", "--format=%h", "HEAD~1"])
                .current_dir(&d)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let Verdict::Found(f) = scan(&d, &lox).await else {
            panic!("not found");
        };
        assert_eq!(f.len(), 2);
        assert!(!f[0].now);
        assert_eq!(f[0].commit.as_deref(), Some(added.trim()));
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
            scan(&d, &lox).await,
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
        assert!(matches!(scan(&d, &lox).await, Verdict::Found(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_merge_of_what_the_remote_has_sends_none_of_it() {
        let (tmp, d, lox) = repo();
        // Someone else pushed a config with personal data (it is on the remote already).
        sh(
            tmp.path(),
            "git clone -q remote.git other && cd other && git config user.email t@t \
             && git config user.name t",
        );
        let other = tmp.path().join("other");
        std::fs::write(other.join("live.Loxone"), USERS).unwrap();
        sh(&other, "git add -A && git commit -qm theirs && git push -q");
        // Here: a commit of ours, then a merge of theirs. Against its first parent, the
        // merge adds live.Loxone; but the push sends none of it.
        sh(
            &d,
            "echo mine > mine.txt && git add -A && git commit -qm mine && git fetch -q \
             && git merge -q --no-edit @{u}",
        );
        assert_eq!(
            scan(&d, &lox).await,
            Verdict::Clean("no config file in what the push sends".into())
        );
        assert_eq!(calls(&tmp), "");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_git_warning_is_not_read_as_git_s_answer() {
        let (_tmp, d, lox) = repo();
        // Every git command warns about this on standard error.
        sh(&d, "git config core.fsyncObjectFiles true");
        std::fs::write(d.join("live.Loxone"), USERS).unwrap();
        sh(&d, "git add -A && git commit -qm live");
        let v = scan(&d, &lox).await;
        let Verdict::Found(f) = v else {
            panic!("not found: {v:?}");
        };
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].file, "live.Loxone");
        assert!(f[0].now);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn without_an_upstream_nothing_is_guessed() {
        let (_tmp, d, _) = repo();
        sh(&d, "git checkout -q -b loose");
        let e = target(&d).await.unwrap_err();
        assert!(e.starts_with("the branch loose has no upstream"), "{e}");
        sh(&d, "git checkout -q --detach");
        let e = target(&d).await.unwrap_err();
        assert!(e.starts_with("HEAD is on no branch"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn the_scratch_copies_are_private_from_the_start() {
        use std::os::unix::fs::PermissionsExt;
        let d = private_dir("vislox-scan-test").unwrap();
        let _gone = RemoveDir(d.clone());
        assert_eq!(
            std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let f = d.join("c0.Loxone");
        write_private(&f, b"x").unwrap();
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Never into a file that is there already (someone else's).
        assert!(write_private(&f, b"y").is_err());
        assert_eq!(std::fs::read(&f).unwrap(), b"x");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn it_fails_closed() {
        let (tmp, d, lox) = repo();
        std::fs::write(d.join("a.Loxone"), "BROKEN").unwrap();
        sh(&d, "git add -A && git commit -qm a");
        // A file lox cannot read is a finding, without lox's words.
        let Verdict::Found(f) = scan(&d, &lox).await else {
            panic!("not found");
        };
        assert_eq!(
            (f[0].kind.as_str(), f[0].what.as_str(), f[0].line),
            ("unreadable", "lox could not read it as a config", None)
        );
        // A lox without scan, and an answer that is no JSON: cannot tell.
        std::fs::write(tmp.path().join("noscan"), "").unwrap();
        let Verdict::CannotScan(why) = scan(&d, &lox).await else {
            panic!("scanned");
        };
        assert!(why.contains("has no `lox logic scan`"), "{why}");
        std::fs::remove_file(tmp.path().join("noscan")).unwrap();
        std::fs::write(tmp.path().join("garbage"), "").unwrap();
        let Verdict::CannotScan(why) = scan(&d, &lox).await else {
            panic!("scanned");
        };
        assert!(why.contains("no answer vislox can read (exit 0)"), "{why}");
        // No lox at all.
        let Verdict::CannotScan(why) = scan(&d, &tmp.path().join("none")).await else {
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
        let Verdict::Found(f) = scan(&d, &lox).await else {
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
        let Verdict::Found(f) = scan(&d, lox).await else {
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
}
