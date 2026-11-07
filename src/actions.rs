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

mod adopt;
mod removals;
mod rollback;
mod scan;
pub use removals::{NEEDS_CONSENT, RemovalConsent, plan_id as removal_plan_id, refused_slug};
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
/// Says what an action committed is not on the git remote (#71).
const NOT_PUSHED: &str = "not pushed to git";
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
    /// The base parsed and indexed for adopting (#72), by its blob id.
    adopt_base: Mutex<Option<(String, Arc<crate::adopt::Indexed>)>>,
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
    /// The pushes in the project's `backups/`, for Roll back (#69).
    pushes: crate::rollback::Pushes,
    /// The last removal plan worked out (#70), for Apply's consent to name.
    removals: Mutex<Option<removals::RemovalPlan>>,
    /// `HEAD` and the upstream's tracking branch when a push that is behind
    /// was last looked at again ([`Actions::recheck_push`]).
    push_key: Mutex<String>,
    /// Tests: what `/prog` says of an upload waiting ([`Actions::upload_waiting`]).
    #[cfg(test)]
    waiting: Mutex<Option<Result<Option<String>, String>>>,
}

/// What the Miniserver runs after a push that failed once lox began its
/// upload ([`Actions::what_runs`]): Roll back (#69) and Apply with removals
/// (#70) record it the same way.
#[derive(Debug, PartialEq)]
enum Runs {
    /// The program the push uploaded.
    Pushed,
    /// Still the program from before, and no upload is waiting to load.
    Before,
    /// Cannot tell: why.
    Unknown(String),
}

/// Whether `out` is the module compiled as it is now ([`Actions::freshness`]).
enum Fresh {
    /// `removals`: the blocks the push of that compile deletes (#70).
    Yes { removals: Vec<removals::Removed> },
    /// It compiles to another config: `out` is from before `changed`.
    Stale {
        changed: Vec<String>,
        removals: Vec<removals::Removed>,
    },
    /// It does not compile against the base (lox's message).
    Broken { changed: Vec<String>, error: String },
}

/// The module and base as a compile read them, and the config it wrote,
/// each by blob id: what is compiled is known by content.
#[derive(Clone, PartialEq)]
struct Compiled {
    module: std::collections::BTreeMap<String, String>,
    out: String,
    base: Option<String>,
    /// The lockfile as the compile left it: one changed since (by hand, or
    /// git) can make the module compile otherwise, or not at all (#70).
    lock: Option<String>,
}

/// A file's blob id, if it can be read.
fn file_blob(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| crate::history::blob_id(&b))
}

/// `GET /api/edits`' `schedules` and `schedule_sets` (#73). Per timer that
/// vislox's file claims now or claimed in the deployed revision: the set and
/// its switching times now (`set`, `modes`) and then (`deployed`, the value a
/// Revert sends), the other timers sharing its set (`shared`), and whether
/// the pending config switches it otherwise than the deployed one
/// (`pending`). And vislox's sets, each with the timers that claim it.
fn edit_schedules(
    now: &edit::Files,
    was: &edit::Files,
    lock: &edit::Lock,
    pending: Option<&loxone_config::LoxoneDoc>,
    deployed: Option<&loxone_config::LoxoneDoc>,
) -> (Value, Value) {
    crate::set_claim::edits_section::<crate::sched_edit::SchedKind>(
        now,
        was,
        lock,
        pending,
        deployed,
        crate::sched_edit::differs,
    )
}

/// `GET /api/edits`' `moods` and `mood_sets` (#74). Per lighting controller
/// that vislox's file claims the moods of now or claimed in the deployed
/// revision: the set and its moods now (`set`, `moods`) and then (`deployed`,
/// the value a Revert sends), the other controllers sharing its set
/// (`shared`), and whether the pending config has other moods on it than the
/// deployed one (`pending`). And vislox's sets, each with its controllers,
/// its moods and the claimants the config shown lost (`stale`).
fn edit_moods(
    now: &edit::Files,
    was: &edit::Files,
    lock: &edit::Lock,
    pending: Option<&loxone_config::LoxoneDoc>,
    deployed: Option<&loxone_config::LoxoneDoc>,
) -> (Value, Value) {
    crate::set_claim::edits_section::<crate::mood_edit::MoodKind>(
        now,
        was,
        lock,
        pending,
        deployed,
        crate::mood_edit::differs,
    )
}

/// `GET /api/edits`' `access` and `access_sets` (#76). Per NFC Code Touch
/// that vislox's file claims the access rights of now or claimed in the
/// deployed revision: the set and its rights now (`set`, `grants`, each a
/// user's or group's UUID and its access schedule's) and then (`deployed`, the
/// value a Revert sends), the other Code Touches sharing its set (`shared`),
/// and whether the pending config lets others in than the deployed one
/// (`pending`). And vislox's sets, each with its Code Touches, its rights and
/// the claimants the config shown lost (`stale`). UUIDs only: the titles are
/// the review's, and the pickers' lists need the admin token.
fn edit_access(
    now: &edit::Files,
    was: &edit::Files,
    lock: &edit::Lock,
    pending: Option<&loxone_config::LoxoneDoc>,
    deployed: Option<&loxone_config::LoxoneDoc>,
) -> (Value, Value) {
    crate::set_claim::edits_section::<crate::access_edit::AccessKind>(
        now,
        was,
        lock,
        pending,
        deployed,
        crate::access_edit::differs,
    )
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

/// lox's command run as the second step of a check: lint (#66) or the
/// tests (#67).
#[derive(Clone, Copy)]
enum Second {
    Lint,
    Test,
}

impl Second {
    fn cmd(self) -> &'static str {
        match self {
            Second::Lint => "lint",
            Second::Test => "test",
        }
    }

    /// Its field of `GET /api/check`.
    fn field(self) -> &'static str {
        self.cmd()
    }
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
    Ok(git_typed_objects(dir, specs)
        .await?
        .into_iter()
        .map(|o| o.map(|(_, b)| b))
        .collect())
}

/// [`git_objects`] with each object's type (`blob`, `tree`, …). The ids are
/// written while git's answer is read, so neither pipe fills up and stalls
/// the other; the time limit covers both.
async fn git_typed_objects(
    dir: &Path,
    specs: &[String],
) -> Result<Vec<Option<(String, Vec<u8>)>>, String> {
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
    let write = async move {
        stdin.write_all(input.as_bytes()).await?;
        // Closed: git ends after the last object.
        drop(stdin);
        Ok::<(), std::io::Error>(())
    };
    let (written, out) = tokio::time::timeout(GIT_TIMEOUT, async {
        tokio::join!(write, child.wait_with_output())
    })
    .await
    .map_err(|_| "git cat-file took too long".to_string())?;
    written.map_err(|e| format!("git cat-file: {e}"))?;
    let out = out.map_err(|e| format!("git cat-file: {e}"))?;
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
        let mut f = header.split(' ');
        let kind = f.nth(1).unwrap_or_default().to_string();
        let size: usize = f
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("git cat-file: {header}"))?;
        let body = rest.get(..size).ok_or("git cat-file: short output")?;
        objects.push(Some((kind, body.to_vec())));
        rest = rest.get(size + 1..).unwrap_or_default();
    }
    Ok(objects)
}

/// The first JSON value in a command's output: lox may print progress
/// messages after it.
fn first_json(out: &str) -> Option<Value> {
    let i = out.find('{')?;
    serde_json::Deserializer::from_str(&out[i..])
        .into_iter::<Value>()
        .next()?
        .ok()
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
    /// Commits the remote does not have, and why (#71): a push the scan
    /// blocked or git refused, for *Push to git*.
    push: Option<PushState>,
}

/// Why the git remote is behind (#71).
#[derive(Clone, Serialize)]
struct PushState {
    /// `found`: personal data in what the push would send; `unscanned`:
    /// vislox could not find out; `failed`: git push failed; `behind`:
    /// commits not pushed (found on start, or made during a push).
    state: &'static str,
    message: String,
    /// Where lox found personal data (never a value): the first
    /// [`SHOWN_FINDINGS`], so every event stays small.
    findings: Vec<scan::Finding>,
    /// How many there are in all.
    total: usize,
    /// What to do.
    advice: Option<String>,
    at: i64,
}

/// The findings `actions.push` carries; the rest are counted.
const SHOWN_FINDINGS: usize = 50;

impl PushState {
    fn new(state: &'static str, message: String, advice: Option<String>) -> PushState {
        PushState {
            state,
            message,
            findings: Vec::new(),
            total: 0,
            advice,
            at: now(),
        }
    }
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
const APPLY_STEPS: [(&str, &str); 12] = [
    ("module", "Module compiles"),
    ("removals", "Removals you confirmed"),
    ("tests", "Tests pass"),
    ("check", "Download and drift check"),
    ("upload", "Upload"),
    ("restart", "Program restarting"),
    ("online", "Back online"),
    ("loaded", "New config loaded"),
    ("verify", "Verified"),
    ("commit", "Committed"),
    ("scan", "No personal data in git"),
    ("push", "Pushed to git"),
];
/// A Roll back's steps (#69): Apply's, from the backup and the module
/// before the Apply.
const ROLLBACK_STEPS: [(&str, &str); 13] = [
    ("backup", "Backup checked"),
    ("module", "Module before the Apply compiles to it"),
    ("tests", "Tests pass"),
    ("stage", "Upload is what you reviewed"),
    ("check", "Download and drift check"),
    ("upload", "Upload"),
    ("restart", "Program restarting"),
    ("online", "Back online"),
    ("loaded", "Config loaded"),
    ("verify", "Verified"),
    ("commit", "Committed"),
    ("scan", "No personal data in git"),
    ("push", "Pushed to git"),
];
const LOG_LINES: usize = 12;

impl Progress {
    fn apply() -> Progress {
        Progress::of(&APPLY_STEPS)
    }

    fn rollback() -> Progress {
        Progress::of(&ROLLBACK_STEPS)
    }

    fn of(steps: &[(&'static str, &'static str)]) -> Progress {
        Progress {
            steps: steps
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
    /// `apply`, `rollback`, `sync`, `discard` or `compile`
    kind: &'static str,
    ok: bool,
    /// Done, but not verified: the Miniserver may not run it (`message`
    /// says why).
    warn: bool,
    /// Done, but the git remote does not have it (#71): `actions.push` says
    /// why.
    unpushed: bool,
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
            adopt_base: Mutex::new(None),
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
            pushes: Default::default(),
            removals: Mutex::new(None),
            push_key: Mutex::new(String::new()),
            #[cfg(test)]
            waiting: Mutex::new(None),
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

    /// What Apply, Roll back, Sync, Discard, an edit or Compile is doing now.
    pub fn running(&self) -> Option<&'static str> {
        self.st.lock().unwrap().running
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
            // The commits the git remote does not have, and why (#71).
            "push": st.push,
            // The removal plan the server keeps (#70): a tab's own preview
            // goes when it is another.
            "removals": self.removal_plan_id(),
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
            unpushed: res.as_ref().is_ok_and(|m| m.contains(NOT_PUSHED)),
            message: match res {
                Ok(m) | Err(m) => m.clone(),
            },
            at: now(),
            deployed,
        });
        let deploys = matches!(kind, "apply" | "rollback");
        if let (Err(e), Some(p)) = (res, st.progress.as_mut().filter(|_| deploys)) {
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
        // Blocks the module lost (#70): deleted only as the user confirmed
        // them, compiled into the project before the tests run on it.
        self.progress(|p| p.start("removals"));
        let consent = opts.allow_removals.as_ref();
        // Puts the lockfile and out back on any failure before the push
        // went through: the consent never outlives this request.
        let mut put_back = match self.removals_step(consent).await {
            Ok((state, detail, guard)) => {
                self.progress(|p| p.finish("removals", state, Some(detail)));
                guard
            }
            Err(why) => {
                self.end_with("apply", &Err(why.clone()), None);
                return Err((StatusCode::CONFLICT, why));
            }
        };
        // Then its tests (#67) must pass, on the config Apply deploys: the
        // check's run when it is of the files as they are, else run now.
        self.progress(|p| p.start("tests"));
        let tests = self.tests_now().await;
        match tests_verdict(&tests) {
            Ok((state, detail)) => self.progress(|p| p.finish("tests", state, Some(detail))),
            Err(why) => {
                drop(put_back.take());
                self.end_with("apply", &Err(why.clone()), None);
                return Err((StatusCode::CONFLICT, why));
            }
        }
        // The removals were compiled in: what is pushed must still be that
        // compile, the module and lockfile untouched since.
        if put_back.is_some() {
            let fresh = match self.project() {
                Ok(p) => matches!(self.freshness(&p).await, Ok(Fresh::Yes { .. })),
                Err(_) => false,
            };
            if !fresh {
                drop(put_back.take());
                let why = "The module or its lockfile changed after the removals you confirmed \
                           were compiled. Nothing was deployed: review the pending config again."
                    .to_string();
                self.end_with("apply", &Err(why.clone()), None);
                return Err((StatusCode::CONFLICT, why));
            }
        }
        let res = self.apply_steps(opts, put_back.as_mut()).await;
        // Failed before the push went through: put back now, before the end
        // is reported (the check reads the project then).
        drop(put_back);
        let (res, deployed) = match res {
            Ok((m, d)) => (Ok(m), d),
            Err(e) => (Err(e), None),
        };
        self.end_with("apply", &res, deployed);
        res.map_err(|e| (StatusCode::BAD_GATEWAY, e))
    }

    async fn apply_steps(
        &self,
        opts: &ApplyOpts,
        put_back: Option<&mut removals::PutBack<'_>>,
    ) -> Result<(String, Option<Deployed>), String> {
        let dir = self.s.project.as_deref().unwrap();
        let mut args = vec!["config", "push", "--force"];
        push_opts(opts, &mut args)?;
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
        let (pushed, ran_before, note) = match self.push_config(&args).await {
            Ok((pushed, ran_before)) => {
                // Pushed and verified: lox wrote the lockfile of what runs now.
                if let Some(g) = put_back {
                    g.keep();
                }
                (pushed, ran_before, None)
            }
            // Removals compiled in (#70): what runs decides what is kept.
            Err(why) => match put_back {
                None => return Err(why),
                Some(g) => {
                    let (pushed, note) = self.removals_pushed(&p, &why, before.as_deref()).await?;
                    // It runs what was confirmed: the lockfile and out stay.
                    g.keep();
                    let ran_before = crate::running::Report::failed("no check before".into());
                    (pushed, ran_before, Some(note))
                }
            },
        };
        let c = Commit {
            subject: &subject,
            body: None,
            force_add: &[],
        };
        self.after_push(&p, &pushed, &ran_before, before, &c)
            .await
            .map(|(m, d)| match note {
                Some(n) => (format!("{n}\n{m}"), d),
                None => (m, d),
            })
    }

    /// A push of removals the user confirmed (#70) failed: as for a Roll back
    /// (#69), before lox's upload line nothing changed; after it, what the
    /// Miniserver runs decides. The program confirmed: recorded as the base,
    /// not verified, and `Ok` with the push's message and the note. Anything
    /// else is `Err`, and the caller's guard puts the lockfile and out back.
    async fn removals_pushed(
        &self,
        p: &loxone_config::Project,
        why: &str,
        before: Option<&str>,
    ) -> Result<(String, String), String> {
        if !self.upload_started() {
            return Err(format!(
                "{why}\nlox stopped before the upload: nothing changed on the Miniserver. The \
                 lockfile and out were put back, so the removals are not kept."
            ));
        }
        let confirmed =
            std::fs::read(&p.out).map_err(|e| format!("{why}\ncannot read out: {e}"))?;
        let runs = self
            .what_runs(
                &crate::history::blob_id(&confirmed),
                before.unwrap_or(""),
                ("the previous", "the one you confirmed"),
            )
            .await;
        match runs {
            Runs::Before => Err(format!(
                "{why}\nThe Miniserver still runs the previous config, and no upload is waiting: \
                 nothing changed. The lockfile and out were put back, so the removals are not kept."
            )),
            Runs::Unknown(how) => Err(format!(
                "{why}\n⚠ lox failed after it started the upload, and vislox cannot tell which config \
                 the Miniserver runs now ({how}). It may run the previous config or the one you \
                 confirmed, with the blocks removed. The lockfile and out were put back, so the \
                 project is at its last commit and what was pending, which may not be what runs: \
                 check \"Which config the Miniserver runs\" in the History, then Pull or Sync."
            )),
            Runs::Pushed => {
                std::fs::write(&p.base, &confirmed).map_err(|e| {
                    format!(
                        "{why}\n⚠ The Miniserver runs the config you confirmed, but vislox could \
                         not record it ({e}). Pull or Sync."
                    )
                })?;
                Ok((
                    format!(
                        "lox config push failed after the upload:\n{}",
                        last_lines(why, 4)
                    ),
                    format!(
                        "{UNVERIFIED}: lox failed after the upload, but the Miniserver runs the \
                         program you confirmed, with the blocks removed (vislox downloaded it and \
                         compared)."
                    ),
                ))
            }
        }
    }

    /// `lox config push` with `args`, followed (see [`Self::push_followed`]),
    /// and what the Miniserver ran just before it, to verify by load.
    async fn push_config(&self, args: &[&str]) -> Result<(String, crate::running::Report), String> {
        let dir = self.s.project.as_deref().unwrap();
        let ran_before = self.s.running.get(true).await;
        let pushed = self
            .push_followed(&self.s.lox, args, dir)
            .await
            .map_err(|e| format!("lox config push failed:\n{e}"))?;
        Ok((pushed, ran_before))
    }

