//! Roll back an Apply (#69). A Roll back is an Apply onto the download
//! `lox config push` kept from before that Apply (see `crate::rollback`), with
//! Apply's preflight, tests, push, restart following, verification, commit and
//! git push, and these checks of its own:
//!
//! - **The plan** ([`Actions::rollback_plan`]) runs under `busy` and the
//!   compile lock, like any action that compiles, and changes nothing: the
//!   module and lockfile from before the Apply, compiled onto that download in
//!   a scratch directory, must give it back. The browser reviews the result
//!   and sends its blob id back; the deploy works the plan out again and
//!   refuses unless it is the same.
//! - **What is uploaded is what was reviewed.** The deploy writes the module
//!   and lockfile from before (only files git tracks: nothing untracked or
//!   ignored is deleted or overwritten), points lox.toml's base at the
//!   download, and compiles in place with the project's own settings
//!   (`lox logic compile`): `out` must be the reviewed program. Then
//!   `lox config push --patched-from …` without `--force` stages the upload,
//!   which must extract to the reviewed program too. Only then the push with
//!   `--force`; afterwards the download lox verified against must be it.
//! - **A failed push is not guessed about.** Before lox printed its upload
//!   line nothing was sent, and the project is put back. After it, vislox
//!   downloads what the Miniserver runs: the rolled-back program is recorded
//!   as deployed (committed, *not verified*); the Apply's program with no
//!   upload waiting means nothing changed (put back); anything else is
//!   reported as unknown.

use super::*;
use axum::http::StatusCode;

/// What a Roll back deploys ([`Actions::rollback_plan`]).
pub struct RollbackPlan {
    /// The commit rolled back, in full.
    pub commit: String,
    pub subject: String,
    /// `backups/<stamp>-pre-push.zip`, for messages.
    pub backup: String,
    /// The download the commit deployed: the Miniserver must still run it.
    post: PathBuf,
    /// The base's blob id in the commit: what the Miniserver runs now.
    pub deployed: String,
    /// The program the Miniserver ran before.
    pre: Vec<u8>,
    /// The module from before, compiled onto it: what will run.
    pub compiled: Vec<u8>,
    /// `compiled` is `pre` byte for byte (else the same program).
    pub same_bytes: bool,
    /// The module's files as git had them before the commit.
    files: Vec<(PathBuf, Vec<u8>)>,
    /// Module files git tracks now that it did not track before the commit.
    remove: Vec<PathBuf>,
    /// The lockfile before the commit (`None`: there was none).
    lock: Option<Vec<u8>>,
}

impl RollbackPlan {
    /// The blob id of what it deploys.
    pub fn blob(&self) -> String {
        crate::history::blob_id(&self.compiled)
    }

    /// `backups/<stamp>-rollback-base.Loxone`, next to the push's downloads.
    fn base_file(&self) -> PathBuf {
        let name = self.post.file_name().unwrap_or_default().to_string_lossy();
        let stem = name.strip_suffix("-post-push.Loxone").unwrap_or(&name);
        self.post
            .with_file_name(format!("{stem}-rollback-base.Loxone"))
    }

    /// Put the module's tracked files and the lockfile back to before the
    /// commit, and point lox.toml's base at a copy of the download from before
    /// it (as lox points the base at its download after a push). The project's
    /// base file stays as it is until the push is verified.
    fn write(&self, p: &loxone_config::Project) -> Result<(), String> {
        let err = |path: &Path, e: std::io::Error| format!("cannot write {}: {e}", path.display());
        for f in &self.remove {
            std::fs::remove_file(f).map_err(|e| err(f, e))?;
        }
        for (path, bytes) in &self.files {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).map_err(|e| err(d, e))?;
            }
            std::fs::write(path, bytes).map_err(|e| err(path, e))?;
        }
        match &self.lock {
            Some(l) => std::fs::write(&p.lock, l).map_err(|e| err(&p.lock, e))?,
            None if p.lock.exists() => {
                std::fs::remove_file(&p.lock).map_err(|e| err(&p.lock, e))?
            }
            None => {}
        }
        let base = self.base_file();
        std::fs::write(&base, &self.pre).map_err(|e| err(&base, e))?;
        point_base(p, &base)
    }
}

/// Point lox.toml's base at `base`.
fn point_base(p: &loxone_config::Project, base: &Path) -> Result<(), String> {
    let toml = p.root.join("lox.toml");
    let text = std::fs::read_to_string(&toml).map_err(|e| format!("cannot read lox.toml: {e}"))?;
    let rel = base.strip_prefix(&p.root).unwrap_or(base).to_string_lossy();
    std::fs::write(&toml, set_base(&text, &rel)).map_err(|e| format!("cannot write lox.toml: {e}"))
}

