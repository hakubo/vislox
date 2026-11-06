//! The revisions vislox can show: compiled (pending) configs, live
//! snapshots of the watched file taken while vislox runs, and the file's git
//! history. Contents are keyed by their git blob id, so a live snapshot and
//! the commit holding the same bytes are recognisably the same.
//!
//! The Miniserver's own backups (`/prog/sps_<ver>_<stamp>.zip`, see
//! `backups.rs`) join by the same key: one with the bytes of the Pull that
//! took it shows as that Pull, the others as the saves between two Pulls.

use chrono::TimeZone;
use chrono_tz::Tz;
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default, Serialize)]
pub struct Revision {
    pub id: String,
    /// `pending`, `live`, `git` or `backup`.
    pub kind: &'static str,
    pub label: String,
    pub detail: String,
    /// Unix seconds.
    pub time: i64,
    /// Git blob id of the content.
    pub blob: String,
    /// The revision this one is compared with, if any.
    pub base: Option<String>,
    /// This revision is what the file on disk holds now.
    pub current: bool,
    /// The Miniserver backups with the same program (file names, oldest
    /// first): they show as this revision.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub backups: Vec<String>,
    /// A backup: the Pull (git revision) before it was saved, and the one
    /// after, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// A backup whose program another revision has too (not the Pulls
    /// around it: a save that undid changes): that revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_as: Option<String>,
}

/// What the History shows, see [`History::lists`].
#[derive(Debug, Default)]
pub struct Lists {
    /// Newest first.
    pub pending: Vec<Revision>,
    /// Newest first.
    pub history: Vec<Revision>,
    /// The Miniserver backups that show on their own, newest first.
    pub backups: Vec<Revision>,
    by_id: HashMap<String, Revision>,
}

/// A Miniserver backup that vislox downloaded, cached as its program
/// (`.Loxone`, the bytes `lox config pull` would commit).
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    /// `sps_<ver>_<stamp>.zip`
    pub file: String,
    /// `<ver>` of the file name: the config's version, as in the `(v…)` of a
    /// `lox config pull` commit.
    pub version: u32,
    /// When it was saved, `YYYYMMDDHHMMSS` in the Miniserver's local time.
    pub stamp: String,
    pub blob: String,
    /// The cached program.
    pub path: PathBuf,
    /// The Miniserver still has it (as of the last listing).
    pub on_miniserver: bool,
}

#[derive(Debug, Clone)]
struct GitCommit {
    hash: String,
    time: i64,
    author: String,
    subject: String,
    blob: String,
}

#[derive(Debug, Default)]
pub struct History {
    current_path: PathBuf,
    proposed_path: Option<PathBuf>,
    git: Option<(PathBuf, String)>,
    git_head: Option<String>,
    commits: Vec<GitCommit>,
    /// Oldest first.
    live: Vec<Revision>,
    /// Oldest first.
    pending: Vec<Revision>,
    contents: HashMap<String, Arc<Vec<u8>>>,
    /// The Miniserver's backups that were downloaded, any order.
    backups: Vec<Backup>,
    /// The Miniserver's time zone, which backup stamps are in.
    zone: Option<Tz>,
    /// [`History::lists`] for a version.
    lists: Mutex<Option<(u64, Arc<Lists>)>>,
    /// Bumped on every change, for the UI's event stream.
    pub version: u64,
    pub last_error: Option<String>,
}