    /// After a verified push: check the Miniserver loaded it, keep the base
    /// in git, commit (`c`) and push. `before` is the deployed base's blob id
    /// before the push.
    async fn after_push(
        &self,
        p: &loxone_config::Project,
        pushed: &str,
        ran_before: &crate::running::Report,
        before: Option<String>,
        c: &Commit<'_>,
    ) -> Result<(String, Option<Deployed>), String> {
        let Commit {
            subject,
            body,
            force_add,
        } = *c;
        let dir = self.s.project.as_deref().unwrap();
        let mut report = vec![last_lines(pushed, 6)];
        report.push(self.verify_load(ran_before).await);
        if let Some(m) = keep_base(dir, &p.base)? {
            report.push(m);
        }
        let deployed = std::fs::read(&p.base).ok().map(|b| Deployed {
            before,
            after: crate::history::blob_id(&b),
        });
        self.progress(|p| p.start("commit"));
        git(dir, &["add", "-A", "."]).await?;
        let forced: Vec<String> = force_add
            .iter()
            .filter(|f| f.exists())
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        if !forced.is_empty() {
            let mut args = vec!["add", "-f", "--"];
            args.extend(forced.iter().map(String::as_str));
            git(dir, &args).await?;
        }
        let staged = git(dir, &["diff", "--cached", "--quiet"]).await.is_err();
        if staged {
            let mut args = vec!["commit", "-q", "-m", subject];
            if let Some(b) = body {
                args.extend(["-m", b]);
            }
            git(dir, &args).await?;
            report.push(format!("Committed: {subject}"));
            self.progress(|p| p.finish("commit", "ok", Some(subject.to_string())));
        } else {
            self.progress(|p| p.finish("commit", "skip", Some("nothing to commit".into())));
        }
        // Only what holds no personal data goes to the remote (#71). The
        // Miniserver runs the new config either way: only git is behind.
        match self.push_git(dir, true).await {
            Ok(m) => report.push(m),
            Err(e) => report.push(format!(
                "Deployed and committed, but {NOT_PUSHED}: {e}\nThe Miniserver runs the new \
                 config and the commit records it; only the git remote is behind. Push to git \
                 (Deploy pane) pushes once it is fixed."
            )),
        }
        Ok((report.join("\n"), deployed))
    }

    /// `git push`, only after `lox logic scan` found no personal data in
    /// what it would send (#71): every push vislox makes goes through here.
    /// It pushes exactly the commit it scanned, to the branch's upstream:
    /// `git push --no-follow-tags <remote> <sha>:<branch>`, so no other
    /// branch or tag goes along, whatever git's configuration says, and no
    /// commit made after the scan. With `steps`, as Apply's *No personal data
    /// in git* and *Pushed to git*. `Err` says why nothing was pushed;
    /// `actions.push` keeps it for *Push to git*, until a push goes through.
    async fn push_git(&self, dir: &Path, steps: bool) -> Result<String, String> {
        let step = |f: &dyn Fn(&mut Progress)| {
            if steps {
                self.progress(f);
            }
        };
        step(&|p| p.start("scan"));
        let ready = async {
            let t = scan::target(dir).await?;
            let sha = scan::head(dir).await?;
            Ok::<_, String>((t, sha))
        }
        .await;
        let (verdict, aim) = match ready {
            Err(why) => (scan::Verdict::CannotScan(why), None),
            Ok((t, sha)) => (
                scan::before_push(dir, &self.s.lox, &t, &sha).await,
                Some((t, sha)),
            ),
        };
        let blocked = match verdict {
            scan::Verdict::Clean(detail) => {
                step(&|p| p.finish("scan", "ok", Some(detail.clone())));
                None
            }
            scan::Verdict::Found(found) => {
                let files: std::collections::BTreeSet<&str> =
                    found.iter().map(|f| f.file.as_str()).collect();
                let unread = found.iter().all(|f| f.kind == "unreadable");
                let message = if unread {
                    format!(
                        "lox logic scan could not read {} in the commits to push, so it may hold \
                         personal data",
                        plural(files.len(), "file")
                    )
                } else {
                    format!(
                        "lox logic scan found personal data in the commits to push: {} in {}",
                        plural(found.len(), "finding"),
                        plural(files.len(), "file")
                    )
                };
                let mut b = PushState::new("found", message, Some(scan::advice(&found)));
                b.total = found.len();
                b.findings = found;
                Some(b)
            }
            scan::Verdict::CannotScan(why) => Some(PushState::new(
                "unscanned",
                format!("vislox could not check the push for personal data: {why}"),
                Some(
                    "vislox never pushes what it could not check. Once it can, push again (Push \
                     to git). Or check the commits yourself and push them from a terminal."
                        .into(),
                ),
            )),
        };
        if let Some(mut b) = blocked {
            let short = if b.state == "found" {
                format!("{}: not pushed", plural(b.total, "finding"))
            } else {
                "couldn't check: not pushed".to_string()
            };
            step(&|p| {
                p.finish("scan", "fail", Some(short.clone()));
                p.finish(
                    "push",
                    "skip",
                    Some("blocked: see Not pushed to git".into()),
                );
            });
            let mut text = b.message.clone();
            const IN_MESSAGE: usize = 20;
            for f in b.findings.iter().take(IN_MESSAGE) {
                text.push_str(&format!("\n  {}", f.text()));
            }
            if b.total > IN_MESSAGE {
                text.push_str(&format!("\n  … and {} more", b.total - IN_MESSAGE));
            }
            if let Some(a) = &b.advice {
                text.push_str(&format!("\n{a}"));
            }
            b.findings.truncate(SHOWN_FINDINGS);
            self.st.lock().unwrap().push = Some(b);
            self.changed();
            return Err(text);
        }
        let (t, sha) = aim.expect("scanned a target");
        step(&|p| p.start("push"));
        let refspec = format!("{sha}:{}", t.remote_ref);
        match git(
            dir,
            &["push", "-q", "--no-follow-tags", &t.remote, &refspec],
        )
        .await
        {
            Ok(_) => {
                // A commit made while this ran was not pushed: say so.
                let left = scan::unpushed(dir, &t).await.unwrap_or(0);
                self.st.lock().unwrap().push = (left > 0).then(|| {
                    PushState::new(
                        "behind",
                        format!(
                            "{} made during the push not pushed to git yet",
                            plural(left, "commit")
                        ),
                        None,
                    )
                });
                step(&|p| p.finish("push", "ok", None));
                self.changed();
                Ok("Pushed to git.".into())
            }
            Err(e) => {
                let why = format!("git push failed: {}", last_lines(&e, 3));
                self.st.lock().unwrap().push = Some(PushState::new(
                    "failed",
                    why.clone(),
                    Some("Fix what git says (credentials, the remote), then push again.".into()),
                ));
                step(&|p| p.finish("push", "fail", Some(last_lines(&e, 1))));
                self.changed();
                Err(why)
            }
        }
    }

    /// The last Apply's or Roll back's steps, once its commits are on the
    /// remote after all (#71): *Pushed to git*, and how.
    fn pushed_later(&self, how: &str) {
        let mut st = self.st.lock().unwrap();
        if let Some(p) = st.progress.as_mut()
            && p.steps.iter().any(|s| s.id == "push")
            && matches!(p.get("push").state, "skip" | "fail")
        {
            if p.get("scan").state == "fail" {
                p.finish("scan", "ok", Some("checked again".into()));
            }
            p.finish("push", "ok", Some(how.to_string()));
        }
    }

    /// *Push to git* (#71): push the commits the upstream does not have,
    /// after the same scan as every push. Changes nothing in the project.
    pub async fn push(&self) -> Result<String, Refusal> {
        use axum::http::StatusCode;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.begin("Push")?;
        let dir = self.s.project.as_deref().unwrap();
        let left = match scan::target(dir).await {
            Ok(t) => scan::unpushed(dir, &t).await.ok(),
            Err(_) => None,
        };
        let res = if left == Some(0) {
            self.st.lock().unwrap().push = None;
            self.pushed_later("the remote has it");
            Ok("Nothing to push: the upstream has every commit.".to_string())
        } else {
            let r = self.push_git(dir, false).await;
            if r.is_ok() {
                self.pushed_later("pushed later (Push to git)");
            }
            r
        };
        self.end("push", &res);
        res.map_err(|e| (StatusCode::CONFLICT, format!("Not pushed: {e}")))
    }

    /// On start: commits the upstream does not have yet (a push blocked or
    /// failed before vislox restarted) offer *Push to git* (#71).
    pub async fn find_unpushed(&self) {
        if self.why_not_apply().is_some() {
            return;
        }
        let dir = self.s.project.as_deref().unwrap();
        let Ok(t) = scan::target(dir).await else {
            return;
        };
        let n = scan::unpushed(dir, &t).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        let mut st = self.st.lock().unwrap();
        if st.push.is_none() {
            st.push = Some(PushState::new(
                "behind",
                format!("{} not pushed to git yet", plural(n, "commit")),
                None,
            ));
        }
        drop(st);
        self.changed();
    }

