//! Changing the config, for a lox.toml project:
//!
//! - **Apply**: `lox config push --force` deploys what is pending (download,
//!   drift check, upload, program restart, verify), then vislox commits the
//!   project and pushes it to its git remote. Each step shows as it happens
//!   (`progress`): lox's own progress lines, and what vislox sees of the
//!   program restart (`restart.rs`).
//! - **Sync**: make what the Miniserver runs now the project's base
//!   (`lox config merge`), then commit and push.
//! - **Discard**: put the module, lock and compiled config back to the last
//!   commit.
//! - **Edit**: write values edited in Properties into vislox's own file of
//!   the module (`edit.rs`), then `lox logic compile`. Never deploys.
//!
//! What is pending comes from the module: anyone (a person, an agent working
//! in the repository, or vislox's Edit) edits it and compiles it with
//! `lox logic`; vislox shows the result as the pending revision.
//!
//! All four need the admin token (`X-Vislox-Token`). vislox itself still
//! writes no config: lox and git do.

use crate::{check, edit, restart};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// `lox config push --force` downloads twice and restarts the program.
const PUSH_TIMEOUT: Duration = Duration::from_secs(600);
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Starts an Apply message whose upload the Miniserver may not run.
const UNVERIFIED: &str = "Not verified";
/// `lox logic compile` reads the base and the module and writes out and the lock;
/// `lox logic lint` compiles them too, in memory.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);
/// `lox logic check` and `lox logic fmt --check` read only the module.
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Settings {
    pub token: Option<String>,
    pub project: Option<PathBuf>,
    pub lox: PathBuf,
    /// The Miniserver, to follow Apply's program restart. None: only lox's
    /// own progress shows.
    pub probe: Option<Arc<dyn restart::Probe>>,
    /// Which config the Miniserver runs, to verify an Apply by its load.
    pub running: Arc<crate::running::Running>,
}

pub struct Actions {
    s: Settings,
    st: Mutex<St>,
    /// Apply and Discard run one at a time.
    busy: tokio::sync::Mutex<()>,
    tx: broadcast::Sender<()>,
    edit_state: Mutex<EditState>,
    /// `GET /api/edits`, for the [`EditState::key`] it was worked out for.
    edits_cache: Mutex<Option<(String, Value)>>,
    /// The module's `.loxl` files as last read (#64): read and hashed again
    /// only when they change, for the key and for `sources`.
    module_files: Arc<Mutex<crate::managed::ModuleFiles>>,
    /// What vislox's last compile compiled: each module file's blob id (by
    /// its path from the project), and the blob id of the `out` it wrote.
    /// An edit checks the module against it by content ([`Self::uncompiled`]).
    compiled: Mutex<Option<Compiled>>,
    /// `GET /api/check` (#65): the last complete check, and the key
    /// ([`Self::check_key`]) it was worked out for. Only the check worker
    /// writes it ([`Self::check_once`]); a request only reads it.
    check_cache: Mutex<Option<CheckEntry>>,
    /// The lox found to have no `logic lint` (#66) or `logic test` (#67),
    /// by the command, then by path, modification time and size: it is not
    /// asked again until it changes.
    unsupported: Mutex<std::collections::BTreeMap<&'static str, String>>,
    /// Counts the checks kept, for the browsers (`actions.check`).
    check_version: std::sync::atomic::AtomicU64,
    /// Wakes the check worker: a file changed, or an action ended.
    check_wake: tokio::sync::Notify,
    /// An action started: background lint gives way ([`Actions::preempted`]).
    preempt: tokio::sync::Notify,
    /// Every compile and every write of the project holds it: an edit,
    /// Compile, Apply, Sync and Discard (after `busy`), and the check
    /// worker's scratch compile. So two compiles never run at once, and a
    /// check never records what an action is changing.
    compile_lock: tokio::sync::Mutex<()>,
    /// The base and out on disk, by blob id, as the history last read them.
    disk: Mutex<(Option<String>, Option<String>)>,
    /// lox.toml as last read, and what it says: parsed again only when its
    /// bytes change.
    project_cache: Mutex<Option<(Vec<u8>, loxone_config::Project)>>,
}

/// Whether `out` is the module compiled as it is now ([`Actions::freshness`]).
enum Fresh {
    Yes,
    /// It compiles to another config: `out` is from before `changed`.
    Stale {
        changed: Vec<String>,
    },
    /// It does not compile against the base (lox's message).
    Broken {
        changed: Vec<String>,
        error: String,
    },
}

/// The module and base as a compile read them, and the config it wrote,
/// each by blob id: what is compiled is known by content.
#[derive(Clone, PartialEq)]
struct Compiled {
    module: std::collections::BTreeMap<String, String>,
    out: String,
    base: Option<String>,
}

/// A file's blob id, if it can be read.
fn file_blob(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| crate::history::blob_id(&b))
}

/// The ops that undo an edit (#62), sent back as an edit of its own, into
/// the reply as `undo`; or, when they cannot be built, `undo: null` and why
/// (`undo_error`). The edit itself stands either way.
fn undo_into(reply: &mut Value, undo: Result<Vec<edit::Op>, String>) {
    match undo {
        Ok(ops) => reply["undo"] = json!(ops),
        Err(why) => {
            reply["undo"] = Value::Null;
            reply["undo_error"] = json!(why);
        }
    }
}

/// Each file of the module (as lox reads it: not hidden ones), by its path
/// from the project, with its blob id.
fn module_blobs(p: &loxone_config::Project) -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    edit::module_files(&p.module, &mut |f, _| {
        if let Ok(b) = std::fs::read(f) {
            let rel = f.strip_prefix(&p.root).unwrap_or(f).display().to_string();
            m.insert(rel, crate::history::blob_id(&b));
        }
    });
    m
}

/// Whether editing is on, and what `GET /api/edits` depends on: worked out
/// on start and on file changes ([`Actions::refresh_edit`]).
#[derive(Default, Clone, PartialEq)]
struct EditState {
    why: Option<String>,
    /// The module's files and the lock ([`crate::managed::ModuleFiles::key`]),
    /// the deployed and the pending config, by blob id.
    files: Option<String>,
    deployed: Option<String>,
    pending: Option<String>,
}

impl EditState {
    /// What `GET /api/edits` is worked out for: all three.
    fn key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.files.as_deref().unwrap_or("-"),
            self.deployed.as_deref().unwrap_or("-"),
            self.pending.as_deref().unwrap_or("-")
        )
    }
}

/// `GET /api/check`'s last check, the key it was worked out for
/// ([`Actions::check_key`]), and whether lint (#66) and the tests (#67) are
/// still to run on it.
struct CheckEntry {
    key: String,
    v: Value,
    lint_due: bool,
    test_due: bool,
}

/// The revision the Miniserver runs, for `GET /api/edits`.
pub struct DeployedRev {
    /// Its commit, when it is a git revision.
    pub commit: Option<String>,
    pub bytes: Arc<Vec<u8>>,
}

/// Clears `running` when an edit ends, also when its request is dropped.
struct Running<'a>(&'a Actions);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.set_running(None);
    }
}

/// What an edit writes (vislox's files) and what the compile writes (the
/// lock and out), as they were: put back on drop unless [`Undo::keep`] was
/// called, so a failed or abandoned edit leaves the project as it found it.
struct Undo {
    /// vislox's directory of the module, and its `.loxl` files by name.
    dir: PathBuf,
    old: std::collections::BTreeMap<String, String>,
    saved: Vec<(PathBuf, Option<Vec<u8>>)>,
    keep: bool,
}

impl Undo {
    fn new(dir: &Path, old: std::collections::BTreeMap<String, String>, others: &[&Path]) -> Undo {
        Undo {
            dir: dir.to_path_buf(),
            old,
            saved: others
                .iter()
                .map(|p| (p.to_path_buf(), std::fs::read(p).ok()))
                .collect(),
            keep: false,
        }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for Undo {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        for name in edit::Files::texts(&self.dir).keys() {
            if !self.old.contains_key(name) {
                let _ = std::fs::remove_file(self.dir.join(name));
            }
        }
        for (name, t) in &self.old {
            let _ = std::fs::write(self.dir.join(name), t);
        }
        // Only removed when empty.
        let _ = std::fs::remove_dir(&self.dir);
        // Out last: it stays newer than the module, as after a compile.
        for (path, bytes) in &self.saved {
            let _ = match bytes {
                Some(b) => std::fs::write(path, b),
                None => std::fs::remove_file(path),
            };
        }
    }
}

/// The lock and out as they were before vislox's first edit since the last
/// commit. Discard puts them back where the commit does not have them: a
/// lock nobody committed (or that git ignores) is never deleted unless
/// vislox's edit made it, since a lost lock re-mints every managed block.
struct Snapshot {
    lock: Option<Vec<u8>>,
    out: Option<Vec<u8>>,
}

/// Where the snapshot is kept: in the git directory, one per project, with
/// the commit it was taken on (`head`).
async fn snapshot_dir(dir: &Path) -> Option<PathBuf> {
    let gp = git(dir, &["rev-parse", "--git-path", "vislox-discard"])
        .await
        .ok()?;
    let id = crate::history::blob_id(dir.canonicalize().ok()?.to_string_lossy().as_bytes());
    Some(dir.join(gp.trim()).join(&id[..16]))
}

async fn head_commit(dir: &Path) -> String {
    git(dir, &["rev-parse", "-q", "--verify", "HEAD"])
        .await
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| "none".into())
}

/// Take the snapshot, unless one was taken on this commit already.
async fn snapshot_before_edit(dir: &Path, p: &loxone_config::Project) {
    let Some(sd) = snapshot_dir(dir).await else {
        return;
    };
    let head = head_commit(dir).await;
    if std::fs::read_to_string(sd.join("head")).ok().as_deref() == Some(head.as_str()) {
        return;
    }
    let _ = std::fs::remove_dir_all(&sd);
    if std::fs::create_dir_all(&sd).is_err() {
        return;
    }
    for (name, path) in [("lock", &p.lock), ("out", &p.out)] {
        let _ = match std::fs::read(path) {
            Ok(b) => std::fs::write(sd.join(name), b),
            Err(_) => std::fs::write(sd.join(format!("{name}.none")), ""),
        };
    }
    // Last: a snapshot without its commit is not used.
    let _ = std::fs::write(sd.join("head"), head);
}

/// The snapshot, if one was taken on the commit checked out now.
async fn read_snapshot(dir: &Path) -> Option<Snapshot> {
    let sd = snapshot_dir(dir).await?;
    if std::fs::read_to_string(sd.join("head")).ok()? != head_commit(dir).await {
        return None;
    }
    let one = |name: &str| -> Option<Option<Vec<u8>>> {
        match std::fs::read(sd.join(name)) {
            Ok(b) => Some(Some(b)),
            Err(_) => sd.join(format!("{name}.none")).exists().then_some(None),
        }
    };
    Some(Snapshot {
        lock: one("lock")?,
        out: one("out")?,
    })
}

async fn clear_snapshot(dir: &Path) {
    if let Some(sd) = snapshot_dir(dir).await {
        let _ = std::fs::remove_dir_all(sd);
    }
}

/// Every file under a directory, hidden ones included.
fn all_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = e.path();
        if path.is_dir() {
            all_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The module's files and the lock and out as they are, to put back when
/// Discard fails halfway.
struct Rollback {
    module: PathBuf,
    files: Vec<(PathBuf, Vec<u8>)>,
    others: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Rollback {
    fn take(module: &Path, others: &[&Path]) -> Rollback {
        let mut paths = Vec::new();
        all_files(module, &mut paths);
        Rollback {
            module: module.to_path_buf(),
            files: paths
                .into_iter()
                .filter_map(|p| Some((p.clone(), std::fs::read(&p).ok()?)))
                .collect(),
            others: others
                .iter()
                .map(|p| (p.to_path_buf(), std::fs::read(p).ok()))
                .collect(),
        }
    }

    fn restore(&self) {
        let mut now = Vec::new();
        all_files(&self.module, &mut now);
        for p in now {
            if !self.files.iter().any(|(q, _)| *q == p) {
                let _ = std::fs::remove_file(p);
            }
        }
        for (p, b) in &self.files {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(p, b);
        }
        for (p, b) in &self.others {
            let _ = match b {
                Some(b) => std::fs::write(p, b),
                None => std::fs::remove_file(p),
            };
        }
    }
}

/// Read objects (`<commit>:<path>`) with one `git cat-file --batch`: each
/// one's bytes, or `None` where git has none.
async fn git_objects(dir: &Path, specs: &[String]) -> Result<Vec<Option<Vec<u8>>>, String> {
    use tokio::io::AsyncWriteExt;
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = tokio::process::Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    let mut input = specs.join("\n");
    input.push('\n');
    let mut stdin = child.stdin.take().expect("piped");
    stdin
        .write_all(input.as_bytes())
        .await
        .map_err(|e| format!("git cat-file: {e}"))?;
    drop(stdin);
    let out = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "git cat-file took too long".to_string())?
        .map_err(|e| format!("git cat-file: {e}"))?;
    let (mut rest, mut objects) = (&out.stdout[..], Vec::new());
    for _ in specs {
        let nl = rest
            .iter()
            .position(|&b| b == b'\n')
            .ok_or("git cat-file: short output")?;
        let header = String::from_utf8_lossy(&rest[..nl]).into_owned();
        rest = &rest[nl + 1..];
        if header.ends_with(" missing") || header.ends_with(" ambiguous") {
            objects.push(None);
            continue;
        }
        let size: usize = header
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("git cat-file: {header}"))?;
        let body = rest.get(..size).ok_or("git cat-file: short output")?;
        objects.push(Some(body.to_vec()));
        rest = rest.get(size + 1..).unwrap_or_default();
    }
    Ok(objects)
}

/// Removes a scratch directory when dropped.
struct RemoveDir(PathBuf);

impl Drop for RemoveDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[derive(Default)]
struct St {
    /// What Apply or Discard is doing now.
    running: Option<&'static str>,
    last: Option<Outcome>,
    /// The steps of the running or last Apply.
    progress: Option<Progress>,
}

/// An Apply, step by step.
#[derive(Clone, Serialize)]
struct Progress {
    steps: Vec<Step>,
    /// lox's last lines, for the details.
    log: Vec<String>,
    at: i64,
}

#[derive(Clone, Serialize)]
struct Step {
    id: &'static str,
    label: &'static str,
    /// `wait`, `run`, `ok`, `skip` or `fail`
    state: &'static str,
    detail: Option<String>,
    /// When it started and ended (Unix seconds).
    at: Option<i64>,
    end: Option<i64>,
}

/// Apply's steps, in order.
const APPLY_STEPS: [(&str, &str); 10] = [
    ("module", "Module compiles"),
    ("tests", "Tests pass"),
    ("check", "Download and drift check"),
    ("upload", "Upload"),
    ("restart", "Program restarting"),
    ("online", "Back online"),
    ("loaded", "New config loaded"),
    ("verify", "Verified"),
    ("commit", "Committed"),
    ("push", "Pushed to git"),
];
const LOG_LINES: usize = 12;

impl Progress {
    fn apply() -> Progress {
        Progress {
            steps: APPLY_STEPS
                .iter()
                .map(|&(id, label)| Step {
                    id,
                    label,
                    state: "wait",
                    detail: None,
                    at: None,
                    end: None,
                })
                .collect(),
            log: Vec::new(),
            at: now(),
        }
    }

    fn get(&mut self, id: &str) -> &mut Step {
        self.steps
            .iter_mut()
            .find(|s| s.id == id)
            .expect("a known step")
    }

    /// `id` starts; the steps before it that still run are done.
    fn start(&mut self, id: &str) {
        let t = now();
        for s in self.steps.iter_mut().take_while(|s| s.id != id) {
            if s.state == "run" {
                s.state = "ok";
                s.end = Some(t);
            }
        }
        let s = self.get(id);
        if s.state == "wait" {
            s.state = "run";
            s.at = Some(t);
        }
    }

    /// `id` ends as `state` (`ok`, `skip`, `fail`), with what to say.
    fn finish(&mut self, id: &str, state: &'static str, detail: Option<String>) {
        let t = now();
        let s = self.get(id);
        s.state = state;
        s.at.get_or_insert(t);
        s.end = Some(t);
        if detail.is_some() {
            s.detail = detail;
        }
    }

    /// End a step that is not done yet; one already done keeps its outcome.
    fn settle(&mut self, id: &str, detail: Option<&str>) {
        if matches!(self.get(id).state, "wait" | "run") {
            self.finish(id, "ok", detail.map(String::from));
        }
    }

    /// A line of lox's output.
    fn lox_line(&mut self, line: &str) {
        let line = line.trim_end();
        if line.trim().is_empty() {
            return;
        }
        if self.log.len() == LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(line.to_string());
        match push_step(line) {
            Some("verified") => self.finish("verify", "ok", None),
            Some("verify") => {
                // lox waited until the program ran the upload.
                self.settle("restart", None);
                self.settle("online", None);
                self.settle("loaded", Some("lox saw the new program running"));
                self.start("verify");
            }
            Some(id) => self.start(id),
            None => {}
        }
    }

    /// What vislox saw of the restart.
    fn restart_event(&mut self, e: restart::Event) {
        match e {
            restart::Event::OutOfService(why) => {
                self.get("restart").detail = Some(format!("out of service ({why})"));
                self.start("online");
            }
            restart::Event::BackOnline(secs) => {
                self.finish("online", "ok", Some(format!("after {secs} s")));
            }
            restart::Event::Loaded(secs, version) => {
                if self.get("online").state == "wait" {
                    self.finish("restart", "ok", Some("no outage seen".into()));
                    self.finish("online", "ok", Some("answered throughout".into()));
                }
                self.finish(
                    "loaded",
                    "ok",
                    Some(format!("after {secs} s · structure {version}")),
                );
            }
        }
    }

    /// The running step failed (else the first one not done).
    fn fail(&mut self, why: &str) {
        let id = self
            .steps
            .iter()
            .find(|s| s.state == "run")
            .or_else(|| self.steps.iter().find(|s| s.state == "wait"))
            .map(|s| s.id);
        if let Some(id) = id {
            self.finish(id, "fail", Some(last_lines(why, 1)));
        }
    }
}

/// The step a line of `lox config push` starts (its `say` lines in
/// `config_push.rs`); `verified` ends the verify step.
fn push_step(line: &str) -> Option<&'static str> {
    let l = line.trim_start();
    if l.starts_with("Downloading the live config") {
        Some("check")
    } else if l.starts_with("Uploading as") {
        Some("upload")
    } else if l.starts_with("Restarting the program") || l.starts_with("Rebooting the Miniserver") {
        Some("restart")
    } else if l.starts_with("Downloading the config again") {
        Some("verify")
    } else if l.starts_with("Pushed and verified") {
        Some("verified")
    } else {
        None
    }
}

#[derive(Clone, Serialize)]
struct Outcome {
    /// `apply`, `sync` or `discard`
    kind: &'static str,
    ok: bool,
    /// Done, but not verified: the Miniserver may not run it (`message`
    /// says why).
    warn: bool,
    message: String,
    at: i64,
    /// After an Apply: the git blob ids of the base before and after, so the
    /// UI can show the deployed revision compared with the one it replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    deployed: Option<Deployed>,
}

#[derive(Clone, Serialize)]
struct Deployed {
    before: Option<String>,
    after: String,
}

/// A refused or failed action: HTTP status and message.
pub type Refusal = (axum::http::StatusCode, String);

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Read a one-line secret file (the admin token).
pub fn read_secret(path: &Path) -> anyhow::Result<String> {
    let s = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let s = s.trim().to_string();
    anyhow::ensure!(!s.is_empty(), "{} is empty", path.display());
    Ok(s)
}

impl Actions {
    pub fn new(s: Settings) -> Actions {
        let (tx, _) = broadcast::channel(16);
        let a = Actions {
            s,
            st: Mutex::new(St::default()),
            busy: tokio::sync::Mutex::new(()),
            tx,
            edit_state: Mutex::new(EditState::default()),
            edits_cache: Mutex::new(None),
            module_files: Arc::default(),
            compiled: Mutex::new(None),
            check_cache: Mutex::new(None),
            unsupported: Mutex::new(Default::default()),
            check_version: Default::default(),
            check_wake: tokio::sync::Notify::new(),
            preempt: tokio::sync::Notify::new(),
            compile_lock: tokio::sync::Mutex::new(()),
            disk: Mutex::new((None, None)),
            project_cache: Mutex::new(None),
        };
        a.edit_state.lock().unwrap().why = a.why_not_edit_now();
        a
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    fn changed(&self) {
        let _ = self.tx.send(());
    }

    /// The project's module (a directory, or one file), to watch.
    pub fn module_path(&self) -> Option<PathBuf> {
        self.project().ok().map(|p| p.module)
    }

    /// The project's lox.toml, parsed again only when its bytes change.
    fn project(&self) -> Result<loxone_config::Project, String> {
        let dir = self
            .s
            .project
            .as_deref()
            .ok_or("only for a lox.toml project")?;
        let path = dir.join("lox.toml");
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read lox.toml: {e}"))?;
        let mut c = self.project_cache.lock().unwrap();
        if let Some((b, p)) = &*c
            && *b == bytes
        {
            return Ok(p.clone());
        }
        let p = loxone_config::Project::load(&path)
            .map_err(|e| format!("cannot read lox.toml: {e}"))?;
        *c = Some((bytes, p.clone()));
        Ok(p)
    }

    /// Apply and Sync could start now (they are on, and nothing runs), so
    /// the Miniserver is worth checking first.
    pub fn wants_miniserver(&self) -> bool {
        self.why_not_apply().is_none() && self.st.lock().unwrap().running.is_none()
    }

    fn why_not_apply(&self) -> Option<&'static str> {
        if self.s.project.is_none() {
            Some("only for a lox.toml project")
        } else if self.s.token.is_none() {
            Some("start vislox with --admin-token-file")
        } else {
            None
        }
    }

