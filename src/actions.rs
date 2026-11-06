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
//!
//! What is pending comes from outside vislox: anyone (a person, or an agent
//! working in the repository) edits the module and compiles it with
//! `lox logic`; vislox shows the result as the pending revision.
//!
//! All three need the admin token (`X-Vislox-Token`). vislox itself still
//! writes no config: lox and git do.

use crate::restart;
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
const APPLY_STEPS: [(&str, &str); 8] = [
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
        Actions {
            s,
            st: Mutex::new(St::default()),
            busy: tokio::sync::Mutex::new(()),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    fn changed(&self) {
        let _ = self.tx.send(());
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

    /// For `/api/history` and the `actions` event.
    pub fn info(&self) -> Value {
        let st = self.st.lock().unwrap();
        let flag = |why: Option<&str>| match why {
            None => json!({ "available": true }),
            Some(r) => json!({ "available": false, "reason": r }),
        };
        json!({
            "shown": self.s.project.is_some(),
            "apply": flag(self.why_not_apply()),
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
            res
        };
        let follow = async {
            if let (Some((probe, before)), Ok(())) = (watch_with, go_rx.await) {
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
        let res = self.sync_steps().await;
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

    /// Put the module, lock and compiled config back to the last commit.
    pub async fn discard(&self) -> Result<String, Refusal> {
        use axum::http::StatusCode;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.begin("Discard")?;
        let res = self.discard_steps().await;
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
        // Only what lox logic writes: the module, the lock and out.
        for path in [&module, &lock, &out] {
            if dir.join(path).exists() || tracked(dir, path).await {
                git(dir, &["checkout", "-q", "HEAD", "--", path]).await?;
            }
        }
        git(dir, &["clean", "-fdq", "--", &module]).await?;
        Ok("The module, lock and compiled config are back at the last commit.".into())
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

async fn tracked(dir: &Path, path: &str) -> bool {
    git(dir, &["ls-files", "--error-unmatch", "--", path])
        .await
        .is_ok()
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