    /// With the git poll: a push that is behind is looked at again when
    /// `HEAD` or the upstream's tracking branch moved, and forgotten once the
    /// upstream has every commit (pushed from a terminal, say) (#71).
    pub async fn recheck_push(&self) {
        if self.st.lock().unwrap().push.is_none() || self.running().is_some() {
            return;
        }
        let dir = self.s.project.as_deref().unwrap();
        let Ok(t) = scan::target(dir).await else {
            return;
        };
        let key = format!(
            "{}:{}",
            scan::head(dir).await.unwrap_or_default(),
            git(dir, &["rev-parse", "-q", "--verify", &t.tracking])
                .await
                .unwrap_or_default()
                .trim()
        );
        {
            let mut k = self.push_key.lock().unwrap();
            if *k == key {
                return;
            }
            *k = key;
        }
        if scan::unpushed(dir, &t).await == Ok(0) {
            let mut st = self.st.lock().unwrap();
            // Not while an action started meanwhile: it sets its own.
            if st.running.is_none() && st.push.take().is_some() {
                drop(st);
                self.pushed_later("pushed outside vislox");
                self.changed();
            }
        }
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

    /// The running push got as far as lox's upload line: from then on the
    /// Miniserver may run what it uploaded.
    fn upload_started(&self) -> bool {
        self.st
            .lock()
            .unwrap()
            .progress
            .as_mut()
            .is_some_and(|p| p.get("upload").state != "wait")
    }

    /// Whether an upload waits in `/prog` to load on the next program start
    /// (its name), from the Miniserver's own listing.
    async fn upload_waiting(&self) -> Result<Option<String>, String> {
        #[cfg(test)]
        if let Some(w) = self.waiting.lock().unwrap().clone() {
            return w;
        }
        if !self.s.running.available() {
            return Err("whether an upload waits cannot be checked".into());
        }
        let r = self.s.running.get(true).await;
        match (r.error, r.waiting) {
            (Some(e), _) => Err(format!("/prog cannot be read: {e}")),
            (None, w) => Ok(w),
        }
    }

    /// After a push that failed once lox began the upload: download what the
    /// Miniserver runs (read-only, `lox config download`) and compare it with
    /// the program the push uploaded (`pushed`) and the one before
    /// (`before`), each by blob id. `names`: how the messages call them.
    async fn what_runs(&self, pushed: &str, before: &str, names: (&str, &str)) -> Runs {
        let (before_name, pushed_name) = names;
        let dir = self.s.project.as_deref().unwrap();
        let scratch = match removals::scratch_dir("vislox-runs") {
            Ok(d) => d,
            Err(e) => return Runs::Unknown(e),
        };
        let _gone = RemoveDir(scratch.clone());
        let zip = scratch.join("now.zip");
        let z = zip.to_string_lossy();
        if let Err(e) = run(
            &self.s.lox,
            &["config", "download", "--save-as", &z],
            dir,
            PUSH_TIMEOUT,
        )
        .await
        {
            return Runs::Unknown(format!("lox config download: {}", last_lines(&e, 1)));
        }
        let xml = match std::fs::read(&zip)
            .map_err(|e| e.to_string())
            .and_then(|z| {
                loxone_config::archive::extract_and_decompress(&z).map_err(|e| e.to_string())
            }) {
            Ok(x) => x,
            Err(e) => return Runs::Unknown(format!("its download cannot be read: {e}")),
        };
        let blob = crate::history::blob_id(&xml);
        if blob == pushed {
            return Runs::Pushed;
        }
        if blob != before {
            return Runs::Unknown(format!(
                "its newest program ({}) is neither {before_name} nor {pushed_name}",
                &blob[..10]
            ));
        }
        // The program from before: unless an upload waits to load on the next start.
        match self.upload_waiting().await {
            Err(e) => Runs::Unknown(format!("it runs {before_name} program, but {e}")),
            Ok(Some(w)) => Runs::Unknown(format!(
                "it runs {before_name} program, but {w} waits to be loaded"
            )),
            Ok(None) => Runs::Before,
        }
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
                let report: Value = first_json(&out).unwrap_or(Value::Null);
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
        match self.push_git(dir, false).await {
            Ok(_) => report.push("Committed and pushed to git.".into()),
            Err(e) => report.push(format!(
                "Committed, but {NOT_PUSHED}: {e}\nPush to git (Deploy pane) pushes once it is \
                 fixed."
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
        let scheduling = req
            .ops
            .iter()
            .any(|o| matches!(o, edit::Op::Schedule { .. }));
        let mooding = req.ops.iter().any(|o| matches!(o, edit::Op::Moods { .. }));
        let membering = req
            .ops
            .iter()
            .any(|o| matches!(o, edit::Op::Members { .. }));
        let accessing = req.ops.iter().any(|o| matches!(o, edit::Op::Access { .. }));
        let names = if scheduling
            || mooding
            || req
                .ops
                .iter()
                .any(|o| matches!(o, edit::Op::AddBlock { .. }))
        {
            edit::module_names(&p.module)
        } else {
            Default::default()
        };
        // The timers whose switching times the rest of the module claims
        // (#73): a timer takes one schedule.
        let mut sched_claimed = std::collections::BTreeMap::new();
        // And the lighting controllers whose moods it claims (#74).
        let mut moods_claimed = std::collections::BTreeMap::new();
        // And the central blocks whose members it claims (#75).
        let mut members_claimed = std::collections::BTreeMap::new();
        // And the NFC Code Touches whose access rights it claims (#76).
        let mut access_claimed = std::collections::BTreeMap::new();
        if scheduling || mooding || membering || accessing {
            let texts = {
                let mut m = self.module_files.lock().unwrap();
                m.refresh(&p.root, &p.module);
                m.texts()
            };
            let mine = format!("{shown}/");
            let claims = crate::managed::set_claims(&texts, &lock)
                .into_iter()
                .chain(crate::managed::members_claims(&texts, &lock));
            for c in claims {
                let into = match c.kind {
                    "schedule" => &mut sched_claimed,
                    "moods" => &mut moods_claimed,
                    "members" => &mut members_claimed,
                    "access" => &mut access_claimed,
                    _ => continue,
                };
                if !c.file.starts_with(&mine) {
                    for b in &c.blocks {
                        into.insert(b.to_ascii_lowercase(), format!("{}:{}", c.file, c.line));
                    }
                }
            }
        }
        let drawn = files.claims.1.clone();
        // Adopting (#72) is worked out on a scratch copy first: lox writes
        // there, and the ops move what it wrote into vislox's files.
        let adopt_op = |o: &edit::Op| {
            matches!(
                o,
                edit::Op::Adopt { .. } | edit::Op::AdoptPage { .. } | edit::Op::Unadopt { .. }
            )
        };
        let adopting = req.ops.iter().any(adopt_op);
        if adopting && !req.ops.iter().all(adopt_op) {
            return Err((
                StatusCode::BAD_REQUEST,
                "adopt, adopt_page and unadopt go in a request of their own".into(),
            ));
        }
        let prepared = if req
            .ops
            .iter()
            .any(|o| matches!(o, edit::Op::Adopt { .. } | edit::Op::AdoptPage { .. }))
        {
            // The cheap check first: lox runs once per block on the copy.
            self.uncompiled(&p).await?;
            self.adopt_prepare(&p, &files, &lock, &base, &doc, &req.ops)
                .await?
        } else {
            adopt::Prepared::default()
        };
        let ctx = edit::Ctx {
            doc: &doc,
            base: &base,
            lock: &lock,
            deployed: deployed.as_ref(),
            names: &names,
            drawn: &drawn,
            adoptions: &prepared.adoptions,
            page_adoptions: &prepared.pages,
            sched_claimed: &sched_claimed,
            moods_claimed: &moods_claimed,
            members_claimed: &members_claimed,
            access_claimed: &access_claimed,
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
        // someone else made to it and has not compiled. Except dropping
        // vislox's claim on switching times (#73) or moods (#74) the module
        // claims too: lox compiles neither claim, and the module's is what
        // it is for.
        let mending = req.ops.iter().all(|o| match o {
            edit::Op::Schedule { uuid, value: None } => {
                sched_claimed.contains_key(&uuid.to_ascii_lowercase())
            }
            edit::Op::Moods { uuid, value: None } => {
                moods_claimed.contains_key(&uuid.to_ascii_lowercase())
            }
            edit::Op::Members { uuid, value: None } => {
                members_claimed.contains_key(&uuid.to_ascii_lowercase())
            }
            edit::Op::Access { uuid, value: None } => {
                access_claimed.contains_key(&uuid.to_ascii_lowercase())
            }
            _ => false,
        });
        if !mending {
            self.uncompiled(&p).await?;
        }
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
        // An adopted block's lockfile entry (#72), or its undo's.
        if !files.lock_edits.is_empty() {
            let shown_lock = p.lock.strip_prefix(&p.root).unwrap_or(&p.lock).display();
            let bytes = std::fs::read(&p.lock)
                .map_err(|e| fail(format!("cannot read {shown_lock}: {e}")))?;
            let edited = crate::adopt::write_lock(&bytes, &files.lock_edits).map_err(bad)?;
            std::fs::write(&p.lock, edited)
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
            lock: file_blob(&p.lock),
        };
        // The UUIDs lox minted for the new blocks.
        let lock = edit::Lock::read(&p.lock);
        // The switching times the request writes (#73): the timers it gives a
        // set, and the sets it writes back for others.
        let mut timed: Vec<String> = Vec::new();
        let mut restored: Vec<String> = Vec::new();
        // The same for moods (#74): the controllers, the sets written back.
        let mut lit: Vec<String> = Vec::new();
        let mut lit_restored: Vec<String> = Vec::new();
        // And the central blocks whose members it writes (#75).
        let mut centrals: Vec<String> = Vec::new();
        // And the Code Touches whose access rights it writes, and the access
        // sets written back (#76).
        let mut doors: Vec<String> = Vec::new();
        let mut doors_restored: Vec<String> = Vec::new();
        for o in &req.ops {
            match o {
                edit::Op::Schedule {
                    uuid,
                    value: Some(v),
                } => {
                    timed.push(uuid.clone());
                    restored.extend(v.restore.keys().cloned());
                }
                edit::Op::Moods {
                    uuid,
                    value: Some(v),
                } => {
                    lit.push(uuid.clone());
                    lit_restored.extend(v.restore.keys().cloned());
                }
                edit::Op::Members {
                    uuid,
                    value: Some(_),
                } => centrals.push(uuid.clone()),
                edit::Op::Access {
                    uuid,
                    value: Some(v),
                } => {
                    doors.push(uuid.clone());
                    doors_restored.extend(v.restore.keys().cloned());
                }
                _ => {}
            }
        }
        // The compiled config, read once for the checks below.
        let out_doc = if !place.moves.is_empty()
            || !timed.is_empty()
            || !lit.is_empty()
            || !centrals.is_empty()
            || !doors.is_empty()
            || adopting
        {
            Some(
                std::fs::read(&p.out)
                    .map_err(|e| fail(e.to_string()))
                    .and_then(|b| edit::parse_doc(&b).map_err(fail))?,
            )
        } else {
            None
        };
        // A move lox did not keep is refused, not shown as done. A page laid
        // out afresh places its blocks anew, moved or not.
        if let (false, Some(out)) = (place.moves.is_empty(), &out_doc) {
            let missed = edit::moves_missed(out, &base, &lock, place);
            if !missed.is_empty() {
                return Err(fail(format!(
                    "lox logic compile did not keep the new position of {}",
                    missed.join(", ")
                )));
            }
        }
        // A schedule claim lox did not write onto its timer is refused, not
        // shown as done (#73).
        if let (false, Some(out)) = (timed.is_empty(), &out_doc) {
            let missed = crate::sched_edit::missed(&files, out, &lock, &timed, &restored);
            if !missed.is_empty() {
                return Err(fail(format!(
                    "lox logic compile did not write the switching times of {}",
                    missed.join(", ")
                )));
            }
        }
        // Nor a moods claim lox did not write onto its controller (#74).
        if let (false, Some(out)) = (lit.is_empty(), &out_doc) {
            let missed = crate::mood_edit::missed(&files, out, &lock, &lit, &lit_restored);
            if !missed.is_empty() {
                return Err(fail(format!(
                    "lox logic compile did not write the moods of {}",
                    missed.join(", ")
                )));
            }
        }
        // Nor a members claim lox did not write onto its central block (#75).
        if let (false, Some(out)) = (centrals.is_empty(), &out_doc) {
            let missed = crate::member_edit::missed(&files, out, &lock, &centrals);
            if !missed.is_empty() {
                return Err(fail(format!(
                    "lox logic compile did not write the members of {}",
                    missed.join(", ")
                )));
            }
        }
        // Nor an access claim lox did not write onto its Code Touch (#76).
        if let (false, Some(out)) = (doors.is_empty(), &out_doc) {
            let missed = crate::access_edit::missed(&files, out, &lock, &doors, &doors_restored);
            if !missed.is_empty() {
                return Err(fail(format!(
                    "lox logic compile did not write the access rights of {}",
                    missed.join(", ")
                )));
            }
        }
        // Adopting changes who owns a block, never what the Miniserver runs:
        // the pending config is what it was, each block exactly the base's.
        let mut adopted = Vec::new();
        if let (true, Some(out)) = (adopting, &out_doc) {
            let mut uuids: Vec<String> = Vec::new();
            for op in &req.ops {
                match op {
                    edit::Op::Adopt { blocks } => {
                        uuids.extend(blocks.iter().map(|b| b.uuid.clone()))
                    }
                    edit::Op::AdoptPage { page } => {
                        uuids.extend(prepared.pages.get(page).cloned().unwrap_or_default())
                    }
                    edit::Op::Unadopt { uuids: u } => uuids.extend(u.iter().cloned()),
                    _ => {}
                }
            }
            crate::adopt::verify(&base, &doc, out, &uuids)
                .map_err(|e| bad(format!("{e}. Nothing was adopted or handed back.")))?;
            for u in &uuids {
                if let Some(a) = prepared.adoptions.get(u) {
                    adopted.push(json!({ "uuid": u, "slug": a.slug }));
                }
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
        if adopting {
            r["adopted"] = json!(adopted);
            r["skipped"] = json!(prepared.skipped);
        }
        undo_into(&mut r, edit::undo_ops(&req.ops, &previous, &minted));
        Ok(r)
    }

    /// Refuse when the module holds changes that are not compiled into the
    /// pending config: an edit's compile would fold them in. See
    /// [`Self::freshness`].
    async fn uncompiled(&self, p: &loxone_config::Project) -> Result<(), Refusal> {
        use axum::http::StatusCode;
        let refuse = |msg: String| (StatusCode::CONFLICT, msg);
        let files = |changed: &[String]| {
            if changed.is_empty() {
                "its files".to_string()
            } else {
                changed.join(", ")
            }
        };
        match self.freshness(p).await.map_err(refuse)? {
            Fresh::Yes { .. } => Ok(()),
            Fresh::Broken { changed, error } => Err(refuse(format!(
                "The module does not compile as it is ({} changed since the last compile). \
                 Fix or discard that, then edit:\n{}",
                files(&changed),
                last_lines(&error, 6)
            ))),
            Fresh::Stale { changed, .. } => Err(refuse(format!(
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
    async fn freshness(&self, p: &loxone_config::Project) -> Result<Fresh, String> {
        let module = module_blobs(p);
        let out_blob = file_blob(&p.out);
        let base_blob = file_blob(&p.base);
        let lock_blob = file_blob(&p.lock);
        let last = self.compiled.lock().unwrap().clone();
        let base_bytes = std::fs::read(&p.base).unwrap_or_default();
        if let (Some(last), Some(o)) = (&last, &out_blob)
            && last.module == module
            && &last.out == o
            && last.base == base_blob
            && last.lock == lock_blob
        {
            let lock = std::fs::read(&p.lock).ok();
            return Ok(Fresh::Yes {
                removals: removals::pending(lock.as_deref(), &base_bytes),
            });
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
        let (scratch, res) = self.scratch_compile(p, &[]).await?;
        let removals = || {
            let lock = std::fs::read(&scratch.lock).ok();
            removals::pending(lock.as_deref(), &base_bytes)
        };
        match res {
            Err(error) => Ok(Fresh::Broken { changed, error }),
            Ok(_) if std::fs::read(&scratch.out).ok().as_deref() == Some(reference.as_slice()) => {
                // Compiled as it is: remembered, so the next edit need not
                // compile it again; unless the module changed meanwhile (an
                // edit while `GET /api/check` compiled), when what compiled
                // is not known.
                if let Some(o) = out_blob
                    && module_blobs(p) == module
                    && file_blob(&p.base) == base_blob
                    && file_blob(&p.lock) == lock_blob
                {
                    *self.compiled.lock().unwrap() = Some(Compiled {
                        module,
                        out: o,
                        base: base_blob,
                        lock: lock_blob,
                    });
                }
                Ok(Fresh::Yes {
                    removals: removals(),
                })
            }
            Ok(_) => Ok(Fresh::Stale {
                changed,
                removals: removals(),
            }),
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
        let (a, b) = tokio::join!(
            async { lint && self.second_run(Second::Lint, k, files).await },
            async { test && self.second_run(Second::Test, k, files).await },
        );
        a || b
    }

    /// `lox logic lint .` (#66) or `lox logic test .` (#67) on the project,
    /// as the second step of a check that found no errors. Both compile the
    /// module in memory and write nothing, so neither holds the compile
    /// lock: an edit or an Apply never waits for it, and one that starts
    /// meanwhile stops it (the action's end wakes the worker, which runs it
    /// again). What it found is kept only when the files are still the ones
    /// it read. Returns whether to work again.
    async fn second_run(&self, which: Second, k: &str, files: &Option<String>) -> bool {
        let ran = tokio::select! {
            r = self.run_logic(which) => r,
            _ = self.preempted() => return false,
        };
        if self.check_key() != k || self.files_key_now().await != *files {
            return true;
        }
        !self.keep_second(which, k, ran)
    }

    /// Keep `v` as `which`'s result of the check of key `k`, and tell the
    /// browsers; `false` when the check kept is of another key, or does
    /// not compile.
    fn keep_second(&self, which: Second, k: &str, v: Value) -> bool {
        let mut cache = self.check_cache.lock().unwrap();
        let Some(c) = cache.as_mut().filter(|c| c.key == k && c.v["ok"] == true) else {
            return false;
        };
        c.v[which.field()] = v;
        match which {
            Second::Lint => c.lint_due = false,
            Second::Test => c.test_due = false,
        }
        drop(cache);
        self.checked();
        true
    }

    /// `lox logic lint .` or `lox logic test .` on the project as it is
    /// now, read and pinned to the module's lines and blocks:
    /// `{ available: true, findings }` (lint) or `{ available: true, tests,
    /// passed, failed }` (test); else `{ available: false, reason }`, with
    /// `unsupported` for a lox without the command, which is then not asked
    /// again.
    async fn run_logic(&self, which: Second) -> Value {
        let cmd = which.cmd();
        let off = |why: String| json!({ "available": false, "reason": why });
        let (Some(dir), Ok(p)) = (self.s.project.as_deref(), self.project()) else {
            return off("only for a lox.toml project".into());
        };
        let args = ["logic", cmd, "."];
        let (code, o, e) = match run_status(&self.s.lox, &args, dir, COMPILE_TIMEOUT).await {
            Ok(r) => r,
            Err(reason) => return off(reason),
        };
        let texts = || {
            let mut m = self.module_files.lock().unwrap();
            m.refresh(&p.root, &p.module);
            m.texts()
        };
        let shown = |f: &str| crate::managed::shown(&p.root, &p.module, &dir.join(f));
        let read = match which {
            Second::Lint => check::parse_lint(&o, shown).map(|mut findings| {
                if !findings.is_empty() {
                    check::locate_findings(&mut findings, &texts(), &edit::Lock::read(&p.lock));
                }
                json!({ "available": true, "findings": findings })
            }),
            Second::Test => check::parse_test(&o).map(|mut t| {
                if !t.tests.is_empty() {
                    check::locate_tests(&mut t.tests, &texts(), &edit::Lock::read(&p.lock));
                }
                let passed = t.tests.iter().filter(|t| t.status == "pass").count();
                json!({ "available": true, "tests": t.tests, "passed": passed,
                        "failed": t.tests.len() - passed })
            }),
        };
        match read {
            Some(v) => v,
            None if check::unsupported(code, &o, &e) => {
                self.unsupported
                    .lock()
                    .unwrap()
                    .insert(cmd, self.lox_identity());
                json!({ "available": false, "unsupported": true,
                        "reason": format!("this lox has no `lox logic {cmd}`") })
            }
            None => {
                let e = last_lines(e.trim().trim_start_matches("Error: "), 3);
                off(if e.is_empty() {
                    format!("lox logic {cmd} gave no answer vislox can read")
                } else {
                    e
                })
            }
        }
    }

    /// The tests of the exact config Apply deploys (#67): the check
    /// worker's run when it is of the files as they are now, done, and an
    /// answer (`available`, or a lox without `logic test`); else a run now,
    /// kept for the browsers when the check is of these files. A run that
    /// timed out or could not be read is never taken: it is run again.
    async fn tests_now(&self) -> Value {
        let k = self.check_key();
        let files = self.edit_state.lock().unwrap().files.clone();
        let fresh = self.files_key_now().await == files;
        if fresh
            && let Some(c) = &*self.check_cache.lock().unwrap()
            && c.key == k
            && !c.test_due
            && c.v["test"]["running"] != true
            && (c.v["test"]["available"] == true || c.v["test"]["unsupported"] == true)
        {
            return c.v["test"].clone();
        }
        let v = self.run_logic(Second::Test).await;
        if self.check_key() == k && self.files_key_now().await == files {
            self.keep_second(Second::Test, &k, v.clone());
        }
        v
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
    /// Lint (#66) and the tests (#67) are [`Self::second_run`]'s, after it. The caller holds the
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
        // And what its push would remove (#70): lox's refusal for a block
        // the module lost, or the blocks it deletes. Apply asks for them.
        let removal = |refused: Option<String>, blocks: &[removals::Removed]| {
            if refused.is_none() && blocks.is_empty() {
                Value::Null
            } else {
                json!({ "refused": refused, "blocks": blocks })
            }
        };
        let (stale, removals) = if !checked.errors.is_empty() {
            (Value::Null, Value::Null)
        } else {
            match self.freshness(&p).await {
                Err(_) => (Value::Null, Value::Null),
                Ok(Fresh::Yes { removals }) => (Value::Null, removal(None, &removals)),
                Ok(Fresh::Stale { changed, removals }) => {
                    (json!({ "files": changed }), removal(None, &removals))
                }
                Ok(Fresh::Broken { changed, error }) => (
                    json!({ "files": changed, "error": last_lines(&error, 6) }),
                    removal(refused_slug(&error), &[]),
                ),
            }
        };
        json!({
            "available": true,
            "ok": checked.errors.is_empty(),
            "errors": checked.errors,
            "truncated": checked.truncated,
            "unformatted": unformatted,
            "stale": stale,
            "removals": removals,
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
            let lock = file_blob(&p.lock);
            *self.compiled.lock().unwrap() = Some(Compiled {
                module,
                out,
                base,
                lock,
            });
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
        let off = || json!({ "available": false, "reason": state.why, "claims": [], "managed": [], "sources": {}, "set_claims": [], "members_claims": [], "schedules": [], "schedule_sets": [], "moods": [], "mood_sets": [], "members": [], "access": [], "access_sets": [] });
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
        // The module's claims on a block's access rights, recipients or
        // switching times (#68), and the block each is on.
        let set_claims = crate::managed::set_claims(&texts, &lock);
        // And on a central block's members (#75).
        let members_claims = crate::managed::members_claims(&texts, &lock);
        // The members vislox's file claims now or claimed in the deployed
        // revision, per central block.
        let members = crate::member_edit::edits_section(
            &now,
            &was,
            &lock,
            pending_doc.as_ref(),
            deployed_doc.as_ref(),
        );
        // The switching times vislox's file claims now or claimed in the
        // deployed revision (#73), per timer, and its sets.
        let (schedules, schedule_sets) = edit_schedules(
            &now,
            &was,
            &lock,
            pending_doc.as_ref(),
            deployed_doc.as_ref(),
        );
        // And the lighting moods (#74), per controller, and its sets.
        let (moods, mood_sets) = edit_moods(
            &now,
            &was,
            &lock,
            pending_doc.as_ref(),
            deployed_doc.as_ref(),
        );
        // And who may open the NFC Code Touches (#76), per Code Touch, and
        // its sets.
        let (access, access_sets) = edit_access(
            &now,
            &was,
            &lock,
            pending_doc.as_ref(),
            deployed_doc.as_ref(),
        );
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
        let mut blocks: Vec<Value> = now
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
                    "adopted": b.adopted,
                    // In vislox's files as the deployed revision's commit has them.
                    "committed": was.block(&b.slug).is_some(),
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
            .map(|d| edit::module_wires(d, &lock, &now, edit::Owner::Module))
            .unwrap_or_default()
            .into_iter()
            .map(|(f, t)| json!({ "from": f, "to": t }))
            .collect();
        // The wires a block adopted here brought along (#72).
        let adopted_wires: Vec<Value> = shown
            .map(|d| edit::module_wires(d, &lock, &now, edit::Owner::Adopted))
            .unwrap_or_default()
            .into_iter()
            .map(|(f, t)| json!({ "from": f, "to": t }))
            .collect();
        // Which blocks Properties offers to adopt (#72), on the config shown:
        // UUID → null, or why not. Only blocks drawn on a page, of a type lox
        // has a table for, that the module does not manage yet.
        // One walk of each config (an index), and the base parsed once per blob.
        let mut adopt = serde_json::Map::new();
        if let (Some(doc), Some(base)) = (shown, self.adopt_base(&p))
            && state.why.is_none()
        {
            let (base_doc, base_ix) = (&base.0, &base.1);
            let ix = crate::adopt::Index::of(doc);
            for (u, _) in ix.page_blocks() {
                if lock.slug_of(u).is_none() {
                    adopt.insert(
                        u.clone(),
                        json!(adopt::why_not(u, base_ix, &ix, &lock, &now)),
                    );
                }
            }
            // Whether each block adopted here can be handed back now (or why not):
            // the same op the button sends, tried on a copy of the files.
            let (none_a, none_p) = (Default::default(), Default::default());
            let ctx = edit::Ctx {
                doc,
                base: base_doc,
                lock: &lock,
                deployed: None,
                names: &Default::default(),
                drawn: &Default::default(),
                adoptions: &none_a,
                page_adoptions: &none_p,
                sched_claimed: &Default::default(),
                moods_claimed: &Default::default(),
                members_claimed: &Default::default(),
                access_claimed: &Default::default(),
            };
            for b in blocks.iter_mut().filter(|b| b["adopted"] == true) {
                let Some(u) = b["uuid"].as_str().map(String::from) else {
                    continue;
                };
                let op = edit::Op::Unadopt { uuids: vec![u] };
                let why = edit::apply_op(&mut now.clone(), &ctx, &op).err();
                b["hand_back"] = json!(why);
            }
        }
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
            "set_claims": set_claims,
            "members_claims": members_claims,
            "members": members,
            "schedules": schedules,
            "schedule_sets": schedule_sets,
            "moods": moods,
            "mood_sets": mood_sets,
            "access": access,
            "access_sets": access_sets,
            "movable": movable,
            "moves": moves,
            "arrange": arrange,
            "wires": wires,
            "module_wires": module_wires,
            "adopted_wires": adopted_wires,
            "adopt": adopt,
            "wire_why": {
                "module": edit::Owner::Module.why(),
                "loxone_config": edit::Owner::LoxoneConfig.why(),
                "adopted": edit::Owner::Adopted.why(),
            },
        })
    }

    /// The project's base, parsed and indexed (#72): kept by blob id, so
    /// `GET /api/edits` parses it once per base, not per refresh.
    fn adopt_base(&self, p: &loxone_config::Project) -> Option<Arc<crate::adopt::Indexed>> {
        let bytes = std::fs::read(&p.base).ok()?;
        let blob = crate::history::blob_id(&bytes);
        if let Some((b, v)) = &*self.adopt_base.lock().unwrap()
            && *b == blob
        {
            return Some(v.clone());
        }
        let doc = edit::parse_doc(&bytes).ok()?;
        let ix = crate::adopt::Index::of(&doc);
        let v = Arc::new((doc, ix));
        *self.adopt_base.lock().unwrap() = Some((blob, v.clone()));
        Some(v)
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

/// The commit after a push: its subject and body, and files to commit even
/// when ignored (a Roll back's module files from before).
#[derive(Clone, Copy)]
struct Commit<'a> {
    subject: &'a str,
    body: Option<&'a str>,
    force_add: &'a [PathBuf],
}

/// The flags of the user's answers to a refused Apply or Roll back.
fn push_opts<'a>(opts: &'a ApplyOpts, args: &mut Vec<&'a str>) -> Result<(), String> {
    if let Some(v) = &opts.accept_version {
        if v.is_empty() || v.len() > 12 || !v.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("not a Loxone Config version: {v}"));
        }
        args.extend(["--accept-version", v.as_str()]);
    }
    if opts.discard_custom_changes {
        args.push("--discard-custom-changes");
    }
    Ok(())
}

/// The user's answers to a refused Apply.
#[derive(serde::Deserialize, Default)]
pub struct ApplyOpts {
    /// Accept a base saved by this Loxone Config release (`--accept-version`).
    pub accept_version: Option<String>,
    /// Delete the custom changes the Miniserver holds (`--discard-custom-changes`).
    #[serde(default)]
    pub discard_custom_changes: bool,
    /// Delete the managed blocks the module lost (#70): exactly these, of
    /// the plan reviewed. Never a flag of the push (see `removals`).
    #[serde(default)]
    pub allow_removals: Option<RemovalConsent>,
}

/// What a refused Apply could be retried with, from lox's message: the
/// Loxone Config release to accept, the custom changes an upload would
/// delete, and a managed block the module lost (#70).
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
        // A block the module lost (#70): the browser asks for the plan.
        "removals": refused_slug(msg)
            .or_else(|| msg.contains(NEEDS_CONSENT).then(|| "pending".to_string())),
    })
}

/// After a verified push, lox points `base` in lox.toml at its download in
/// `backups/`, which is git-ignored (configs carry personal data). This
/// project keeps its deployed config as its base instead (in git when the
/// project tracks it; the push scan, #71, stops one with personal data):
/// copy that download to where the base was and point lox.toml back at it.
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

/// What Apply's *Tests pass* step makes of `lox logic test` (#67): `ok`
/// with how many pass, `skip` only when the module has no tests or this lox
/// has no `logic test`, or the refusal: tests that could not run at all
/// (fail closed), or each test that fails or could not run (the first
/// five), with its file and line and the `expect`s that did not hold.
fn tests_verdict(t: &Value) -> Result<(&'static str, String), String> {
    if t["unsupported"] == true {
        let why = t["reason"]
            .as_str()
            .unwrap_or("this lox has no `lox logic test`");
        return Ok(("skip", why.to_string()));
    }
    if t["available"] != true {
        // A timeout, a lox that did not start, an answer vislox cannot
        // read: the tests may fail, so Apply does not go on.
        let why = t["reason"]
            .as_str()
            .unwrap_or("lox logic test gave no answer");
        return Err(format!(
            "Tests couldn't run (lox logic test), so nothing was deployed: {}",
            last_lines(why, 3)
        ));
    }
    let tests = t["tests"].as_array().cloned().unwrap_or_default();
    if tests.is_empty() {
        return Ok(("skip", "the module has no tests".into()));
    }
    let bad: Vec<&Value> = tests.iter().filter(|t| t["status"] != "pass").collect();
    if bad.is_empty() {
        let n = tests.len();
        return Ok(("ok", format!("{} passed", plural(n, "test"))));
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

/// `3 blocks`, `1 block`.
pub(crate) fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// Why Apply must not run: the module has errors (#65), the first five of
/// them; `None` when it has none.
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

    /// A stand-in lox's `logic scan --json` (#71): no personal data in any
    /// file it is given.
    const CLEAN_SCAN: &str = "  'logic scan')\n    shift 3; printf '{'; sep=; for f in \"$@\"; do printf '%s\"%s\": []' \"$sep\" \"$f\"; sep=,; done; echo '}' ;;\n";

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
                "#!/bin/sh\ncase \"$1 $2\" in\n{CLEAN_SCAN}  'logic diff') echo 'Add a hallway light' ;;\n  'logic test') echo 'no tests in pages' ;;\n  'config push')\n    echo 'Downloading the live config...' >&2\n    echo 'Uploading as sps_new.zip...' >&2\n    echo 'Restarting the program...' >&2\n    sleep 2\n    {push} ;;\nesac\n"
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
    /// `../notest` exists; cut off before its count, with an error, while
    /// `../garbletest` exists), whose
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
    [ -e ../garbletest ] && { echo 'test "Teiler" ... ok'; echo 'Error: the simulator crashed' >&2; exit 1; }
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
        assert_eq!(steps[1]["id"], "removals", "{steps}");
        assert_eq!(steps[1]["state"], "skip", "{steps}");
        assert_eq!(steps[2]["id"], "tests", "{steps}");
        assert_eq!(steps[2]["state"], "fail", "{steps}");
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
        assert_eq!(steps[2]["state"], "ok", "{steps}");
        assert_eq!(steps[2]["detail"], "1 test passed", "{steps}");
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
    async fn tests_that_give_no_answer_stop_apply_and_are_run_again_by_it() {
        use axum::http::StatusCode;
        let (tmp, d, a) = project_to_check();
        std::fs::write(d.join("pages/main.loxl"), with_test("ok")).unwrap();
        a.compile().await.unwrap();
        // The check's run gives no answer vislox can read.
        std::fs::write(tmp.path().join("garbletest"), "").unwrap();
        let c = check_of(&a, &d).await;
        assert_eq!(
            c["test"],
            json!({ "available": false, "reason": "the simulator crashed" }),
            "{c}"
        );
        // Apply does not take it: it runs the tests again, and with no
        // answer still, refuses (fails closed), before anything is sent.
        let runs = || {
            calls_in(&tmp)
                .lines()
                .filter(|l| *l == "logic test .")
                .count()
        };
        let before = runs();
        let (code, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT, "{msg}");
        assert_eq!(
            msg,
            "Tests couldn't run (lox logic test), so nothing was deployed: the simulator crashed"
        );
        assert_eq!(runs(), before + 1, "{}", calls_in(&tmp));
        assert_eq!(a.info()["progress"]["steps"][2]["state"], "fail");
        assert!(!calls_in(&tmp).contains("config push"));
        // lox answers again (nothing in the project changed, so the check's
        // run stays the one without an answer): Apply's own run passes.
        std::fs::remove_file(tmp.path().join("garbletest")).unwrap();
        assert_eq!(check_of(&a, &d).await["test"]["available"], false);
        let (code, msg) = a.apply(&ApplyOpts::default()).await.unwrap_err();
        assert_eq!(code, StatusCode::BAD_GATEWAY, "{msg}");
        let steps = a.info()["progress"]["steps"].clone();
        assert_eq!(steps[2]["state"], "ok", "{steps}");
        assert_eq!(runs(), before + 2, "{}", calls_in(&tmp));
        // And it is kept for the browsers.
        assert_eq!(a.check_result()["test"]["passed"], 1);
    }

    #[test]
    fn only_a_lox_without_test_or_a_module_without_tests_skips_the_step() {
        let skip = |v: Value| tests_verdict(&v).map(|(s, _)| s);
        assert_eq!(
            skip(json!({ "available": false, "unsupported": true, "reason": "x" })),
            Ok("skip")
        );
        assert_eq!(
            skip(json!({ "available": true, "tests": [], "passed": 0, "failed": 0 })),
            Ok("skip")
        );
        for v in [
            json!({ "available": false, "reason": "lox logic test took longer than 120 s" }),
            json!({ "available": false, "reason": "cannot run lox: No such file or directory" }),
            json!({ "available": false, "reason": "lox logic test gave no answer vislox can read" }),
            json!({}),
            Value::Null,
        ] {
            let e = tests_verdict(&v).unwrap_err();
            assert!(e.starts_with("Tests couldn't run"), "{v}: {e}");
        }
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
        assert_eq!(steps[2]["state"], "skip", "{steps}");
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
        assert_eq!(e["set_claims"], json!([]), "{e}");
        // A claim on a block's switching times (#68), on the extern the lock
        // resolved by title: its line, its set's line and the block.
        std::fs::write(
            d.join("pages/zeit.loxl"),
            "extern temp = DayTimer(title: \"Uhr\")\nschedule abend\n\tweekdays 17:00-22:00\nend\ntemp.schedule = abend\n",
        )
        .unwrap();
        let e = edits_of(&a, &d).await;
        assert_eq!(
            e["set_claims"],
            json!([{ "kind": "schedule", "slug": "temp", "set": "abend", "file": "pages/zeit.loxl", "line": 5,
                     "set_file": "pages/zeit.loxl", "set_line": 2, "blocks": [AND] }]),
            "{e}"
        );
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

    /// A stand-in lox for adopting (#72): `logic adopt` declares the Divide
    /// block on the ground floor as lox would (refusing the meter), and
    /// `logic compile` copies the base, or renames the block when `../break`
    /// exists (a compile that changes the config).
    #[cfg(unix)]
    fn lox_that_adopts(tmp: &Path) {
        std::fs::write(
            tmp.join("lox"),
            r#"#!/bin/sh
echo "$*" >> ../calls.log
case "$1 $2" in
"logic adopt")
  u=$5; s=$7; m=$9; l=${11}
  [ -f ../delete-other ] && rm "$m/prog.c"
  { [ "$u" = "10000005-0000-0050-ffff504f94000001" ] || [ -f ../refuse-all ]; } && { echo "error: compile error: cannot adopt MeterAbsUni \"Zähler\": its outputs are hidden" >&2; exit 1; }
  printf 'page "Erdgeschoss"\n\nextern temperatur = VirtualIn(iname: "VI1")\n\n%s = Div("O1002", In2: 1000, In1: temperatur)\n' "$s" > "$m/erdgeschoss.loxl"
  printf '{"objects":{"%s":{"uuid":"%s","type":"Div","ports":{"AQ":"10000006-0000-0063-02ff504f94000001"},"page_uuid":"10000002-0000-0020-ffff504f94000001"}},"externals":{"temperatur":{"uuid":"10000001-0000-0010-ffff504f94000001","matched_by":"iname"}},"extern_wires":[]}\n' "$s" "$u" > "$l"
  echo "adopted uuid $u as \`$s\` on page \"Erdgeschoss\""
  ;;
"logic compile")
  out=out.Loxone; prev=
  for a in "$@"; do [ "$prev" = --out ] && out=$a; prev=$a; done
  if [ -f ../break ] && [ -d pages/vislox ]; then sed 's/Title="O1002"/Title="X"/' base.Loxone > "$out"; else cp base.Loxone "$out"; fi
  ;;
*) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#,
        )
        .unwrap();
    }

    /// Adopting with a stand-in lox (#72): what lox writes on the scratch copy
    /// moves into vislox's files and the lock, the config stays as it is, the
    /// undo hands it back, and a compile that changes the config, a block lox
    /// refuses, one vislox claims or a mixed request change nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_block_is_adopted_without_changing_the_config() {
        use axum::http::StatusCode;
        let (tmp, a) = project_for_edits();
        lox_that_adopts(tmp.path());
        let d = tmp.path().join("project");
        std::fs::write(d.join("pages/own.loxl"), "page \"Obergeschoss\"\n").unwrap();
        std::fs::write(d.join("logic.lock.json"), "{\"objects\":{}}\n").unwrap();
        std::fs::write(d.join("pages/prog.c"), "int x;\n").unwrap();
        sh(&d, "git add -A && git commit -qm module");
        let base = std::fs::read(d.join("base.Loxone")).unwrap();
        let adopt = |uuid: &str, slug: Option<&str>| edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![edit::Op::Adopt {
                blocks: vec![crate::adopt::Pick {
                    uuid: uuid.into(),
                    slug: slug.map(Into::into),
                }],
            }],
        };
        // lox refuses the meter: its message, and nothing written.
        let e = a
            .edit(
                &adopt("10000005-0000-0050-ffff504f94000001", None),
                deployed(&d),
            )
            .await
            .unwrap_err();
        assert_eq!(e.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            e.1.contains("lox cannot adopt Zähler: cannot adopt MeterAbsUni"),
            "{e:?}"
        );
        assert!(!d.join("pages/vislox").exists());
        // A slug that is not vislox's.
        let e = a
            .edit(&adopt(DIV, Some("teiler")), deployed(&d))
            .await
            .unwrap_err();
        assert!(e.1.contains("vx_"), "{e:?}");
        // lox deleting a file that is not a `.loxl` one: not vislox's to write.
        std::fs::write(tmp.path().join("delete-other"), "").unwrap();
        let e = a.edit(&adopt(DIV, None), deployed(&d)).await.unwrap_err();
        assert!(e.1.contains("a file of its own (prog.c)"), "{e:?}");
        assert!(d.join("pages/prog.c").exists() && !d.join("pages/vislox").exists());
        std::fs::remove_file(tmp.path().join("delete-other")).unwrap();
        // A compile that changes the config: refused, and everything put back.
        std::fs::write(tmp.path().join("break"), "").unwrap();
        let lock = std::fs::read(d.join("logic.lock.json")).unwrap();
        let e = a.edit(&adopt(DIV, None), deployed(&d)).await.unwrap_err();
        assert!(
            e.1.contains("does not rebuild O1002 exactly as the base has it"),
            "{e:?}"
        );
        assert!(!d.join("pages/vislox/erdgeschoss.loxl").exists());
        assert_eq!(std::fs::read(d.join("logic.lock.json")).unwrap(), lock);
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        std::fs::remove_file(tmp.path().join("break")).unwrap();
        // Adopted: its declaration with vislox's extern, its lock entry.
        let r = a.edit(&adopt(DIV, None), deployed(&d)).await.unwrap();
        assert_eq!(
            r["adopted"],
            json!([{ "uuid": DIV, "slug": "vx_o1002" }]),
            "{r}"
        );
        let file = std::fs::read_to_string(d.join("pages/vislox/erdgeschoss.loxl")).unwrap();
        assert!(
            file.contains(
                "vx_o1002 = Div(\"O1002\", In2: 1000, In1: vx_10000001_0000_0010_ffff504f94000001)"
            ),
            "{file}"
        );
        let claims = std::fs::read_to_string(d.join("pages/vislox/edits.loxl")).unwrap();
        assert!(
            claims.contains("extern vx_10000001_0000_0010_ffff504f94000001 = VirtualIn(uuid: "),
            "{claims}"
        );
        let l: Value =
            serde_json::from_slice(&std::fs::read(d.join("logic.lock.json")).unwrap()).unwrap();
        assert_eq!(l["objects"]["vx_o1002"]["uuid"], DIV, "{l}");
        assert!(
            l["externals"].get("temperatur").is_none(),
            "lox's extern stays on the scratch copy: {l}"
        );
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls
                .lines()
                .all(|l| l.starts_with("logic adopt") || l.starts_with("logic compile")),
            "{calls}"
        );
        assert!(
            calls
                .lines()
                .filter(|l| l.starts_with("logic adopt"))
                .all(|l| !l.contains(&d.join("logic.lock.json").display().to_string())),
            "lox adopts on a copy of the lock: {calls}"
        );
        // Not offered again; adopted, deployed.
        let e = edits_of(&a, &d).await;
        assert!(e["adopt"].get(DIV).is_none(), "{}", e["adopt"]);
        assert_eq!(
            e["adopt"]["10000004-0000-0040-ffff504f94000001"],
            Value::Null,
            "{}",
            e["adopt"]
        );
        let b = &e["blocks"][0];
        assert_eq!(b["hand_back"], Value::Null, "{b}");
        assert_eq!(
            (b["uuid"].as_str(), b["adopted"].as_bool()),
            (Some(DIV), Some(true)),
            "{e}"
        );
        let e = a.edit(&adopt(DIV, None), deployed(&d)).await.unwrap_err();
        assert!(e.1.contains("managed by the module already"), "{e:?}");
        // Remove is not how it goes back; the undo is.
        let rm = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![edit::Op::RemoveBlock { uuid: DIV.into() }],
        };
        let e = a.edit(&rm, deployed(&d)).await.unwrap_err();
        assert!(e.1.contains("Undo the adoption"), "{e:?}");
        // Adopt and other ops do not share a request.
        let mixed = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![
                edit::Op::Unadopt {
                    uuids: vec![DIV.into()],
                },
                edit::Op::Title {
                    uuid: DIV.into(),
                    value: Some("x".into()),
                },
            ],
        };
        assert_eq!(
            a.edit(&mixed, deployed(&d)).await.unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        let undo: Vec<edit::Op> = serde_json::from_value(r["undo"].clone()).unwrap();
        let back = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: undo,
                },
                deployed(&d),
            )
            .await
            .unwrap();
        assert_eq!(
            back["undo"],
            json!([{ "op": "adopt", "blocks": [{ "uuid": DIV, "slug": "vx_o1002" }] }]),
            "{back}"
        );
        assert!(!d.join("pages/vislox/erdgeschoss.loxl").exists());
        let l: Value =
            serde_json::from_slice(&std::fs::read(d.join("logic.lock.json")).unwrap()).unwrap();
        assert!(l["objects"].as_object().unwrap().is_empty(), "{l}");
        // A block vislox claims a value on is not adopted until that goes.
        a.edit(&title(blob(&d, "out.Loxone"), Some("Teiler")), deployed(&d))
            .await
            .unwrap();
        let e = a.edit(&adopt(DIV, None), deployed(&d)).await.unwrap_err();
        assert!(e.1.contains("revert them first, then adopt it"), "{e:?}");
        // Nor by its page: why, and nothing adopted when no block is left.
        let page = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![edit::Op::AdoptPage {
                page: "10000002-0000-0020-ffff504f94000001".into(),
            }],
        };
        std::fs::write(tmp.path().join("refuse-all"), "").unwrap();
        let e = a.edit(&page, deployed(&d)).await.unwrap_err();
        assert_eq!(e.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            e.1.contains("lox can adopt no block of this page")
                && e.1.contains("revert them first"),
            "{e:?}"
        );
    }

    /// A project of the fixture compiled by `lox` (the real one), with a
    /// module of its own on the upper floor.
    #[cfg(unix)]
    fn project_with_lox(lox: &Path) -> (tempfile::TempDir, Actions) {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/house.Loxone");
        project_with_lox_base(lox, &std::fs::read(&fixture).unwrap())
    }

    fn project_with_lox_base(lox: &Path, base: &[u8]) -> (tempfile::TempDir, Actions) {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("project");
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::write(d.join("base.Loxone"), base).unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\nserial = \"504F94000001\"\n",
        )
        .unwrap();
        std::fs::write(
            d.join("pages/own.loxl"),
            "# The module's own.\npage \"Obergeschoss\"\n",
        )
        .unwrap();
        sh(&d, &format!("{} logic compile >/dev/null", lox.display()));
        sh(
            &d,
            "git init -q && git config user.email t@t && git config user.name t \
             && git add -A && git commit -qm init",
        );
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d),
            lox: lox.to_path_buf(),
            probe: None,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
        });
        (tmp, a)
    }

    /// Adopting with the real lox (#72): set `VISLOX_TEST_LOX` to a lox
    /// binary (v0.83.0) to run it; it never talks to a Miniserver. The block
    /// and its neighbours' wires move into vislox's files, the pending config
    /// stays byte for byte the base, its name is then edited in its
    /// declaration, and the undo hands it back exactly.
    #[cfg(unix)]
    #[tokio::test]
    async fn adopting_with_lox_changes_nothing_the_miniserver_runs() {
        let Some(lox) = std::env::var_os("VISLOX_TEST_LOX") else {
            eprintln!("VISLOX_TEST_LOX is not set: skipped");
            return;
        };
        let (tmp, a) = project_with_lox(Path::new(&lox));
        let d = tmp.path().join("project");
        const GATE: &str = "10000004-0000-0040-ffff504f94000001";
        let lock_was = std::fs::read(d.join("logic.lock.json")).unwrap();
        let base = std::fs::read(d.join("base.Loxone")).unwrap();
        let r = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Adopt {
                        blocks: vec![crate::adopt::Pick {
                            uuid: GATE.into(),
                            slug: Some("vx_tor".into()),
                        }],
                    }],
                },
                deployed(&d),
            )
            .await
            .unwrap();
        assert_eq!(
            r["adopted"],
            json!([{ "uuid": GATE, "slug": "vx_tor" }]),
            "{r}"
        );
        assert_eq!(
            r["undo"],
            json!([{ "op": "unadopt", "uuids": [GATE] }]),
            "{r}"
        );
        let page = std::fs::read_to_string(d.join("pages/vislox/erdgeschoss.loxl")).unwrap();
        assert!(
            page.contains("vx_tor = And(\"O1001\", In1: vx_10000001_0000_0010_ffff504f94000001)"),
            "{page}"
        );
        assert!(
            page.contains("vx_10000005_0000_0050_ffff504f94000001.Power <- vx_tor"),
            "{page}"
        );
        let claims = std::fs::read_to_string(d.join("pages/vislox/edits.loxl")).unwrap();
        assert!(
            claims.contains("extern vx_10000009_0000_0090_ffff504f94000001 = Or(uuid:"),
            "{claims}"
        );
        assert_eq!(
            std::fs::read(d.join("out.Loxone")).unwrap(),
            base,
            "the config is unchanged"
        );
        let lock: Value =
            serde_json::from_slice(&std::fs::read(d.join("logic.lock.json")).unwrap()).unwrap();
        assert_eq!(lock["objects"]["vx_tor"]["uuid"], GATE);
        // Managed by vislox's file now: in `blocks`, adopted, not offered again.
        let e = edits_of(&a, &d).await;
        let b = e["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["uuid"] == GATE)
            .unwrap()
            .clone();
        assert_eq!(
            (b["adopted"].clone(), b["deployed"].clone()),
            (json!(true), json!(true)),
            "{b}"
        );
        assert!(e["adopt"].get(GATE).is_none(), "{}", e["adopt"]);
        assert!(
            e["adopt"]["10000006-0000-0060-ffff504f94000001"].is_null(),
            "{}",
            e["adopt"]
        );
        assert_eq!(
            e["adopted_wires"].as_array().unwrap().len(),
            3,
            "{}",
            e["adopted_wires"]
        );
        // Its name is edited in its declaration, like a block added here.
        let renamed = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Title {
                        uuid: GATE.into(),
                        value: Some("Tor".into()),
                    }],
                },
                deployed(&d),
            )
            .await
            .unwrap();
        assert!(
            std::fs::read_to_string(d.join("out.Loxone"))
                .unwrap()
                .contains("Title=\"Tor\"")
        );
        assert!(
            std::fs::read_to_string(d.join("pages/vislox/erdgeschoss.loxl"))
                .unwrap()
                .contains("vx_tor = And(\"Tor\""),
        );
        // Handing it back with its name edited is refused; after the undo, it goes.
        let refused = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Unadopt {
                        uuids: vec![GATE.into()],
                    }],
                },
                deployed(&d),
            )
            .await
            .unwrap_err();
        assert!(refused.1.contains("revert them first"), "{refused:?}");
        let undo: Vec<edit::Op> = serde_json::from_value(renamed["undo"].clone()).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: undo,
            },
            deployed(&d),
        )
        .await
        .unwrap();
        let undo: Vec<edit::Op> = serde_json::from_value(r["undo"].clone()).unwrap();
        let back = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: undo,
                },
                deployed(&d),
            )
            .await
            .unwrap();
        assert_eq!(back["undo"][0]["op"], "adopt", "{back}");
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        assert!(!d.join("pages/vislox/erdgeschoss.loxl").exists());
        let lock: Value =
            serde_json::from_slice(&std::fs::read(d.join("logic.lock.json")).unwrap()).unwrap();
        let was: Value = serde_json::from_slice(&lock_was).unwrap();
        assert_eq!(lock["objects"], was["objects"]);
        assert_eq!(lock["extern_wires"], was["extern_wires"]);
        // The whole page: what lox can rebuild, sources before what they feed.
        let page = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::AdoptPage {
                        page: "10000002-0000-0020-ffff504f94000001".into(),
                    }],
                },
                deployed(&d),
            )
            .await
            .unwrap();
        assert_eq!(page["adopted"].as_array().unwrap().len(), 3, "{page}");
        // The input reference stays Loxone Config's, with lox's reason.
        assert_eq!(page["skipped"][0]["title"], "Temperatur", "{page}");
        assert!(
            page["skipped"][0]["why"]
                .as_str()
                .unwrap()
                .starts_with("cannot adopt"),
            "{page}"
        );
        // The meter is fed by the gate: adopted after it, its wire bound in its declaration.
        let file = std::fs::read_to_string(d.join("pages/vislox/erdgeschoss.loxl")).unwrap();
        assert!(
            file.contains("vx_zaehler = MeterAbsUni(\"Zähler\", Power: vx_o1001)"),
            "{file}"
        );
        const METER: &str = "10000005-0000-0050-ffff504f94000001";
        let read = || std::fs::read_to_string(d.join("pages/vislox/erdgeschoss.loxl")).unwrap();
        let step = |ops: Vec<edit::Op>| {
            let base = blob(&d, "out.Loxone");
            let dep = deployed(&d);
            let a = &a;
            async move { a.edit(&edit::Request { base, ops }, dep).await }
        };
        // The meter handed back alone: the gate's wire into it stays, onto vislox's extern of it.
        let back = step(vec![edit::Op::Unadopt {
            uuids: vec![METER.into()],
        }])
        .await
        .unwrap();
        assert!(
            read().contains("vx_10000005_0000_0050_ffff504f94000001.Power <- vx_o1001"),
            "{}",
            read()
        );
        assert!(!read().contains("vx_zaehler"), "{}", read());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        // Its undo adopts it again under that extern: the wire goes back into its declaration.
        let undo: Vec<edit::Op> = serde_json::from_value(back["undo"].clone()).unwrap();
        step(undo).await.unwrap();
        assert!(
            read().contains("vx_zaehler = MeterAbsUni(\"Zähler\", Power: vx_o1001)"),
            "{}",
            read()
        );
        assert!(!read().contains(".Power <-"), "{}", read());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        // The gate handed back alone: the meter's declaration names it as vislox's extern.
        let back = step(vec![edit::Op::Unadopt {
            uuids: vec![GATE.into()],
        }])
        .await
        .unwrap();
        assert!(
            read().contains("Power: vx_10000004_0000_0040_ffff504f94000001)"),
            "{}",
            read()
        );
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        let undo: Vec<edit::Op> = serde_json::from_value(back["undo"].clone()).unwrap();
        step(undo).await.unwrap();
        assert!(
            read().contains("vx_zaehler = MeterAbsUni(\"Zähler\", Power: vx_o1001)"),
            "{}",
            read()
        );
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        let undo: Vec<edit::Op> = serde_json::from_value(page["undo"].clone()).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: undo,
            },
            deployed(&d),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), base);
        assert!(!d.join("pages/vislox/erdgeschoss.loxl").exists());
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
            "check", "upload", "restart", "online", "loaded", "verify", "commit", "scan", "push",
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

    /// [`project_with_fake_lox`] whose lox finds a user's mail address in a
    /// config with `Ema="…"` (#71), and a `live.Loxone` with one that no
    /// `.gitignore` keeps out of git.
    #[cfg(unix)]
    fn project_with_personal_data() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (tmp, lox) = project_with_fake_lox(true);
        let d = tmp.path().join("project");
        let finds = "  'logic scan')\n    shift 3; code=0; printf '{'; sep=\n    for f in \"$@\"; do printf '%s\"%s\": [' \"$sep\" \"$f\"; sep=,\n      if grep -q 'Ema=' \"$f\"; then printf '{\"rule\":\"mail\",\"object\":\"User #1\",\"attr\":\"Ema\",\"len\":17}'; code=1; fi\n      printf ']'; done; echo '}'; exit $code ;;\n  'config merge') echo '{}'; echo synced > base.Loxone ;;\n";
        let script = std::fs::read_to_string(&lox)
            .unwrap()
            .replace(CLEAN_SCAN, finds);
        std::fs::write(&lox, script).unwrap();
        std::fs::write(
            d.join("live.Loxone"),
            "<C Type=\"Document\">\n\t<C Type=\"User\" Title=\"alice\" Ema=\"alice@example.com\"/>\n</C>\n",
        )
        .unwrap();
        (tmp, d, lox)
    }

    fn actions_for(d: &Path, lox: PathBuf) -> Actions {
        Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d.to_path_buf()),
            lox,
            probe: None,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
        })
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn personal_data_stops_the_push_not_the_deploy_and_push_to_git_retries() {
        let (tmp, d, lox) = project_with_personal_data();
        let a = actions_for(&d, lox.clone());
        let remote = |t: &Path| git_out(t, &["--git-dir=remote.git", "log", "-1", "--format=%s"]);
        // Deployed and committed: the Miniserver runs it, the commit records it.
        let msg = a.apply(&ApplyOpts::default()).await.unwrap();
        assert!(msg.contains("Deployed and committed, but not pushed to git: lox logic scan found personal data in the commits to push: 1 finding in 1 file"), "{msg}");
        assert!(
            msg.contains("\n  live.Loxone:2: mail — User #1 Ema (17 chars)"),
            "{msg}"
        );
        assert!(msg.contains("git rm --cached -- live.Loxone"), "{msg}");
        assert!(msg.contains("only the git remote is behind"), "{msg}");
        assert_eq!(
            git_out(&d, &["log", "-1", "--format=%s"]),
            "Add a hallway light"
        );
        assert_eq!(remote(tmp.path()), "init");
        let info = a.info();
        let steps = step_states(&a);
        assert_eq!(
            [
                &steps["verify"],
                &steps["commit"],
                &steps["scan"],
                &steps["push"]
            ],
            ["ok", "ok", "fail", "skip"]
        );
        assert_eq!(info["last"]["ok"], true);
        assert_eq!(info["last"]["unpushed"], true);
        assert_eq!(info["push"]["state"], "found");
        assert_eq!(info["push"]["findings"][0]["file"], "live.Loxone");
        assert_eq!(info["push"]["findings"][0]["line"], 2);
        // Never the value: not in the message, the state or the steps.
        assert!(!(msg + &info.to_string()).contains("alice@example.com"));
        // Push to git scans again: still there, still not pushed.
        let (_, e) = a.push().await.unwrap_err();
        assert!(
            e.starts_with("Not pushed: lox logic scan found personal data"),
            "{e}"
        );
        assert_eq!(remote(tmp.path()), "init");
        // After a restart, the commits not pushed still offer it.
        let b = actions_for(&d, lox.clone());
        b.find_unpushed().await;
        assert_eq!(b.info()["push"]["state"], "behind");
        assert_eq!(
            b.info()["push"]["message"],
            "1 commit not pushed to git yet"
        );
        // Kept out of git as the advice says: pushed.
        sh(
            &d,
            "git rm -q --cached -- live.Loxone && echo /live.Loxone > .gitignore \
                && git add .gitignore && git commit -q --amend --no-edit",
        );
        assert_eq!(a.push().await.unwrap(), "Pushed to git.");
        assert_eq!(remote(tmp.path()), "Add a hallway light");
        // Apply's steps say so now.
        let steps = step_states(&a);
        assert_eq!([&steps["scan"], &steps["push"]], ["ok", "ok"]);
        assert_eq!(
            git_out(
                tmp.path(),
                &["--git-dir=remote.git", "ls-tree", "--name-only", "HEAD"]
            ),
            ".gitignore\nbase.Loxone\nlox.toml\nout.Loxone"
        );
        assert!(a.info()["push"].is_null());
        assert_eq!(
            a.push().await.unwrap(),
            "Nothing to push: the upstream has every commit."
        );
    }

    fn branch(d: &Path) -> String {
        git_out(d, &["symbolic-ref", "--short", "HEAD"])
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn only_the_scanned_commit_goes_to_the_upstream_branch() {
        let (tmp, lox) = project_with_fake_lox(true);
        let d = tmp.path().join("project");
        let a = actions_for(&d, lox.clone());
        let main = branch(&d);
        // git would push every branch and the tags along with these.
        sh(
            &d,
            "git config push.default matching && git config push.followTags true \
             && git config remote.origin.push 'refs/heads/*:refs/heads/*' \
             && git checkout -q -b other && echo o > o.txt && git add -A && git commit -qm other",
        );
        sh(&d, &format!("git checkout -q {main}"));
        std::fs::write(d.join("clean.Loxone"), "<C Type=\"Document\"/>\n").unwrap();
        sh(
            &d,
            "git add -A && git commit -qm clean && git tag -a v1 -m v1",
        );
        // The scan commits once more while lox reads (as someone in a terminal could).
        let late = format!(
            "  'logic scan')\n    [ -e {late} ] || {{ touch {late}; git -C {d} commit -q --allow-empty -m late; }}\n",
            late = tmp.path().join("late").display(),
            d = d.display()
        );
        let script = std::fs::read_to_string(&lox)
            .unwrap()
            .replace("  'logic scan')\n", &late);
        std::fs::write(&lox, script).unwrap();
        let scanned = git_out(&d, &["rev-parse", "HEAD"]);
        assert_eq!(a.push().await.unwrap(), "Pushed to git.");
        let remote = |args: &[&str]| {
            let mut v = vec!["--git-dir=remote.git"];
            v.extend(args);
            git_out(tmp.path(), &v)
        };
        assert_eq!(
            remote(&["for-each-ref", "--format=%(refname)"]),
            format!("refs/heads/{main}")
        );
        assert_eq!(remote(&["rev-parse", &main]), scanned);
        assert_eq!(git_out(&d, &["log", "-1", "--format=%s"]), "late");
        // The late commit is left for the next push.
        assert_eq!(a.info()["push"]["state"], "behind");
        assert_eq!(
            a.info()["push"]["message"],
            "1 commit made during the push not pushed to git yet"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn what_another_remote_has_is_not_on_the_upstream() {
        let (tmp, lox) = project_with_fake_lox(true);
        let d = tmp.path().join("project");
        let a = actions_for(&d, lox);
        let main = branch(&d);
        sh(tmp.path(), "git init -q --bare backup.git");
        sh(
            &d,
            &format!(
                "echo x > x.txt && git add -A && git commit -qm x && git remote add backup ../backup.git \
                 && git push -q backup {main}"
            ),
        );
        a.find_unpushed().await;
        assert_eq!(
            a.info()["push"]["message"],
            "1 commit not pushed to git yet"
        );
        // Not "nothing to push": the upstream lacks it, whatever backup has.
        assert_eq!(a.push().await.unwrap(), "Pushed to git.");
        assert_eq!(
            git_out(
                tmp.path(),
                &["--git-dir=remote.git", "log", "-1", "--format=%s"]
            ),
            "x"
        );
        assert!(a.info()["push"].is_null());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_push_from_a_terminal_clears_the_blocked_state() {
        let (tmp, d, lox) = project_with_personal_data();
        let a = actions_for(&d, lox);
        a.apply(&ApplyOpts::default()).await.unwrap();
        assert_eq!(a.info()["push"]["state"], "found");
        assert_eq!(a.info()["push"]["total"], 1);
        sh(&d, "git push -q");
        a.recheck_push().await;
        assert!(a.info()["push"].is_null());
        let steps = step_states(&a);
        assert_eq!(steps["push"], "ok");
        let _ = tmp;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_findings_kept_are_capped() {
        let (tmp, d, lox) = project_with_personal_data();
        let a = actions_for(&d, lox);
        for i in 0..59 {
            std::fs::write(
                d.join(format!("p{i}.Loxone")),
                format!("<C Type=\"User\" Ema=\"x{i}\"/>\n"),
            )
            .unwrap();
        }
        sh(&d, "git add -A && git commit -qm many");
        let (_, e) = a.push().await.unwrap_err();
        assert!(
            e.contains("60 findings in 60 files") && e.contains("… and 40 more"),
            "{e}"
        );
        let info = a.info();
        assert_eq!(info["push"]["total"], 60);
        assert_eq!(
            info["push"]["findings"].as_array().unwrap().len(),
            SHOWN_FINDINGS
        );
        let _ = tmp;
    }

    #[tokio::test]
    async fn many_objects_are_read_without_a_stall() {
        let tmp = tempfile::tempdir().unwrap();
        sh(
            tmp.path(),
            "git init -q && printf '%0200d' 0 > f && git add f",
        );
        let id = git_out(tmp.path(), &["rev-parse", ":f"]);
        // More ids than a pipe holds, and far more answer than that.
        let specs = vec![id; 5000];
        let got = tokio::time::timeout(Duration::from_secs(30), git_objects(tmp.path(), &specs))
            .await
            .expect("no stall")
            .unwrap();
        assert_eq!(got.len(), 5000);
        assert!(
            got.iter()
                .all(|o| o.as_ref().is_some_and(|b| b.len() == 200))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sync_and_a_lox_without_scan_never_push_unchecked() {
        let (tmp, d, lox) = project_with_personal_data();
        let a = actions_for(&d, lox.clone());
        let remote = || {
            git_out(
                tmp.path(),
                &["--git-dir=remote.git", "log", "-1", "--format=%s"],
            )
        };
        // A commit of the stray config made by hand, not pushed: Sync's push would send it too.
        sh(&d, "git add live.Loxone && git commit -qm stray");
        let msg = a.sync().await.unwrap();
        assert_eq!(
            git_out(&d, &["log", "-1", "--format=%s"]),
            "Sync with the Miniserver's running config"
        );
        assert!(
            msg.contains("Committed, but not pushed to git: lox logic scan found personal data"),
            "{msg}"
        );
        assert_eq!(remote(), "init");
        assert_eq!(a.info()["last"]["unpushed"], true);
        // A lox without scan: vislox cannot tell, and does not push.
        let script = std::fs::read_to_string(&lox).unwrap().replace(
            "  'logic scan')",
            "  'logic scan') echo 'unknown command `scan`. Run `lox logic help`.' >&2; exit 2 ;;\n  'logic scan-old')",
        );
        std::fs::write(&lox, script).unwrap();
        let (_, e) = a.push().await.unwrap_err();
        assert!(
            e.contains("vislox could not check the push for personal data: this lox"),
            "{e}"
        );
        assert!(
            e.contains("has no `lox logic scan`")
                && e.contains("vislox never pushes what it could not check"),
            "{e}"
        );
        assert_eq!(a.info()["push"]["state"], "unscanned");
        assert_eq!(remote(), "init");
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
    const DOC_A: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ControlList Version=\"1\" NextObj=\"100\">\n<C Type=\"Document\" V=\"178\" U=\"00000000-0000-0001-ffff504f94000001\" Title=\"Haus\">\n</C>\n</ControlList>\n";

    fn doc_b() -> String {
        DOC_A.replace("Haus", "Haus B")
    }

    pub(super) fn zip_of(xml: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("sps0.LoxCC", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(&loxone_config::archive::compress_loxcc(xml).unwrap())
            .unwrap();
        w.finish().unwrap().into_inner()
    }

    /// A stand-in `lox` for Roll back, in `t`. Files in `t` steer it:
    /// - `logic compile --base … --out …` (the plan's scratch compile) copies
    ///   the base (`compile-differs`: changes it); `logic compile` in the
    ///   project copies lox.toml's base to out (`inplace-differs`: changes it);
    /// - `config push` without `--force` downloads what runs (the Apply's
    ///   config) as its pre-push, refuses with `push-fails`, else stages
    ///   lox.toml's base (`stage-differs`: changed) and says where; arguments
    ///   in `stage-args`;
    /// - `config push --force` (arguments in `push-args`) fails before its
    ///   upload line with `force-fails-early`, after it with `upload-fails`,
    ///   else "deploys" lox.toml's base:
    ///   out, a post-push download, lox.toml's base pointed at it;
    /// - `config download --save-as` copies `runs.zip` (`download-fails`:
    ///   fails).
    #[cfg(unix)]
    fn rollback_lox(t: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let lox = t.join("lox");
        let t_s = t.display();
        std::fs::write(
            &lox,
            format!(
                r#"#!/bin/sh
base() {{ sed -n 's/^base *= *"\(.*\)"/\1/p' lox.toml; }}
case "$1 $2" in
{CLEAN_SCAN}  'logic test')
    if [ -f '{t_s}/test-fails' ]; then
      echo 'test "Licht" ... FAILED'; echo '    step 1: expect licht.Q == 1 — actual 0'
      echo '1 tests: 0 passed, 1 failed'; exit 1
    fi
    echo 'no tests in pages' ;;
  'logic compile')
    if [ $# -eq 2 ]; then
      if [ -f '{t_s}/inplace-differs' ]; then sed 's/Haus/Halle/' "$(base)" > out.Loxone; else cp "$(base)" out.Loxone; fi
      exit 0
    fi
    shift 2
    while [ $# -gt 0 ]; do case "$1" in --base) b=$2; shift;; --out) o=$2; shift;; esac; shift; done
    if [ -f '{t_s}/compile-differs' ]; then sed 's/Haus/Halle/' "$b" > "$o"; else cp "$b" "$o"; fi ;;
  'config push')
    case "$3" in
      --force)
        echo "$@" > '{t_s}/push-args'
        echo 'Downloading the live config...' >&2
        cp backups/20261107-100000-post-push.Loxone backups/20991231-235959-pre-push.Loxone
        if [ -f '{t_s}/force-fails-early' ]; then echo 'Error: The Miniserver holds custom changes.' >&2; exit 1; fi
        echo 'Uploading as sps_new.zip...' >&2
        if [ -f '{t_s}/upload-fails' ]; then echo 'The Miniserver did not load the upload within a minute.' >&2; exit 1; fi
        echo 'Restarting the program...' >&2
        echo 'Downloading the config again to verify...' >&2
        cp "$(base)" out.Loxone
        cp "$(base)" backups/20991231-235959-post-push.Loxone
        sed 's|^base *=.*|base = "backups/20991231-235959-post-push.Loxone"|' lox.toml > lox.toml.new && mv lox.toml.new lox.toml
        echo 'Pushed and verified.' >&2 ;;
      *)
        echo "$@" > '{t_s}/stage-args'
        echo 'Downloading the live config...' >&2
        cp backups/20261107-100000-post-push.Loxone backups/20991231-235958-pre-push.Loxone
        if [ -f '{t_s}/push-fails' ]; then echo 'The live config is no longer the download the patched base was built from.' >&2; exit 1; fi
        if [ -f '{t_s}/stage-differs' ]; then sed 's/Haus/Halle/' "$(base)" > backups/20991231-235958-push.Loxone; else cp "$(base)" backups/20991231-235958-push.Loxone; fi
        echo 'Staged backups/20991231-235958-push.zip (extracts to exactly backups/20991231-235958-push.Loxone). Run again with --force to upload it and restart the program.' >&2 ;;
    esac ;;
  'config download')
    if [ -f '{t_s}/download-fails' ]; then echo 'Error: FTP: connection refused' >&2; exit 1; fi
    cp '{t_s}/runs.zip' "$4" ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
        lox
    }

    fn rollback_actions(d: &Path, lox: PathBuf) -> Actions {
        Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d.to_path_buf()),
            lox,
            probe: None,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
        })
    }

    /// A project after two deploys: `init` (base A, module v1 and an ignored
    /// file git tracks, `pages/keep.local`; lock1) and an Apply (base B,
    /// module v2 with a second file, keep.local untracked; lock2), whose push
    /// left its downloads in `backups/` (pre-push A, post-push B). Its lox is
    /// [`rollback_lox`]; the Miniserver runs A (`runs.zip`). `nested`: the
    /// project is a directory below the repository's top.
    #[cfg(unix)]
    fn project_after_an_apply(nested: bool) -> (tempfile::TempDir, PathBuf, Actions) {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path();
        let repo = if nested {
            t.join("repo")
        } else {
            t.join("project")
        };
        let d = if nested {
            repo.join("home")
        } else {
            repo.clone()
        };
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::create_dir_all(d.join("backups")).unwrap();
        std::fs::write(d.join("backups/.gitignore"), "*\n").unwrap();
        std::fs::write(d.join(".gitignore"), "pages/*.local\n").unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\nlock = \"logic.lock.json\"\n",
        )
        .unwrap();
        std::fs::write(d.join("base.Loxone"), DOC_A).unwrap();
        std::fs::write(d.join("out.Loxone"), DOC_A).unwrap();
        std::fs::write(d.join("pages/main.loxl"), "v1\n").unwrap();
        std::fs::write(d.join("pages/keep.local"), "kept\n").unwrap();
        std::fs::write(d.join("logic.lock.json"), "lock1\n").unwrap();
        sh(t, "git init -q --bare remote.git");
        sh(&repo, "git init -q");
        sh(
            &d,
            &format!(
                "git config user.email t@t && git config user.name t \
                 && git add -A && git add -f pages/keep.local && git commit -qm init \
                 && git remote add origin {} && git push -q -u origin HEAD",
                t.join("remote.git").display()
            ),
        );
        std::fs::write(d.join("base.Loxone"), doc_b()).unwrap();
        std::fs::write(d.join("out.Loxone"), doc_b()).unwrap();
        std::fs::write(d.join("pages/main.loxl"), "v2\n").unwrap();
        std::fs::write(d.join("pages/extra.loxl"), "new\n").unwrap();
        std::fs::write(d.join("logic.lock.json"), "lock2\n").unwrap();
        sh(
            &d,
            "git rm -q pages/keep.local && git add -A && git commit -qm 'Rename Haus' && git push -q",
        );
        let b = d.join("backups");
        std::fs::write(b.join("20261107-100000-pre-push.Loxone"), DOC_A).unwrap();
        std::fs::write(
            b.join("20261107-100000-pre-push.zip"),
            zip_of(DOC_A.as_bytes()),
        )
        .unwrap();
        std::fs::write(b.join("20261107-100000-post-push.Loxone"), doc_b()).unwrap();
        std::fs::write(t.join("runs.zip"), zip_of(DOC_A.as_bytes())).unwrap();
        let a = rollback_actions(&d, rollback_lox(t));
        (tmp, d, a)
    }

    fn git_out(d: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(d)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    fn step_states(a: &Actions) -> std::collections::BTreeMap<String, String> {
        a.info()["progress"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["id"].as_str().unwrap().to_string(),
                    s["state"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// The project as the Apply left it: committed, nothing written.
    fn as_the_apply_left_it(d: &Path, head: &str) {
        assert_eq!(git_out(d, &["status", "--porcelain"]), "");
        assert_eq!(git_out(d, &["rev-parse", "HEAD"]), head);
        assert_eq!(read(&d.join("pages/main.loxl")), "v2\n");
        assert_eq!(read(&d.join("base.Loxone")), doc_b());
        assert!(read(&d.join("lox.toml")).contains("base = \"base.Loxone\""));
        assert!(
            !d.join("backups/20261107-100000-rollback-base.Loxone")
                .exists()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rollback_deploys_the_download_from_before_the_apply_the_way_apply_does() {
        rollback_deploys(false).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rollback_works_in_a_project_below_the_repository_top() {
        rollback_deploys(true).await;
    }

    #[cfg(unix)]
    async fn rollback_deploys(nested: bool) {
        let (tmp, d, a) = project_after_an_apply(nested);
        let t = tmp.path();
        let head = git_out(&d, &["rev-parse", "HEAD"]);
        // History offers it on the deployed commit only, once per push.
        let rev = |id: &str, doc: &str| crate::history::Revision {
            id: id.into(),
            kind: "git",
            blob: crate::history::blob_id(doc.as_bytes()),
            ..Default::default()
        };
        let hist = vec![
            rev(&format!("git:{head}"), &doc_b()),
            rev("git:older", DOC_A),
            rev("git:same-bytes", &doc_b()),
        ];
        let r = a.rollbacks(&hist);
        assert_eq!(r[format!("git:{head}")]["available"], true, "{r}");
        assert_eq!(
            r[format!("git:{head}")]["backup"],
            "backups/20261107-100000-pre-push.zip"
        );
        assert!(
            r.get("git:same-bytes").is_none() && r.get("git:older").is_none(),
            "{r}"
        );
        // An ignored file in the module, which no git command would bring back.
        std::fs::write(d.join("pages/notes.local"), "mine\n").unwrap();
        // The plan changes nothing.
        let plan = a.rollback_plan(&head).await.unwrap();
        assert_eq!(plan.compiled, DOC_A.as_bytes());
        assert!(plan.same_bytes);
        assert_eq!(plan.subject, "Rename Haus");
        assert_eq!(git_out(&d, &["status", "--porcelain"]), "");
        let msg = a
            .rollback(&head, Some(&plan.blob()), &ApplyOpts::default())
            .await
            .unwrap();
        assert!(
            msg.starts_with(&format!(
                "Rolled back {} to backups/20261107-100000-pre-push.zip.\nThe download lox verified is the program you reviewed.",
                &head[..10]
            )),
            "{msg}"
        );
        // lox staged first, then pushed, onto the download the Apply deployed.
        let post = d.join("backups/20261107-100000-post-push.Loxone");
        assert_eq!(
            read(&t.join("stage-args")).trim(),
            format!("config push --patched-from {}", post.display())
        );
        assert_eq!(
            read(&t.join("push-args")).trim(),
            format!("config push --force --patched-from {}", post.display())
        );
        // The module and lockfile are back to before the Apply, the base is what ran before.
        assert_eq!(read(&d.join("pages/main.loxl")), "v1\n");
        assert!(!d.join("pages/extra.loxl").exists());
        assert_eq!(read(&d.join("pages/keep.local")), "kept\n");
        assert_eq!(read(&d.join("logic.lock.json")), "lock1\n");
        assert_eq!(read(&d.join("base.Loxone")), DOC_A);
        assert!(read(&d.join("lox.toml")).contains("base   = \"base.Loxone\""));
        // The ignored file is still there, and not committed.
        assert_eq!(read(&d.join("pages/notes.local")), "mine\n");
        // Committed (the tracked ignored file too) and pushed.
        assert_eq!(git_out(&d, &["status", "--porcelain"]), "");
        assert_eq!(
            git_out(&d, &["ls-files", "pages"]),
            "pages/keep.local\npages/main.loxl"
        );
        assert_eq!(
            git_out(&d, &["log", "-1", "--format=%s"]),
            "Roll back: Rename Haus"
        );
        assert!(
            git_out(&d, &["log", "-1", "--format=%b"])
                .contains("Restored backups/20261107-100000-pre-push.zip")
        );
        assert_eq!(
            git_out(t, &["--git-dir=remote.git", "log", "-1", "--format=%s"]),
            "Roll back: Rename Haus"
        );
        let info = a.info();
        assert_eq!(info["last"]["kind"], "rollback");
        assert_eq!(
            info["last"]["deployed"]["before"],
            crate::history::blob_id(doc_b().as_bytes())
        );
        assert_eq!(
            info["last"]["deployed"]["after"],
            crate::history::blob_id(DOC_A.as_bytes())
        );
        let steps = step_states(&a);
        for id in [
            "backup", "module", "stage", "check", "upload", "restart", "verify", "commit", "scan",
            "push",
        ] {
            assert_eq!(steps[id], "ok", "{id}: {steps:?}");
        }
        // The module has no tests.
        assert_eq!(steps["tests"], "skip");
        // The rollback is a push too: rolling it back would deploy the Apply again.
        let head2 = git_out(&d, &["rev-parse", "HEAD"]);
        let again = a.rollback_plan(&head2).await.unwrap();
        assert_eq!(again.compiled, doc_b().as_bytes());
        // The Apply is no longer the newest deploy.
        let old = a.rollback_plan(&head).await.err().unwrap();
        assert!(
            old.starts_with("Only the newest deploy can be rolled back"),
            "{old}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rollback_refuses_on_any_doubt_and_changes_nothing() {
        let (tmp, d, a) = project_after_an_apply(false);
        let t = tmp.path();
        let head = git_out(&d, &["rev-parse", "HEAD"]);
        let refused = |e: Result<String, Refusal>| e.err().unwrap();
        let none = ApplyOpts::default();
        let go = || a.rollback(&head, None, &none);
        let b = d.join("backups");
        // A request that is wrong in itself: refused before anything starts.
        let bad = ApplyOpts {
            accept_version: Some("16.x".into()),
            ..Default::default()
        };
        let (status, e) = refused(a.rollback(&head, None, &bad).await);
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert!(e.contains("not a Loxone Config version"), "{e}");
        assert!(a.info()["progress"].is_null());
        // Changes nobody committed.
        std::fs::write(d.join("pages/main.loxl"), "v3\n").unwrap();
        let e = refused(go().await).1;
        assert!(e.contains("not committed"), "{e}");
        assert_eq!(read(&d.join("pages/main.loxl")), "v3\n");
        sh(&d, "git checkout -q -- pages");
        // An ignored file where the module from before has one: never overwritten.
        std::fs::write(d.join("pages/keep.local"), "someone's\n").unwrap();
        let e = refused(go().await).1;
        assert!(e.starts_with("pages/keep.local is in the way"), "{e}");
        assert_eq!(read(&d.join("pages/keep.local")), "someone's\n");
        std::fs::remove_file(d.join("pages/keep.local")).unwrap();
        // The download from before is gone.
        let zip = std::fs::read(b.join("20261107-100000-pre-push.zip")).unwrap();
        std::fs::remove_file(b.join("20261107-100000-pre-push.zip")).unwrap();
        std::fs::remove_file(b.join("20261107-100000-pre-push.Loxone")).unwrap();
        let e = refused(go().await).1;
        assert!(e.contains("is gone"), "{e}");
        // Two different programs.
        std::fs::write(b.join("20261107-100000-pre-push.zip"), &zip).unwrap();
        std::fs::write(b.join("20261107-100000-pre-push.Loxone"), doc_b()).unwrap();
        let e = refused(go().await).1;
        assert!(e.contains("hold different programs"), "{e}");
        std::fs::write(b.join("20261107-100000-pre-push.Loxone"), DOC_A).unwrap();
        // The module from before does not give the download back.
        std::fs::write(t.join("compile-differs"), "").unwrap();
        let e = refused(go().await).1;
        assert!(
            e.contains("does not compile back to what the Miniserver ran then"),
            "{e}"
        );
        assert!(e.contains("Halle"), "{e}");
        std::fs::remove_file(t.join("compile-differs")).unwrap();
        // Not a push vislox knows of.
        std::fs::rename(
            b.join("20261107-100000-post-push.Loxone"),
            b.join("post.bak"),
        )
        .unwrap();
        let e = refused(go().await).1;
        assert!(e.contains("is not a verified push"), "{e}");
        std::fs::rename(
            b.join("post.bak"),
            b.join("20261107-100000-post-push.Loxone"),
        )
        .unwrap();
        // Not what was reviewed: the plan changed since.
        let e = refused(
            a.rollback(&head, Some("0123456789abcdef"), &ApplyOpts::default())
                .await,
        )
        .1;
        assert!(
            e.starts_with("What a Roll back deploys changed since you reviewed it"),
            "{e}"
        );
        // A test of the module from before fails (#67): stopped there.
        std::fs::write(t.join("test-fails"), "").unwrap();
        let e = refused(go().await).1;
        assert!(e.contains("1 of 1 test fails"), "{e}");
        as_the_apply_left_it(&d, &head);
        std::fs::remove_file(t.join("test-fails")).unwrap();
        assert!(!t.join("stage-args").exists() && !t.join("push-args").exists());
        // lox's drift check refuses (the Miniserver no longer runs the Apply's config).
        std::fs::write(t.join("push-fails"), "").unwrap();
        let e = refused(go().await).1;
        assert!(e.contains("no longer the download"), "{e}");
        assert!(e.contains("nothing uploaded"), "{e}");
        as_the_apply_left_it(&d, &head);
        assert!(!t.join("push-args").exists(), "never pushed with --force");
        let steps = step_states(&a);
        assert_eq!(steps["stage"], "fail", "{steps:?}");
        assert_eq!(a.info()["last"]["ok"], false);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rollback_uploads_only_the_program_that_was_reviewed() {
        let (tmp, d, a) = project_after_an_apply(false);
        let t = tmp.path();
        let head = git_out(&d, &["rev-parse", "HEAD"]);
        let plan = a.rollback_plan(&head).await.unwrap();
        // Compiled in the project, with its own settings, it gives something else.
        std::fs::write(t.join("inplace-differs"), "").unwrap();
        let (_, e) = a
            .rollback(&head, Some(&plan.blob()), &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(
            e.starts_with("Compiled in the project, the module from before gives another program"),
            "{e}"
        );
        assert_eq!(step_states(&a)["module"], "fail");
        as_the_apply_left_it(&d, &head);
        assert!(!t.join("stage-args").exists() && !t.join("push-args").exists());
        std::fs::remove_file(t.join("inplace-differs")).unwrap();
        // lox stages something else.
        std::fs::write(t.join("stage-differs"), "").unwrap();
        let (_, e) = a
            .rollback(&head, Some(&plan.blob()), &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(
            e.starts_with("lox staged another program than the one you reviewed"),
            "{e}"
        );
        assert_eq!(step_states(&a)["stage"], "fail");
        as_the_apply_left_it(&d, &head);
        assert!(!t.join("push-args").exists(), "never pushed with --force");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_push_that_fails_after_its_upload_is_recorded_by_what_the_miniserver_runs() {
        let (tmp, d, a) = project_after_an_apply(false);
        let t = tmp.path();
        let head = git_out(&d, &["rev-parse", "HEAD"]);
        // Before its upload line: nothing was sent, the project is put back.
        std::fs::write(t.join("force-fails-early"), "").unwrap();
        let (_, e) = a
            .rollback(&head, None, &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(
            e.contains("lox stopped before the upload: nothing changed on the Miniserver"),
            "{e}"
        );
        as_the_apply_left_it(&d, &head);
        assert_eq!(step_states(&a)["check"], "fail");
        std::fs::remove_file(t.join("force-fails-early")).unwrap();
        std::fs::write(t.join("upload-fails"), "").unwrap();
        // vislox cannot tell what runs: it says so, and leaves the project at its commit.
        std::fs::write(t.join("download-fails"), "").unwrap();
        let (_, e) = a
            .rollback(&head, None, &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(e.contains("did not load the upload"), "{e}");
        assert!(e.contains("cannot tell which config the Miniserver runs now (lox config download: Error: FTP: connection refused)"), "{e}");
        assert!(
            e.contains("It may run the rolled-back config or the Apply's"),
            "{e}"
        );
        as_the_apply_left_it(&d, &head);
        // It runs a third program: the same.
        std::fs::remove_file(t.join("download-fails")).unwrap();
        std::fs::write(
            t.join("runs.zip"),
            zip_of(DOC_A.replace("Haus", "Werkstatt").as_bytes()),
        )
        .unwrap();
        let (_, e) = a
            .rollback(&head, None, &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(
            e.contains("neither the Apply's nor the rolled-back one"),
            "{e}"
        );
        as_the_apply_left_it(&d, &head);
        // It runs the Apply's, but whether an upload waits cannot be checked here: the same.
        std::fs::write(t.join("runs.zip"), zip_of(doc_b().as_bytes())).unwrap();
        let (_, e) = a
            .rollback(&head, None, &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(
            e.contains("whether an upload waits cannot be checked"),
            "{e}"
        );
        as_the_apply_left_it(&d, &head);
        // It runs the Apply's, and nothing waits: nothing changed, put back.
        *a.waiting.lock().unwrap() = Some(Ok(None));
        let (_, e) = a
            .rollback(&head, None, &ApplyOpts::default())
            .await
            .err()
            .unwrap();
        assert!(
            e.contains("The Miniserver still runs the Apply's config, and no upload is waiting"),
            "{e}"
        );
        as_the_apply_left_it(&d, &head);
        *a.waiting.lock().unwrap() = None;
        // It runs the rolled-back program: recorded as deployed, not verified.
        std::fs::write(t.join("runs.zip"), zip_of(DOC_A.as_bytes())).unwrap();
        let msg = a
            .rollback(&head, None, &ApplyOpts::default())
            .await
            .unwrap();
        assert!(msg.contains("Not verified: lox failed after the upload, but the Miniserver runs the rolled-back program"), "{msg}");
        assert_eq!(a.info()["last"]["warn"], true);
        assert_eq!(git_out(&d, &["status", "--porcelain"]), "");
        assert_eq!(
            git_out(&d, &["log", "-1", "--format=%s"]),
            "Roll back: Rename Haus"
        );
        assert_eq!(read(&d.join("base.Loxone")), DOC_A);
        assert_eq!(read(&d.join("out.Loxone")), DOC_A);
        assert_eq!(read(&d.join("pages/main.loxl")), "v1\n");
        assert!(read(&d.join("lox.toml")).contains("base   = \"base.Loxone\""));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_first_commit_has_nothing_to_roll_back_to() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path();
        let d = t.join("project");
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::create_dir_all(d.join("backups")).unwrap();
        std::fs::write(d.join("backups/.gitignore"), "*\n").unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\n",
        )
        .unwrap();
        std::fs::write(d.join("base.Loxone"), doc_b()).unwrap();
        std::fs::write(d.join("out.Loxone"), doc_b()).unwrap();
        std::fs::write(d.join("pages/main.loxl"), "v1\n").unwrap();
        sh(
            &d,
            "git init -q && git config user.email t@t && git config user.name t \
             && git add -A && git commit -qm 'First push'",
        );
        // Its base is a push's download, with the download from before kept.
        std::fs::write(d.join("backups/20261107-100000-post-push.Loxone"), doc_b()).unwrap();
        std::fs::write(d.join("backups/20261107-100000-pre-push.Loxone"), DOC_A).unwrap();
        let a = rollback_actions(&d, rollback_lox(t));
        let head = git_out(&d, &["rev-parse", "HEAD"]);
        let e = a.rollback_plan(&head).await.err().unwrap();
        assert!(
            e.ends_with("is the first commit: there is nothing before it to go back to"),
            "{e}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_plan_waits_for_the_compile_lock_and_refuses_during_an_action() {
        let (_tmp, d, a) = project_after_an_apply(false);
        let head = git_out(&d, &["rev-parse", "HEAD"]);
        // An action runs (Apply, Discard, a Roll back): refused at once.
        {
            let _busy = a.busy.lock().await;
            a.st.lock().unwrap().running = Some("Apply");
            let e = a.rollback_plan(&head).await.err().unwrap();
            assert_eq!(e, "Apply is running");
            a.st.lock().unwrap().running = None;
        }
        // An edit or Compile holds the compile lock: the plan waits for it.
        let held = a.compile_lock.lock().await;
        let plan = a.rollback_plan(&head);
        tokio::pin!(plan);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut plan)
                .await
                .is_err(),
            "the plan did not wait"
        );
        drop(held);
        assert!(plan.await.is_ok());
    }

    const UHR: &str = "10000020-0000-0200-ffff504f94000001";

    /// The fixture with an analog Daytimer on the upper floor, on from 06:30
    /// to 08:00 on Mondays.
    fn house_with_timer() -> String {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/house.Loxone");
        let mut s = std::fs::read_to_string(fixture).unwrap();
        let timer = format!(
            "\t\t\t<C Type=\"DayTimer\" V=\"178\" U=\"{UHR}\" Title=\"Uhr\" Px=\"960\" Py=\"4800\" \
             Px2=\"3648\" Py2=\"5600\" Nio=\"2\" WF=\"16384\" Analog=\"true\" DefValue=\"18\" \
             Modes=\"00000000-0000-0004-1500000000000000\"><Co K=\"Mode\" U=\"10000020-0000-0201-00ff504f94000001\"/>\
             <Co K=\"AQ\" U=\"10000020-0000-0202-01ff504f94000001\"/><Entry From=\"390\" To=\"480\" V=\"21\"/></C>\n"
        );
        let end = "\n\t\t</C>\n\t</C>\n</ControlList>";
        let at = s.rfind(end).unwrap();
        s.insert_str(at + 1, &timer);
        s
    }

    fn mon(spans: &[(u32, u32, &str)]) -> Option<crate::sched_edit::Sched> {
        Some(crate::sched_edit::Sched {
            set: None,
            modes: Some(std::collections::BTreeMap::from([(
                crate::sched_edit::builtin_uuid(4),
                spans
                    .iter()
                    .map(|(from, to, v)| crate::sched_edit::Span {
                        from: *from,
                        to: *to,
                        value: Some(v.to_string()),
                        activate: false,
                    })
                    .collect(),
            )])),
            ..Default::default()
        })
    }

    /// With something pending and no deployed revision (a project never
    /// committed), a claimed timer counts as not deployed, as a value does.
    #[test]
    fn switching_times_with_no_deployed_revision_are_pending() {
        let doc = edit::parse_doc(house_with_timer().as_bytes()).unwrap();
        let s = edit::slug(UHR);
        let files = edit::Files::from_texts(&std::collections::BTreeMap::from([(
            "edits.loxl".to_string(),
            format!(
                "extern {s} = DayTimer(uuid: \"{UHR}\")\nschedule vx_a\n\tmon 01:00-02:00 = 3\nend\n{s}.schedule = vx_a\n"
            ),
        )]))
        .unwrap();
        let lock = edit::Lock::default();
        let none = edit::Files::default();
        let (v, sets) = edit_schedules(&files, &none, &lock, Some(&doc), None);
        assert_eq!(v[0]["uuid"], UHR);
        assert_eq!(v[0]["pending"], true);
        assert_eq!(sets[0]["name"], "vx_a");
        // Nothing pending: the pending config is the deployed one.
        let (v, _) = edit_schedules(&files, &none, &lock, None, Some(&doc));
        assert_eq!(v[0]["pending"], false);
        let (v, _) = edit_schedules(&files, &files, &lock, Some(&doc), Some(&doc));
        assert_eq!(v[0]["pending"], false);
    }

    /// A compile that does not write the claim onto its timer is refused,
    /// and the module, lock and out are put back; nothing is ever deployed.
    #[cfg(unix)]
    #[tokio::test]
    async fn switching_times_lox_did_not_write_are_refused_and_put_back() {
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        std::fs::write(d.join("base.Loxone"), house_with_timer()).unwrap();
        std::fs::write(d.join("out.Loxone"), house_with_timer()).unwrap();
        sh(&d, "git add -A && git commit -qm timer");
        // A stand-in that compiles nothing: out is the base.
        std::fs::write(
            tmp.path().join("lox"),
            "#!/bin/sh\necho \"$*\" >> ../calls.log\n[ \"$1 $2\" = \"logic compile\" ] || exit 2\n\
             out=out.Loxone; prev=\nfor a in \"$@\"; do [ \"$prev\" = --out ] && out=$a; prev=$a; done\n\
             cp base.Loxone \"$out\"\n",
        )
        .unwrap();
        let req = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![edit::Op::Schedule {
                uuid: UHR.into(),
                value: mon(&[(360, 480, "21.5")]),
            }],
        };
        let out_was = std::fs::read(d.join("out.Loxone")).unwrap();
        let (code, msg) = a.edit(&req, None).await.unwrap_err();
        assert_eq!(code, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("did not write the switching times of Uhr"),
            "{msg}"
        );
        assert!(!d.join("pages/vislox/edits.loxl").exists());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out_was);
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// Switching times with the real lox (#73): set `VISLOX_TEST_LOX` to a lox
    /// binary (v0.83.0) to run it; it never talks to a Miniserver. The set
    /// and claim are compiled onto the timer, `GET /api/edits` lists it as
    /// not deployed, a timer the module's own file claims is refused, and the
    /// reply's undo puts the base's switching times back.
    #[cfg(unix)]
    #[tokio::test]
    async fn switching_times_are_compiled_by_lox_and_undone() {
        let Some(lox) = std::env::var_os("VISLOX_TEST_LOX") else {
            eprintln!("VISLOX_TEST_LOX is not set: skipped");
            return;
        };
        let (tmp, a) = project_with_lox_base(Path::new(&lox), house_with_timer().as_bytes());
        let d = tmp.path().join("project");
        let base_entries = |p: &str| {
            let doc = edit::parse_doc(&std::fs::read(d.join(p)).unwrap()).unwrap();
            let el = edit::find(&doc.xml.root, UHR).unwrap().clone();
            let t = crate::schedule::read(&el, UHR, |_| None).unwrap();
            t.entries
                .iter()
                .map(|e| (e.from, e.to, e.value))
                .collect::<Vec<_>>()
        };
        assert_eq!(base_entries("out.Loxone"), [(390, 480, 21.0)]);
        let r = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Schedule {
                        uuid: UHR.into(),
                        value: mon(&[(360, 480, "21.5"), (1020, 1320, "22")]),
                    }],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(r["previous"][0], Value::Null);
        assert_eq!(r["undo"][0]["op"], "schedule");
        assert_eq!(r["undo"][0]["value"], Value::Null);
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(
            file.contains("schedule vx_uhr_schedule\n\tmon 06:00-08:00 = 21.5, 17:00-22:00 = 22\n"),
            "{file}"
        );
        assert_eq!(
            base_entries("out.Loxone"),
            [(360, 480, 21.5), (1020, 1320, 22.0)]
        );
        let e = edits_of(&a, &d).await;
        let s = &e["schedules"][0];
        assert_eq!(s["uuid"], UHR);
        assert_eq!(s["set"], "vx_uhr_schedule");
        assert_eq!(s["pending"], true);
        assert_eq!(s["deployed"], Value::Null);
        assert_eq!(e["schedule_sets"][0]["timers"][0], UHR);
        // Undone: the base's switching times again.
        let undo: Vec<edit::Op> = serde_json::from_value(r["undo"].clone()).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: undo,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(base_entries("out.Loxone"), [(390, 480, 21.0)]);
        // The module's own file claims it: refused, naming that line.
        std::fs::write(
            d.join("pages/own.loxl"),
            format!(
                "# The module's own.\npage \"Obergeschoss\"\nextern uhr = DayTimer(uuid: \"{UHR}\")\n\
                 schedule abend\n\tmon 17:00-22:00 = 20\nend\nuhr.schedule = abend\n"
            ),
        )
        .unwrap();
        sh(
            &d,
            &format!("{} logic compile >/dev/null", Path::new(&lox).display()),
        );
        let (code, msg) = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Schedule {
                        uuid: UHR.into(),
                        value: mon(&[(0, 60, "1")]),
                    }],
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(code, axum::http::StatusCode::UNPROCESSABLE_ENTITY, "{msg}");
        assert!(
            msg.contains("project/pages/own.loxl:7 claims")
                || msg.contains("pages/own.loxl:7 claims"),
            "{msg}"
        );

        // Claimed twice: vislox claims it again, then the module's file does
        // too, uncompiled (lox compiles neither). Dropping vislox's claim is let
        // through and compiles the module's.
        std::fs::write(
            d.join("pages/own.loxl"),
            "# The module's own.\npage \"Obergeschoss\"\n",
        )
        .unwrap();
        sh(
            &d,
            &format!("{} logic compile >/dev/null", Path::new(&lox).display()),
        );
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: vec![edit::Op::Schedule {
                    uuid: UHR.into(),
                    value: mon(&[(0, 60, "1")]),
                }],
            },
            None,
        )
        .await
        .unwrap();
        std::fs::write(
            d.join("pages/own.loxl"),
            format!(
                "# The module's own.\npage \"Obergeschoss\"\nextern uhr = DayTimer(uuid: \"{UHR}\")\n\
                 schedule abend\n\tmon 17:00-22:00 = 20\nend\nuhr.schedule = abend\n"
            ),
        )
        .unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: vec![edit::Op::Schedule {
                    uuid: UHR.into(),
                    value: None,
                }],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(base_entries("out.Loxone"), [(1020, 1320, 20.0)]);
        assert!(
            !std::fs::read_to_string(d.join("pages").join(edit::FILE))
                .unwrap()
                .contains(".schedule")
        );
    }

    // ── lighting moods (#74) ───────────────────────────────────────────

    fn moods_val(rows: &[(&str, &str)]) -> Option<crate::mood_edit::MoodsVal> {
        Some(crate::mood_edit::MoodsVal {
            moods: Some(
                rows.iter()
                    .map(|(n, v)| crate::mood_edit::MoodRow {
                        name: n.to_string(),
                        values: v.to_string(),
                    })
                    .collect(),
            ),
            ..Default::default()
        })
    }

    /// With something pending and no deployed revision, a claimed controller
    /// counts as not deployed, as a value does.
    #[test]
    fn moods_with_no_deployed_revision_are_pending() {
        use crate::mood_edit::fixture::{LICHT, house};
        let doc = edit::parse_doc(house().as_bytes()).unwrap();
        let s = edit::slug(LICHT);
        let files = edit::Files::from_texts(&std::collections::BTreeMap::from([(
            "edits.loxl".to_string(),
            format!(
                "extern {s} = LightController2(uuid: \"{LICHT}\")\nmoods vx_a\n\t\"Viel Licht\": all on\nend\n{s}.moods = vx_a\n"
            ),
        )]))
        .unwrap();
        let lock = edit::Lock::default();
        let none = edit::Files::default();
        let (v, sets) = edit_moods(&files, &none, &lock, Some(&doc), None);
        assert_eq!(v[0]["uuid"], LICHT);
        assert_eq!(v[0]["pending"], true);
        assert_eq!(v[0]["moods"][0]["values"], "all on");
        assert_eq!(sets[0]["name"], "vx_a");
        assert_eq!(sets[0]["controllers"][0], LICHT);
        let (v, _) = edit_moods(&files, &none, &lock, None, Some(&doc));
        assert_eq!(v[0]["pending"], false);
        let (v, _) = edit_moods(&files, &files, &lock, Some(&doc), Some(&doc));
        assert_eq!(v[0]["pending"], false);
        assert_eq!(v[0]["deployed"]["set"], "vx_a");
    }

    /// A compile that does not write the claim onto its controller is
    /// refused, and the module, lock and out are put back; nothing is deployed.
    #[cfg(unix)]
    #[tokio::test]
    async fn moods_lox_did_not_write_are_refused_and_put_back() {
        use crate::mood_edit::fixture::{LICHT, house};
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        std::fs::write(d.join("base.Loxone"), house()).unwrap();
        std::fs::write(d.join("out.Loxone"), house()).unwrap();
        sh(&d, "git add -A && git commit -qm lights");
        std::fs::write(
            tmp.path().join("lox"),
            "#!/bin/sh\necho \"$*\" >> ../calls.log\n[ \"$1 $2\" = \"logic compile\" ] || exit 2\n\
             out=out.Loxone; prev=\nfor a in \"$@\"; do [ \"$prev\" = --out ] && out=$a; prev=$a; done\n\
             cp base.Loxone \"$out\"\n",
        )
        .unwrap();
        let req = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![edit::Op::Moods {
                uuid: LICHT.into(),
                value: moods_val(&[("Viel Licht", "all on"), ("Lesen", "AQ1 on")]),
            }],
        };
        let out_was = std::fs::read(d.join("out.Loxone")).unwrap();
        let (code, msg) = a.edit(&req, None).await.unwrap_err();
        assert_eq!(code, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(msg.contains("did not write the moods of Licht"), "{msg}");
        assert!(!d.join("pages/vislox/edits.loxl").exists());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out_was);
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// Moods with the real lox (#74): set `VISLOX_TEST_LOX` to a lox binary
    /// (v0.83.0) to run it; it never talks to a Miniserver. The set and claim
    /// are compiled onto the controller, `GET /api/edits` lists it as not
    /// deployed, the reply's undo puts the base's moods back, and a
    /// controller the module's own file claims is refused, its drop let through.
    #[cfg(unix)]
    #[tokio::test]
    async fn moods_are_compiled_by_lox_and_undone() {
        use crate::mood_edit::fixture::{LICHT, house};
        let Some(lox) = std::env::var_os("VISLOX_TEST_LOX") else {
            eprintln!("VISLOX_TEST_LOX is not set: skipped");
            return;
        };
        let (tmp, a) = project_with_lox_base(Path::new(&lox), house().as_bytes());
        let d = tmp.path().join("project");
        let moods = |p: &str| {
            let doc = edit::parse_doc(&std::fs::read(d.join(p)).unwrap()).unwrap();
            let el = edit::find(&doc.xml.root, LICHT).unwrap().clone();
            loxone_config::moods::Moods::read(&el)
                .unwrap()
                .moods
                .iter()
                .map(|m| format!("{}={}", m.name, loxone_config::moods::render(&m.qs)))
                .collect::<Vec<_>>()
        };
        let before = moods("out.Loxone");
        let r = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Moods {
                        uuid: LICHT.into(),
                        value: moods_val(&[
                            ("Viel Licht", "all on"),
                            ("Lesen", "AQ1 on, AQ2 40%"),
                            ("Abend", "AQ3 temp(30%, 2700K)"),
                        ]),
                    }],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(r["undo"][0]["op"], "moods");
        assert_eq!(r["undo"][0]["value"], Value::Null);
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(file.contains("moods vx_licht_moods\n"), "{file}");
        assert_eq!(
            moods("out.Loxone"),
            [
                "Viel Licht=all on, AQ2 off, AQ3 off",
                "Lesen=AQ1 on, AQ2 40%",
                "Abend=AQ3 temp(30%, 2700K)",
                "Aus=all off"
            ]
        );
        let e = edits_of(&a, &d).await;
        assert_eq!(e["moods"][0]["uuid"], LICHT);
        assert_eq!(e["moods"][0]["pending"], true);
        assert_eq!(e["mood_sets"][0]["controllers"][0], LICHT);
        // Undone: the base's moods again.
        let undo: Vec<edit::Op> = serde_json::from_value(r["undo"].clone()).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: undo,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(moods("out.Loxone"), before);
        // The module's own file claims it: refused, naming that line.
        std::fs::write(
            d.join("pages/own.loxl"),
            format!(
                "# The module's own.\npage \"Obergeschoss\"\nextern licht = LightController2(uuid: \"{LICHT}\")\n\
                 moods wohnen\n\t\"Viel Licht\": all on\n\t\"Lesen\": AQ1 on\nend\nlicht.moods = wohnen\n"
            ),
        )
        .unwrap();
        sh(
            &d,
            &format!("{} logic compile >/dev/null", Path::new(&lox).display()),
        );
        let (code, msg) = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Moods {
                        uuid: LICHT.into(),
                        value: moods_val(&[("Viel Licht", "all off")]),
                    }],
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(code, axum::http::StatusCode::UNPROCESSABLE_ENTITY, "{msg}");
        assert!(msg.contains("pages/own.loxl:8 claims its moods"), "{msg}");
    }

    // ── access rights (#76) ────────────────────────────────────────────

    fn access_val(grants: &[(&str, &str, &str)]) -> Option<crate::access_edit::AccessVal> {
        Some(crate::access_edit::AccessVal {
            grants: Some(
                grants
                    .iter()
                    .map(|(k, u, s)| crate::access_edit::Grant {
                        kind: k.to_string(),
                        uuid: u.to_string(),
                        schedule: s.to_string(),
                    })
                    .collect(),
            ),
            ..Default::default()
        })
    }

    /// `GET /api/edits` lists a claimed Code Touch by UUIDs only: no title of
    /// a user, group or schedule.
    #[test]
    fn access_is_listed_by_uuid_and_pending() {
        use crate::access_edit::fixture::{ALWAYS, BOB, DOOR, house};
        let doc = edit::parse_doc(house().as_bytes()).unwrap();
        let s = edit::slug(DOOR);
        let files = edit::Files::from_texts(&std::collections::BTreeMap::from([(
            "edits.loxl".to_string(),
            format!(
                "extern {s} = NfcCodeTouch(uuid: \"{DOOR}\")\naccess vx_a\n\tuser \"bob\": \"Always\"\nend\n{s}.access = vx_a\n"
            ),
        )]))
        .unwrap();
        let lock = edit::Lock::default();
        let none = edit::Files::default();
        let (v, sets) = edit_access(&files, &none, &lock, Some(&doc), None);
        assert_eq!(v[0]["uuid"], DOOR);
        assert_eq!(v[0]["pending"], true);
        assert_eq!(
            v[0]["grants"],
            json!([{ "kind": "user", "uuid": BOB, "schedule": ALWAYS }])
        );
        assert_eq!(sets[0]["code_touches"][0], DOOR);
        let text = format!("{v}{sets}");
        assert!(!text.contains("bob") && !text.contains("Always"), "{text}");
        let (v, _) = edit_access(&files, &files, &lock, Some(&doc), Some(&doc));
        assert_eq!(v[0]["pending"], false);
        assert_eq!(v[0]["deployed"]["set"], "vx_a");
    }

    /// A compile that does not write the claim onto its Code Touch is
    /// refused, and the module, lock and out are put back; nothing is deployed.
    #[cfg(unix)]
    #[tokio::test]
    async fn access_lox_did_not_write_is_refused_and_put_back() {
        use crate::access_edit::fixture::{ALWAYS, BOB, DOOR, house};
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        std::fs::write(d.join("base.Loxone"), house()).unwrap();
        std::fs::write(d.join("out.Loxone"), house()).unwrap();
        sh(&d, "git add -A && git commit -qm doors");
        std::fs::write(
            tmp.path().join("lox"),
            "#!/bin/sh\necho \"$*\" >> ../calls.log\n[ \"$1 $2\" = \"logic compile\" ] || exit 2\n\
             out=out.Loxone; prev=\nfor a in \"$@\"; do [ \"$prev\" = --out ] && out=$a; prev=$a; done\n\
             cp base.Loxone \"$out\"\n",
        )
        .unwrap();
        let req = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![edit::Op::Access {
                uuid: DOOR.into(),
                value: access_val(&[("user", BOB, ALWAYS)]),
            }],
        };
        let out_was = std::fs::read(d.join("out.Loxone")).unwrap();
        let (code, msg) = a.edit(&req, None).await.unwrap_err();
        assert_eq!(code, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("did not write the access rights of Haustür"),
            "{msg}"
        );
        assert!(!d.join("pages/vislox/edits.loxl").exists());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out_was);
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// Access with the real lox (#76): set `VISLOX_TEST_LOX` to a lox binary
    /// (v0.83.0) to run it; it never talks to a Miniserver. The set and claim
    /// are compiled onto the Code Touch (alice loses access, bob gains it,
    /// alice's `ACT=` stays), Apply's comparison says so, `GET /api/edits`
    /// lists it as not deployed, the reply's undo puts alice back, and a Code
    /// Touch the module's own file claims is refused, its drop let through.
    #[cfg(unix)]
    #[tokio::test]
    async fn access_is_compiled_by_lox_and_undone() {
        use crate::access_edit::fixture::{ALWAYS, BOB, DOOR, house};
        let Some(lox) = std::env::var_os("VISLOX_TEST_LOX") else {
            eprintln!("VISLOX_TEST_LOX is not set: skipped");
            return;
        };
        let (tmp, a) = project_with_lox_base(Path::new(&lox), house().as_bytes());
        let d = tmp.path().join("project");
        let doc = |p: &str| edit::parse_doc(&std::fs::read(d.join(p)).unwrap()).unwrap();
        let opens = |p: &str| {
            crate::rights::read(&doc(p))[DOOR]
                .list
                .iter()
                .map(|x| format!("{}={}", x.title, x.schedule.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        };
        let before = opens("out.Loxone");
        assert_eq!(before, ["alice=Always"]);
        let r = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Access {
                        uuid: DOOR.into(),
                        value: access_val(&[("user", BOB, ALWAYS)]),
                    }],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(r["undo"][0]["op"], "access");
        assert_eq!(r["undo"][0]["value"], Value::Null);
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(
            file.contains("access vx_haustuer_access\n\tuser \"bob\": \"Always\"\nend\n"),
            "{file}"
        );
        assert_eq!(opens("out.Loxone"), ["bob=Always"]);
        let xml = std::fs::read_to_string(d.join("out.Loxone")).unwrap();
        assert!(
            xml.contains(&format!("REF=\"{DOOR}\" ACT=\"3\"")),
            "alice's ACT= is kept"
        );
        let ch = crate::model::apply_changes(&doc("out.Loxone"), &doc("base.Loxone")).rights;
        let door = ch.iter().find(|c| c.uuid == DOOR).unwrap();
        assert_eq!(door.lose[0].title, "alice");
        assert_eq!(door.gain[0].title, "bob");
        let e = edits_of(&a, &d).await;
        assert_eq!(e["access"][0]["uuid"], DOOR);
        assert_eq!(e["access"][0]["pending"], true);
        assert_eq!(e["access_sets"][0]["code_touches"][0], DOOR);
        // Undone: alice again.
        let undo: Vec<edit::Op> = serde_json::from_value(r["undo"].clone()).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: undo,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(opens("out.Loxone"), before);
        // The module's own file claims it: refused, naming that line.
        std::fs::write(
            d.join("pages/own.loxl"),
            format!(
                "# The module's own.\npage \"Zugang\"\nextern haustuer = NfcCodeTouch(uuid: \"{DOOR}\")\n\
                 access bewohner\n\tuser \"alice\": \"Always\"\nend\nhaustuer.access = bewohner\n"
            ),
        )
        .unwrap();
        sh(
            &d,
            &format!("{} logic compile >/dev/null", Path::new(&lox).display()),
        );
        let (code, msg) = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![edit::Op::Access {
                        uuid: DOOR.into(),
                        value: access_val(&[("user", BOB, ALWAYS)]),
                    }],
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(code, axum::http::StatusCode::UNPROCESSABLE_ENTITY, "{msg}");
        assert!(
            msg.contains("pages/own.loxl:7 claims its access rights"),
            "{msg}"
        );
    }

    // ── members of a central block (#75) ───────────────────────────────

    fn members_op(uuid: &str, list: Option<&[&str]>) -> edit::Op {
        edit::Op::Members {
            uuid: uuid.into(),
            value: list.map(|l| l.iter().map(|s| s.to_string()).collect()),
        }
    }

    /// A compile that does not write the claim onto its central block is
    /// refused, and the module, lock and out are put back; nothing is deployed.
    #[cfg(unix)]
    #[tokio::test]
    async fn members_lox_did_not_write_are_refused_and_put_back() {
        use crate::member_edit::fixture::{ALLE, FLUR, LICHT, house};
        let (tmp, a) = project_for_edits();
        let d = tmp.path().join("project");
        std::fs::write(d.join("base.Loxone"), house()).unwrap();
        std::fs::write(d.join("out.Loxone"), house()).unwrap();
        sh(&d, "git add -A && git commit -qm centrals");
        std::fs::write(
            tmp.path().join("lox"),
            "#!/bin/sh\necho \"$*\" >> ../calls.log\n[ \"$1 $2\" = \"logic compile\" ] || exit 2\n\
             out=out.Loxone; prev=\nfor a in \"$@\"; do [ \"$prev\" = --out ] && out=$a; prev=$a; done\n\
             cp base.Loxone \"$out\"\n",
        )
        .unwrap();
        let req = edit::Request {
            base: blob(&d, "out.Loxone"),
            ops: vec![members_op(ALLE, Some(&[FLUR, LICHT]))],
        };
        let out_was = std::fs::read(d.join("out.Loxone")).unwrap();
        let (code, msg) = a.edit(&req, None).await.unwrap_err();
        assert_eq!(code, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("did not write the members of Alle Lichter"),
            "{msg}"
        );
        assert!(!d.join("pages/vislox/edits.loxl").exists());
        assert_eq!(std::fs::read(d.join("out.Loxone")).unwrap(), out_was);
        let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("logic compile")),
            "{calls}"
        );
    }

    /// Members with the real lox (#75): set `VISLOX_TEST_LOX` to a lox binary
    /// (v0.83.0) to run it; it never talks to a Miniserver. The claim is
    /// compiled onto the central block in the order given, `GET /api/edits`
    /// lists it as not deployed, the reply's undo puts the base's list back,
    /// and a central block the module's own file claims is refused, its drop
    /// let through.
    #[cfg(unix)]
    #[tokio::test]
    async fn members_are_compiled_by_lox_and_undone() {
        use crate::member_edit::fixture::{ALLE, BAD, FLUR, LICHT, house};
        let Some(lox) = std::env::var_os("VISLOX_TEST_LOX") else {
            eprintln!("VISLOX_TEST_LOX is not set: skipped");
            return;
        };
        let (tmp, a) = project_with_lox_base(Path::new(&lox), house().as_bytes());
        let d = tmp.path().join("project");
        let rec = |p: &str| {
            let doc = edit::parse_doc(&std::fs::read(d.join(p)).unwrap()).unwrap();
            crate::member_edit::of(edit::find(&doc.xml.root, ALLE).unwrap())
        };
        assert_eq!(rec("out.Loxone"), [LICHT, FLUR]);
        let r = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![members_op(ALLE, Some(&[BAD, FLUR]))],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(r["undo"][0]["op"], "members");
        assert_eq!(r["undo"][0]["value"], Value::Null);
        let file = std::fs::read_to_string(d.join("pages").join(edit::FILE)).unwrap();
        assert!(
            file.contains(&format!(
                "{}.members = ({}, {})\n",
                edit::slug(ALLE),
                edit::slug(BAD),
                edit::slug(FLUR)
            )),
            "{file}"
        );
        assert_eq!(rec("out.Loxone"), [BAD, FLUR]);
        let e = edits_of(&a, &d).await;
        assert_eq!(e["members"][0]["uuid"], ALLE);
        assert_eq!(e["members"][0]["pending"], true);
        assert_eq!(e["members"][0]["members"], json!([BAD, FLUR]));
        // `()` empties the list: no `rec=` at all.
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: vec![members_op(ALLE, Some(&[]))],
            },
            None,
        )
        .await
        .unwrap();
        assert!(rec("out.Loxone").is_empty());
        // Back to the first edit's list, then undone: the base's list again.
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: vec![members_op(ALLE, Some(&[BAD, FLUR]))],
            },
            None,
        )
        .await
        .unwrap();
        let undo: Vec<edit::Op> = serde_json::from_value(r["undo"].clone()).unwrap();
        a.edit(
            &edit::Request {
                base: blob(&d, "out.Loxone"),
                ops: undo,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(rec("out.Loxone"), [LICHT, FLUR]);
        // The module's own file claims it: refused, naming that line.
        std::fs::write(
            d.join("pages/own.loxl"),
            format!(
                "# The module's own.\npage \"Obergeschoss\"\nextern alle = CentralLight(uuid: \"{ALLE}\")\n\
                 extern licht = LightController2(uuid: \"{LICHT}\")\nalle.members = (licht)\n"
            ),
        )
        .unwrap();
        sh(
            &d,
            &format!("{} logic compile >/dev/null", Path::new(&lox).display()),
        );
        assert_eq!(rec("out.Loxone"), [LICHT]);
        let (code, msg) = a
            .edit(
                &edit::Request {
                    base: blob(&d, "out.Loxone"),
                    ops: vec![members_op(ALLE, Some(&[FLUR]))],
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(code, axum::http::StatusCode::UNPROCESSABLE_ENTITY, "{msg}");
        assert!(msg.contains("pages/own.loxl:5 claims its members"), "{msg}");
    }
}