    /// Why Properties cannot edit values now, if it cannot: as Apply, and the
    /// module must be a directory, or not exist yet (the first edit creates
    /// it): vislox writes a file of its own into it.
    fn why_not_edit_now(&self) -> Option<String> {
        if let Some(why) = self.why_not_apply() {
            return Some(why.into());
        }
        let dir = self.s.project.as_deref()?;
        let p = match loxone_config::Project::load(&dir.join("lox.toml")) {
            Err(e) => return Some(format!("cannot read lox.toml: {e}")),
            Ok(p) => p,
        };
        let name = p.module.file_name().unwrap_or_default().to_string_lossy();
        if p.module.is_dir() {
            None
        } else if p.module.exists() {
            Some(format!(
                "the module ({name}) is one file; make it a directory to edit from vislox"
            ))
        } else if p.module.extension().is_some_and(|x| x == "loxl") {
            Some(format!("the module ({name}) does not exist"))
        } else {
            None
        }
    }

    /// Work out again whether editing is on, and the key of what
    /// `GET /api/edits` says (vislox's file, the deployed and the pending
    /// config): on start and on every file change, not on every event.
    /// Tells the browsers when either changed.
    pub fn refresh_edit(&self, deployed: Option<&str>, pending: Option<&str>) {
        let why = self.why_not_edit_now();
        let new = EditState {
            why,
            files: self.files_key(),
            deployed: deployed.map(String::from),
            pending: pending.map(String::from),
        };
        let mut st = self.edit_state.lock().unwrap();
        if *st != new {
            *st = new;
            drop(st);
            self.changed();
        }
        // The check worker compares its key and works only on a change.
        self.check_wake.notify_one();
    }

    /// The module's files (vislox's among them) and the lockfile as they are
    /// on disk now, as one blob id: where each managed block is declared
    /// (#64) moves with them, compiled or not, and which blocks lox can move
    /// comes from the lock, which lox or a Discard may change alone. Only
    /// a file whose size or time changed is read and hashed again.
    fn files_key(&self) -> Option<String> {
        let p = self.project().ok()?;
        self.module_files
            .lock()
            .unwrap()
            .key(&p.root, &p.module, &p.lock)
    }

    /// [`Self::files_key`] off the async worker: the reads block.
    async fn files_key_now(&self) -> Option<String> {
        let p = self.project().ok()?;
        let m = self.module_files.clone();
        tokio::task::spawn_blocking(move || m.lock().unwrap().key(&p.root, &p.module, &p.lock))
            .await
            .ok()
            .flatten()
    }

    /// The base and out on disk, by blob id, as the history read them (it
    /// reads and hashes both on every poll): part of the check's key.
    pub fn set_disk(&self, base: Option<String>, out: Option<String>) {
        *self.disk.lock().unwrap() = (base, out);
    }

    /// For `/api/history` and the `actions` event.
    pub fn info(&self) -> Value {
        let edit = self.edit_state.lock().unwrap().clone();
        let st = self.st.lock().unwrap();
        let flag = |why: Option<&str>| match why {
            None => json!({ "available": true }),
            Some(r) => json!({ "available": false, "reason": r }),
        };
        let mut e = flag(edit.why.as_deref());
        e["key"] = json!(edit.key());
        e["check"] = json!(
            self.check_version
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        json!({
            "shown": self.s.project.is_some(),
            "apply": flag(self.why_not_apply()),
            "edit": e,
            "running": st.running,
            "last": st.last,
            "progress": st.progress,
        })
    }

    /// The admin token, from the `X-Vislox-Token` header.
    pub fn check(&self, headers: &axum::http::HeaderMap) -> Result<(), Refusal> {
        use axum::http::StatusCode;
        let Some(want) = &self.s.token else {
            return Err((
                StatusCode::FORBIDDEN,
                "no admin token configured (--admin-token-file)".into(),
            ));
        };
        let got = headers
            .get("x-vislox-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        // Compare without an early exit on the first differing byte.
        let same = got.len() == want.len()
            && got
                .bytes()
                .zip(want.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if same {
            Ok(())
        } else {
            Err((StatusCode::UNAUTHORIZED, "wrong admin token".into()))
        }
    }

    fn begin(&self, what: &'static str) -> Result<(), Refusal> {
        use axum::http::StatusCode;
        if let Some(why) = self.why_not_apply() {
            return Err((StatusCode::CONFLICT, why.to_string()));
        }
        let mut st = self.st.lock().unwrap();
        st.running = Some(what);
        drop(st);
        self.preempt.notify_waiters();
        self.changed();
        Ok(())
    }

    fn end(&self, kind: &'static str, res: &Result<String, String>) {
        self.end_with(kind, res, None);
    }

    fn end_with(
        &self,
        kind: &'static str,
        res: &Result<String, String>,
        deployed: Option<Deployed>,
    ) {
        let mut st = self.st.lock().unwrap();
        st.running = None;
        st.last = Some(Outcome {
            kind,
            ok: res.is_ok(),
            warn: res.as_ref().is_ok_and(|m| m.contains(UNVERIFIED)),
            message: match res {
                Ok(m) | Err(m) => m.clone(),
            },
            at: now(),
            deployed,
        });
        if let (Err(e), Some(p)) = (res, st.progress.as_mut().filter(|_| kind == "apply")) {
            p.fail(e);
        }
        // A check skipped while this ran is worked out now, files changed or not.
        self.check_wake.notify_one();
        drop(st);
        self.changed();
    }

    /// Change the progress of the running Apply, and tell the browsers.
    fn progress(&self, f: impl FnOnce(&mut Progress)) {
        let mut st = self.st.lock().unwrap();
        if let Some(p) = st.progress.as_mut() {
            f(p);
        }
        drop(st);
        self.changed();
    }

    /// Deploy what is pending, then commit and push. `opts` are the
    /// user's answers to an earlier refusal (a newer Loxone Config, custom
    /// changes on the Miniserver).
    pub async fn apply(&self, opts: &ApplyOpts) -> Result<String, Refusal> {
        use axum::http::StatusCode;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.st.lock().unwrap().progress = Some(Progress::apply());
        self.begin("Apply")?;
        let _compiling = self.compile_lock.lock().await;
        // First, the module must compile: else lox deploys a compile of
        // something else than the review shows (#65). Asked of lox anew,
        // not of the check's cache, which may be a refresh old.
        self.progress(|p| p.start("module"));
        match self.module_errors().await {
            None => self.progress(|p| {
                p.finish(
                    "module",
                    "skip",
                    Some("lox logic check gave no answer".into()),
                )
            }),
            Some(c) => match broken_message(&c) {
                None => self.progress(|p| p.finish("module", "ok", None)),
                Some(why) => {
                    // The running step, `module`, fails with it.
                    self.end_with("apply", &Err(why.clone()), None);
                    return Err((StatusCode::CONFLICT, why));
                }
            },
        }
        // Then its tests (#67) must pass, on the config Apply deploys: the
        // check's run when it is of the files as they are, else run now.
        self.progress(|p| p.start("tests"));
        let tests = self.tests_now().await;
        match tests_verdict(&tests) {
            Ok((state, detail)) => self.progress(|p| p.finish("tests", state, Some(detail))),
            Err(why) => {
                self.end_with("apply", &Err(why.clone()), None);
                return Err((StatusCode::CONFLICT, why));
            }
        }
        let res = self.apply_steps(opts).await;
        let (res, deployed) = match res {
            Ok((m, d)) => (Ok(m), d),
            Err(e) => (Err(e), None),
        };
        self.end_with("apply", &res, deployed);
        res.map_err(|e| (StatusCode::BAD_GATEWAY, e))
    }

    async fn apply_steps(&self, opts: &ApplyOpts) -> Result<(String, Option<Deployed>), String> {
        let dir = self.s.project.as_deref().unwrap();
        let mut args = vec!["config", "push", "--force"];
        if let Some(v) = &opts.accept_version {
            if v.is_empty() || v.len() > 12 || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("not a Loxone Config version: {v}"));
            }
            args.extend(["--accept-version", v.as_str()]);
        }
        if opts.discard_custom_changes {
            args.push("--discard-custom-changes");
        }
        let lox = &self.s.lox;
        // The commit subject: the change, by name (before the base moves).
        let p = loxone_config::Project::load(&dir.join("lox.toml"))
            .map_err(|e| format!("cannot read lox.toml: {e}"))?;
        let (base, out) = (p.base.to_string_lossy(), p.out.to_string_lossy());
        let subject = run(
            lox,
            &["logic", "diff", "--summary", &base, &out],
            dir,
            GIT_TIMEOUT,
        )
        .await
        .ok()
        .and_then(|o| o.lines().find(|l| !l.trim().is_empty()).map(String::from))
        .unwrap_or_else(|| "Apply pending changes".into());
        let before = std::fs::read(&p.base)
            .ok()
            .map(|b| crate::history::blob_id(&b));
        let ran_before = self.s.running.get(true).await;
        let pushed = self
            .push_followed(lox, &args, dir)
            .await
            .map_err(|e| format!("lox config push failed:\n{e}"))?;
        let mut report = vec![last_lines(&pushed, 6)];
        report.push(self.verify_load(&ran_before).await);
        if let Some(m) = keep_base(dir, &p.base)? {
            report.push(m);
        }
        let deployed = std::fs::read(&p.base).ok().map(|b| Deployed {
            before,
            after: crate::history::blob_id(&b),
        });
        self.progress(|p| p.start("commit"));
        git(dir, &["add", "-A", "."]).await?;
        let staged = git(dir, &["diff", "--cached", "--quiet"]).await.is_err();
        if staged {
            git(dir, &["commit", "-q", "-m", &subject]).await?;
            report.push(format!("Committed: {subject}"));
            self.progress(|p| p.finish("commit", "ok", Some(subject.clone())));
        } else {
            self.progress(|p| p.finish("commit", "skip", Some("nothing to commit".into())));
        }
        self.progress(|p| p.start("push"));
        match git(dir, &["push", "-q"]).await {
            Ok(_) => {
                report.push("Pushed to git.".into());
                self.progress(|p| p.finish("push", "ok", None));
            }
            Err(e) => {
                report.push(format!(
                    "Deployed and committed, but git push failed: {}",
                    last_lines(&e, 3)
                ));
                self.progress(|p| p.finish("push", "fail", Some(last_lines(&e, 1))));
            }
        }
        Ok((report.join("\n"), deployed))
    }

    /// `lox config push`, its progress lines shown as they come. From the
    /// program restart on, vislox also watches the Miniserver itself.
    async fn push_followed(&self, lox: &Path, args: &[&str], dir: &Path) -> Result<String, String> {
        self.progress(|p| p.start("check"));
        // Follow the restart only when the Miniserver answers vislox now.
        let watch_with = match &self.s.probe {
            Some(pr) => restart::baseline(pr).await.map(|v| (pr.clone(), v)),
            None => None,
        };
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        let mut go_tx = Some(go_tx);
        let (stop_tx, stop_rx) = watch::channel(false);
        let push = async {
            let res = run_lines(lox, args, dir, PUSH_TIMEOUT, |line| {
                let step = push_step(line);
                if step == Some("restart")
                    && let Some(tx) = go_tx.take()
                {
                    let _ = tx.send(());
                }
                if step == Some("verify") {
                    let _ = stop_tx.send(true);
                }
                self.progress(|p| p.lox_line(line));
            })
            .await;
            let _ = stop_tx.send(true);
            // No restart line (a push that failed before it): the follower
            // stops waiting for one.
            go_tx.take();
            res
        };
        let follow = async {
            let Some((probe, before)) = watch_with else {
                return;
            };
            if go_rx.await.is_ok() {
                restart::follow(probe, before, stop_rx, |e| {
                    self.progress(|p| p.restart_event(e))
                })
                .await;
            }
        };
        let (res, ()) = tokio::join!(push, follow);
        res
    }

    /// lox verified the newest backup holds the upload; check that the
    /// Miniserver loaded it too: `sps_new.zip` gone, and a program start in
    /// def.log that loaded it and runs the newest backup. The log can lag
    /// the restart a little, so look a few times.
    async fn verify_load(&self, before: &crate::running::Report) -> String {
        if !self.s.running.available() {
            return format!(
                "{UNVERIFIED}: no Miniserver connection to check which config it runs."
            );
        }
        let mut why = Vec::new();
        for i in 0..4 {
            if i > 0 {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let after = self.s.running.get(true).await;
            why = crate::running::verify_load(before, &after);
            if why.is_empty() {
                return format!(
                    "Loaded: the Miniserver runs {} (def.log, {}).",
                    after.running.as_deref().unwrap_or("?"),
                    after.start.as_ref().map(|s| s.at.as_str()).unwrap_or("?")
                );
            }
        }
        format!("{UNVERIFIED} by load: {}.", why.join("; "))
    }

    /// Make the Miniserver's running config the project's base
    /// (`lox config merge`), then commit and push.
    pub async fn sync(&self) -> Result<String, Refusal> {
        use axum::http::StatusCode;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.begin("Sync")?;
        let res = {
            let _compiling = self.compile_lock.lock().await;
            self.sync_steps().await
        };
        self.end("sync", &res);
        res.map_err(|e| (StatusCode::BAD_GATEWAY, e))
    }

    async fn sync_steps(&self) -> Result<String, String> {
        let dir = self.s.project.as_deref().unwrap();
        // A sync commits the project; pending (uncommitted) changes would go
        // with it and Discard could no longer drop them.
        let dirty = git(dir, &["status", "--porcelain", "--", "."]).await?;
        if !dirty.trim().is_empty() {
            return Err(
                "Apply or discard the pending changes first: a sync commits the project.".into(),
            );
        }
        let p = loxone_config::Project::load(&dir.join("lox.toml"))
            .map_err(|e| format!("cannot read lox.toml: {e}"))?;
        let mut notes = Vec::new();
        let rules = match run(
            &self.s.lox,
            &["config", "merge", "-o", "json"],
            dir,
            PUSH_TIMEOUT,
        )
        .await
        {
            Ok(out) => {
                let report: Value = out
                    .find('{')
                    // The first JSON value; lox's progress messages may follow it.
                    .and_then(|i| {
                        serde_json::Deserializer::from_str(&out[i..])
                            .into_iter::<Value>()
                            .next()?
                            .ok()
                    })
                    .unwrap_or(Value::Null);
                report["rules_updated"].as_array().map_or(0, Vec::len)
                    + report["rules_removed"].as_array().map_or(0, Vec::len)
            }
            // merge refuses while the Miniserver holds custom changes no
            // download carries (some keep reappearing). The download is
            // still what the Miniserver runs: make it the base directly.
            Err(e) if e.contains("custom change(s) that no download carries") => {
                self.rebase_on_download(dir, &p.base).await?;
                notes.push(
                    "lox config merge refused because the Miniserver holds custom changes no download \
                     carries, so the project was synced from a plain download instead (rules edited in \
                     the app are not taken over). Apply will ask before deleting those changes."
                        .to_string(),
                );
                0
            }
            Err(e) => return Err(format!("lox config merge failed:\n{}", last_lines(&e, 8))),
        };
        if let Some(m) = keep_base(dir, &p.base)? {
            notes.push(m);
        }
        git(dir, &["add", "-A", "."]).await?;
        if git(dir, &["diff", "--cached", "--quiet"]).await.is_ok() {
            let mut m = vec![
                "Already up to date: the project's base is what the Miniserver runs.".to_string(),
            ];
            m.extend(notes);
            return Ok(m.join("\n"));
        }
        let mut msg = String::from("Sync with the Miniserver's running config\n");
        if rules > 0 {
            msg.push_str(&format!("\nTook over {rules} rule(s) edited in the app.\n"));
        }
        git(dir, &["commit", "-q", "-m", &msg]).await?;
        let mut report =
            vec!["Synced: the project's base is now the Miniserver's running config.".to_string()];
        if rules > 0 {
            report.push(format!("Took over {rules} rule(s) edited in the app."));
        }
        report.extend(notes);
        match git(dir, &["push", "-q"]).await {
            Ok(_) => report.push("Committed and pushed to git.".into()),
            Err(e) => report.push(format!(
                "Committed, but git push failed: {}",
                last_lines(&e, 3)
            )),
        }
        Ok(report.join("\n"))
    }

    /// Download the running config, make it the base, and compile the
    /// module on top (accepting the Loxone Config release it was saved by).
    async fn rebase_on_download(&self, dir: &Path, base: &Path) -> Result<(), String> {
        let lox = &self.s.lox;
        std::fs::create_dir_all(dir.join("backups"))
            .map_err(|e| format!("cannot create backups/: {e}"))?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let zip = format!("backups/{stamp}-sync.zip");
        run(
            lox,
            &["config", "download", "--extract", "--save-as", &zip],
            dir,
            PUSH_TIMEOUT,
        )
        .await
        .map_err(|e| format!("lox config download failed:\n{}", last_lines(&e, 6)))?;
        let downloaded = dir.join(format!("backups/{stamp}-sync.Loxone"));
        std::fs::copy(&downloaded, base).map_err(|e| {
            format!(
                "cannot copy {} to {}: {e}",
                downloaded.display(),
                base.display()
            )
        })?;
        let compile = |extra: Vec<String>| async move {
            let mut args = vec!["logic".to_string(), "compile".to_string()];
            args.extend(extra);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run(lox, &args, dir, PUSH_TIMEOUT).await
        };
        if let Err(e) = compile(Vec::new()).await {
            // The live config may be saved by a newer Loxone Config: it is
            // what the Miniserver runs, so accept its release.
            let Some(v) = apply_hints(&e)["accept_version"].as_str().map(String::from) else {
                return Err(format!("lox logic compile failed:\n{}", last_lines(&e, 6)));
            };
            compile(vec!["--accept-version".into(), v])
                .await
                .map_err(|e| format!("lox logic compile failed:\n{}", last_lines(&e, 6)))?;
        }
        Ok(())
    }

    /// Write the edits into vislox's file of the module and compile it. The
    /// result shows as the pending revision; nothing reaches the Miniserver.
    /// Returns, per op, the value vislox claimed before (to undo it), and the
    /// blob id of the new pending config.
    ///
    /// `deployed` is the deployed revision (the History's, as for
    /// `GET /api/edits`): a block added here and in it is not taken out.
    ///
    /// Safe to drop at any point (a closed tab): `running` is cleared, and a
    /// half-done edit is put back ([`Undo`]).
    pub async fn edit(
        &self,
        req: &edit::Request,
        deployed: Option<Arc<Vec<u8>>>,
    ) -> Result<Value, Refusal> {
        use axum::http::StatusCode;
        if req.ops.is_empty() || req.ops.len() > edit::MAX_OPS {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("send 1 to {} edits", edit::MAX_OPS),
            ));
        }
        let _one = self.busy.try_lock().map_err(|_| {
            (
                StatusCode::CONFLICT,
                "Apply, Sync, Discard or another edit is running; edit again when it is done"
                    .to_string(),
            )
        })?;
        if let Some(why) = self.why_not_edit_now() {
            return Err((StatusCode::CONFLICT, why));
        }
        self.set_running(Some("Edit"));
        let _running = Running(self);
        let _compiling = self.compile_lock.lock().await;
        self.edit_steps(req, deployed).await
    }

    fn set_running(&self, what: Option<&'static str>) {
        self.st.lock().unwrap().running = what;
        if what.is_some() {
            self.preempt.notify_waiters();
        }
        self.changed();
        if what.is_none() {
            // A check skipped while it ran is worked out now.
            self.check_wake.notify_one();
        }
    }

