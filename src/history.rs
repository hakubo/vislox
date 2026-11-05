//! The revisions vislox can show: compiled (pending) configs, live
//! snapshots of the watched file taken while vislox runs, and the file's git
//! history. Contents are keyed by their git blob id, so a live snapshot and
//! the commit holding the same bytes are recognisably the same.

use serde::Serialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct Revision {
    pub id: String,
    /// `pending`, `live` or `git`.
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
                        base: None,
                        current: false,
                    });
                    changed = true;
                }
            }
            Err(e) => {
                self.last_error = Some(format!("{}: {e}", self.current_path.display()));
            }
        }
        if let Some(path) = self.proposed_path.clone() {
            if let Ok(bytes) = std::fs::read(&path) {
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
                        base: None,
                        current: false,
                    });
                    changed = true;
                }
            }
        }
        if changed {
            self.version += 1;
        }
        changed
    }

    /// (pending, history), newest first, each with its comparison base.
    pub fn list(&self) -> (Vec<Revision>, Vec<Revision>) {
        let mut hist: Vec<Revision> = Vec::new();
        let commit_blobs: std::collections::HashSet<&str> =
            self.commits.iter().map(|c| c.blob.as_str()).collect();
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
                base: None,
                current: false,
            });
        }
        for i in 0..hist.len() {
            hist[i].base = hist.get(i + 1).map(|r| r.id.clone());
        }
        if let Some(first) = hist.first_mut() {
            first.current = Some(&first.blob) == disk_blob.as_ref();
        }
        let head_id = hist.first().map(|r| r.id.clone());
        let pending: Vec<Revision> = self
            .pending
            .iter()
            .rev()
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
            .collect();
        (pending, hist)
    }

    pub fn find(&self, id: &str) -> Option<Revision> {
        let (p, h) = self.list();
        p.into_iter().chain(h).find(|r| r.id == id)
    }

    /// The bytes of a blob: from memory or from git.
    pub fn content(&mut self, blob: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(c) = self.contents.get(blob) {
            return Some(c.clone());
        }
        let (repo, _) = self.git.as_ref()?;
        let bytes = Arc::new(run_git(repo, &["cat-file", "blob", blob])?);
        self.contents.insert(blob.to_string(), bytes.clone());
        Some(bytes)
    }
}

fn mtime(p: &Path) -> Option<i64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(t).timestamp())
}