/// What the Miniserver runs after a push that failed after its upload.
#[derive(Debug, PartialEq)]
enum Runs {
    /// The rolled-back program.
    RolledBack,
    /// Still the Apply's, and no upload is waiting to load.
    Unchanged,
    /// Cannot tell: why.
    Unknown(String),
}

/// The `compiled` path of lox's "Staged <zip> (extracts to exactly <path>)".
fn staged_path(out: &str) -> Option<PathBuf> {
    let rest = out.split("extracts to exactly ").nth(1)?;
    let path = rest.split(')').next()?.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

impl Actions {
    /// Which of the History's commits can be rolled back (#69), by revision
    /// id: the pushes whose download from before is in `backups/`. Only the
    /// newest commit of the base can be: the Miniserver runs it. Blocking: it
    /// may hash downloads that are new (the others are cached).
    pub fn rollbacks(&self, hist: &[crate::history::Revision]) -> Value {
        if self.why_not_apply().is_some() {
            return json!({});
        }
        let Ok(p) = self.project() else {
            return json!({});
        };
        let pushes = self.pushes.list(&p.root.join("backups"));
        let deployed = hist.iter().find(|r| r.kind == "git").map(|r| r.id.as_str());
        let mut out = serde_json::Map::new();
        // A push made one commit: the newest with its program. An older one
        // with the same bytes (a rollback goes back to them) did not push it.
        let mut taken = std::collections::HashSet::new();
        for r in hist.iter().filter(|r| r.kind == "git") {
            let Some(push) = pushes
                .get(&r.blob)
                .filter(|_| taken.insert(r.blob.as_str()))
            else {
                continue;
            };
            let backup = push.shown(&p.root);
            let why = if Some(r.id.as_str()) != deployed {
                Some("Only the newest deploy can be rolled back: the Miniserver runs a later config. Roll that back first.".to_string())
            } else if !push.pre_zip.exists() && !push.pre_xml.exists() {
                Some(format!("The download from before it is gone ({backup})."))
            } else {
                None
            };
            out.insert(
                r.id.clone(),
                json!({ "stamp": push.stamp, "backup": backup,
                        "available": why.is_none(), "reason": why }),
            );
        }
        Value::Object(out)
    }

    /// What rolling back commit `commit` would deploy (#69), worked out
    /// without changing the project, under `busy` and the compile lock (it
    /// compiles, and reads what an action may be writing).
    pub async fn rollback_plan(&self, commit: &str) -> Result<RollbackPlan, String> {
        let _one = self.busy.try_lock().map_err(|_| match self.running() {
            Some(what) => format!("{what} is running"),
            None => "already running".to_string(),
        })?;
        let _compiling = self.compile_lock.lock().await;
        self.plan_locked(commit, false).await
    }

    /// [`Self::rollback_plan`], with the locks held. Refused on any doubt: the
    /// commit must be the newest commit of the base, a verified push (a
    /// post-push download holds its base), with nothing changed since; the
    /// download from before it must be there and readable; nothing git does not
    /// track may be in the way; and the module and lockfile from before must
    /// compile onto that download and give it back. `steps`: mark Apply's
    /// steps as they start.
    async fn plan_locked(&self, commit: &str, steps: bool) -> Result<RollbackPlan, String> {
        let dir = self
            .s
            .project
            .as_deref()
            .ok_or("only for a lox.toml project")?;
        if let Some(why) = self.why_not_apply() {
            return Err(why.into());
        }
        let p = self.project()?;
        let commit = commit.trim_start_matches("git:");
        if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("not a commit: {commit}"));
        }
        let full = git(
            dir,
            &[
                "rev-parse",
                "-q",
                "--verify",
                &format!("{commit}^{{commit}}"),
            ],
        )
        .await
        .map_err(|_| format!("no commit {commit}"))?
        .trim()
        .to_string();
        let short = full[..10.min(full.len())].to_string();
        let rel = |q: &Path| {
            q.strip_prefix(&p.root)
                .unwrap_or(q)
                .to_string_lossy()
                .into_owned()
        };
        let (base, module, lock, out) = (rel(&p.base), rel(&p.module), rel(&p.lock), rel(&p.out));
        let newest = git(dir, &["log", "-1", "--format=%H", "--", &base]).await?;
        if newest.trim() != full {
            return Err(format!(
                "Only the newest deploy can be rolled back: {base} changed after {short}, so the \
                 Miniserver runs a later config. Roll that back first."
            ));
        }
        let paths = [
            "lox.toml",
            base.as_str(),
            module.as_str(),
            lock.as_str(),
            out.as_str(),
        ];
        let mut status = vec!["status", "--porcelain", "--untracked-files=all", "--"];
        status.extend(paths);
        let dirty = git(dir, &status).await?;
        if !dirty.trim().is_empty() {
            return Err(format!(
                "The project has changes that are not committed ({}). Apply or Discard them \
                 first: a Roll back puts the module back to before the Apply.",
                last_lines(dirty.trim(), 3).replace('\n', ", ")
            ));
        }
        let mut since = vec!["diff", "--quiet", full.as_str(), "HEAD", "--"];
        since.extend(&paths[..4]);
        if git(dir, &since).await.is_err() {
            return Err(format!(
                "lox.toml, the module or the lockfile changed in a commit after {short}. A Roll \
                 back would drop that too: revert it first."
            ));
        }
        let deployed = git(dir, &["rev-parse", &format!("{full}:./{base}")])
            .await
            .map_err(|_| format!("{short} has no {base}"))?
            .trim()
            .to_string();
        if file_blob(&p.base).as_deref() != Some(deployed.as_str()) {
            return Err(format!("{base} on disk is not the one {short} committed."));
        }
        let subject = git(dir, &["log", "-1", "--format=%s", &full])
            .await?
            .trim()
            .to_string();
        let parent = git(dir, &["rev-parse", "-q", "--verify", &format!("{full}^")])
            .await
            .map_err(|_| {
                format!("{short} is the first commit: there is nothing before it to go back to")
            })?
            .trim()
            .to_string();
        let push = crate::rollback::Pushes::find_now(&p.root.join("backups"), &deployed)
            .ok_or_else(|| {
                format!(
                    "{short} is not a verified push vislox can roll back: no \
                     backups/<stamp>-post-push.Loxone holds its {base}, so what the Miniserver \
                     ran before it is not known."
                )
            })?;
        let backup = push.shown(&p.root);
        let pre = crate::rollback::read_pre_push(&push, &p.root)?;
        // The module's tracked files and the lockfile as they were before.
        let names = |listed: String| -> Vec<String> {
            listed
                .split('\0')
                .filter(|n| !n.is_empty())
                .map(String::from)
                .collect()
        };
        let before = names(
            git(
                dir,
                &["ls-tree", "-r", "-z", "--name-only", &parent, "--", &module],
            )
            .await?,
        );
        if before.is_empty() {
            return Err(format!("{module} did not exist before {short}."));
        }
        let tracked = names(git(dir, &["ls-files", "-z", "--", &module]).await?);
        // Nothing git does not track (an ignored file, say) is overwritten.
        if let Some(n) = before
            .iter()
            .find(|n| !tracked.contains(n) && dir.join(n).exists())
        {
            return Err(format!(
                "{n} is in the way: git does not track it now (it may be ignored), and the \
                 module before {short} has a file there. Move it away first."
            ));
        }
        let lock_tracked = git(dir, &["ls-files", "--error-unmatch", "--", &lock])
            .await
            .is_ok();
        let mut specs: Vec<String> = before.iter().map(|n| format!("{parent}:./{n}")).collect();
        specs.push(format!("{parent}:./{lock}"));
        let mut objects = git_objects(dir, &specs).await?;
        let lock_bytes = objects.pop().flatten();
        if !lock_tracked && p.lock.exists() {
            return Err(format!(
                "{lock} is in the way: git does not track it now, and a Roll back would replace it."
            ));
        }
        let mut files = Vec::new();
        for (name, bytes) in before.iter().zip(objects) {
            let bytes = bytes.ok_or_else(|| format!("cannot read {name} of {short}^"))?;
            files.push((dir.join(name), bytes));
        }
        let remove = tracked
            .iter()
            .filter(|n| !before.contains(n))
            .map(|n| dir.join(n))
            .collect();
        if steps {
            self.progress(|p| {
                p.finish("backup", "ok", Some(backup.clone()));
                p.start("module");
            });
        }
        // Compiled in a scratch directory, to show what it deploys. The deploy
        // compiles in the project itself, and checks it gives the same.
        // Unique within this process too: two plans may start in one tick.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let scratch = std::env::temp_dir().join(format!(
            "vislox-rollback-{}-{}-{n}",
            std::process::id(),
            now_nanos()
        ));
        let _gone = RemoveDir(scratch.clone());
        let module_file = before.len() == 1 && before[0] == module;
        let s_module = if module_file {
            scratch.join(p.module.file_name().unwrap_or_default())
        } else {
            scratch.join("module")
        };
        let write = |path: &Path, bytes: &[u8]| -> Result<(), String> {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d)
                    .map_err(|e| format!("cannot create {}: {e}", d.display()))?;
            }
            std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
        };
        for (path, bytes) in &files {
            let inner = path.strip_prefix(&p.module).unwrap_or(Path::new(""));
            let to = if module_file {
                s_module.clone()
            } else {
                s_module.join(inner)
            };
            write(&to, bytes)?;
        }
        let (s_lock, s_base, s_out) = (
            scratch.join("lock.json"),
            scratch.join("base.Loxone"),
            scratch.join("out.Loxone"),
        );
        if let Some(l) = &lock_bytes {
            write(&s_lock, l)?;
        }
        write(&s_base, &pre)?;
        let (m, l, b, o) = (
            s_module.to_string_lossy(),
            s_lock.to_string_lossy(),
            s_base.to_string_lossy(),
            s_out.to_string_lossy(),
        );
        let args = [
            "logic", "compile", "--base", &b, "--module", &m, "--lock", &l, "--out", &o,
        ];
        run(&self.s.lox, &args, dir, COMPILE_TIMEOUT)
            .await
            .map_err(|e| {
                format!(
                    "The module from before {short} does not compile onto {backup}: {}",
                    last_lines(&e, 3)
                )
            })?;
        let compiled =
            std::fs::read(&s_out).map_err(|e| format!("lox logic compile wrote no config: {e}"))?;
        let same_bytes = crate::rollback::same_program(&pre, &compiled).map_err(|why| {
            format!(
                "The module from before {short} does not compile back to what the Miniserver \
                 ran then ({backup}): it {why}. A Roll back would not restore it."
            )
        })?;
        Ok(RollbackPlan {
            commit: full,
            subject,
            backup,
            post: push.post_xml,
            deployed,
            pre,
            compiled,
            same_bytes,
            files,
            remove,
            lock: lock_bytes,
        })
    }

    /// Roll back commit `commit` (#69): Apply's path, onto the download from
    /// before it (see the module's documentation). `reviewed`: the blob id of
    /// the program the user reviewed; the plan worked out now must be it.
    pub async fn rollback(
        &self,
        commit: &str,
        reviewed: Option<&str>,
        opts: &ApplyOpts,
    ) -> Result<String, Refusal> {
        // Everything the request says is checked before any file is touched.
        push_opts(opts, &mut Vec::new()).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| (StatusCode::CONFLICT, "already running".to_string()))?;
        self.st.lock().unwrap().progress = Some(Progress::rollback());
        self.begin("Roll back")?;
        let _compiling = self.compile_lock.lock().await;
        let refuse = |why: String, status: StatusCode| {
            self.end_with("rollback", &Err(why.clone()), None);
            Err((status, why))
        };
        self.progress(|p| p.start("backup"));
        let plan = match self.plan_locked(commit, true).await {
            Ok(plan) => plan,
            Err(why) => return refuse(why, StatusCode::CONFLICT),
        };
        let want = plan.blob();
        if let Some(r) = reviewed
            && r != want
        {
            return refuse(
                format!(
                    "What a Roll back deploys changed since you reviewed it (now {}, reviewed {}). \
                     Nothing was sent: review it again.",
                    &want[..10],
                    &r[..10.min(r.len())]
                ),
                StatusCode::CONFLICT,
            );
        }
        let p = match self.project() {
            Ok(p) => p,
            Err(why) => return refuse(why, StatusCode::CONFLICT),
        };
        let dir = self.s.project.as_deref().unwrap();
        let toml = dir.join("lox.toml");
        let base_file = plan.base_file();
        // Everything a step writes, to put back when the Roll back stops
        // before anything changed on the Miniserver. Ignored files in the
        // module are kept as they are.
        let mut keep: Vec<&Path> = vec![&p.lock, &p.base, &p.out, &toml, &base_file];
        if p.module.is_file() {
            keep.push(&p.module);
        }
        let guard = Rollback::take(&p.module, &keep);
        let fail = |why: String, status: StatusCode| {
            guard.restore();
            *self.compiled.lock().unwrap() = None;
            self.end_with("rollback", &Err(why.clone()), None);
            Err((status, why))
        };
        // In the project, with its own settings: out must be what was reviewed.
        if let Err(why) = plan.write(&p) {
            return fail(why, StatusCode::BAD_GATEWAY);
        }
        *self.compiled.lock().unwrap() = None;
        if let Err(e) = run(&self.s.lox, &["logic", "compile"], dir, COMPILE_TIMEOUT).await {
            return fail(
                format!(
                    "lox logic compile failed in the project: {}\nNothing was sent.",
                    last_lines(&e, 3)
                ),
                StatusCode::BAD_GATEWAY,
            );
        }
        let out = file_blob(&p.out).unwrap_or_default();
        if out != want {
            return fail(
                format!(
                    "Compiled in the project, the module from before gives another program than \
                     the one you reviewed (blob {} ≠ {}). Nothing was sent.",
                    &out[..10.min(out.len())],
                    &want[..10]
                ),
                StatusCode::CONFLICT,
            );
        }
        self.progress(|p| {
            p.finish(
                "module",
                "ok",
                Some(if plan.same_bytes {
                    "gives it back byte for byte".into()
                } else {
                    "gives back its program".into()
                }),
            )
        });
        // The tests (#67) of the module it deploys.
        self.progress(|p| p.start("tests"));
        match tests_verdict(&self.tests_now().await) {
            Ok((state, detail)) => self.progress(|p| p.finish("tests", state, Some(detail))),
            Err(why) => return fail(why, StatusCode::CONFLICT),
        }
        // lox stages the upload without sending it: it must be the program
        // reviewed. This also runs lox's drift check: the Miniserver must still
        // run exactly the Apply's download.
        self.progress(|p| p.start("stage"));
        let post = plan.post.to_string_lossy().into_owned();
        let mut args = vec!["config", "push", "--patched-from", post.as_str()];
        if let Err(why) = push_opts(opts, &mut args) {
            return fail(why, StatusCode::BAD_REQUEST);
        }
        let staged = match run(&self.s.lox, &args, dir, PUSH_TIMEOUT).await {
            Ok(o) => o,
            Err(e) => {
                return fail(
                    format!("lox config push (staging, nothing uploaded) failed:\n{e}"),
                    StatusCode::BAD_GATEWAY,
                );
            }
        };
        let staged_blob = staged_path(&staged)
            .map(|f| if f.is_absolute() { f } else { dir.join(f) })
            .and_then(|f| file_blob(&f));
        if staged_blob.as_deref() != Some(want.as_str()) {
            return fail(
                format!(
                    "lox staged another program than the one you reviewed ({}). Nothing was sent.\n{}",
                    staged_blob
                        .as_deref()
                        .map(|b| format!("blob {} ≠ {}", &b[..10], &want[..10]))
                        .unwrap_or_else(|| "its staged config could not be read".into()),
                    last_lines(&staged, 3)
                ),
                StatusCode::CONFLICT,
            );
        }
        self.progress(|p| {
            p.finish(
                "stage",
                "ok",
                Some("lox staged exactly the reviewed program".into()),
            )
        });
        args.insert(2, "--force");
        let short = plan.commit[..10].to_string();
        let subject = format!("Roll back: {}", plan.subject);
        let body = format!(
            "Restored {}: what the Miniserver ran before {short}. The module and lockfile are as \
             before that commit.",
            plan.backup
        );
        let force_add: Vec<PathBuf> = plan.files.iter().map(|(f, _)| f.clone()).collect();
        let (pushed, ran_before, check) = match self.push_config(&args).await {
            Ok((pushed, ran_before)) => {
                // lox verified against its new download: it must be the program reviewed.
                let now = loxone_config::Project::load(&toml)
                    .ok()
                    .and_then(|q| file_blob(&q.base));
                let check = if now.as_deref() == Some(want.as_str()) {
                    "The download lox verified is the program you reviewed.".to_string()
                } else {
                    format!(
                        "{UNVERIFIED}: the download lox verified is not byte for byte the program \
                         you reviewed ({}).",
                        now.as_deref().map(|b| &b[..10]).unwrap_or("unreadable")
                    )
                };
                (pushed, ran_before, check)
            }
            Err(why) => {
                let uploaded = self
                    .st
                    .lock()
                    .unwrap()
                    .progress
                    .as_mut()
                    .is_some_and(|p| p.get("upload").state != "wait");
                if !uploaded {
                    return fail(
                        format!(
                            "{why}\nlox stopped before the upload: nothing changed on the Miniserver, and the project is as it was."
                        ),
                        StatusCode::BAD_GATEWAY,
                    );
                }
                match self.what_runs(&plan).await {
                    Runs::Unchanged => {
                        return fail(
                            format!(
                                "{why}\nThe Miniserver still runs the Apply's config, and no upload is waiting: nothing changed, and the project is as it was."
                            ),
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                    Runs::Unknown(how) => {
                        return fail(
                            format!(
                                "{why}\n⚠ lox failed after it started the upload, and vislox cannot tell which config \
                                 the Miniserver runs now ({how}). It may run the rolled-back config or the Apply's. \
                                 The project was left at its last commit (the Apply's), which may not be what runs: \
                                 check \"Which config the Miniserver runs\" in the History, then Pull or Sync."
                            ),
                            StatusCode::BAD_GATEWAY,
                        );
                    }
                    Runs::RolledBack => {
                        // It runs what was reviewed: record that, unverified.
                        let rec = std::fs::write(&p.base, &plan.compiled)
                            .map_err(|e| format!("cannot write the base: {e}"))
                            .and_then(|()| point_base(&p, &p.base));
                        if let Err(e) = rec {
                            return fail(
                                format!(
                                    "{why}\n⚠ The Miniserver runs the rolled-back config, but vislox could not record it ({e}): the project is at the Apply's commit. Pull or Sync."
                                ),
                                StatusCode::BAD_GATEWAY,
                            );
                        }
                        let ran_before = crate::running::Report::failed("no check before".into());
                        (
                            format!(
                                "lox config push failed after the upload:\n{}",
                                last_lines(&why, 4)
                            ),
                            ran_before,
                            format!(
                                "{UNVERIFIED}: lox failed after the upload, but the Miniserver runs the \
                                 rolled-back program (vislox downloaded it and compared)."
                            ),
                        )
                    }
                }
            }
        };
        let res = self
            .after_push(
                &p,
                &pushed,
                &ran_before,
                Some(plan.deployed.clone()),
                &Commit {
                    subject: &subject,
                    body: Some(&body),
                    force_add: &force_add,
                },
            )
            .await
            .map(|(m, d)| {
                (
                    format!("Rolled back {short} to {}.\n{check}\n{m}", plan.backup),
                    d,
                )
            });
        let _ = std::fs::remove_file(&base_file);
        let (res, deployed) = match res {
            Ok((m, d)) => (Ok(m), d),
            Err(e) => (Err(e), None),
        };
        self.end_with("rollback", &res, deployed);
        res.map_err(|e| (StatusCode::BAD_GATEWAY, e))
    }

    /// After a push that failed once lox began the upload: download what the
    /// Miniserver runs (read-only, `lox config download`) and compare.
    async fn what_runs(&self, plan: &RollbackPlan) -> Runs {
        let dir = self.s.project.as_deref().unwrap();
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let scratch = std::env::temp_dir().join(format!(
            "vislox-runs-{}-{}-{n}",
            std::process::id(),
            now_nanos()
        ));
        if let Err(e) = std::fs::create_dir_all(&scratch) {
            return Runs::Unknown(format!("cannot create {}: {e}", scratch.display()));
        }
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
        if blob == plan.blob() {
            return Runs::RolledBack;
        }
        if blob != plan.deployed {
            return Runs::Unknown(format!(
                "its newest program ({}) is neither the Apply's nor the rolled-back one",
                &blob[..10]
            ));
        }
        // The Apply's program: unless an upload waits to load on the next start.
        if !self.s.running.available() {
            return Runs::Unknown(
                "it runs the Apply's program, but whether an upload waits cannot be checked".into(),
            );
        }
        let r = self.s.running.get(true).await;
        match (&r.error, &r.waiting) {
            (Some(e), _) => Runs::Unknown(format!(
                "it runs the Apply's program, but /prog cannot be read: {e}"
            )),
            (None, Some(w)) => Runs::Unknown(format!(
                "it runs the Apply's program, but {w} waits to be loaded"
            )),
            (None, None) => Runs::Unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_staged_config_is_read_from_lox_s_line() {
        let out = "Changes (live → compiled):\n  x\nStaged /p/backups/1-push.zip (extracts to exactly /p/backups/1-push.Loxone). Run again with --force…\n";
        assert_eq!(
            staged_path(out),
            Some(PathBuf::from("/p/backups/1-push.Loxone"))
        );
        assert_eq!(
            staged_path("Nothing to push: the Miniserver already runs it."),
            None
        );
    }
}