    async fn edit_steps(
        &self,
        req: &edit::Request,
        deployed: Option<Arc<Vec<u8>>>,
    ) -> Result<Value, Refusal> {
        use axum::http::StatusCode;
        let bad = |e: String| (StatusCode::UNPROCESSABLE_ENTITY, e);
        let fail = |e: String| (StatusCode::INTERNAL_SERVER_ERROR, e);
        let dir = self.s.project.as_deref().unwrap();
        let p = loxone_config::Project::load(&dir.join("lox.toml"))
            .map_err(|e| fail(format!("cannot read lox.toml: {e}")))?;
        let pending = if p.out.exists() { &p.out } else { &p.base };
        let bytes = std::fs::read(pending)
            .map_err(|e| fail(format!("cannot read {}: {e}", pending.display())))?;
        if crate::history::blob_id(&bytes) != req.base {
            return Err((
                StatusCode::CONFLICT,
                "The pending config changed since this page showed it (a compile, Apply or Discard). \
                 Look at the new one, then edit again."
                    .into(),
            ));
        }
        let doc = edit::parse_doc(&bytes)
            .map_err(|e| fail(format!("cannot read {}: {e}", pending.display())))?;
        let dir_path = p.module.join(edit::DIR);
        let shown = dir_path
            .strip_prefix(&p.root)
            .unwrap_or(&dir_path)
            .display()
            .to_string();
        let old = edit::Files::texts(&dir_path);
        let mut files = edit::Files::from_texts(&old)
            .map_err(|e| (StatusCode::CONFLICT, format!("{shown}/{e}")))?;
        let lock = edit::Lock::read(&p.lock);
        // The base, which lox compiles the module against: its pages.
        let base = std::fs::read(&p.base)
            .map_err(|e| fail(format!("cannot read {}: {e}", p.base.display())))
            .and_then(|b| {
                edit::parse_doc(&b)
                    .map_err(|e| fail(format!("cannot read {}: {e}", p.base.display())))
            })?;
        // The deployed revision: a block added here and in it is deployed.
        let deployed = match deployed {
            Some(b)
                if req
                    .ops
                    .iter()
                    .any(|o| matches!(o, edit::Op::RemoveBlock { .. })) =>
            {
                Some(
                    edit::parse_doc(&b)
                        .map_err(|e| fail(format!("cannot read the deployed config: {e}")))?,
                )
            }
            _ => None,
        };
        let names = if req
            .ops
            .iter()
            .any(|o| matches!(o, edit::Op::AddBlock { .. }))
        {
            edit::module_names(&p.module)
        } else {
            Default::default()
        };
        let drawn = files.claims.1.clone();
        let ctx = edit::Ctx {
            doc: &doc,
            base: &base,
            lock: &lock,
            deployed: deployed.as_ref(),
            names: &names,
            drawn: &drawn,
        };
        // `removed` lines lox needs no more go first: their slugs are free.
        files.tidy(&lock);
        // A page Loxone Config renamed: its file follows it.
        files.follow_pages(&base);
        let before = files.clone();
        let mut previous = Vec::new();
        // Per op, the slug of the block it added.
        let mut slugs = Vec::new();
        for op in &req.ops {
            let a = edit::apply_op(&mut files, &ctx, op).map_err(bad)?;
            previous.push(a.previous);
            slugs.push(a.added);
        }
        if files == before {
            let mut r = json!({ "previous": previous, "blob": req.base, "unchanged": true });
            undo_into(&mut r, edit::undo_ops(&req.ops, &previous, &[]));
            return Ok(r);
        }
        let mut new = files.render();
        // lox refuses a module directory without a `.loxl` file.
        if new.is_empty() {
            let mut others = false;
            edit::module_files(&p.module, &mut |f, _| {
                others |= f.extension().is_some_and(|x| x == "loxl") && !f.starts_with(&dir_path)
            });
            if !others {
                new.extend([edit::Files::placeholder()]);
            }
        }
        // An edit compiles the whole module: it must not take along changes
        // someone else made to it and has not compiled.
        self.uncompiled(&p, dir).await?;
        // Discard puts the lock and out back as they were before vislox's
        // first edit, where the last commit does not have them.
        snapshot_before_edit(dir, &p).await;
        let mut undo = Undo::new(&dir_path, old.clone(), &[&p.lock, &p.out]);
        std::fs::create_dir_all(&dir_path)
            .map_err(|e| fail(format!("cannot create {shown}: {e}")))?;
        for (name, text) in &new {
            if old.get(name) != Some(text) {
                std::fs::write(dir_path.join(name), text)
                    .map_err(|e| fail(format!("cannot write {shown}/{name}: {e}")))?;
            }
        }
        for name in old.keys().filter(|n| !new.contains_key(*n)) {
            std::fs::remove_file(dir_path.join(name))
                .map_err(|e| fail(format!("cannot remove {shown}/{name}: {e}")))?;
        }
        if new.is_empty() {
            let _ = std::fs::remove_dir(&dir_path);
        }
        edit::check_module(&p.module).map_err(bad)?;
        // A move goes into the lockfile, where lox keeps a managed block's
        // position (docs/editing.md); Undo puts the lock back on failure.
        let place = &files.layout;
        if !place.moves.is_empty() {
            let shown_lock = p.lock.strip_prefix(&p.root).unwrap_or(&p.lock).display();
            let bytes = std::fs::read(&p.lock)
                .map_err(|e| fail(format!("cannot read {shown_lock}: {e}")))?;
            let moved = edit::write_moves(&bytes, &base, &place.moves).map_err(bad)?;
            std::fs::write(&p.lock, moved)
                .map_err(|e| fail(format!("cannot write {shown_lock}: {e}")))?;
        }
        let mut args = vec!["logic", "compile"];
        for title in place.relayout.values() {
            args.extend(["--relayout", title.as_str()]);
        }
        run(&self.s.lox, &args, dir, COMPILE_TIMEOUT)
            .await
            .map_err(|e| {
                bad(format!(
                    "lox logic compile refused the edit:\n{}",
                    last_lines(&e, 8)
                ))
            })?;
        let blob = std::fs::read(&p.out)
            .map(|b| crate::history::blob_id(&b))
            .map_err(|e| {
                fail(format!(
                    "lox logic compile wrote no {}: {e}",
                    p.out.display()
                ))
            })?;
        // What this compile compiled, for the next edit's check. Taken
        // before it is kept: a refused move puts the lock and out back, and
        // then out no longer matches it.
        let compiled = Compiled {
            module: module_blobs(&p),
            out: blob.clone(),
            base: file_blob(&p.base),
        };
        // The UUIDs lox minted for the new blocks.
        let lock = edit::Lock::read(&p.lock);
        // A move lox did not keep is refused, not shown as done. A page laid
        // out afresh places its blocks anew, moved or not.
        if !place.moves.is_empty() {
            let out = std::fs::read(&p.out)
                .map_err(|e| fail(e.to_string()))
                .and_then(|b| edit::parse_doc(&b).map_err(fail))?;
            let missed = edit::moves_missed(&out, &base, &lock, place);
            if !missed.is_empty() {
                return Err(fail(format!(
                    "lox logic compile did not keep the new position of {}",
                    missed.join(", ")
                )));
            }
        }
        let minted: Vec<Option<String>> = slugs
            .iter()
            .map(|s| s.as_deref().and_then(|s| lock.uuid_of(s)).map(String::from))
            .collect();
        let added: Vec<Value> = req
            .ops
            .iter()
            .zip(&slugs)
            .filter_map(|(op, s)| {
                let s = s.as_deref()?;
                let mut a = json!({ "slug": s, "uuid": lock.uuid_of(s) });
                if let edit::Op::AddBlock {
                    restores: Some(was),
                    ..
                } = op
                {
                    a["restores"] = json!(was);
                }
                Some(a)
            })
            .collect();
        undo.keep();
        *self.compiled.lock().unwrap() = Some(compiled);
        let mut r = json!({ "previous": previous, "blob": blob, "added": added });
        undo_into(&mut r, edit::undo_ops(&req.ops, &previous, &minted));
        Ok(r)
    }

    /// Refuse when the module holds changes that are not compiled into the
    /// pending config: an edit's compile would fold them in. See
    /// [`Self::freshness`].
    async fn uncompiled(&self, p: &loxone_config::Project, dir: &Path) -> Result<(), Refusal> {
        use axum::http::StatusCode;
        let refuse = |msg: String| (StatusCode::CONFLICT, msg);
        let files = |changed: &[String]| {
            if changed.is_empty() {
                "its files".to_string()
            } else {
                changed.join(", ")
            }
        };
        match self.freshness(p, dir).await.map_err(refuse)? {
            Fresh::Yes => Ok(()),
            Fresh::Broken { changed, error } => Err(refuse(format!(
                "The module does not compile as it is ({} changed since the last compile). \
                 Fix or discard that, then edit:\n{}",
                files(&changed),
                last_lines(&error, 6)
            ))),
            Fresh::Stale { changed } => Err(refuse(format!(
                "The module has changes that are not compiled yet ({}). An edit compiles the \
                 whole module and would take them along: compile them (lox logic compile) or \
                 discard them first.",
                files(&changed)
            ))),
        }
    }