pub fn blob_id(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    hex::encode(h.finalize())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn run_git(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

impl History {
    pub fn new(current: PathBuf, proposed: Option<PathBuf>) -> History {
        let git = current.parent().and_then(|dir| {
            let top = run_git(dir, &["rev-parse", "--show-toplevel"])?;
            let top = PathBuf::from(String::from_utf8_lossy(&top).trim());
            let top = top.canonicalize().ok()?;
            let rel = current.strip_prefix(&top).ok()?;
            Some((top, rel.to_string_lossy().replace('\\', "/")))
        });
        let mut h = History {
            current_path: current,
            proposed_path: proposed,
            git,
            ..History::default()
        };
        h.refresh_git();
        h.poll_files();
        h.version = 1;
        h
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        std::iter::once(self.current_path.clone())
            .chain(self.proposed_path.clone())
            .collect()
    }

    pub fn git_root(&self) -> Option<&Path> {
        self.git.as_ref().map(|(r, _)| r.as_path())
    }

    /// Reload the git log when HEAD moved. Returns whether anything changed.
    pub fn refresh_git(&mut self) -> bool {
        let Some((repo, rel)) = self.git.clone() else {
            return false;
        };
        let head = run_git(&repo, &["rev-parse", "HEAD"])
            .map(|o| String::from_utf8_lossy(&o).trim().to_string());
        if head == self.git_head {
            return false;
        }
        self.git_head = head;
        let Some(out) = run_git(
            &repo,
            &[
                "log",
                "-n",
                "300",
                "--format=\x1e%H\x1f%ct\x1f%an\x1f%s",
                "--raw",
                "--no-abbrev",
                "--",
                &rel,
            ],
        ) else {
            return false;
        };
        let text = String::from_utf8_lossy(&out);
        let mut commits = Vec::new();
        for rec in text.split('\x1e').filter(|r| !r.trim().is_empty()) {
            let mut lines = rec.lines();
            let head: Vec<&str> = lines.next().unwrap_or("").split('\x1f').collect();
            if head.len() < 4 {
                continue;
            }
            // `:100644 100644 <old> <new> M\tpath`
            let blob = lines
                .find(|l| l.starts_with(':'))
                .and_then(|l| l.split_whitespace().nth(3))
                .unwrap_or("");
            if blob.is_empty() || blob.chars().all(|c| c == '0') {
                continue; // deleted in this commit
            }
            commits.push(GitCommit {
                hash: head[0].to_string(),
                time: head[1].parse().unwrap_or(0),
                author: head[2].to_string(),
                subject: head[3].to_string(),
                blob: blob.to_string(),
            });
        }
        self.commits = commits;
        self.version += 1;
        true
    }

    /// Re-read the watched files; record a snapshot of each that changed.
    /// Returns whether anything changed.
    pub fn poll_files(&mut self) -> bool {
        let mut changed = false;
        match std::fs::read(&self.current_path) {
            Ok(bytes) => {
                let blob = blob_id(&bytes);
                if self.live.last().map(|r| &r.blob) != Some(&blob) {
                    let first = self.live.is_empty();
                    let n = self.live.len();
                    self.contents.insert(blob.clone(), Arc::new(bytes));
                    self.live.push(Revision {
                        id: format!("live:{n}"),
                        kind: "live",
                        label: if first {
                            "On disk at start".into()
                        } else {
                            "Changed on disk".into()
                        },
                        detail: self.current_path.display().to_string(),
                        time: mtime(&self.current_path).unwrap_or_else(now),
                        blob,
                        ..Revision::default()
                    });
                    changed = true;
                }
            }
            Err(e) => {
                self.last_error = Some(format!("{}: {e}", self.current_path.display()));
            }
        }
        if let Some(path) = self.proposed_path.clone()
            && let Ok(bytes) = std::fs::read(&path)
        {
            let blob = blob_id(&bytes);
            if self.pending.last().map(|r| &r.blob) != Some(&blob) {
                let n = self.pending.len();
                self.contents.insert(blob.clone(), Arc::new(bytes));
                self.pending.push(Revision {
                    id: format!("pending:{n}"),
                    kind: "pending",
                    label: "Compiled, not deployed".into(),
                    detail: path.display().to_string(),
                    time: mtime(&path).unwrap_or_else(now),
                    blob,
                    ..Revision::default()
                });
                changed = true;
            }
        }
        if changed {
            self.version += 1;
        }
        changed
    }

    /// (pending, history), newest first, each with its comparison base.
    pub fn list(&self) -> (Vec<Revision>, Vec<Revision>) {
        let l = self.lists();
        (l.pending.clone(), l.history.clone())
    }

    /// Everything the History shows, worked out once per version.
    pub fn lists(&self) -> Arc<Lists> {
        let mut cache = self.lists.lock().unwrap();
        if let Some((v, l)) = cache.as_ref()
            && *v == self.version
        {
            return l.clone();
        }
        let l = Arc::new(self.compute());
        *cache = Some((self.version, l.clone()));
        l
    }

    fn compute(&self) -> Lists {
        let mut hist: Vec<Revision> = Vec::new();
        let commit_blobs: HashSet<&str> = self.commits.iter().map(|c| c.blob.as_str()).collect();
        // Live snapshots, newest first; one that a commit records is shown
        // as that commit.
        let disk_blob = self.live.last().map(|r| r.blob.clone());
        for r in self.live.iter().rev() {
            let same_as_head = self.commits.first().is_some_and(|c| c.blob == r.blob);
            if same_as_head || (r.id == "live:0" && commit_blobs.contains(r.blob.as_str())) {
                continue;
            }
            if hist.last().is_some_and(|p: &Revision| p.blob == r.blob) {
                continue;
            }
            let mut r = r.clone();
            if self.git.is_some() {
                r.label = if commit_blobs.contains(r.blob.as_str()) {
                    "Changed on disk (same as an older commit)".into()
                } else {
                    "Changed on disk, not committed".into()
                };
            }
            hist.push(r);
        }
        for c in &self.commits {
            if hist.last().is_some_and(|p| p.blob == c.blob) && hist.last().unwrap().kind == "git" {
                continue;
            }
            hist.push(Revision {
                id: format!("git:{}", c.hash),
                kind: "git",
                label: c.subject.clone(),
                detail: format!("{} · {}", &c.hash[..10.min(c.hash.len())], c.author),
                time: c.time,
                blob: c.blob.clone(),
                ..Revision::default()
            });
        }
        for i in 0..hist.len() {
            hist[i].base = hist.get(i + 1).map(|r| r.id.clone());
        }
        let backups = place_backups(&mut hist, &self.backups, self.zone);
        if let Some(first) = hist.first_mut() {
            first.current = Some(&first.blob) == disk_blob.as_ref();
        }
        let head_id = hist.first().map(|r| r.id.clone());
        // A compile that is the deployed config would deploy nothing; when
        // the newest one is, nothing is pending at all.
        let head_blob = hist.first().map(|r| r.blob.clone());
        let pending: Vec<Revision> = if self.pending.last().map(|r| &r.blob) == head_blob.as_ref() {
            Vec::new()
        } else {
            self.pending
                .iter()
                .rev()
                .filter(|r| Some(&r.blob) != head_blob.as_ref())
                .enumerate()
                .map(|(i, r)| {
                    let mut r = r.clone();
                    r.base = head_id.clone();
                    r.current = i == 0;
                    if i > 0 {
                        r.label = "Earlier compile".into();
                    }
                    r
                })
                .collect()
        };
        let by_id = pending
            .iter()
            .chain(&hist)
            .chain(&backups)
            .map(|r| (r.id.clone(), r.clone()))
            .collect();
        Lists {
            pending,
            history: hist,
            backups,
            by_id,
        }
    }

    pub fn find(&self, id: &str) -> Option<Revision> {
        self.lists().by_id.get(id).cloned()
    }

    /// Show these backups (see `backups.rs`), whose stamps are in `zone`,
    /// the Miniserver's time zone. Returns whether anything changed.
    pub fn set_backups(&mut self, backups: Vec<Backup>, zone: Option<Tz>) -> bool {
        if backups == self.backups && zone == self.zone {
            return false;
        }
        self.backups = backups;
        self.zone = zone;
        self.version += 1;
        true
    }

    /// Something else the History shows changed (a backup's download
    /// progress): the new version, for the browsers.
    pub fn touch(&mut self) -> u64 {
        self.version += 1;
        self.version
    }

    /// The bytes of a blob: from memory, a cached backup, or git.
    pub fn content(&mut self, blob: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(c) = self.contents.get(blob) {
            return Some(c.clone());
        }
        if let Some(b) = self.backups.iter().find(|b| b.blob == blob)
            && let Ok(bytes) = std::fs::read(&b.path)
            && blob_id(&bytes) == blob
        {
            let bytes = Arc::new(bytes);
            self.contents.insert(blob.to_string(), bytes.clone());
            return Some(bytes);
        }
        let (repo, _) = self.git.as_ref()?;
        let bytes = Arc::new(run_git(repo, &["cat-file", "blob", blob])?);
        self.contents.insert(blob.to_string(), bytes.clone());
        Some(bytes)
    }
}

/// `YYYYMMDDHHMMSS` in the Miniserver's time zone as Unix seconds. Without
/// a zone (no backup said which), as UTC: never this computer's zone, which
/// in a container is UTC anyway and elsewhere need not be the Miniserver's.
pub fn stamp_unix(stamp: &str, zone: Option<Tz>) -> Option<i64> {
    let t = chrono::NaiveDateTime::parse_from_str(stamp.get(..14)?, "%Y%m%d%H%M%S").ok()?;
    match zone {
        Some(z) => z.from_local_datetime(&t).earliest().map(|t| t.timestamp()),
        None => Some(t.and_utc().timestamp()),
    }
}

/// Unix seconds as a stamp in the Miniserver's time zone (UTC without one).
fn unix_stamp(unix: i64, zone: Option<Tz>) -> String {
    let Some(t) = chrono::DateTime::from_timestamp(unix, 0) else {
        return String::new();
    };
    match zone {
        Some(z) => t.with_timezone(&z).format("%Y%m%d%H%M%S").to_string(),
        None => t.format("%Y%m%d%H%M%S").to_string(),
    }
}

/// When the config a Pull committed was saved, as a backup's stamp: the date
/// `lox config pull` writes into its subject (`Config backup 2026-11-05
/// 23:55:01 (v274)`, the stamp of the backup it downloaded, in the
/// Miniserver's time). Else its commit time in the Miniserver's zone; with no
/// zone known, the first backup with its bytes, else the commit time in UTC.
fn pull_stamp(r: &Revision, zone: Option<Tz>, first_with: &HashMap<&str, &str>) -> String {
    if let Some(rest) = r.label.split("Config backup ").nth(1)
        && let Some(date) = rest.get(..19)
    {
        let digits: String = date.chars().filter(char::is_ascii_digit).collect();
        if digits.len() == 14 {
            return digits;
        }
    }
    match (zone, first_with.get(r.blob.as_str())) {
        (None, Some(stamp)) => stamp.to_string(),
        _ => unix_stamp(r.time, zone),
    }
}

/// Place the backups among the Pulls (the git revisions of `hist`, newest
/// first), and return the ones that show on their own, newest first.
///
/// In time order, saves with the same program in a row are one: when a Pull
/// is among them, its backups show as that Pull (`backups`); otherwise the
/// oldest shows, compared with the save before it. A program that comes back
/// later (a save that undoes changes) shows on its own, `same_as` the
/// revision it equals.
fn place_backups(hist: &mut [Revision], backups: &[Backup], zone: Option<Tz>) -> Vec<Revision> {
    // The first backup with each program, for a Pull without a date.
    let mut first_with: HashMap<&str, &str> = HashMap::new();
    for b in backups {
        let e = first_with.entry(&b.blob).or_insert(&b.stamp);
        if b.stamp.as_str() < *e {
            *e = &b.stamp;
        }
    }
    // (stamp, at the same stamp a backup comes first: it is saved before a
    // Pull takes it, blob, Pull index in hist, backup index)
    type Save<'a> = (String, u8, &'a str, Option<usize>, Option<usize>);
    let mut saves: Vec<Save> = Vec::new();
    for (i, r) in hist.iter().enumerate().filter(|(_, r)| r.kind == "git") {
        saves.push((pull_stamp(r, zone, &first_with), 1, &r.blob, Some(i), None));
    }
    for (i, b) in backups.iter().enumerate() {
        saves.push((b.stamp.clone(), 0, &b.blob, None, Some(i)));
    }
    saves.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    let n = saves.len();
    // The first Pull after each save.
    let mut next_pull: Vec<Option<usize>> = vec![None; n];
    let mut np = None;
    for k in (0..n).rev() {
        next_pull[k] = np;
        if saves[k].3.is_some() {
            np = saves[k].3;
        }
    }
    let mut shown: HashMap<String, String> = HashMap::new(); // blob -> id
    let mut out = Vec::new();
    let mut prev: Option<String> = None;
    let mut last_pull: Option<usize> = None;
    let mut collapsed: Vec<(usize, String)> = Vec::new();
    let mut k = 0;
    while k < n {
        let blob = saves[k].2;
        let mut e = k;
        while e < n && saves[e].2 == blob {
            e += 1;
        }
        let run = &saves[k..e];
        if let Some(p) = run.iter().filter_map(|s| s.3).next_back() {
            for s in run {
                if let Some(b) = s.4 {
                    collapsed.push((p, backups[b].file.clone()));
                }
                if let Some(pp) = s.3 {
                    last_pull = Some(pp);
                }
            }
            prev = Some(hist[p].id.clone());
        } else {
            let b = &backups[run[0].4.expect("a save without a Pull is a backup")];
            let id = format!("backup:{}", b.file);
            let same_as = hist
                .iter()
                .find(|r| r.blob == b.blob)
                .map(|r| r.id.clone())
                .or_else(|| shown.get(&b.blob).cloned());
            out.push(Revision {
                id: id.clone(),
                kind: "backup",
                label: format!("Miniserver backup v{}", b.version),
                detail: if b.on_miniserver {
                    b.file.clone()
                } else {
                    format!("{} · no longer on the Miniserver", b.file)
                },
                time: stamp_unix(&b.stamp, zone).unwrap_or_default(),
                blob: b.blob.clone(),
                base: prev.clone(),
                after: last_pull.map(|p| hist[p].id.clone()),
                before: next_pull[e - 1].map(|p| hist[p].id.clone()),
                same_as,
                ..Revision::default()
            });
            shown.entry(b.blob.clone()).or_insert_with(|| id.clone());
            prev = Some(id);
        }
        k = e;
    }
    for (p, file) in collapsed {
        hist[p].backups.push(file);
    }
    out.reverse();
    out
}

fn mtime(p: &Path) -> Option<i64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(t).timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn blob_id_matches_git() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            blob_id(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }

    #[test]
    fn git_history_and_live_snapshots() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap();
        git(&repo, &["init", "-q"]);
        let file = repo.join("config.Loxone");
        fs::write(&file, "v1").unwrap();
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-m",
                "[504F94000001] Config backup 2026-11-01 10:00:00 (v1)",
            ],
        );
        fs::write(&file, "v2").unwrap();
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-am",
                "[504F94000001] Config backup 2026-11-02 10:00:00 (v2)",
            ],
        );

        let mut h = History::new(file.clone(), None);
        assert_eq!(h.git_root(), Some(repo.as_path()));
        let (pending, hist) = h.list();
        assert!(pending.is_empty());
        // The file on disk equals HEAD: shown as that commit, not as a live snapshot.
        assert_eq!(hist.len(), 2);
        assert!(hist.iter().all(|r| r.kind == "git"));
        assert!(hist[0].current && hist[0].label.ends_with("(v2)"));
        assert_eq!(hist[0].base.as_deref(), Some(hist[1].id.as_str()));
        assert_eq!(hist[1].base, None);
        // Old content comes from git.
        assert_eq!(h.content(&hist[1].blob).unwrap().as_slice(), b"v1");

        // An edit on disk is a new, uncommitted revision on top.
        fs::write(&file, "v3").unwrap();
        assert!(h.poll_files());
        assert!(!h.poll_files(), "unchanged file adds nothing");
        let (_, hist) = h.list();
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[0].kind, "live");
        assert!(hist[0].current);
        assert_eq!(hist[0].label, "Changed on disk, not committed");
        assert_eq!(hist[0].base.as_deref(), Some(hist[1].id.as_str()));
        assert_eq!(h.content(&hist[0].blob).unwrap().as_slice(), b"v3");

        // Committing it folds the snapshot into the new commit.
        git(&repo, &["commit", "-q", "-am", "v3"]);
        assert!(h.refresh_git());
        let (_, hist) = h.list();
        assert_eq!(hist.len(), 3);
        assert!(hist.iter().all(|r| r.kind == "git"));
        assert!(hist[0].current);
    }

    #[test]
    fn compiled_config_is_pending_against_the_deployed_one() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.Loxone");
        let out = tmp.path().join("out.Loxone");
        fs::write(&base, "deployed").unwrap();
        fs::write(&out, "compiled").unwrap();
        let mut h = History::new(base, Some(out.clone()));
        let (pending, hist) = h.list();
        assert_eq!(pending.len(), 1);
        assert_eq!(hist.len(), 1, "no git: the file as seen at start");
        assert_eq!(pending[0].kind, "pending");
        assert_eq!(pending[0].base.as_deref(), Some(hist[0].id.as_str()));

        fs::write(&out, "compiled again").unwrap();
        assert!(h.poll_files());
        let (pending, _) = h.list();
        assert_eq!(pending.len(), 2);
        assert!(pending[0].current && !pending[1].current);
        assert_eq!(pending[1].label, "Earlier compile");
        assert!(h.find(&pending[1].id).is_some());
    }

    #[test]
    fn a_compile_identical_to_the_deployed_config_is_not_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.Loxone");
        let out = tmp.path().join("out.Loxone");
        fs::write(&base, "deployed").unwrap();
        fs::write(&out, "deployed").unwrap();
        let mut h = History::new(base, Some(out.clone()));
        assert!(h.list().0.is_empty());
        fs::write(&out, "a change").unwrap();
        assert!(h.poll_files());
        assert_eq!(h.list().0.len(), 1, "the identical compile is not listed");
        // Compiled back to the deployed config (a sync, a discard): nothing
        // is pending, the earlier compile included.
        fs::write(&out, "deployed").unwrap();
        assert!(h.poll_files());
        assert!(h.list().0.is_empty());
    }

    fn backup(dir: &Path, file: &str, xml: &[u8]) -> Backup {
        let b = loxone_client::ftp::parse_backup_name(file, 0).unwrap();
        let blob = blob_id(xml);
        let path = dir.join(format!("{blob}.Loxone"));
        fs::write(&path, xml).unwrap();
        Backup {
            file: file.into(),
            version: b.version,
            stamp: b.timestamp,
            blob,
            path,
            on_miniserver: true,
        }
    }

    #[test]
    fn miniserver_backups_fill_the_gaps_between_pulls() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap();
        let cache = tempfile::tempdir().unwrap();
        git(&repo, &["init", "-q"]);
        let file = repo.join("config.Loxone");
        // Two Pulls; three saves in between, and one after the last Pull.
        fs::write(&file, "v1").unwrap();
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-m",
                "[504F94000001] Config backup 2026-11-01 10:00:00 (v1)",
            ],
        );
        fs::write(&file, "v4").unwrap();
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-am",
                "[504F94000001] Config backup 2026-11-04 10:00:00 (v4)",
            ],
        );
        let mut h = History::new(file.clone(), None);
        let backups = vec![
            backup(cache.path(), "sps_1_20261101100000.zip", b"v1"),
            backup(cache.path(), "sps_2_20261102100000.zip", b"v2"),
            // Saved again without a change: nothing new to look at.
            backup(cache.path(), "sps_2_20261102110000.zip", b"v2"),
            backup(cache.path(), "sps_3_20261103100000.zip", b"v3"),
            backup(cache.path(), "sps_4_20261104100000.zip", b"v4"),
            backup(cache.path(), "sps_5_20261105100000.zip", b"v5"),
        ];
        assert!(h.set_backups(backups.clone(), None));
        assert!(
            !h.set_backups(backups, None),
            "the same backups change nothing"
        );

        let (_, hist) = h.list();
        // The backups with a commit's bytes show as that commit.
        assert_eq!(hist[0].backups, ["sps_4_20261104100000.zip"]);
        assert_eq!(hist[1].backups, ["sps_1_20261101100000.zip"]);
        // A Pull is still compared with the Pull before it.
        assert_eq!(hist[0].base.as_deref(), Some(hist[1].id.as_str()));

        let b = h.lists().backups.clone();
        let ids: Vec<&str> = b.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "backup:sps_5_20261105100000.zip",
                "backup:sps_3_20261103100000.zip",
                "backup:sps_2_20261102100000.zip",
            ]
        );
        assert!(b.iter().all(|r| r.kind == "backup"));
        assert_eq!(b[2].label, "Miniserver backup v2");
        // Each save is compared with the one before it.
        assert_eq!(b[2].base.as_deref(), Some(hist[1].id.as_str()));
        assert_eq!(b[1].base.as_deref(), Some(b[2].id.as_str()));
        assert_eq!(b[0].base.as_deref(), Some(hist[0].id.as_str()));
        // Between which Pulls.
        assert_eq!(b[1].after.as_deref(), Some(hist[1].id.as_str()));
        assert_eq!(b[1].before.as_deref(), Some(hist[0].id.as_str()));
        assert_eq!(b[0].after.as_deref(), Some(hist[0].id.as_str()));
        assert_eq!(b[0].before, None, "saved after the last Pull");
        // Viewable: found, with its content from the cache.
        let r = h.find(&b[1].id).unwrap();
        assert_eq!(h.content(&r.blob).unwrap().as_slice(), b"v3");
    }

    #[test]
    fn a_pull_without_a_date_in_its_subject_is_placed_by_its_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap();
        let cache = tempfile::tempdir().unwrap();
        git(&repo, &["init", "-q"]);
        let file = repo.join("base.Loxone");
        fs::write(&file, "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "Apply: 2 blocks changed"]);
        let mut h = History::new(file, None);
        h.set_backups(
            vec![
                backup(cache.path(), "sps_1_20000101000000.zip", b"before"),
                backup(cache.path(), "sps_2_20000102000000.zip", b"a"),
                backup(cache.path(), "sps_3_20000103000000.zip", b"after"),
            ],
            None,
        );
        let (_, hist) = h.list();
        let b = h.lists().backups.clone();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].base.as_deref(), Some(hist[0].id.as_str()));
        assert_eq!(b[0].after.as_deref(), Some(hist[0].id.as_str()));
        assert_eq!(b[1].before.as_deref(), Some(hist[0].id.as_str()));
        assert_eq!(b[1].base, None);
    }

    #[test]
    fn a_save_that_goes_back_to_an_older_pull_stays_in_its_gap() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap();
        let cache = tempfile::tempdir().unwrap();
        git(&repo, &["init", "-q"]);
        let file = repo.join("config.Loxone");
        fs::write(&file, "v1").unwrap();
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-m",
                "[x] Config backup 2026-11-01 10:00:00 (v1)",
            ],
        );
        fs::write(&file, "v4").unwrap();
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-am",
                "[x] Config backup 2026-11-04 10:00:00 (v4)",
            ],
        );
        let mut h = History::new(file, None);
        h.set_backups(
            vec![
                backup(cache.path(), "sps_1_20261101100000.zip", b"v1"),
                backup(cache.path(), "sps_2_20261102100000.zip", b"v2"),
                // Undoes the change of v2: the program of the first Pull again.
                backup(cache.path(), "sps_3_20261103100000.zip", b"v1"),
                backup(cache.path(), "sps_4_20261104100000.zip", b"v4"),
            ],
            None,
        );
        let l = h.lists();
        let (hist, b) = (&l.history, &l.backups);
        // Only the backup the first Pull took shows as it.
        assert_eq!(hist[1].backups, ["sps_1_20261101100000.zip"]);
        let ids: Vec<&str> = b.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "backup:sps_3_20261103100000.zip",
                "backup:sps_2_20261102100000.zip"
            ]
        );
        assert_eq!(b[0].same_as.as_deref(), Some(hist[1].id.as_str()));
        assert_eq!(b[0].base.as_deref(), Some(b[1].id.as_str()));
        assert_eq!(b[0].before.as_deref(), Some(hist[0].id.as_str()));
        assert_eq!(b[1].same_as, None);
        // The Pull after it is compared with the Pull before (as always), and
        // the revert is found by its id.
        assert_eq!(h.find(&b[0].id).unwrap().blob, hist[1].blob);
    }

    #[test]
    fn backup_times_are_in_the_miniserver_s_zone_not_this_computer_s() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap();
        let cache = tempfile::tempdir().unwrap();
        git(&repo, &["init", "-q"]);
        let file = repo.join("base.Loxone");
        fs::write(&file, "applied").unwrap();
        git(&repo, &["add", "."]);
        // An Apply (no date in its subject) at 10:30 UTC, 11:30 in Warsaw.
        let ok = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false", "commit", "-q", "-m", "Apply"])
            .env("GIT_COMMITTER_DATE", "2026-11-05T10:30:00Z")
            .env("GIT_AUTHOR_DATE", "2026-11-05T10:30:00Z")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut h = History::new(file, None);
        let backups = vec![
            // Saved at 11:00 and 12:00 Warsaw time: before and after the Apply.
            backup(cache.path(), "sps_1_20261105110000.zip", b"before"),
            backup(cache.path(), "sps_2_20261105120000.zip", b"after"),
        ];
        let warsaw: Tz = "Europe/Warsaw".parse().unwrap();
        h.set_backups(backups.clone(), Some(warsaw));
        let l = h.lists();
        let apply = l.history[0].id.clone();
        let b = &l.backups;
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].after.as_deref(), Some(apply.as_str()));
        assert_eq!(b[0].before, None);
        assert_eq!(b[1].before.as_deref(), Some(apply.as_str()));
        assert_eq!(b[1].after, None);
        // 11:00 in Warsaw (CET) is 10:00 UTC.
        assert_eq!(b[1].time, 1_793_872_800);
        // Without a zone, the stamps count as UTC (never the host's zone):
        // the Apply at 10:30 now seems to come before both.
        h.set_backups(backups, None);
        let l = h.lists();
        assert!(
            l.backups
                .iter()
                .all(|r| r.after.as_deref() == Some(apply.as_str()))
        );
        assert_eq!(l.backups[1].time, 1_793_872_800 + 3600);
    }
}