    /// Whether `out` is the module compiled as it is now. By content, never
    /// by modification times alone (two writes can share a timestamp): a
    /// module and `out` exactly as vislox's last compile left them are
    /// compiled; anything else is compiled as it is into a scratch copy and
    /// compared with `out`. `Err`: it cannot be told (no config to compare
    /// with, no scratch directory).
    async fn freshness(&self, p: &loxone_config::Project, dir: &Path) -> Result<Fresh, String> {
        let module = module_blobs(p);
        let out_blob = file_blob(&p.out);
        let base_blob = file_blob(&p.base);
        let last = self.compiled.lock().unwrap().clone();
        if let (Some(last), Some(o)) = (&last, &out_blob)
            && last.module == module
            && &last.out == o
            && last.base == base_blob
        {
            return Ok(Fresh::Yes);
        }
        // The files that changed: since vislox's last compile, else those
        // written no earlier than out (a guess, for the message only).
        let changed: Vec<String> = match &last {
            Some(last) => module
                .iter()
                .filter(|(f, b)| last.module.get(*f) != Some(*b))
                .map(|(f, _)| f.clone())
                .chain(
                    last.module
                        .keys()
                        .filter(|f| !module.contains_key(*f))
                        .cloned(),
                )
                .collect(),
            None => {
                let out_time = std::fs::metadata(&p.out).and_then(|m| m.modified()).ok();
                let mut v = Vec::new();
                edit::module_files(&p.module, &mut |f, t| {
                    if out_time.is_none_or(|o| t >= o) {
                        v.push(f.strip_prefix(&p.root).unwrap_or(f).display().to_string());
                    }
                });
                v
            }
        };
        // And the base, when out was compiled against another one.
        let mut changed = changed;
        if last.as_ref().is_some_and(|l| l.base != base_blob) {
            changed.push(
                p.base
                    .strip_prefix(&p.root)
                    .unwrap_or(&p.base)
                    .display()
                    .to_string(),
            );
        }
        let reference = std::fs::read(&p.out)
            .or_else(|_| std::fs::read(&p.base))
            .map_err(|e| format!("cannot read the pending config: {e}"))?;
        let scratch = std::env::temp_dir().join(format!(
            "vislox-check-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::create_dir_all(&scratch)
            .map_err(|e| format!("cannot create {}: {e}", scratch.display()))?;
        let _gone = RemoveDir(scratch.clone());
        let (lock, out) = (scratch.join("lock.json"), scratch.join("out.Loxone"));
        if p.lock.exists() {
            std::fs::copy(&p.lock, &lock).map_err(|e| format!("cannot copy the lock: {e}"))?;
        }
        let (lock_s, out_s) = (lock.to_string_lossy(), out.to_string_lossy());
        let args = ["logic", "compile", "--lock", &lock_s, "--out", &out_s];
        match run(&self.s.lox, &args, dir, COMPILE_TIMEOUT).await {
            Err(error) => Ok(Fresh::Broken { changed, error }),
            Ok(_) if std::fs::read(&out).ok().as_deref() == Some(reference.as_slice()) => {
                // Compiled as it is: remembered, so the next edit need not
                // compile it again; unless the module changed meanwhile (an
                // edit while `GET /api/check` compiled), when what compiled
                // is not known.
                if let Some(o) = out_blob
                    && module_blobs(p) == module
                    && file_blob(&p.base) == base_blob
                {
                    *self.compiled.lock().unwrap() = Some(Compiled {
                        module,
                        out: o,
                        base: base_blob,
                    });
                }
                Ok(Fresh::Yes)
            }
            Ok(_) => Ok(Fresh::Stale { changed }),
        }
    }

    /// `lox logic check --json` on the module as it is now (#65): its errors,
    /// or `None` when lox gave no answer vislox can read (an older lox, a
    /// stand-in): then nothing is known, and nothing is refused.
    async fn check_module(&self, p: &loxone_config::Project, dir: &Path) -> Option<check::Checked> {
        let arg = check::module_arg(&p.root, &p.module);
        let (_, stdout, _) = run_out(
            &self.s.lox,
            &["logic", "check", "--json", &arg],
            dir,
            CHECK_TIMEOUT,
        )
        .await
        .ok()?;
        check::parse(&stdout, |f| {
            crate::managed::shown(&p.root, &p.module, &dir.join(f))
        })
    }

    /// [`Self::check_module`] of the project.
    async fn module_errors(&self) -> Option<check::Checked> {
        let dir = self.s.project.as_deref()?;
        let p = self.project().ok()?;
        self.check_module(&p, dir).await
    }

    /// What the check depends on: the module's files and the lock (the
    /// edit key, with the deployed and pending blobs), and the base and out
    /// on disk, all by content.
    fn check_key(&self) -> String {
        let (base, out) = self.disk.lock().unwrap().clone();
        format!(
            "{}|{}|{}",
            self.edit_state.lock().unwrap().key(),
            base.as_deref().unwrap_or("-"),
            out.as_deref().unwrap_or("-")
        )
    }

    /// `GET /api/check`: the last complete check, never a new one (lox runs
    /// in the check worker, on file changes). `current`: nothing changed
    /// since.
    pub fn check_result(&self) -> Value {
        let k = self.check_key();
        match &*self.check_cache.lock().unwrap() {
            Some(c) => {
                let mut v = c.v.clone();
                v["current"] = json!(c.key == k);
                v
            }
            None => json!({ "available": false, "reason": "not checked yet" }),
        }
    }

    /// The check worker: waits for a change ([`Self::refresh_edit`], an
    /// action's end), lets the writer finish, then checks until the key it
    /// checked is the current one.
    pub async fn check_worker(&self) {
        loop {
            self.check_wake.notified().await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            while self.check_once().await {}
        }
    }

    /// Check the module once, if its key changed since the last check kept,
    /// and keep it if nothing changed meanwhile; then lint it (#66) and run
    /// its tests (#67), side by side, as a second step. Returns whether to
    /// work again (the key moved while lox ran, or the second step is still
    /// to run). Never while an action runs: that check would be of a project
    /// half written, so it is not kept; the action's end wakes the worker
    /// again. The check holds the compile lock, so its scratch compile never
    /// runs beside another compile, and an action that starts meanwhile
    /// waits for it; lint and the tests do not (see [`Self::second_once`]).
    pub async fn check_once(&self) -> bool {
        let k = self.check_key();
        let second = match &*self.check_cache.lock().unwrap() {
            Some(c) if c.key == k && !c.lint_due && !c.test_due => return false,
            Some(c) => c.key == k,
            None => false,
        };
        if self.action_running() {
            return false;
        }
        // The key must be of the files lox reads. It is taken when the
        // watcher or the poll looks (`refresh_edit`); a file changed since,
        // and maybe changed back before it looks again, would keep what lox
        // said of the change under the key of the files as they are after
        // it. So the files are looked at now, before and after lox: kept
        // only when both are the key's. A key that lags is brought up to
        // date here, and the worker goes again.
        let files = self.edit_state.lock().unwrap().files.clone();
        let now = self.files_key_now().await;
        if now != files {
            self.set_files(now);
            return true;
        }
        if second {
            return self.second_once(&k, &files).await;
        }
        let v = {
            let Ok(_compiling) = self.compile_lock.try_lock() else {
                return false;
            };
            self.check_now().await
        };
        if self.check_key() != k || self.files_key_now().await != files {
            return true;
        }
        // Errors and hints are shown at once; lint and the tests follow,
        // while what the last ones found stays up (`running`).
        let mut v = v;
        let compiles = v["ok"] == true;
        let mut due = [false; 2];
        for (i, (cmd, list)) in [("lint", "findings"), ("test", "tests")]
            .into_iter()
            .enumerate()
        {
            let unsupported = self.unsupported_now(cmd);
            v[cmd] = if !compiles {
                Value::Null
            } else if let Some(why) = &unsupported {
                json!({ "available": false, "unsupported": true, "reason": why })
            } else {
                let last = self
                    .check_cache
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|c| c.v[cmd][list].as_array().cloned())
                    .unwrap_or_default();
                json!({ "available": true, "running": true, list: last })
            };
            due[i] = compiles && unsupported.is_none();
        }
        *self.check_cache.lock().unwrap() = Some(CheckEntry {
            key: k,
            v,
            lint_due: due[0],
            test_due: due[1],
        });
        self.checked();
        due[0] || due[1]
    }

    /// An action (an edit, Compile, Apply, Sync, Discard) runs.
    fn action_running(&self) -> bool {
        self.busy.try_lock().is_err() || self.st.lock().unwrap().running.is_some()
    }

    /// A check or lint was kept: tell the browsers.
    fn checked(&self) {
        self.check_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.changed();
    }

    /// The module's files changed since the watcher or the poll looked:
    /// the edit key (and so the check's) is the files as they are now.
    fn set_files(&self, files: Option<String>) {
        let mut st = self.edit_state.lock().unwrap();
        if st.files != files {
            st.files = files;
            drop(st);
            self.changed();
        }
    }

    /// The lox binary as it is now: its path, and its time and size when it
    /// is a file (a name looked up in PATH has neither).
    fn lox_identity(&self) -> String {
        let meta = std::fs::metadata(&self.s.lox).ok();
        format!(
            "{}|{:?}|{:?}",
            self.s.lox.display(),
            meta.as_ref().and_then(|m| m.modified().ok()),
            meta.map(|m| m.len())
        )
    }

    /// Why `lox logic <cmd>` (`lint`, `test`) is not run: this lox has
    /// none (found out once).
    fn unsupported_now(&self, cmd: &str) -> Option<String> {
        let me = self.lox_identity();
        (self.unsupported.lock().unwrap().get(cmd) == Some(&me))
            .then(|| format!("this lox has no `lox logic {cmd}`"))
    }

    /// The second step of a check that found no errors: lint (#66) and the
    /// tests (#67), each when still due, side by side. Returns whether to
    /// work again.
    async fn second_once(&self, k: &str, files: &Option<String>) -> bool {
        let (lint, test) = match &*self.check_cache.lock().unwrap() {
            Some(c) if c.key == k => (c.lint_due, c.test_due),
            _ => return true,
        };
        let (a, b) = tokio::join!(async { lint && self.lint_once(k, files).await }, async {
            test && self.test_once(k, files).await
        },);
        a || b
    }

    /// `lox logic test .` on the project (#67), as [`Self::lint_once`] is
    /// run: it compiles the module in memory and writes nothing, so it does
    /// not hold the compile lock, and an action that starts meanwhile stops
    /// it. Kept only when the files are still the ones it read. Returns
    /// whether to work again.
    async fn test_once(&self, k: &str, files: &Option<String>) -> bool {
        let ran = tokio::select! {
            r = self.run_tests() => r,
            _ = self.preempted() => return false,
        };
        if self.check_key() != k || self.files_key_now().await != *files {
            return true;
        }
        let mut cache = self.check_cache.lock().unwrap();
        let Some(c) = cache.as_mut().filter(|c| c.key == k) else {
            return true;
        };
        c.v["test"] = ran;
        c.test_due = false;
        drop(cache);
        self.checked();
        false
    }

    /// `lox logic test .` on the project as it is now, read and pinned to
    /// the module's lines and blocks: `{ available: true, tests, passed,
    /// failed }`, or `{ available: false, reason }` (with `unsupported` for
    /// a lox without it, which is then not asked again).
    async fn run_tests(&self) -> Value {
        let off = |why: String| json!({ "available": false, "reason": why });
        let (Some(dir), Ok(p)) = (self.s.project.as_deref(), self.project()) else {
            return off("only for a lox.toml project".into());
        };
        let args = ["logic", "test", "."];
        let (code, o, e) = match run_status(&self.s.lox, &args, dir, COMPILE_TIMEOUT).await {
            Ok(r) => r,
            Err(reason) => return off(reason),
        };
        match check::parse_test(&o) {
            Some(mut t) => {
                if !t.tests.is_empty() {
                    let texts = {
                        let mut m = self.module_files.lock().unwrap();
                        m.refresh(&p.root, &p.module);
                        m.texts()
                    };
                    let lock = edit::Lock::read(&p.lock);
                    check::locate_tests(&mut t.tests, &texts, &lock);
                }
                let passed = t.tests.iter().filter(|t| t.status == "pass").count();
                json!({ "available": true, "tests": t.tests, "passed": passed,
                        "failed": t.tests.len() - passed })
            }
            None if check::unsupported(code, &o, &e) => {
                self.unsupported
                    .lock()
                    .unwrap()
                    .insert("test", self.lox_identity());
                json!({ "available": false, "unsupported": true,
                        "reason": "this lox has no `lox logic test`" })
            }
            None => {
                let e = last_lines(e.trim().trim_start_matches("Error: "), 3);
                off(if e.is_empty() {
                    "lox logic test gave no answer vislox can read".to_string()
                } else {
                    e
                })
            }
        }
    }

    /// The tests of the exact config Apply deploys (#67): the check
    /// worker's run when it is of the files as they are now and done, else
    /// a run now (kept for the browsers when the check is of these files).
    async fn tests_now(&self) -> Value {
        let k = self.check_key();
        let files = self.edit_state.lock().unwrap().files.clone();
        let fresh = self.files_key_now().await == files;
        if fresh
            && let Some(c) = &*self.check_cache.lock().unwrap()
            && c.key == k
            && !c.test_due
            && c.v["test"].is_object()
        {
            return c.v["test"].clone();
        }
        let v = self.run_tests().await;
        if self.check_key() == k && self.files_key_now().await == files {
            let mut cache = self.check_cache.lock().unwrap();
            if let Some(c) = cache.as_mut().filter(|c| c.key == k && c.v["ok"] == true) {
                c.v["test"] = v.clone();
                c.test_due = false;
                drop(cache);
                self.checked();
            }
        }
        v
    }

    /// `lox logic lint .` on the project (#66), as the second step of a
    /// check that found no errors: lint compiles the module in memory for
    /// its dead-output analysis and writes nothing. It does not hold the
    /// compile lock, so an edit or an Apply never waits for it; one that
    /// starts meanwhile stops it (the action's end wakes the worker, which
    /// lints again). What it found is kept only when the files are still
    /// the ones it read. Returns whether to work again.
    async fn lint_once(&self, k: &str, files: &Option<String>) -> bool {
        let (Some(dir), Ok(p)) = (self.s.project.as_deref(), self.project()) else {
            return false;
        };
        let args = ["logic", "lint", "."];
        let ran = tokio::select! {
            r = run_status(&self.s.lox, &args, dir, COMPILE_TIMEOUT) => r,
            _ = self.preempted() => return false,
        };
        if self.check_key() != k || self.files_key_now().await != *files {
            return true;
        }
        let shown = |f: &str| crate::managed::shown(&p.root, &p.module, &dir.join(f));
        let lint = match ran {
            Ok((code, o, e)) => match check::parse_lint(&o, shown) {
                Some(mut findings) => {
                    if !findings.is_empty() {
                        let texts = {
                            let mut m = self.module_files.lock().unwrap();
                            m.refresh(&p.root, &p.module);
                            m.texts()
                        };
                        let lock = edit::Lock::read(&p.lock);
                        check::locate_findings(&mut findings, &texts, &lock);
                    }
                    json!({ "available": true, "findings": findings })
                }
                None if check::unsupported(code, &o, &e) => {
                    self.unsupported
                        .lock()
                        .unwrap()
                        .insert("lint", self.lox_identity());
                    json!({ "available": false, "unsupported": true,
                            "reason": "this lox has no `lox logic lint`" })
                }
                None => {
                    let e = last_lines(e.trim(), 3);
                    let reason = if e.is_empty() {
                        "lox logic lint gave no answer vislox can read".to_string()
                    } else {
                        e
                    };
                    json!({ "available": false, "reason": reason })
                }
            },
            Err(reason) => json!({ "available": false, "reason": reason }),
        };
        let mut cache = self.check_cache.lock().unwrap();
        let Some(c) = cache.as_mut().filter(|c| c.key == k) else {
            return true;
        };
        c.v["lint"] = lint;
        c.lint_due = false;
        drop(cache);
        self.checked();
        false
    }

    /// Resolves when an action starts: background lint gives way to it.
    async fn preempted(&self) {
        // Registered before the look, so no start in between is missed.
        let started = self.preempt.notified();
        if self.action_running() {
            return;
        }
        started.await;
    }

    /// Whether the module compiles (#65): lox's errors with the blocks each
    /// one's line names, the files `lox logic fmt --check` would format, and
    /// whether the pending config is the module compiled as it is (`stale`).
    /// Lint (#66) is [`Self::lint_once`]'s, after it. The caller holds the
    /// compile lock.
    async fn check_now(&self) -> Value {
        let off = |why: String| json!({ "available": false, "reason": why });
        let Some(dir) = self.s.project.as_deref() else {
            return off("only for a lox.toml project".into());
        };
        let p = match self.project() {
            Ok(p) => p,
            Err(e) => return off(e),
        };
        let arg = check::module_arg(&p.root, &p.module);
        let fmt_args = ["logic", "fmt", "--check", arg.as_str()];
        let (checked, fmt) = tokio::join!(
            self.check_module(&p, dir),
            run_out(&self.s.lox, &fmt_args, dir, CHECK_TIMEOUT)
        );
        let Some(mut checked) = checked else {
            return off("lox logic check gave no answer vislox can read".into());
        };
        let shown = |f: &str| crate::managed::shown(&p.root, &p.module, &dir.join(f));
        if !checked.errors.is_empty() {
            let texts = {
                let mut m = self.module_files.lock().unwrap();
                m.refresh(&p.root, &p.module);
                m.texts()
            };
            let lock = edit::Lock::read(&p.lock);
            let sources = crate::managed::sources(&texts, &lock.slugs);
            check::locate(&mut checked.errors, &texts, &lock, &sources);
        }
        let unformatted = match fmt {
            Ok((_, o, e)) => check::unformatted(&o, &e, shown),
            Err(_) => Vec::new(),
        };
        // Is the pending config the module as it is? Not asked while the
        // module does not compile (it cannot be). Else by content: the
        // record of vislox's last compile, or a scratch compile.
        let stale = if !checked.errors.is_empty() {
            Value::Null
        } else {
            match self.freshness(&p, dir).await {
                Ok(Fresh::Yes) | Err(_) => Value::Null,
                Ok(Fresh::Stale { changed }) => json!({ "files": changed }),
                Ok(Fresh::Broken { changed, error }) => {
                    json!({ "files": changed, "error": last_lines(&error, 6) })
                }
            }
        };
        json!({
            "available": true,
            "ok": checked.errors.is_empty(),
            "errors": checked.errors,
            "truncated": checked.truncated,
            "unformatted": unformatted,
            "stale": stale,
            "lint": Value::Null,
            "test": Value::Null,
        })
    }

    /// Compile the module as it is into the pending config
    /// (`lox logic compile`), as an edit does, without changing it: for a
    /// module changed outside vislox (#65). Never deploys.
    pub async fn compile(&self) -> Result<String, Refusal> {
        use axum::http::StatusCode;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.begin("Compile")?;
        let res = {
            let _compiling = self.compile_lock.lock().await;
            self.compile_steps().await
        };
        self.end("compile", &res);
        res.map_err(|e| (StatusCode::BAD_GATEWAY, e))
    }

    async fn compile_steps(&self) -> Result<String, String> {
        let dir = self.s.project.as_deref().unwrap();
        let p = loxone_config::Project::load(&dir.join("lox.toml"))
            .map_err(|e| format!("cannot read lox.toml: {e}"))?;
        // Discard puts back a lock or out the last commit does not have.
        snapshot_before_edit(dir, &p).await;
        let module = module_blobs(&p);
        let base = file_blob(&p.base);
        run(&self.s.lox, &["logic", "compile"], dir, COMPILE_TIMEOUT)
            .await
            .map_err(|e| format!("lox logic compile failed:\n{}", last_lines(&e, 8)))?;
        if let Some(out) = file_blob(&p.out)
            && module_blobs(&p) == module
        {
            *self.compiled.lock().unwrap() = Some(Compiled { module, out, base });
        }
        Ok(
            "Compiled the module as it is: the pending revision shows it. Nothing was deployed."
                .into(),
        )
    }

    /// What vislox's file claims, and which of it is not deployed: the
    /// pending config's value differs from the deployed config's
    /// (`GET /api/edits`). `deployed` is the revision the Miniserver runs
    /// (its commit, if any, and its bytes); `pending` the newest compile,
    /// `None` when nothing is pending. Cached until [`Self::refresh_edit`]
    /// sees a change.
    pub async fn edits(
        &self,
        deployed: Option<DeployedRev>,
        pending: Option<Arc<Vec<u8>>>,
    ) -> Value {
        let state = self.edit_state.lock().unwrap().clone();
        if let Some((k, v)) = &*self.edits_cache.lock().unwrap()
            && *k == state.key()
        {
            return v.clone();
        }
        let v = self.edits_now(&state, deployed, pending).await;
        *self.edits_cache.lock().unwrap() = Some((state.key(), v.clone()));
        v
    }

    async fn edits_now(
        &self,
        state: &EditState,
        deployed: Option<DeployedRev>,
        pending: Option<Arc<Vec<u8>>>,
    ) -> Value {
        let off = || json!({ "available": false, "reason": state.why, "claims": [], "managed": [], "sources": {} });
        let Some(dir) = self.s.project.as_deref() else {
            return off();
        };
        let Ok(p) = loxone_config::Project::load(&dir.join("lox.toml")) else {
            return off();
        };
        let dir_path = p.module.join(edit::DIR);
        let rel_of = |q: &Path| {
            q.strip_prefix(&p.root)
                .unwrap_or(q)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let rel = rel_of(&p.module.join(edit::FILE));
        let rel_dir = rel_of(&dir_path);
        let mut error = None;
        let now = edit::Files::from_texts(&edit::Files::texts(&dir_path)).unwrap_or_else(|e| {
            error = Some(format!("{rel_dir}/{e}"));
            edit::Files::default()
        });
        // The files as the deployed revision's commit had them: listed and
        // read by their path from the top of the repository (the project
        // may be a directory of it).
        let mut was = edit::Files::default();
        if let Some(commit) = deployed.as_ref().and_then(|d| d.commit.as_deref())
            && let Ok(list) = git(
                dir,
                &[
                    "ls-tree",
                    "--full-name",
                    "--name-only",
                    commit,
                    "--",
                    &format!("./{rel_dir}/"),
                ],
            )
            .await
        {
            let paths: Vec<&str> = list.lines().filter(|l| l.ends_with(".loxl")).collect();
            let specs: Vec<String> = paths.iter().map(|p| format!("{commit}:{p}")).collect();
            let mut texts = std::collections::BTreeMap::new();
            for (path, bytes) in paths
                .iter()
                .zip(git_objects(dir, &specs).await.unwrap_or_default())
            {
                if let Some(b) = bytes {
                    let name = path.rsplit('/').next().unwrap_or(path).to_string();
                    texts.insert(name, String::from_utf8_lossy(&b).into_owned());
                }
            }
            was = edit::Files::from_texts(&texts).unwrap_or_default();
        }
        let lock = edit::Lock::read(&p.lock);
        let deployed_doc = deployed
            .as_ref()
            .and_then(|d| edit::parse_doc(&d.bytes).ok());
        let pending_doc = pending.as_ref().and_then(|b| edit::parse_doc(b).ok());
        // (op without its value) → (claimed now, claimed when deployed)
        type Slot = (edit::Op, (Option<String>, Option<String>));
        let mut slots: std::collections::BTreeMap<String, Slot> = Default::default();
        let key = |op: &edit::Op| serde_json::to_string(op).unwrap_or_default();
        for (ops, now_side) in [(now.ops(&lock), true), (was.ops(&lock), false)] {
            for op in ops {
                let k = op.with_value(None);
                let slot = slots.entry(key(&k)).or_insert((k, (None, None)));
                let v = op.value().map(String::from);
                if now_side { slot.1.0 = v } else { slot.1.1 = v }
            }
        }
        let claims: Vec<Value> = slots
            .into_values()
            .map(|(op, (now, was))| {
                let value_in = |d: &Option<loxone_config::LoxoneDoc>| {
                    d.as_ref().and_then(|d| edit::current(d, &op))
                };
                let deployed_value = value_in(&deployed_doc);
                // Nothing pending: the pending config is the deployed one.
                let pending_value = if pending.is_some() {
                    value_in(&pending_doc)
                } else {
                    deployed_value.clone()
                };
                let mut v = serde_json::to_value(&op).unwrap_or_default();
                v["value"] = json!(now);
                v["deployed"] = json!(was);
                v["pending"] = json!(pending_value != deployed_value);
                v
            })
            .collect();
        // The module's blocks, but for those added here: they are edited here.
        let mut managed: Vec<String> = lock
            .slugs
            .iter()
            .filter(|(s, _)| now.block(s).is_none())
            .map(|(_, u)| u.clone())
            .collect();
        managed.sort();
        // Where the module declares each block it manages, the blocks added
        // here among them (#64): UUID → slug, file and line.
        let texts = {
            let mut m = self.module_files.lock().unwrap();
            m.refresh(&p.root, &p.module);
            m.texts()
        };
        let sources = crate::managed::sources(&texts, &lock.slugs);
        // The blocks lox can move (#60): those it manages that are drawn on a
        // page, and of them those the pending config draws elsewhere than the
        // deployed one, on the same page.
        let mut movable: Vec<&str> = lock.placed.keys().filter_map(|s| lock.uuid_of(s)).collect();
        movable.sort();
        // One walk per config, not one per block.
        let pending_at = pending_doc.as_ref().map(edit::drawn);
        let deployed_at = deployed_doc.as_ref().map(edit::drawn);
        let moves: Vec<Value> = match (&pending_at, &deployed_at) {
            (Some(pd), Some(dd)) => movable
                .iter()
                .filter_map(|u| {
                    let (now_page, now) = pd.get(*u)?;
                    let (was_page, was) = dd.get(*u)?;
                    ((now_page == was_page) && (now.px, now.py) != (was.px, was.py)).then(|| {
                        json!({ "uuid": u, "x": now.px, "y": now.py, "deployed": [was.px, was.py] })
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        // Whether each page with a movable block can be laid out afresh
        // (Arrange): page UUID → null, or why not. On the config shown: the
        // pending one, else the deployed one (lox compiles on its pages).
        let mut arrange = serde_json::Map::new();
        let shown = pending_doc.as_ref().or(deployed_doc.as_ref());
        let shown_at = pending_at.as_ref().or(deployed_at.as_ref());
        if let (Some(doc), Some(at)) = (shown, shown_at) {
            let pages: std::collections::BTreeSet<&str> = movable
                .iter()
                .filter_map(|u| at.get(*u).map(|(p, _)| p.as_str()))
                .collect();
            for page in pages {
                let why = edit::relayout_title(doc, doc, &lock, page).err();
                arrange.insert(page.to_string(), json!(why));
            }
        }
        let blocks: Vec<Value> = now
            .pages
            .iter()
            .flat_map(|(name, f)| f.blocks.iter().map(move |b| (name, f, b)))
            .map(|(name, f, b)| {
                let uuid = lock.uuid_of(&b.slug);
                json!({
                    "slug": b.slug,
                    "uuid": uuid,
                    "type": b.ty,
                    "page": f.page,
                    "file": format!("{rel_dir}/{name}"),
                    "settings": edit::new_block_settings(b),
                    "deployed": uuid.is_some_and(|u| deployed_doc
                        .as_ref()
                        .is_some_and(|d| edit::find(&d.xml.root, u).is_some())),
                })
            })
            .collect();
        // The wires vislox's files draw now and in the deployed revision (#61),
        // by their ends as the page names them.
        let mut wires: std::collections::BTreeMap<(String, String, String, String), (bool, bool)> =
            Default::default();
        for (files, now_side) in [(&now, true), (&was, false)] {
            for (f, t) in edit::wire_ends(files, &lock) {
                let [fu, fk] = f;
                let [tu, tk] = t;
                let slot = wires.entry((fu, fk, tu, tk)).or_default();
                if now_side {
                    slot.0 = true
                } else {
                    slot.1 = true
                }
            }
        }
        let wires: Vec<Value> = wires
            .into_iter()
            .map(|((fu, fk, tu, tk), (now, was))| {
                json!({ "from": [fu, fk], "to": [tu, tk], "now": now, "deployed": was })
            })
            .collect();
        // The wires the rest of the module draws on the config shown, and
        // why vislox leaves them and Loxone Config's alone: one rule with
        // `unwire`'s.
        let module_wires: Vec<Value> = shown
            .map(|d| edit::module_wires(d, &lock, &now))
            .unwrap_or_default()
            .into_iter()
            .map(|(f, t)| json!({ "from": f, "to": t }))
            .collect();
        json!({
            "available": state.why.is_none(),
            "reason": state.why,
            "file": rel,
            "dir": rel_dir,
            "error": error,
            "claims": claims,
            "blocks": blocks,
            "managed": managed,
            "sources": sources,
            "movable": movable,
            "moves": moves,
            "arrange": arrange,
            "wires": wires,
            "module_wires": module_wires,
            "wire_why": {
                "module": edit::Owner::Module.why(),
                "loxone_config": edit::Owner::LoxoneConfig.why(),
            },
        })
    }

    /// Put the module, lock and compiled config back to the last commit.
    pub async fn discard(&self) -> Result<String, Refusal> {
        use axum::http::StatusCode;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.begin("Discard")?;
        let res = {
            let _compiling = self.compile_lock.lock().await;
            self.discard_steps().await
        };
        self.end("discard", &res);
        res.map_err(|e| (StatusCode::BAD_GATEWAY, e))
    }

    async fn discard_steps(&self) -> Result<String, String> {
        let dir = self.s.project.as_deref().unwrap();
        let p = loxone_config::Project::load(&dir.join("lox.toml"))
            .map_err(|e| format!("cannot read lox.toml: {e}"))?;
        let rel = |q: &Path| {
            q.strip_prefix(&p.root)
                .unwrap_or(q)
                .to_string_lossy()
                .into_owned()
        };
        let (module, lock, out) = (rel(&p.module), rel(&p.lock), rel(&p.out));
        // What the last commit has (not the index: a staged file is not in HEAD).
        let in_head = |path: String| async move {
            git(dir, &["cat-file", "-e", &format!("HEAD:./{path}")])
                .await
                .is_ok()
        };
        let (module_in, lock_in, out_in) = (
            in_head(module.clone()).await,
            in_head(lock.clone()).await,
            in_head(out.clone()).await,
        );
        let snapshot = read_snapshot(dir).await;
        // All or nothing: a step that fails puts back what the others did.
        let rollback = Rollback::take(&p.module, &[&p.lock, &p.out]);
        let mut kept = Vec::new();
        let steps = async {
            if module_in {
                git(dir, &["checkout", "-q", "HEAD", "--", &module]).await?;
            }
            git(dir, &["clean", "-fdq", "--", &module]).await?;
            let saved = snapshot.as_ref().map(|s| [&s.lock, &s.out]);
            for (i, (path, abs, committed)) in [(&lock, &p.lock, lock_in), (&out, &p.out, out_in)]
                .into_iter()
                .enumerate()
            {
                if committed {
                    git(dir, &["checkout", "-q", "HEAD", "--", path]).await?;
                    continue;
                }
                // Not in the last commit: back as it was before vislox's
                // first edit. One vislox never saw is not vislox's to touch.
                match saved.map(|s| s[i]) {
                    Some(Some(bytes)) => std::fs::write(abs, bytes)
                        .map_err(|e| format!("cannot write {path}: {e}"))?,
                    Some(None) if abs.exists() => std::fs::remove_file(abs)
                        .map_err(|e| format!("cannot remove {path}: {e}"))?,
                    Some(None) => {}
                    None if abs.exists() => kept.push(path.clone()),
                    None => {}
                }
            }
            Ok::<(), String>(())
        };
        if let Err(e) = steps.await {
            rollback.restore();
            return Err(format!("{e}\nNothing was discarded."));
        }
        clear_snapshot(dir).await;
        // What vislox last compiled is not what is there now: the next check
        // compares by content, and names the files written after out.
        *self.compiled.lock().unwrap() = None;
        let mut msg =
            String::from("The module, lock and compiled config are back at the last commit.");
        if !kept.is_empty() {
            msg.push_str(&format!(
                " {} not in git and changed by someone other than vislox: left as it is.",
                kept.join(" and ")
            ));
        }
        Ok(msg)
    }
}

/// The user's answers to a refused Apply.
#[derive(serde::Deserialize, Default)]
pub struct ApplyOpts {
    /// Accept a base saved by this Loxone Config release (`--accept-version`).
    pub accept_version: Option<String>,
    /// Delete the custom changes the Miniserver holds (`--discard-custom-changes`).
    #[serde(default)]
    pub discard_custom_changes: bool,
}

/// What a refused Apply could be retried with, from lox's message: the
/// Loxone Config release to accept, and the custom changes an upload would
/// delete.
pub fn apply_hints(msg: &str) -> Value {
    let accept_version = msg.split("--accept-version").nth(1).and_then(|rest| {
        let v: String = rest
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        (!v.is_empty()).then_some(v)
    });
    let mut custom = Vec::new();
    let mut lines = msg.lines();
    while let Some(l) = lines.next() {
        if l.contains("custom change(s)") {
            for item in lines.by_ref() {
                if !item.starts_with(char::is_whitespace) || item.trim().is_empty() {
                    break;
                }
                custom.push(item.trim().to_string());
            }
            break;
        }
    }
    json!({
        "accept_version": accept_version,
        "custom_changes": (!custom.is_empty() && msg.contains("--discard-custom-changes")).then_some(custom),
    })
}

/// After a verified push, lox points `base` in lox.toml at its download in
/// `backups/`, which is git-ignored (configs carry personal data). This
/// project keeps its deployed config in git instead: copy that download to
/// where the base was and point lox.toml back at it.
fn keep_base(dir: &Path, old_base: &Path) -> Result<Option<String>, String> {
    let toml = dir.join("lox.toml");
    let p =
        loxone_config::Project::load(&toml).map_err(|e| format!("cannot read lox.toml: {e}"))?;
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    if same(&p.base, old_base) {
        return Ok(None);
    }
    std::fs::copy(&p.base, old_base).map_err(|e| {
        format!(
            "cannot copy {} to {}: {e}",
            p.base.display(),
            old_base.display()
        )
    })?;
    let rel = old_base
        .strip_prefix(&p.root)
        .unwrap_or(old_base)
        .to_string_lossy()
        .into_owned();
    let text = std::fs::read_to_string(&toml).map_err(|e| format!("cannot read lox.toml: {e}"))?;
    std::fs::write(&toml, set_base(&text, &rel))
        .map_err(|e| format!("cannot write lox.toml: {e}"))?;
    Ok(Some(format!(
        "Kept the verified download as {rel} (lox had pointed lox.toml at {}).",
        p.base.strip_prefix(&p.root).unwrap_or(&p.base).display()
    )))
}

/// lox.toml with its `base = …` line pointing at `rel`; comments and other
/// keys stay as they are.
fn set_base(toml: &str, rel: &str) -> String {
    let mut out: Vec<String> = toml
        .lines()
        .map(|l| {
            let key = l.trim_start();
            let is_base = key
                .strip_prefix("base")
                .is_some_and(|rest| rest.trim_start().starts_with('='));
            if is_base {
                format!("base   = {}", toml_string(rel))
            } else {
                l.to_string()
            }
        })
        .collect();
    if toml.ends_with('\n') {
        out.push(String::new());
    }
    out.join("\n")
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// [`run`], with each line of its output (standard output and error) handed
/// to `on_line` as it comes.
async fn run_lines(
    cmd: &Path,
    args: &[&str],
    dir: &Path,
    limit: Duration,
    mut on_line: impl FnMut(&str),
) -> Result<String, String> {
    use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
    let mut c = tokio::process::Command::new(cmd);
    c.args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = c
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", cmd.display()))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    fn pipe(
        r: impl AsyncRead + Unpin + Send + 'static,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) {
        tokio::spawn(async move {
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    pipe(child.stdout.take().expect("piped"), tx.clone());
    pipe(child.stderr.take().expect("piped"), tx);
    let mut text = String::new();
    let work = async {
        while let Some(l) = rx.recv().await {
            on_line(&l);
            text.push_str(&l);
            text.push('\n');
        }
        child.wait().await
    };
    let status = tokio::time::timeout(limit, work)
        .await
        .map_err(|_| format!("took longer than {} s", limit.as_secs()))?
        .map_err(|e| format!("cannot run {}: {e}", cmd.display()))?;
    if status.success() {
        Ok(text)
    } else if text.trim().is_empty() {
        Err(format!("exit {status}"))
    } else {
        Err(text)
    }
}

/// Run a command to its end: whether it succeeded, its standard output and
/// its standard error, apart. `Err` only when it did not run (or ran too
/// long).
async fn run_out(
    cmd: &Path,
    args: &[&str],
    dir: &Path,
    limit: Duration,
) -> Result<(bool, String, String), String> {
    let (code, o, e) = run_status(cmd, args, dir, limit).await?;
    Ok((code == Some(0), o, e))
}

/// [`run_out`] with the exit code (`None`: ended by a signal). Dropped
/// before its end, it kills the command.
async fn run_status(
    cmd: &Path,
    args: &[&str],
    dir: &Path,
    limit: Duration,
) -> Result<(Option<i32>, String, String), String> {
    let mut c = tokio::process::Command::new(cmd);
    c.args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(limit, c.output())
        .await
        .map_err(|_| format!("took longer than {} s", limit.as_secs()))?
        .map_err(|e| format!("cannot run {}: {e}", cmd.display()))?;
    Ok((
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// Why Apply must not run: the module has errors (#65), the first five of
/// them; `None` when it has none.
/// What Apply's *Tests pass* step makes of `lox logic test` (#67): `ok`
/// with how many pass, `skip` with why none ran, or the refusal: each test
/// that fails or could not run (the first five), with its file and line and
/// the `expect`s that did not hold.
fn tests_verdict(t: &Value) -> Result<(&'static str, String), String> {
    if t["available"] != true {
        let why = t["reason"]
            .as_str()
            .unwrap_or("lox logic test gave no answer");
        return Ok(("skip", why.to_string()));
    }
    let tests = t["tests"].as_array().cloned().unwrap_or_default();
    if tests.is_empty() {
        return Ok(("skip", "the module has no tests".into()));
    }
    let bad: Vec<&Value> = tests.iter().filter(|t| t["status"] != "pass").collect();
    if bad.is_empty() {
        let n = tests.len();
        return Ok((
            "ok",
            format!("{n} test{} passed", if n == 1 { "" } else { "s" }),
        ));
    }
    let mut list = Vec::new();
    for t in bad.iter().take(5) {
        let at = match (t["file"].as_str(), t["line"].as_u64()) {
            (Some(f), Some(l)) => format!(" ({f}:{l})"),
            _ => String::new(),
        };
        let how = if t["status"] == "error" {
            "could not run"
        } else {
            "failed"
        };
        list.push(format!(
            "  test \"{}\" {how}{at}",
            t["name"].as_str().unwrap_or("")
        ));
        for f in t["failures"].as_array().into_iter().flatten().take(3) {
            list.push(format!(
                "    {} — actual {}",
                f["expect"].as_str().unwrap_or(""),
                f["actual"].as_str().unwrap_or("?")
            ));
        }
        if let Some(e) = t["error"].as_str() {
            list.push(format!("    {}", last_lines(e, 1)));
        }
    }
    let n = bad.len();
    Err(format!(
        "{n} of {} test{} {} (lox logic test), so nothing was deployed. Fix the module or its tests, then apply.\n{}{}",
        tests.len(),
        if tests.len() == 1 { "" } else { "s" },
        if n == 1 { "fails" } else { "fail" },
        list.join("\n"),
        if n > 5 { "\n  …" } else { "" }
    ))
}

fn broken_message(c: &check::Checked) -> Option<String> {
    if c.errors.is_empty() {
        return None;
    }
    let n = c.errors.len() + c.truncated;
    let list: Vec<String> = c
        .errors
        .iter()
        .take(5)
        .map(|e| {
            let at = match (&e.file, e.line) {
                (Some(f), Some(l)) => format!("{f}:{l}: "),
                (Some(f), None) => format!("{f}: "),
                _ => String::new(),
            };
            format!("  {at}{}", e.message)
        })
        .collect();
    Some(format!(
        "The module doesn't compile ({n} error{}), so the pending config is from the last \
         compile that worked. Fix the module, then apply.\n{}{}",
        if n == 1 { "" } else { "s" },
        list.join("\n"),
        if n > 5 { "\n  …" } else { "" }
    ))
}

async fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(Path::new("git"), args, dir, GIT_TIMEOUT)
        .await
        .map_err(|e| format!("git {}: {e}", args.first().unwrap_or(&"")))
}

/// Run a command; its output, or its output as the error when it fails.
async fn run(cmd: &Path, args: &[&str], dir: &Path, limit: Duration) -> Result<String, String> {
    let mut c = tokio::process::Command::new(cmd);
    c.args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    let out = tokio::time::timeout(limit, c.output())
        .await
        .map_err(|_| format!("took longer than {} s", limit.as_secs()))?
        .map_err(|e| format!("cannot run {}: {e}", cmd.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(if text.trim().is_empty() {
            format!("exit {}", out.status)
        } else {
            text
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn actions(token: Option<&str>, project: Option<&Path>) -> Actions {
        Actions::new(Settings {
            token: token.map(String::from),
            project: project.map(Path::to_path_buf),
            lox: PathBuf::from("lox"),
            probe: None,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
        })
    }

    #[test]
    fn actions_need_a_project_and_a_token() {
        let p = Path::new("/tmp");
        let i = actions(None, None).info();
        assert_eq!(i["shown"], false);
        assert_eq!(i["apply"]["reason"], "only for a lox.toml project");
        let i = actions(None, Some(p)).info();
        assert_eq!(i["shown"], true);
        assert!(
            i["apply"]["reason"]
                .as_str()
                .unwrap()
                .contains("--admin-token-file")
        );
        let i = actions(Some("t"), Some(p)).info();
        assert_eq!(i["apply"]["available"], true);
        assert!(i.get("ai").is_none() && i.get("requests").is_none());
    }

    #[test]
    fn the_token_must_match() {
        let a = actions(Some("s3cret"), None);
        let mut h = HeaderMap::new();
        assert!(a.check(&h).is_err());
        h.insert("x-vislox-token", "s3cre".parse().unwrap());
        assert!(a.check(&h).is_err());
        h.insert("x-vislox-token", "s3cret".parse().unwrap());
        assert!(a.check(&h).is_ok());
        assert!(actions(None, None).check(&h).is_err());
    }

    #[test]
    fn a_refused_push_says_what_it_could_be_retried_with() {
        let version = "Error: Compiling the module failed: compile error: Base config saved by Loxone \
                       Config 17021021, but the lockfile expects 17020828. Run `loxl compile --accept-version 17021021`.";
        assert_eq!(apply_hints(version)["accept_version"], "17021021");
        assert!(apply_hints(version)["custom_changes"].is_null());
        let custom = "Downloading the live config...\n\
            The Miniserver holds 2 custom change(s) that are not in the program file (made in the app):\n  \
              2190f3b7-0017 (LoxAIRDevice, not in the program file)\n  \
              \"Budzik\" (AlarmClock, page \"Pokój\")\n\
            Error: refusing: loading the upload deletes them (pass --discard-custom-changes to delete them)";
        let h = apply_hints(custom);
        assert_eq!(
            h["custom_changes"][1],
            "\"Budzik\" (AlarmClock, page \"Pokój\")"
        );
        assert!(h["accept_version"].is_null());
        assert!(apply_hints("lox config push failed: network down")["custom_changes"].is_null());
    }

    #[test]
    fn the_base_line_is_pointed_back_and_the_rest_kept() {
        let toml = "# comment about base = here\nbase = \"backups/20261106-003332-post-push.Loxone\"\nmodule = \"pages\"\nbaseline = 1\n";
        assert_eq!(
            set_base(toml, "base.Loxone"),
            "# comment about base = here\nbase   = \"base.Loxone\"\nmodule = \"pages\"\nbaseline = 1\n"
        );
    }

    #[test]
    fn the_verified_download_becomes_the_tracked_base() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::create_dir_all(d.join("backups")).unwrap();
        std::fs::write(d.join("base.Loxone"), "old").unwrap();
        std::fs::write(d.join("backups/post.Loxone"), "verified").unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"backups/post.Loxone\"\nmodule = \"pages\"\n",
        )
        .unwrap();
        let msg = keep_base(d, &d.join("base.Loxone")).unwrap().unwrap();
        assert!(msg.contains("base.Loxone"), "{msg}");
        assert_eq!(
            std::fs::read_to_string(d.join("base.Loxone")).unwrap(),
            "verified"
        );
        assert!(
            std::fs::read_to_string(d.join("lox.toml"))
                .unwrap()
                .starts_with("base   = \"base.Loxone\"")
        );
        // Nothing to do when the base did not move.
        assert!(keep_base(d, &d.join("base.Loxone")).unwrap().is_none());
    }

    #[test]
    fn lox_lines_move_the_steps_along() {
        let mut p = Progress::apply();
        let state = |p: &mut Progress, id| p.get(id).state;
        p.lox_line("Downloading the live config...");
        assert_eq!(state(&mut p, "check"), "run");
        p.lox_line("Changes (live → compiled):");
        p.lox_line("Uploading as sps_new.zip...");
        assert_eq!(state(&mut p, "check"), "ok");
        assert_eq!(state(&mut p, "upload"), "run");
        p.lox_line("Restarting the program...");
        assert_eq!(state(&mut p, "upload"), "ok");
        assert_eq!(state(&mut p, "restart"), "run");
        p.restart_event(restart::Event::OutOfService("503, restarting".into()));
        assert_eq!(state(&mut p, "online"), "run");
        p.restart_event(restart::Event::BackOnline(7));
        assert_eq!(p.get("online").detail.as_deref(), Some("after 7 s"));
        p.restart_event(restart::Event::Loaded(8, "2026-11-06 10:00:00".into()));
        p.lox_line("Downloading the config again to verify...");
        // What vislox saw stays; the restart is done.
        assert_eq!(state(&mut p, "restart"), "ok");
        assert_eq!(
            p.get("restart").detail.as_deref(),
            Some("out of service (503, restarting)")
        );
        assert_eq!(state(&mut p, "loaded"), "ok");
        assert!(
            p.get("loaded")
                .detail
                .as_deref()
                .unwrap()
                .contains("2026-11-06")
        );
        assert_eq!(state(&mut p, "verify"), "run");
        p.lox_line("Pushed and verified. lox.toml now points at backups/x.Loxone; commit …");
        assert_eq!(state(&mut p, "verify"), "ok");
        assert_eq!(p.log.len(), 6);
    }

    #[test]
    fn a_restart_without_an_outage_and_a_failure() {
        let mut p = Progress::apply();
        p.lox_line("Restarting the program...");
        p.restart_event(restart::Event::Loaded(2, "v2".into()));
        assert_eq!(p.get("restart").state, "ok");
        assert_eq!(
            p.get("online").detail.as_deref(),
            Some("answered throughout")
        );
        p.lox_line("Downloading the config again to verify...");
        p.fail("lox config push failed:\nThe Miniserver does not run what was pushed");
        assert_eq!(p.get("verify").state, "fail");
        assert_eq!(
            p.get("verify").detail.as_deref(),
            Some("The Miniserver does not run what was pushed")
        );
        assert_eq!(p.get("commit").state, "wait");
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

    /// A lox.toml project in git with a remote, and a `lox` that says what
    /// `lox config push` says and "deploys" by copying out over the base.
    #[cfg(unix)]
    fn project_with_fake_lox(push_ok: bool) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("project");
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::write(d.join("base.Loxone"), "old").unwrap();
        std::fs::write(d.join("out.Loxone"), "new").unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\n",
        )
        .unwrap();
        sh(tmp.path(), "git init -q --bare remote.git");
        sh(
            &d,
            "git init -q && git config user.email t@t && git config user.name t \
             && git add -A && git commit -qm init && git remote add origin ../remote.git \
             && git push -q -u origin HEAD",
        );
        let lox = tmp.path().join("lox");
        let push = if push_ok {
            "echo 'Downloading the config again to verify...' >&2\n    cp out.Loxone base.Loxone\n    echo 'Pushed and verified.' >&2"
        } else {
            "echo 'The Miniserver did not load the upload within a minute.' >&2\n    exit 1"
        };
        std::fs::write(
            &lox,
            format!(
                "#!/bin/sh\ncase \"$1 $2\" in\n  'logic diff') echo 'Add a hallway light' ;;\n  'config push')\n    echo 'Downloading the live config...' >&2\n    echo 'Uploading as sps_new.zip...' >&2\n    echo 'Restarting the program...' >&2\n    sleep 2\n    {push} ;;\nesac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
        (tmp, lox)
    }

    /// A lox.toml project on the test fixture with nothing pending, and a
    /// `lox` whose `logic compile` writes the title vislox claims for the
    /// Divide block into out (or into `--out`). It refuses a title with
    /// "refuse" in it, writes half an out and hangs for one with "slow", and
    /// marks out when the module has a `foo.loxl` of someone else's. It logs
    /// its calls, and fails on any other command.
    #[cfg(unix)]
    fn project_for_edits() -> (tempfile::TempDir, Actions) {
        project_for_edits_in("project")
    }

    /// [`project_for_edits`] at `rel` in the temporary directory, in a git
    /// repository at its first component: `repo/project` is a project in a
    /// directory of its repository.
    #[cfg(unix)]
    fn project_for_edits_in(rel: &str) -> (tempfile::TempDir, Actions) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join(rel);
        let repo = tmp.path().join(rel.split('/').next().unwrap());
        std::fs::create_dir_all(d.join("pages")).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/house.Loxone");
        std::fs::copy(&fixture, d.join("base.Loxone")).unwrap();
        std::fs::copy(&fixture, d.join("out.Loxone")).unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\n",
        )
        .unwrap();
        sh(
            &repo,
            "git init -q && git config user.email t@t && git config user.name t \
             && git add -A && git commit -qm init",
        );
        let lox = tmp.path().join("lox");
        std::fs::write(
            &lox,
            r#"#!/bin/sh
echo "$*" >> ../calls.log
[ "$1 $2" = "logic compile" ] || { echo "unexpected: $*" >&2; exit 2; }
out=out.Loxone; prev=
for a in "$@"; do [ "$prev" = --out ] && out=$a; prev=$a; done
t=$(sed -n 's/.*\.title = "\(.*\)"$/\1/p' pages/vislox/edits.loxl 2>/dev/null)
case "$t" in
  *refuse*) echo 'compile error: refused' >&2; exit 1 ;;
  *slow*) echo half > "$out"; echo half > logic.lock.json; exec sleep 30 ;;
esac
# No claim: lox puts the original title back.
if [ -z "$t" ]; then sed 's/Title="Teiler"/Title="O1002"/' base.Loxone > "$out"; else sed "s/Title=\"O1002\"/Title=\"$t\"/" base.Loxone > "$out"; fi
if [ -f pages/foo.loxl ]; then echo "<!-- foo -->" >> "$out"; fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d),
            lox,
            probe: None,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
        });
        (tmp, a)
    }

    const DIV: &str = "10000006-0000-0060-ffff504f94000001";

    fn title(base: String, v: Option<&str>) -> edit::Request {
        edit::Request {
            base,
            ops: vec![edit::Op::Title {
                uuid: DIV.into(),
                value: v.map(Into::into),
            }],
        }
    }

    /// The deployed revision as main.rs hands it to an edit: the base as
    /// its last commit has it.
    fn deployed(d: &Path) -> Option<Arc<Vec<u8>>> {
        let out = std::process::Command::new("git")
            .args(["show", "HEAD:./base.Loxone"])
            .current_dir(d)
            .output()
            .ok()?;
        out.status.success().then(|| Arc::new(out.stdout))
    }

    fn blob(d: &Path, p: &str) -> String {
        crate::history::blob_id(&std::fs::read(d.join(p)).unwrap())
    }

    /// `GET /api/edits` as main.rs asks for it: the deployed revision is the
    /// last commit of the base, pending is out when it differs.
    async fn edits_of(a: &Actions, d: &Path) -> Value {
        let commit = git(d, &["log", "-1", "--format=%H", "--", "base.Loxone"])
            .await
            .unwrap()
            .trim()
            .to_string();
        let bytes = Arc::new(std::fs::read(d.join("base.Loxone")).unwrap());
        let out = Arc::new(std::fs::read(d.join("out.Loxone")).unwrap());
        let pending = (out != bytes).then_some(out);
        let pending_blob = pending.as_ref().map(|b| crate::history::blob_id(b));
        a.refresh_edit(Some(&blob(d, "base.Loxone")), pending_blob.as_deref());
        let deployed = DeployedRev {
            commit: Some(commit),
            bytes,
        };
        a.edits(Some(deployed), pending).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_edit_is_compiled_into_the_pending_config_and_never_deployed() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        let blob = |p: &str| blob(&d, p);
        assert_eq!(a.info()["edit"]["available"], true);

        let r = a
            .edit(&title(blob("out.Loxone"), Some("Teiler")), None)
            .await
            .unwrap();
        assert_eq!(r["previous"], json!([null]));
        assert_eq!(r["blob"], json!(blob("out.Loxone")));
        assert!(
            std::fs::read_to_string(d.join("out.Loxone"))
                .unwrap()
                .contains("Title=\"Teiler\"")
        );
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(file.contains(".title = \"Teiler\""), "{file}");
        let e = edits_of(&a, &d).await;
        assert_eq!(e["file"], "pages/vislox/edits.loxl");
        let c = &e["claims"][0];
        assert_eq!(
            (&c["value"], &c["deployed"], &c["pending"]),
            (&json!("Teiler"), &Value::Null, &json!(true))
        );

        // Made against a config that is no longer pending.
        let (code, msg) = a
            .edit(&title(blob("base.Loxone"), Some("X")), None)
            .await
            .unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT, "{msg}");
        // lox refuses: the file, the lock and out are as they were.
        let out = blob("out.Loxone");
        let (code, msg) = a
            .edit(&title(out.clone(), Some("refuse me")), None)
            .await
            .unwrap_err();
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(msg.contains("compile error: refused"), "{msg}");
        assert_eq!(
            std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap(),
            file
        );
        assert_eq!(blob("out.Loxone"), out);
        // A value vislox refuses never reaches lox.
        let calls = || std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        let before = calls();
        let op = edit::Op::Param {
            uuid: DIV.into(),
            port: "Input1".into(),
            value: Some("3".into()),
        };
        let (code, msg) = a
            .edit(
                &edit::Request {
                    base: out.clone(),
                    ops: vec![op],
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(msg.contains("wired"), "{msg}");
        assert_eq!(calls(), before);

        // A plain commit does not deploy anything: still not deployed.
        sh(&d, "git add -A && git commit -qm 'just a commit'");
        assert_eq!(edits_of(&a, &d).await["claims"][0]["pending"], true);
        // Applied (the base is what was compiled, committed): deployed.
        sh(
            &d,
            "cp out.Loxone base.Loxone && git add -A && git commit -qm applied",
        );
        let c = edits_of(&a, &d).await["claims"][0].clone();
        assert_eq!(
            (&c["deployed"], &c["pending"]),
            (&json!("Teiler"), &json!(false))
        );
        // Dropping the claim is an edit too: back to Loxone Config's title,
        // not deployed; Revert would claim the deployed value again.
        let r = a
            .edit(&title(blob("out.Loxone"), None), None)
            .await
            .unwrap();
        assert_eq!(r["previous"], json!(["Teiler"]));
        let c = edits_of(&a, &d).await["claims"][0].clone();
        assert_eq!(
            (&c["value"], &c["deployed"], &c["pending"]),
            (&Value::Null, &json!("Teiler"), &json!(true))
        );
        // Only `lox logic compile` ran: nothing was pushed.
        assert!(
            calls().lines().all(|l| l.starts_with("logic compile")),
            "{}",
            calls()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_dropped_edit_clears_running_and_puts_everything_back() {
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        let out = std::fs::read(d.join("out.Loxone")).unwrap();
        let req = title(blob(&d, "out.Loxone"), Some("slow"));
        // The browser goes away while lox compiles (and has written half of out).
        let edit = a.edit(&req, None);
        let r = tokio::time::timeout(Duration::from_millis(1500), edit).await;
        assert!(r.is_err(), "the compile should still run: {r:?}");
        assert_eq!(a.info()["running"], Value::Null);
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out);
        assert!(!d.join("logic.lock.json").exists());
        assert!(!d.join("pages/vislox").exists());
        // And the next edit runs.
        a.edit(&title(blob(&d, "out.Loxone"), Some("Teiler")), None)
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_edit_does_not_take_along_module_changes_nobody_compiled() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        // A file newer than out that compiles to the same config: fine.
        std::fs::write(d.join("pages/notes.txt"), "x").unwrap();
        a.edit(&title(blob(&d, "out.Loxone"), Some("Teiler")), None)
            .await
            .unwrap();
        // As the last edit left it: no scratch compile needed.
        let compiles = || {
            std::fs::read_to_string(tmp.path().join("calls.log"))
                .unwrap()
                .lines()
                .count()
        };
        let n = compiles();
        a.edit(&title(blob(&d, "out.Loxone"), Some("Teiler2")), None)
            .await
            .unwrap();
        assert_eq!(compiles(), n + 1);
        // Someone else's change, not compiled: refused, nothing written. By
        // content, whatever the file's time: here older than out (copied
        // in with its time kept), or written in the same clock tick.
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        let out = blob(&d, "out.Loxone");
        let out_time = std::fs::metadata(d.join("out.Loxone"))
            .unwrap()
            .modified()
            .unwrap();
        for when in [out_time - std::time::Duration::from_secs(3600), out_time] {
            std::fs::write(d.join("pages/foo.loxl"), "# someone else's\n").unwrap();
            std::fs::File::options()
                .write(true)
                .open(d.join("pages/foo.loxl"))
                .unwrap()
                .set_modified(when)
                .unwrap();
            let (code, msg) = a
                .edit(&title(out.clone(), Some("Neu")), None)
                .await
                .unwrap_err();
            assert_eq!(code, StatusCode::CONFLICT);
            assert!(
                msg.contains("not compiled yet") && msg.contains("pages/foo.loxl"),
                "{msg}"
            );
            assert_eq!(
                std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap(),
                file
            );
            assert_eq!(blob(&d, "out.Loxone"), out);
        }
    }

    /// Without a compile of vislox's to go by (after a restart), the module
    /// is compiled as it is and compared, whatever the files' times.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_uncompiled_change_is_found_without_a_previous_edit() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        let old = std::fs::metadata(d.join("out.Loxone"))
            .unwrap()
            .modified()
            .unwrap()
            - std::time::Duration::from_secs(3600);
        std::fs::write(d.join("pages/foo.loxl"), "# someone else's\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(d.join("pages/foo.loxl"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let out = blob(&d, "out.Loxone");
        let (code, msg) = a
            .edit(&title(out.clone(), Some("Neu")), None)
            .await
            .unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(msg.contains("not compiled yet"), "{msg}");
        assert!(!d.join("pages").join(edit::FILE).exists());
        assert_eq!(blob(&d, "out.Loxone"), out);
    }

    /// The stand-in lox of [`project_for_edits`], made to add blocks: its
    /// `logic compile` mints a UUID for each block of vislox's page files
    /// (in the lock) and draws it into out, titled by its slug.
    #[cfg(unix)]
    fn lox_that_adds_blocks(tmp: &Path) {
        std::fs::write(
            tmp.join("lox"),
            r#"#!/bin/sh
echo "$*" >> ../calls.log
[ "$1 $2" = "logic compile" ] || { echo "unexpected: $*" >&2; exit 2; }
out=out.Loxone; lock=logic.lock.json; prev=
for a in "$@"; do [ "$prev" = --out ] && out=$a; [ "$prev" = --lock ] && lock=$a; prev=$a; done
n=0; objs=""; els=""
for s in $(cat pages/vislox/*.loxl 2>/dev/null | sed -n 's/^\(vx_[a-z0-9_]*\) = .*/\1/p'); do
  n=$((n+1)); u="aaaaaaa$n-0000-0000-ffff504f94000001"
  objs="$objs${objs:+,}\"$s\":{\"uuid\":\"$u\",\"type\":\"And\"}"
  els="$els<C Type=\"And\" U=\"$u\" Title=\"$s\"><Co K=\"I1\"/></C>
"
done
echo "{\"objects\":{$objs}}" > "$lock"
{ sed '$d' base.Loxone | sed '$d'; printf '%s' "$els"; printf '\t</C>\n</ControlList>\n'; } > "$out"
"#,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_block_is_added_compiled_and_taken_out_never_deployed() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        lox_that_adds_blocks(tmp.path());
        let d = tmp.path().join("project");
        let req = |ops: Vec<edit::Op>| edit::Request {
            base: blob(&d, "out.Loxone"),
            ops,
        };
        let add = edit::Op::AddBlock {
            page: "10000008-0000-0080-ffff504f94000001".into(),
            ty: "And".into(),
            title: Some("Licht".into()),
            params: [("I2".to_string(), "1".to_string())].into(),
            settings: Default::default(),
            wires: vec![],
            restores: None,
        };
        let r = a.edit(&req(vec![add.clone()]), None).await.unwrap();
        let uuid = "aaaaaaa1-0000-0000-ffff504f94000001";
        assert_eq!(r["previous"], json!([null]));
        assert_eq!(r["added"], json!([{ "slug": "vx_and_1", "uuid": uuid }]));
        let file = std::fs::read_to_string(d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        assert!(file.contains("page \"Obergeschoss\"\n"), "{file}");
        assert!(
            file.contains("vx_and_1 = And(\"Licht\", In2: 1)\n"),
            "{file}"
        );
        assert!(!d.join("pages").join(edit::FILE).exists());

        let e = edits_of(&a, &d).await;
        assert_eq!(e["managed"], json!([]), "{e}");
        let b = &e["blocks"][0];
        assert_eq!(
            (&b["slug"], &b["uuid"], &b["page"], &b["deployed"]),
            (
                &json!("vx_and_1"),
                &json!(uuid),
                &json!("Obergeschoss"),
                &json!(false)
            )
        );
        assert_eq!(b["file"], "pages/vislox/obergeschoss.loxl");

        // Named again: the declaration changes, no claim is made.
        let title = edit::Op::Title {
            uuid: uuid.into(),
            value: Some("Licht an".into()),
        };
        let r = a.edit(&req(vec![title]), None).await.unwrap();
        assert_eq!(r["previous"], json!(["Licht"]));
        let file = std::fs::read_to_string(d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        assert!(
            file.contains("vx_and_1 = And(\"Licht an\", In2: 1)\n"),
            "{file}"
        );

        // Taken out: its previous adds it again; the file says `removed`.
        let r = a
            .edit(
                &req(vec![edit::Op::RemoveBlock { uuid: uuid.into() }]),
                None,
            )
            .await
            .unwrap();
        assert_eq!(r["previous"][0]["op"], "add_block");
        assert_eq!(r["previous"][0]["title"], "Licht an");
        assert_eq!(r["previous"][0]["params"], json!({ "I2": "1" }));
        let file = std::fs::read_to_string(d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        assert!(
            file.contains("removed vx_and_1\n") && !file.contains("= And"),
            "{file}"
        );
        // The stand-in's lock forgets it at once (lox keeps a tombstone, and
        // the slug is not picked again): the next edit drops the line.
        let r = a.edit(&req(vec![add.clone()]), None).await.unwrap();
        assert_eq!(r["added"][0]["slug"], "vx_and_1");
        let file = std::fs::read_to_string(d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        assert!(!file.contains("removed"), "{file}");

        // Applied: deployed, and no longer taken out here.
        sh(
            &d,
            "cp out.Loxone base.Loxone && git add -A && git commit -qm applied",
        );
        assert_eq!(edits_of(&a, &d).await["blocks"][0]["deployed"], true);
        let (code, msg) = a
            .edit(
                &req(vec![edit::Op::RemoveBlock { uuid: uuid.into() }]),
                deployed(&d),
            )
            .await
            .unwrap_err();
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(msg.contains("is deployed"), "{msg}");
        // A file of vislox's that someone edited by hand is refused, untouched.
        std::fs::write(
            d.join("pages/vislox/obergeschoss.loxl"),
            "page \"Obergeschoss\"\nmine = And()\n",
        )
        .unwrap();
        let (code, msg) = a.edit(&req(vec![add]), None).await.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(
            msg.contains("obergeschoss.loxl") && msg.contains("did not write"),
            "{msg}"
        );
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// A stand-in lox for moves: it draws the Divide block, which the
    /// module manages (as `teiler`), where the lock's `layout` puts it, and
    /// keeps what it was handed. It does not keep a block put at x 9600, and
    /// `--relayout` puts it at the page's corner (in the lock too).
    #[cfg(unix)]
    fn lox_that_places(tmp: &Path) {
        std::fs::write(
            tmp.join("lox"),
            r#"#!/bin/sh
echo "$*" >> ../calls.log
[ "$1 $2" = "logic compile" ] || { echo "unexpected: $*" >&2; exit 2; }
out=out.Loxone; lock=logic.lock.json; prev=
for a in "$@"; do [ "$prev" = --out ] && out=$a; [ "$prev" = --lock ] && lock=$a; prev=$a; done
cp "$lock" ../lock-seen.json
px=$(sed -n '/"layout"/,/}/s/.*"px": *\(-\{0,1\}[0-9]*\).*/\1/p' "$lock" | head -1)
py=$(sed -n '/"layout"/,/}/s/.*"py": *\(-\{0,1\}[0-9]*\).*/\1/p' "$lock" | head -1)
# Laid out afresh: placed at the corner, and kept there in the lock, as lox does.
case "$*" in *--relayout*) px=96; py=96; sed -i.bak 's/"px": *[0-9-]*/"px":96/; s/"py": *[0-9-]*/"py":96/' "$lock"; rm -f "$lock.bak" ;; esac
[ "$px" = 9600 ] && px=4032
sed '/U="10000006-0000-0060-ffff504f94000001"/s/Px="[0-9-]*" Py="[0-9-]*"/Px="'"$px"'" Py="'"$py"'"/' base.Loxone > "$out"
"#,
        )
        .unwrap();
    }

    /// A stand-in lox for wires: `logic compile` writes the base, marked
    /// with a checksum of the wires vislox's file draws, into out (or `--out`).
    #[cfg(unix)]
    fn lox_that_wires(tmp: &Path) {
        std::fs::write(
            tmp.join("lox"),
            r#"#!/bin/sh
echo "$*" >> ../calls.log
[ "$1 $2" = "logic compile" ] || { echo "unexpected: $*" >&2; exit 2; }
out=out.Loxone; prev=
for a in "$@"; do [ "$prev" = --out ] && out=$a; prev=$a; done
w=$(grep ' <- ' pages/vislox/edits.loxl 2>/dev/null)
[ -z "$w" ] && { cp base.Loxone "$out"; exit 0; }
w=$(echo "$w" | cksum | cut -d' ' -f1)
sed "s/Title=\"Testhaus\"/Title=\"Testhaus $w\"/" base.Loxone > "$out"
"#,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_wire_is_drawn_and_taken_out_in_the_module_never_deployed() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        lox_that_wires(tmp.path());
        let d = tmp.path().join("project");
        const OR: &str = "10000009-0000-0090-ffff504f94000001";
        const AND: &str = "10000004-0000-0040-ffff504f94000001";
        let end = |u: &str, k: &str| [u.to_string(), k.to_string()];
        let req = |ops: Vec<edit::Op>| edit::Request {
            base: blob(&d, "out.Loxone"),
            ops,
        };
        let w = edit::Op::Wire {
            from: end(DIV, "AQ"),
            to: end(OR, "I2"),
        };
        let r = a.edit(&req(vec![w.clone()]), None).await.unwrap();
        assert_eq!(r["previous"][0]["op"], "unwire");
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(
            file.contains(&format!(
                "{}.In2 <- {}.AQ\n",
                edit::slug(OR),
                edit::slug(DIV)
            )),
            "{file}"
        );
        let e = edits_of(&a, &d).await;
        assert_eq!(
            e["wires"],
            json!([{ "from": [DIV, "AQ"], "to": [OR, "I2"], "now": true, "deployed": false }])
        );
        // The rule the page shares with `unwire`: nothing here is the module's.
        assert_eq!(e["module_wires"], json!([]));
        assert!(
            e["wire_why"]["loxone_config"]
                .as_str()
                .unwrap()
                .contains("remove it in Loxone Config")
        );
        // A wire Loxone Config drew is not lox's to take out; nothing changes.
        let gui = edit::Op::Unwire {
            from: end(AND, "Q"),
            to: end(OR, "I1"),
        };
        let before = std::fs::read(d.join("out.Loxone")).unwrap();
        let (code, msg) = a.edit(&req(vec![gui]), None).await.unwrap_err();
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{msg}");
        assert!(msg.contains("Drawn in Loxone Config"), "{msg}");
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), before);
        // Reconnected: one request takes it out and draws the new one.
        let moved = edit::Op::Wire {
            from: end(DIV, "AQ"),
            to: end(AND, "I2"),
        };
        let un = edit::Op::Unwire {
            from: end(DIV, "AQ"),
            to: end(OR, "I2"),
        };
        a.edit(&req(vec![un, moved.clone()]), None).await.unwrap();
        let e = edits_of(&a, &d).await;
        assert_eq!(
            e["wires"],
            json!([{ "from": [DIV, "AQ"], "to": [AND, "I2"], "now": true, "deployed": false }])
        );
        // Deleted: the file draws nothing, and the compile has no wire.
        let un = edit::Op::Unwire {
            from: end(DIV, "AQ"),
            to: end(AND, "I2"),
        };
        let r = a.edit(&req(vec![un]), None).await.unwrap();
        assert_eq!(r["previous"][0], serde_json::to_value(&moved).unwrap());
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(!file.contains("<-") && !file.contains("extern"), "{file}");
        assert_eq!(edits_of(&a, &d).await["wires"], json!([]));
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// A lox.toml project in git, and a `lox` whose `logic check` finds an
    /// error on each line of `pages/main.loxl` that says BROKEN, whose
    /// `logic fmt --check` finds a file with two spaces in a row not
    /// formatted (main.loxl's on standard output, without a last newline;
    /// other.loxl's on standard error), whose `logic lint .` finds an unused
    /// extern on main.loxl's `extern` line and dead outputs on `teiler` when
    /// main.loxl says `# dead` (and fails when it says LINTFAIL; takes as
    /// many seconds as `../slowlint` says, then logs LINTEND; and is a lox
    /// without lint while `../nolint` exists), whose `logic test .` (#67)
    /// passes a test `Teiler` when main.loxl has one, unless main.loxl says
    /// FAILTEST (its `expect teiler.AQ == 1` reads 0) or TESTERROR (a block
    /// without a model), finds none otherwise (taking as many seconds as
    /// `../slowtest` says, then logging TESTEND; a lox without test while
    /// `../notest` exists), whose
    /// `logic compile` writes the base
    /// and the module's files into out (taking as many seconds as `../slow`
    /// says, and logging CONCURRENT when another compile runs), and whose
    /// `config push` fails. It logs its calls, and fails on anything else.
    #[cfg(unix)]
    fn project_to_check() -> (tempfile::TempDir, PathBuf, Actions) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("project");
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::write(d.join("base.Loxone"), "base\n").unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\n",
        )
        .unwrap();
        std::fs::write(d.join("pages/main.loxl"), "teiler = Div(\"Teiler\")\n").unwrap();
        std::fs::write(d.join("pages/other.loxl"), "# other\n").unwrap();
        std::fs::write(
            d.join("logic.lock.json"),
            format!(r#"{{"objects":{{"teiler":{{"uuid":"{DIV}","type":"Div"}}}}}}"#),
        )
        .unwrap();
        sh(
            &d,
            "git init -q && git config user.email t@t && git config user.name t \
             && git add -A && git commit -qm init",
        );
        let lox = tmp.path().join("lox");
        std::fs::write(
            &lox,
            r#"#!/bin/sh
echo "$*" >> ../calls.log
case "$1 $2 $3" in
  "logic check --json")
    n=$(grep -n BROKEN pages/main.loxl | cut -d: -f1 | head -1)
    if [ -n "$n" ]; then
      echo "{\"errors\":[{\"line\":$n,\"message\":\"compile error: unknown block type BROKEN\",\"path\":\"pages/main.loxl\"}],\"ok\":false,\"path\":\"pages/main.loxl\"}"
      exit 1
    fi
    echo '{"counts":{"blocks":1},"ok":true,"path":"pages"}' ;;
  "logic fmt --check")
    bad=
    if grep -q '  ' pages/main.loxl; then printf 'pages/main.loxl: not canonical (run `lox logic fmt --write pages`)'; bad=1; fi
    if grep -q '  ' pages/other.loxl 2>/dev/null; then echo 'pages/other.loxl: not canonical (run `lox logic fmt --write pages`)' >&2; bad=1; fi
    [ -z "$bad" ] ;;
  "logic compile"*)
    grep -q BROKEN pages/main.loxl && { echo 'compile error: unknown block type BROKEN' >&2; exit 1; }
    mkdir ../compiling 2>/dev/null || echo CONCURRENT >> ../calls.log
    sleep $(cat ../slow 2>/dev/null || echo 0)
    out=out.Loxone; prev=
    for a in "$@"; do [ "$prev" = --out ] && out=$a; prev=$a; done
    cat base.Loxone pages/*.loxl > "$out"
    rmdir ../compiling 2>/dev/null ;;
  "logic lint .")
    [ -e ../nolint ] && { echo 'unknown command `lint`. Run `lox logic help`.' >&2; exit 2; }
    sleep $(cat ../slowlint 2>/dev/null || echo 0)
    echo LINTEND >> ../calls.log
    grep -q LINTFAIL pages/main.loxl && { echo 'error: compile for dead-output analysis failed: boom' >&2; exit 1; }
    n=0
    ext=$(sed -n 's/^extern \([a-z_0-9]*\) = .*/\1/p' pages/main.loxl)
    if [ -n "$ext" ]; then echo 'unused-extern (1)'; echo "  pages/main.loxl: \`$ext\` — extern Div is never referenced"; n=1; fi
    if grep -q '# dead' pages/main.loxl; then echo 'dead-outputs (1)'; echo '  pages/main.loxl: `teiler` — no wire draws from any of its ports'; n=$((n+1)); fi
    [ $n = 0 ] && { echo 'lint: clean'; exit 0; }
    echo "lint: $n findings"; exit 1 ;;
  "logic test .")
    [ -e ../notest ] && { echo 'unknown command `test`. Run `lox logic help`.' >&2; exit 2; }
    sleep $(cat ../slowtest 2>/dev/null || echo 0)
    echo TESTEND >> ../calls.log
    grep -q '^test ' pages/main.loxl || { echo 'no tests in pages'; exit 0; }
    if grep -q FAILTEST pages/main.loxl; then
      echo 'test "Teiler" ... FAILED'; echo '    step 1: expect teiler.AQ == 1 — actual 0'
      echo '1 tests: 0 passed, 1 failed'; exit 1
    fi
    if grep -q TESTERROR pages/main.loxl; then
      echo 'test "Teiler" ... ERROR'
      echo '    step 1: `teiler.AQ` cannot be simulated: Div "Teiler" (10000006-0000-0060-ffff504f94000001) is not simulated: no model for `Div` blocks. Set a port between it and the expectation in the test.'
      echo '1 tests: 0 passed, 1 failed'; exit 1
    fi
    echo 'test "Teiler" ... ok'; echo '1 tests: 1 passed, 0 failed' ;;
  "config push"*) echo 'The Miniserver refused the upload.' >&2; exit 1 ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d.clone()),
            lox,
            probe: None,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
        });
        (tmp, d, a)
    }

    /// What main.rs does on a file change, then the check worker's work:
    /// `GET /api/check` after it.
    async fn check_of(a: &Actions, d: &Path) -> Value {
        a.set_disk(
            file_blob(&d.join("base.Loxone")),
            file_blob(&d.join("out.Loxone")),
        );
        a.refresh_edit(None, None);
        while a.check_once().await {}
        a.check_result()
    }

    fn calls_in(tmp: &tempfile::TempDir) -> String {
        std::fs::read_to_string(tmp.path().join("calls.log")).unwrap_or_default()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_module_that_does_not_compile_is_reported_and_stops_apply() {
        use axum::http::StatusCode;
        let (tmp, d, a) = project_to_check();
        // Compiled as it is: nothing to say.
        a.compile().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("out.Loxone")).unwrap(),
            "base\nteiler = Div(\"Teiler\")\n# other\n"
        );
        let c = check_of(&a, &d).await;
        assert_eq!(c["available"], true, "{c}");
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(c["current"], true, "{c}");
        assert_eq!(c["errors"], json!([]));
        assert_eq!(c["stale"], Value::Null, "{c}");
        assert_eq!(c["unformatted"], json!([]));
        assert_eq!(
            c["lint"],
            json!({ "available": true, "findings": [] }),
            "{c}"
        );

        // An error on line 2, which names the managed block `teiler`.
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\")\nteiler.I1 <- BROKEN.Q\n",
        )
        .unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["ok"], false, "{c}");
        assert_eq!(
            c["errors"],
            json!([{ "file": "pages/main.loxl", "line": 2,
                     "message": "unknown block type BROKEN", "blocks": [DIV] }]),
        );
        // Not compiled into a scratch copy: it cannot compile. Nor linted.
        assert_eq!(c["stale"], Value::Null);
        assert_eq!(c["lint"], Value::Null);
        let compiles = || {
            calls_in(&tmp)
                .lines()
                .filter(|l| l.starts_with("logic compile"))
                .count()
        };
        assert_eq!(compiles(), 1, "only the Compile");
        // Kept: neither a request nor the worker runs lox again for it.
        let calls = calls_in(&tmp);
        assert_eq!(a.check_result(), c);
        assert!(!a.check_once().await);
        assert_eq!(calls_in(&tmp), calls);
        // Apply is refused at its first step, naming the error.
        let (code, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(
            msg.contains("The module doesn't compile (1 error)"),
            "{msg}"
        );
        assert!(
            msg.contains("pages/main.loxl:2: unknown block type BROKEN"),
            "{msg}"
        );
        let progress = a.info()["progress"].clone();
        assert_eq!(progress["steps"][0]["id"], "module", "{progress}");
        assert_eq!(progress["steps"][0]["state"], "fail", "{progress}");
        assert_eq!(progress["steps"][1]["state"], "wait", "{progress}");
        assert!(!calls_in(&tmp).contains("config push"), "nothing deployed");
        // And so is Compile, with lox's message.
        let (_, msg) = a.compile().await.unwrap_err();
        assert!(msg.contains("unknown block type BROKEN"), "{msg}");

        // Fixed, but not compiled: the pending config is from before it.
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\")\n# changed\n",
        )
        .unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(c["stale"], json!({ "files": ["pages/main.loxl"] }), "{c}");
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["stale"], Value::Null, "{c}");

        // Not formatted: a hint, and nothing else. lox names one file on
        // standard output without a last newline, the other on standard
        // error: both are read.
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\")\n#  a\n",
        )
        .unwrap();
        std::fs::write(d.join("pages/other.loxl"), "#  b\n").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(
            c["unformatted"],
            json!(["pages/main.loxl", "pages/other.loxl"]),
            "{c}"
        );
        assert!(!calls_in(&tmp).contains("config push"), "nothing deployed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lint_findings_are_pinned_to_their_blocks_and_never_stop_apply() {
        const AND: &str = "10000004-0000-0040-ffff504f94000001";
        let (tmp, d, a) = project_to_check();
        std::fs::write(
            d.join("pages/main.loxl"),
            format!("teiler = Div(\"Teiler\") # dead\nextern vx_e = Div(uuid: \"{AND}\")\n"),
        )
        .unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(
            c["lint"],
            json!({ "available": true, "findings": [
                { "kind": "unused-extern", "name": "vx_e", "message": "extern Div is never referenced",
                  "file": "pages/main.loxl", "line": 2, "blocks": [AND] },
                { "kind": "dead-outputs", "name": "teiler", "message": "no wire draws from any of its ports",
                  "file": "pages/main.loxl", "line": 1, "blocks": [DIV] },
            ] }),
            "{c}"
        );
        // A hint only: Apply passes its first step and goes on (to a push
        // the stand-in refuses).
        let (_, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert!(!msg.contains("doesn't compile"), "{msg}");
        let progress = a.info()["progress"].clone();
        assert_eq!(progress["steps"][0]["id"], "module", "{progress}");
        assert_eq!(progress["steps"][0]["state"], "ok", "{progress}");
        // lint fails: said, with lox's message, and nothing else changes.
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\") # LINTFAIL\n",
        )
        .unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(
            c["lint"],
            json!({ "available": false, "reason": "error: compile for dead-output analysis failed: boom" }),
            "{c}"
        );
        let lints = calls_in(&tmp)
            .lines()
            .filter(|l| *l == "logic lint .")
            .count();
        assert!(lints >= 2, "{}", calls_in(&tmp));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn errors_and_hints_are_shown_without_waiting_for_lint() {
        let (tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        check_of(&a, &d).await;
        std::fs::write(tmp.path().join("slowlint"), "3").unwrap();
        // Errors: lint does not run at all.
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\")\nBROKEN\n",
        )
        .unwrap();
        let t = std::time::Instant::now();
        let c = check_of(&a, &d).await;
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(c["ok"], false, "{c}");
        assert_eq!(c["lint"], Value::Null, "{c}");
        // No errors: the check is kept (and told) before lint ends, with
        // lint `running`; lint's findings follow.
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\") # dead\n",
        )
        .unwrap();
        a.set_disk(
            file_blob(&d.join("base.Loxone")),
            file_blob(&d.join("out.Loxone")),
        );
        a.refresh_edit(None, None);
        let (_, early) = tokio::join!(async { while a.check_once().await {} }, async {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            a.check_result()
        });
        assert_eq!(early["current"], true, "{early}");
        assert_eq!(early["ok"], true, "{early}");
        assert_eq!(
            early["stale"]["files"],
            json!(["pages/main.loxl"]),
            "{early}"
        );
        assert_eq!(early["lint"]["running"], true, "{early}");
        let c = a.check_result();
        assert_eq!(c["lint"]["running"], Value::Null, "{c}");
        assert_eq!(c["lint"]["findings"][0]["name"], "teiler", "{c}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_action_never_waits_for_lint_and_stops_it() {
        let (tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        check_of(&a, &d).await;
        std::fs::write(tmp.path().join("slowlint"), "5").unwrap();
        std::fs::write(
            d.join("pages/main.loxl"),
            "teiler = Div(\"Teiler\") # dead\n",
        )
        .unwrap();
        let ends = || calls_in(&tmp).lines().filter(|l| *l == "LINTEND").count();
        let before = ends();
        a.set_disk(
            file_blob(&d.join("base.Loxone")),
            file_blob(&d.join("out.Loxone")),
        );
        a.refresh_edit(None, None);
        let (_, took) = tokio::join!(async { while a.check_once().await {} }, async {
            // Lint runs now: Compile takes the compile lock at once.
            tokio::time::sleep(Duration::from_millis(800)).await;
            assert_eq!(a.check_result()["lint"]["running"], true);
            let t = std::time::Instant::now();
            a.compile().await.unwrap();
            t.elapsed()
        });
        assert!(took < Duration::from_secs(2), "{took:?}");
        // The lint it stopped never ended, and kept nothing; the next one,
        // after Compile, does.
        assert_eq!(ends(), before, "{}", calls_in(&tmp));
        std::fs::write(tmp.path().join("slowlint"), "0").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["lint"]["findings"][0]["name"], "teiler", "{c}");
        assert_eq!(ends(), before + 1, "{}", calls_in(&tmp));
    }

    /// main.loxl with the test `Teiler`, which `teiler` and `extra` say.
    fn with_test(extra: &str) -> String {
        format!(
            "teiler = Div(\"Teiler\")\n\ntest \"Teiler\" # {extra}\n\tteiler.I1 = 2\n\ttick 1s\n\texpect teiler.AQ == 1\nend\n"
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_test_is_pinned_to_its_lines_and_blocks_and_stops_apply() {
        use axum::http::StatusCode;
        let (tmp, d, a) = project_to_check();
        // No tests: said, and Apply's step is skipped.
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(
            c["test"],
            json!({ "available": true, "tests": [], "passed": 0, "failed": 0 }),
            "{c}"
        );
        std::fs::write(d.join("pages/main.loxl"), with_test("FAILTEST")).unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(
            c["test"],
            json!({ "available": true, "passed": 0, "failed": 1, "tests": [{
                "name": "Teiler", "status": "fail", "error": null, "unmodeled": [],
                "file": "pages/main.loxl", "line": 3, "blocks": [DIV],
                "failures": [{ "step": 1, "expect": "expect teiler.AQ == 1", "signal": "teiler.AQ",
                    "cmp": "==", "expected": "1", "actual": "0", "line": 6, "blocks": [DIV] }],
            }] }),
            "{c}"
        );
        // Apply: the module compiles, its test fails: refused there, before
        // anything is sent, and the step says so.
        let runs = || {
            calls_in(&tmp)
                .lines()
                .filter(|l| *l == "logic test .")
                .count()
        };
        let before = runs();
        let (code, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(msg.starts_with("1 of 1 test fails"), "{msg}");
        assert!(
            msg.contains("test \"Teiler\" failed (pages/main.loxl:3)"),
            "{msg}"
        );
        assert!(msg.contains("expect teiler.AQ == 1 — actual 0"), "{msg}");
        // The check's run was of these files: not run again.
        assert_eq!(runs(), before, "{}", calls_in(&tmp));
        let steps = a.info()["progress"]["steps"].clone();
        assert_eq!(steps[0]["state"], "ok", "{steps}");
        assert_eq!(steps[1]["id"], "tests", "{steps}");
        assert_eq!(steps[1]["state"], "fail", "{steps}");
        assert!(!calls_in(&tmp).contains("config push"), "nothing deployed");
        assert!(!calls_in(&tmp).contains("logic diff"), "{}", calls_in(&tmp));

        // A test that cannot run (a block without a model) stops it too, and
        // names that block.
        std::fs::write(d.join("pages/main.loxl"), with_test("TESTERROR")).unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        let t = &c["test"]["tests"][0];
        assert_eq!(t["status"], "error", "{c}");
        assert_eq!(
            t["unmodeled"],
            json!([{ "uuid": DIV, "type": "Div", "title": "Teiler" }])
        );
        let (_, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert!(msg.contains("test \"Teiler\" could not run"), "{msg}");
        assert!(msg.contains("no model for `Div` blocks"), "{msg}");

        // It passes: Apply goes on (to a push the stand-in refuses).
        std::fs::write(d.join("pages/main.loxl"), with_test("ok")).unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["test"]["passed"], 1, "{c}");
        let (code, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert_eq!(code, StatusCode::BAD_GATEWAY, "{msg}");
        let steps = a.info()["progress"]["steps"].clone();
        assert_eq!(steps[1]["state"], "ok", "{steps}");
        assert_eq!(steps[1]["detail"], "1 test passed", "{steps}");
        assert!(calls_in(&tmp).contains("config push"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn apply_runs_the_tests_again_when_the_check_is_not_of_the_files() {
        let (tmp, d, a) = project_to_check();
        std::fs::write(d.join("pages/main.loxl"), with_test("ok")).unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["test"]["passed"], 1, "{c}");
        // Changed, and neither the watcher nor the worker has looked: the
        // pass is of other files, so Apply runs the tests itself.
        std::fs::write(d.join("pages/main.loxl"), with_test("FAILTEST")).unwrap();
        let runs = || {
            calls_in(&tmp)
                .lines()
                .filter(|l| *l == "logic test .")
                .count()
        };
        let before = runs();
        let (_, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert!(msg.starts_with("1 of 1 test fails"), "{msg}");
        assert_eq!(runs(), before + 1, "{}", calls_in(&tmp));
        assert!(!calls_in(&tmp).contains("config push"));
        // And while the check's tests of these files still run, Apply does
        // not take the last ones (a pass, of the files before).
        std::fs::write(tmp.path().join("slowtest"), "3").unwrap();
        a.set_disk(
            file_blob(&d.join("base.Loxone")),
            file_blob(&d.join("out.Loxone")),
        );
        a.refresh_edit(None, None);
        let (_, res) = tokio::join!(async { while a.check_once().await {} }, async {
            let t = std::time::Instant::now();
            while a.check_result()["test"]["running"] != true {
                assert!(t.elapsed() < Duration::from_secs(5), "{}", a.check_result());
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // The last run's pass stays up meanwhile.
            assert_eq!(a.check_result()["test"]["tests"][0]["status"], "pass");
            std::fs::write(tmp.path().join("slowtest"), "0").unwrap();
            a.apply(&ApplyOpts::default()).await
        });
        let (_, msg) = res.unwrap_err();
        assert!(msg.starts_with("1 of 1 test fails"), "{msg}");
        assert!(!calls_in(&tmp).contains("config push"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_edit_never_waits_for_the_tests_and_stops_them() {
        let (tmp, d, a) = project_to_check();
        std::fs::write(d.join("pages/main.loxl"), with_test("ok")).unwrap();
        a.compile().await.unwrap();
        std::fs::write(tmp.path().join("slowtest"), "5").unwrap();
        let ends = || calls_in(&tmp).lines().filter(|l| *l == "TESTEND").count();
        a.set_disk(
            file_blob(&d.join("base.Loxone")),
            file_blob(&d.join("out.Loxone")),
        );
        a.refresh_edit(None, None);
        let (_, took) = tokio::join!(async { while a.check_once().await {} }, async {
            tokio::time::sleep(Duration::from_millis(800)).await;
            assert_eq!(a.check_result()["test"]["running"], true);
            let t = std::time::Instant::now();
            a.compile().await.unwrap();
            t.elapsed()
        });
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert_eq!(ends(), 0, "{}", calls_in(&tmp));
        std::fs::write(tmp.path().join("slowtest"), "0").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["test"]["passed"], 1, "{c}");
        assert_eq!(ends(), 1, "{}", calls_in(&tmp));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_lox_without_test_is_asked_once_and_does_not_stop_apply() {
        let (tmp, d, a) = project_to_check();
        std::fs::write(tmp.path().join("notest"), "").unwrap();
        std::fs::write(d.join("pages/main.loxl"), with_test("FAILTEST")).unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(
            c["test"],
            json!({ "available": false, "unsupported": true, "reason": "this lox has no `lox logic test`" }),
            "{c}"
        );
        let (_, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert!(msg.contains("lox config push failed"), "{msg}");
        let steps = a.info()["progress"]["steps"].clone();
        assert_eq!(steps[1]["state"], "skip", "{steps}");
        let runs = calls_in(&tmp)
            .lines()
            .filter(|l| *l == "logic test .")
            .count();
        assert_eq!(runs, 1, "{}", calls_in(&tmp));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_lox_without_lint_is_asked_once() {
        let (tmp, d, a) = project_to_check();
        std::fs::write(tmp.path().join("nolint"), "").unwrap();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(
            c["lint"],
            json!({ "available": false, "unsupported": true, "reason": "this lox has no `lox logic lint`" }),
            "{c}"
        );
        std::fs::write(d.join("pages/other.loxl"), "# changed\n").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["lint"]["unsupported"], true, "{c}");
        assert_eq!(c["stale"]["files"], json!(["pages/other.loxl"]), "{c}");
        let lints = calls_in(&tmp)
            .lines()
            .filter(|l| *l == "logic lint .")
            .count();
        assert_eq!(lints, 1, "{}", calls_in(&tmp));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_key_that_lags_the_files_is_brought_up_to_date() {
        let (_tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        check_of(&a, &d).await;
        // Changed, and the watcher has not looked: the worker, woken for
        // another reason, looks itself and checks the files as they are.
        std::fs::write(d.join("pages/other.loxl"), "# changed\n").unwrap();
        a.set_disk(file_blob(&d.join("base.Loxone")), None);
        while a.check_once().await {}
        let c = a.check_result();
        assert_eq!(c["current"], true, "{c}");
        assert_eq!(c["stale"]["files"], json!(["pages/other.loxl"]), "{c}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_check_is_kept_only_under_the_key_of_the_files_lox_read() {
        let (_tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["lint"]["findings"], json!([]), "{c}");
        let main = std::fs::read_to_string(d.join("pages/main.loxl")).unwrap();
        // A file changes, and the worker runs (for another reason: out on
        // disk) before the watcher looks at the files.
        std::fs::write(d.join("pages/main.loxl"), format!("{main}# dead\n")).unwrap();
        a.set_disk(file_blob(&d.join("base.Loxone")), None);
        while a.check_once().await {}
        // It changes back, and only then does the watcher look: the key is
        // the one from before the change, and what lox said of the change
        // must not be kept under it.
        std::fs::write(d.join("pages/main.loxl"), &main).unwrap();
        a.refresh_edit(None, None);
        while a.check_once().await {}
        let c = a.check_result();
        assert_eq!(c["current"], true, "{c}");
        assert_eq!(c["lint"]["findings"], json!([]), "{c}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_check_comes_back_after_an_action_that_changed_nothing() {
        let (_tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        std::fs::write(d.join("pages/other.loxl"), "# changed\n").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["stale"], json!({ "files": ["pages/other.loxl"] }), "{c}");
        // An Apply that fails and changes no file: the check stands.
        assert!(a.apply(&ApplyOpts::default()).await.is_err());
        let c = check_of(&a, &d).await;
        assert_eq!(c["stale"], json!({ "files": ["pages/other.loxl"] }), "{c}");
        assert_eq!(c["current"], true, "{c}");
        // While an action runs, nothing is checked, and nothing kept: the old
        // check shows, not current; once it ends, the check is worked out.
        let one = a.busy.try_lock().unwrap();
        std::fs::write(d.join("pages/other.loxl"), "# changed again\n").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["current"], false, "{c}");
        assert_eq!(c["stale"], json!({ "files": ["pages/other.loxl"] }), "{c}");
        drop(one);
        let c = check_of(&a, &d).await;
        assert_eq!(c["current"], true, "{c}");
        assert_eq!(c["stale"], json!({ "files": ["pages/other.loxl"] }), "{c}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_pending_config_is_checked_by_content_not_by_times() {
        let (_tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        assert_eq!(check_of(&a, &d).await["stale"], Value::Null);
        // A module file deleted after the compile.
        std::fs::remove_file(d.join("pages/other.loxl")).unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["stale"], json!({ "files": ["pages/other.loxl"] }), "{c}");
        a.compile().await.unwrap();
        assert_eq!(check_of(&a, &d).await["stale"], Value::Null);
        // A change written with an old modification time (cp -p, rsync -t).
        std::fs::write(d.join("pages/other.loxl"), "# copied\n").unwrap();
        sh(&d, "touch -t 200001010000 pages/other.loxl");
        let c = check_of(&a, &d).await;
        assert_eq!(c["stale"], json!({ "files": ["pages/other.loxl"] }), "{c}");
        a.compile().await.unwrap();
        assert_eq!(check_of(&a, &d).await["stale"], Value::Null);
        // A new base: out was compiled against the old one.
        std::fs::write(d.join("base.Loxone"), "base 2\n").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(c["stale"], json!({ "files": ["base.Loxone"] }), "{c}");
        a.compile().await.unwrap();
        assert_eq!(check_of(&a, &d).await["stale"], Value::Null);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_checks_compile_never_runs_beside_another() {
        let (tmp, d, a) = project_to_check();
        a.compile().await.unwrap();
        std::fs::write(tmp.path().join("slow"), "1").unwrap();
        // A request never compiles: it reads what the worker kept.
        std::fs::write(d.join("pages/other.loxl"), "# changed\n").unwrap();
        a.set_disk(
            file_blob(&d.join("base.Loxone")),
            file_blob(&d.join("out.Loxone")),
        );
        a.refresh_edit(None, None);
        let calls = calls_in(&tmp);
        let _ = a.check_result();
        assert_eq!(calls_in(&tmp), calls);
        // The worker's scratch compile, and a Compile that starts while it
        // runs: the Compile waits for it.
        let (_, compiled) = tokio::join!(a.check_once(), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            a.compile().await
        });
        compiled.unwrap();
        // A Compile, and the worker woken while it runs: the worker leaves it.
        std::fs::write(d.join("pages/other.loxl"), "# changed again\n").unwrap();
        let (_, checked) = tokio::join!(a.compile(), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            check_of(&a, &d).await
        });
        assert_eq!(checked["current"], false, "{checked}");
        let calls = calls_in(&tmp);
        assert_eq!(
            calls
                .lines()
                .filter(|l| l.starts_with("logic compile"))
                .count(),
            4,
            "{calls}"
        );
        assert!(!calls.contains("CONCURRENT"), "{calls}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_lox_that_cannot_check_refuses_nothing() {
        let (_tmp, a) = project_for_edits();
        let d = _tmp.path().join("project");
        let c = check_of(&a, &d).await;
        assert_eq!(c["available"], false, "{c}");
        assert!(a.module_errors().await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_managed_blocks_point_at_their_source_lines() {
        const AND: &str = "10000004-0000-0040-ffff504f94000001";
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        std::fs::write(
            d.join("pages/main.loxl"),
            "# the module\nteiler = Div(\"Teiler\")\n",
        )
        .unwrap();
        std::fs::write(
            d.join("logic.lock.json"),
            format!(
                r#"{{"objects":{{"teiler":{{"uuid":"{DIV}","type":"Div"}}}},"externals":{{"temp":{{"uuid":"{AND}"}}}}}}"#
            ),
        )
        .unwrap();
        let e = edits_of(&a, &d).await;
        // The extern is not managed: Loxone Config owns it.
        assert_eq!(
            e["sources"],
            json!({ DIV: { "slug": "teiler", "file": "pages/main.loxl", "line": 2, "how": "declared" } }),
            "{e}"
        );
        // A line moved in the module, not compiled yet: the key changes, and
        // the line follows.
        let key = a.edit_state.lock().unwrap().key();
        std::fs::write(
            d.join("pages/main.loxl"),
            "# the module\n\n# Teiler\nteiler = Div(\"Teiler\")\n",
        )
        .unwrap();
        let e = edits_of(&a, &d).await;
        assert_ne!(a.edit_state.lock().unwrap().key(), key);
        assert_eq!(e["sources"][DIV]["line"], 4, "{e}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_managed_block_is_moved_in_the_lock_never_deployed() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        lox_that_places(tmp.path());
        let d = tmp.path().join("project");
        const EG: &str = "10000002-0000-0020-ffff504f94000001";
        std::fs::write(d.join("pages/main.loxl"), "# the module\n").unwrap();
        let lock = format!(
            r#"{{"objects":{{"teiler":{{"uuid":"{DIV}","type":"Div","page_uuid":"{EG}","layout":{{"px":4032,"py":2880,"px2":5376,"py2":3576}}}}}}}}"#
        );
        std::fs::write(d.join("logic.lock.json"), &lock).unwrap();
        // Compiled after the module's files: nothing uncompiled.
        let base = std::fs::read(d.join("base.Loxone")).unwrap();
        std::fs::write(d.join("out.Loxone"), base).unwrap();
        sh(&d, "git add -A && git commit -qm managed");
        let req = |ops: Vec<edit::Op>| edit::Request {
            base: blob(&d, "out.Loxone"),
            ops,
        };
        let mv = |x, y| edit::Op::Move {
            uuid: DIV.into(),
            x,
            y,
        };
        let e = edits_of(&a, &d).await;
        assert_eq!(
            (&e["movable"], &e["moves"]),
            (&json!([DIV]), &json!([])),
            "{e}"
        );
        // Its page can be laid out afresh.
        assert_eq!(e["arrange"], json!({ EG: null }), "{e}");
        // The lockfile alone changing (lox, a Discard) changes the key.
        let key = a.edit_state.lock().unwrap().key();
        std::fs::write(
            d.join("logic.lock.json"),
            lock.replace("\"Div\"", "\"Div\" "),
        )
        .unwrap();
        a.refresh_edit(Some(&blob(&d, "base.Loxone")), None);
        assert_ne!(a.edit_state.lock().unwrap().key(), key);
        std::fs::write(d.join("logic.lock.json"), &lock).unwrap();
        a.refresh_edit(Some(&blob(&d, "base.Loxone")), None);
        assert_eq!(a.edit_state.lock().unwrap().key(), key);

        // Snapped to the grid; the position the base still draws is
        // superseded, so lox keeps the new one. vislox's files stay as they were.
        let r = a.edit(&req(vec![mv(1930, 1000)]), None).await.unwrap();
        assert_eq!(
            r["previous"],
            json!([{ "op": "move", "uuid": DIV, "x": 4032, "y": 2880 }])
        );
        let seen: Value =
            serde_json::from_slice(&std::fs::read(tmp.path().join("lock-seen.json")).unwrap())
                .unwrap();
        let t = &seen["objects"]["teiler"];
        assert_eq!(
            (&t["layout"], &t["superseded_layout"]),
            (
                &json!({ "px": 1920, "py": 960, "px2": 3264, "py2": 1656 }),
                &json!({ "px": 4032, "py": 2880, "px2": 5376, "py2": 3576 })
            )
        );
        assert!(!d.join("pages/vislox").exists());
        let out = std::fs::read_to_string(d.join("out.Loxone")).unwrap();
        assert!(out.contains(r#"Title="O1002" Px="1920" Py="960""#));
        let e = edits_of(&a, &d).await;
        assert_eq!(
            e["moves"],
            json!([{ "uuid": DIV, "x": 1920, "y": 960, "deployed": [4032, 2880] }])
        );

        // Back where it is deployed: nothing superseded, nothing moved.
        a.edit(&req(vec![mv(4032, 2880)]), None).await.unwrap();
        let seen: Value =
            serde_json::from_slice(&std::fs::read(tmp.path().join("lock-seen.json")).unwrap())
                .unwrap();
        assert_eq!(seen["objects"]["teiler"].get("superseded_layout"), None);
        assert_eq!(edits_of(&a, &d).await["moves"], json!([]));

        // A move lox does not keep is refused, and the lock and out are put back.
        let (lock_before, out_before) = (
            std::fs::read(d.join("logic.lock.json")).unwrap(),
            std::fs::read(d.join("out.Loxone")).unwrap(),
        );
        let (code, msg) = a.edit(&req(vec![mv(9600, 0)]), None).await.unwrap_err();
        assert_eq!(code, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("did not keep the new position of teiler"),
            "{msg}"
        );
        assert_eq!(
            std::fs::read(d.join("logic.lock.json")).unwrap(),
            lock_before
        );
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out_before);

        // Laid out afresh: lox is asked by the page's title; previous moves it back.
        let r = a
            .edit(&req(vec![edit::Op::Relayout { page: EG.into() }]), None)
            .await
            .unwrap();
        assert_eq!(
            r["previous"],
            json!([{ "op": "place", "moves": [{ "uuid": DIV, "x": 4032, "y": 2880 }] }])
        );
        // Only compiles: nothing was deployed.
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
        assert!(
            calls
                .lines()
                .last()
                .unwrap()
                .ends_with("--relayout Erdgeschoss"),
            "{calls}"
        );
    }

    /// Undo and redo (#62) as the browser does them: each reply's `undo`
    /// sent back as an edit of its own puts the module, the lock's layout
    /// and the pending config back exactly; the undo's own `undo` is the redo.
    #[cfg(unix)]
    #[tokio::test]
    async fn undo_and_redo_are_the_replys_undo_sent_back_as_edits() {
        const OR: &str = "10000009-0000-0090-ffff504f94000001";
        const AND: &str = "10000004-0000-0040-ffff504f94000001";
        const EG: &str = "10000002-0000-0020-ffff504f94000001";
        const OG: &str = "10000008-0000-0080-ffff504f94000001";
        let end = |u: &str, k: &str| [u.to_string(), k.to_string()];
        /// What an edit changes: vislox's files, the lock's layout, out.
        fn state(
            d: &Path,
        ) -> (
            std::collections::BTreeMap<String, String>,
            Option<Value>,
            String,
        ) {
            // What the files declare. Left out: comments (a file of them only
            // is the placeholder lox needs in a module with no other `.loxl`),
            // and a page file's `page` and `removed` lines, which say that a
            // block added here was taken out again (lox's lock keeps a
            // tombstone of it).
            let files: std::collections::BTreeMap<String, String> =
                edit::Files::texts(&d.join("pages").join(edit::DIR))
                    .into_iter()
                    .map(|(f, t)| {
                        let t: Vec<&str> = t
                            .lines()
                            .filter(|l| {
                                !l.trim().is_empty()
                                    && !l.starts_with('#')
                                    && !l.starts_with("page ")
                                    && !l.starts_with("removed ")
                            })
                            .collect();
                        (f, t.join("\n"))
                    })
                    .filter(|(_, t)| !t.is_empty())
                    .collect();
            let lock = std::fs::read(d.join("logic.lock.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .map(|l| l["objects"]["teiler"]["layout"].clone())
                .filter(|l| !l.is_null())
                // Where it is: a move keeps the size the page draws, which
                // the stand-in's lock does not have.
                .map(|l| json!([l["px"], l["py"]]));
            (files, lock, blob(d, "out.Loxone"))
        }
        async fn send(a: &Actions, d: &Path, ops: Value) -> Value {
            let ops: Vec<edit::Op> = serde_json::from_value(ops).unwrap();
            let req = edit::Request {
                base: blob(d, "out.Loxone"),
                ops,
            };
            let r = a.edit(&req, None).await.unwrap();
            assert!(r["undo"].is_array(), "{r}");
            r
        }
        /// Edit, undo, redo, undo again: each lands where it should. Returns
        /// the replies of the edit, the first undo and the redo.
        async fn round_trip(a: &Actions, d: &Path, ops: Vec<edit::Op>) -> [Value; 3] {
            let before = state(d);
            let r = send(a, d, serde_json::to_value(&ops).unwrap()).await;
            let after = state(d);
            assert_ne!(after, before, "{ops:?} changed nothing");
            let u = send(a, d, r["undo"].clone()).await;
            assert_eq!(state(d), before, "undo of {ops:?}");
            let re = send(a, d, u["undo"].clone()).await;
            assert_eq!(state(d), after, "redo of {ops:?}");
            send(a, d, re["undo"].clone()).await;
            assert_eq!(state(d), before, "undo again of {ops:?}");
            [r, u, re]
        }

        // Values: a claim, then one changed again and one handed back.
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        let t = |v: Option<&str>| edit::Op::Title {
            uuid: DIV.into(),
            value: v.map(Into::into),
        };
        let [r, ..] = round_trip(&a, &d, vec![t(Some("Teiler"))]).await;
        assert_eq!(
            r["undo"],
            json!([{ "op": "title", "uuid": DIV, "value": null }])
        );
        send(&a, &d, json!([t(Some("Teiler"))])).await;
        // Two ops of one request on one value: undone last first.
        let [r, ..] = round_trip(&a, &d, vec![t(Some("Eins")), t(None)]).await;
        assert_eq!(
            r["undo"],
            json!([
                { "op": "title", "uuid": DIV, "value": "Eins" },
                { "op": "title", "uuid": DIV, "value": "Teiler" }
            ])
        );

        // Wires: drawn, reconnected (one request), taken out.
        let (tmp, a) = project_for_edits();
        lox_that_wires(tmp.path());
        let d = tmp.path().join("project");
        let wire = |to: &str| edit::Op::Wire {
            from: end(DIV, "AQ"),
            to: end(to, "I2"),
        };
        let unwire = |to: &str| edit::Op::Unwire {
            from: end(DIV, "AQ"),
            to: end(to, "I2"),
        };
        round_trip(&a, &d, vec![wire(OR)]).await;
        send(&a, &d, json!([wire(OR)])).await;
        let [r, ..] = round_trip(&a, &d, vec![unwire(OR), wire(AND)]).await;
        assert_eq!(r["undo"], json!([unwire(AND), wire(OR)]));
        round_trip(&a, &d, vec![unwire(OR)]).await;

        // Blocks: added, then taken out. Adding it again mints a new UUID
        // in lox: the reply names the one it restores.
        let (tmp, a) = project_for_edits();
        lox_that_adds_blocks(tmp.path());
        let d = tmp.path().join("project");
        let add = edit::Op::AddBlock {
            page: OG.into(),
            ty: "And".into(),
            title: Some("Licht".into()),
            params: [("I2".to_string(), "1".to_string())].into(),
            settings: Default::default(),
            wires: vec![],
            restores: None,
        };
        let [r, u, re] = round_trip(&a, &d, vec![add.clone()]).await;
        let uuid = r["added"][0]["uuid"].as_str().unwrap().to_string();
        assert_eq!(r["undo"], json!([{ "op": "remove_block", "uuid": uuid }]));
        assert_eq!(r["added"][0].get("restores"), None);
        assert_eq!(u["undo"][0]["op"], "add_block");
        assert_eq!(u["undo"][0]["restores"], json!(uuid));
        assert_eq!(re["added"][0]["restores"], json!(uuid));
        let again = re["added"][0]["uuid"].as_str().unwrap();
        assert_eq!(re["undo"], json!([{ "op": "remove_block", "uuid": again }]));
        let r = send(&a, &d, json!([add])).await;
        let uuid = r["added"][0]["uuid"].as_str().unwrap().to_string();
        round_trip(&a, &d, vec![edit::Op::RemoveBlock { uuid }]).await;

        // Moves and Arrange, which go into the lock.
        let (tmp, a) = project_for_edits();
        lox_that_places(tmp.path());
        let d = tmp.path().join("project");
        std::fs::write(d.join("pages/main.loxl"), "# the module\n").unwrap();
        std::fs::write(
            d.join("logic.lock.json"),
            format!(
                r#"{{"objects":{{"teiler":{{"uuid":"{DIV}","type":"Div","page_uuid":"{EG}","layout":{{"px":4032,"py":2880,"px2":5376,"py2":3576}}}}}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            d.join("out.Loxone"),
            std::fs::read(d.join("base.Loxone")).unwrap(),
        )
        .unwrap();
        sh(&d, "git add -A && git commit -qm managed");
        let mv = edit::Op::Move {
            uuid: DIV.into(),
            x: 1920,
            y: 960,
        };
        round_trip(&a, &d, vec![mv.clone()]).await;
        send(&a, &d, json!([mv])).await;
        let [r, ..] = round_trip(&a, &d, vec![edit::Op::Relayout { page: EG.into() }]).await;
        assert_eq!(
            r["undo"],
            json!([{ "op": "place", "moves": [{ "uuid": DIV, "x": 1920, "y": 960 }] }])
        );
        // Only compiles: nothing was deployed.
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// The first compile writes a lock the last commit does not have:
    /// Discard deletes it, with the files vislox wrote.
    #[cfg(unix)]
    #[tokio::test]
    async fn discard_drops_added_blocks_and_a_lock_nobody_committed() {
        let (tmp, a) = project_for_edits();
        lox_that_adds_blocks(tmp.path());
        let d = tmp.path().join("project");
        let out = std::fs::read(d.join("out.Loxone")).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: vec![edit::Op::AddBlock {
                    page: "10000008-0000-0080-ffff504f94000001".into(),
                    ty: "Or".into(),
                    title: None,
                    params: Default::default(),
                    settings: Default::default(),
                    wires: vec![],
                    restores: None,
                }],
            },
            None,
        )
        .await
        .unwrap();
        assert!(d.join("logic.lock.json").exists());
        a.discard().await.unwrap();
        assert!(!d.join("logic.lock.json").exists());
        assert!(!d.join("pages/vislox").exists());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out);
    }

    /// A project in a directory of its repository: the deployed commit's
    /// files are read by their path from the repository's top.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_deployed_claims_are_read_in_a_project_below_the_repository_top() {
        let (tmp, a) = project_for_edits_in("repo/sub/project");
        let d = tmp.path().join("repo/sub/project");
        a.edit(&title(blob(&d, "out.Loxone"), Some("Teiler")), None)
            .await
            .unwrap();
        sh(
            &d,
            "cp out.Loxone base.Loxone && git add -A && git commit -qm applied",
        );
        let c = edits_of(&a, &d).await["claims"][0].clone();
        assert_eq!(
            (&c["value"], &c["deployed"], &c["pending"]),
            (&json!("Teiler"), &json!("Teiler"), &json!(false)),
            "{c}"
        );
    }

    /// Discard never deletes a lock vislox did not write: one git does not
    /// have (uncommitted or ignored) stays, or goes back to how it was before
    /// vislox's first edit.
    #[cfg(unix)]
    #[tokio::test]
    async fn discard_keeps_a_lock_it_did_not_write() {
        let (tmp, a) = project_for_edits();
        lox_that_adds_blocks(tmp.path());
        let d = tmp.path().join("project");
        sh(
            &d,
            "echo logic.lock.json > .gitignore && git add .gitignore && git commit -qm ignore",
        );
        std::fs::write(d.join("logic.lock.json"), "theirs").unwrap();
        // Nothing edited here: the lock is not vislox's.
        let msg = a.discard().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("logic.lock.json")).unwrap(),
            "theirs"
        );
        assert!(
            msg.contains("logic.lock.json") && msg.contains("left as it is"),
            "{msg}"
        );
        // Edited here: back to how it was before the first edit.
        let add = |base: String| edit::Request {
            base,
            ops: vec![edit::Op::AddBlock {
                page: "10000008-0000-0080-ffff504f94000001".into(),
                ty: "Or".into(),
                title: None,
                params: Default::default(),
                settings: Default::default(),
                wires: vec![],
                restores: None,
            }],
        };
        a.edit(&add(blob(&d, "out.Loxone")), None).await.unwrap();
        assert_ne!(
            std::fs::read_to_string(d.join("logic.lock.json")).unwrap(),
            "theirs"
        );
        a.edit(&add(blob(&d, "out.Loxone")), None).await.unwrap();
        a.discard().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("logic.lock.json")).unwrap(),
            "theirs"
        );
        assert!(!d.join("pages/vislox").exists());
    }

    /// The last commit decides, not the index: a staged lock is not in it.
    /// And Discard is all or nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn discard_reads_the_last_commit_and_puts_everything_back_when_it_fails() {
        let (tmp, a) = project_for_edits();
        lox_that_adds_blocks(tmp.path());
        let d = tmp.path().join("project");
        let req = |base: String| edit::Request {
            base,
            ops: vec![edit::Op::AddBlock {
                page: "10000008-0000-0080-ffff504f94000001".into(),
                ty: "Or".into(),
                title: None,
                params: Default::default(),
                settings: Default::default(),
                wires: vec![],
                restores: None,
            }],
        };
        a.edit(&req(blob(&d, "out.Loxone")), None).await.unwrap();
        sh(&d, "git add logic.lock.json");
        a.discard().await.unwrap();
        assert!(!d.join("logic.lock.json").exists());
        assert!(!d.join("pages/vislox").exists());

        // The lock was someone's before the edit; now it cannot be written.
        std::fs::write(d.join("logic.lock.json"), "theirs").unwrap();
        a.edit(&req(blob(&d, "out.Loxone")), None).await.unwrap();
        let file = std::fs::read_to_string(d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        let out = std::fs::read(d.join("out.Loxone")).unwrap();
        std::fs::remove_file(d.join("logic.lock.json")).unwrap();
        std::fs::create_dir(d.join("logic.lock.json")).unwrap();
        let (_, msg) = a.discard().await.unwrap_err();
        assert!(msg.contains("Nothing was discarded"), "{msg}");
        assert_eq!(
            std::fs::read_to_string(d.join("pages/vislox/obergeschoss.loxl")).unwrap(),
            file
        );
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out);
    }

    #[test]
    fn editing_needs_a_module_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let on = |module: &str| {
            std::fs::write(
                d.join("lox.toml"),
                format!("base = \"b.Loxone\"\nmodule = \"{module}\"\n"),
            )
            .unwrap();
            actions(Some("t"), Some(d)).info()["edit"].clone()
        };
        std::fs::write(d.join("m.loxl"), "").unwrap();
        let e = on("m.loxl");
        assert!(
            e["reason"]
                .as_str()
                .unwrap()
                .contains("make it a directory"),
            "{e}"
        );
        let e = on("gone.loxl");
        assert!(
            e["reason"].as_str().unwrap().contains("does not exist"),
            "{e}"
        );
        // A module directory not made yet: the first edit makes it.
        assert_eq!(on("pages")["available"], true);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn apply_shows_each_step_and_what_it_deployed() {
        let (tmp, lox) = project_with_fake_lox(true);
        let d = tmp.path().join("project");
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d.clone()),
            lox,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
            probe: Some(restart::tests::Script::probe(vec![
                (Ok(()), Some("v1".into())),
                (Err("503, restarting".into()), None),
                (Ok(()), Some("v2".into())),
            ])),
        });
        let msg = a.apply(&ApplyOpts::default()).await.unwrap();
        assert!(msg.contains("Committed: Add a hallway light"), "{msg}");
        let info = a.info();
        let steps = info["progress"]["steps"].as_array().unwrap();
        let st = |id: &str| {
            let s = steps.iter().find(|s| s["id"] == id).unwrap();
            (
                s["state"].as_str().unwrap().to_string(),
                s["detail"].as_str().unwrap_or("").to_string(),
            )
        };
        for id in [
            "check", "upload", "restart", "online", "loaded", "verify", "commit", "push",
        ] {
            assert_eq!(st(id).0, "ok", "{id}: {steps:?}");
        }
        assert!(st("restart").1.contains("503"), "{steps:?}");
        assert!(st("online").1.starts_with("after "), "{steps:?}");
        assert!(st("loaded").1.contains("v2"), "{steps:?}");
        let dep = &info["last"]["deployed"];
        assert_eq!(dep["before"], crate::history::blob_id(b"old"));
        assert_eq!(dep["after"], crate::history::blob_id(b"new"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_push_marks_the_step_it_failed_in() {
        let (tmp, lox) = project_with_fake_lox(false);
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(tmp.path().join("project")),
            lox,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
            probe: None,
        });
        assert!(a.apply(&ApplyOpts::default()).await.is_err());
        let info = a.info();
        let steps = info["progress"]["steps"].as_array().unwrap();
        let restart = steps.iter().find(|s| s["id"] == "restart").unwrap();
        assert_eq!(restart["state"], "fail", "{steps:?}");
        assert!(restart["detail"].as_str().unwrap().contains("did not load"));
        assert!(info["last"].get("deployed").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_miniserver_vislox_cannot_reach_is_not_called_out_of_service() {
        let (tmp, lox) = project_with_fake_lox(true);
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(tmp.path().join("project")),
            lox,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
            probe: Some(restart::tests::Script::probe(vec![(
                Err("unreachable".into()),
                None,
            )])),
        });
        a.apply(&ApplyOpts::default()).await.unwrap();
        let info = a.info();
        let steps = info["progress"]["steps"].as_array().unwrap();
        let restart = steps.iter().find(|s| s["id"] == "restart").unwrap();
        assert_eq!(restart["state"], "ok");
        assert!(restart["detail"].is_null(), "{steps:?}");
    }
}
