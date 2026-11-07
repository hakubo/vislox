//! Managed blocks a push would delete (#70). lox refuses to compile a module
//! that lost a block its lockfile manages, unless the removal is allowed:
//! `removed <slug>` in the module, or `--allow-removals`, which deletes
//! *every* vanished block. Either way the compile leaves a tombstone in the
//! lockfile for each block the base has, and the next push deletes those
//! blocks from the Miniserver, flag or not. So vislox asks for every one:
//!
//! - **The plan** ([`Actions::removal_plan`]) changes nothing: the module
//!   compiled in a scratch directory with `--allow-removals` and a fixed
//!   `--time`, onto a copy of the lockfile. The blocks it deletes are the
//!   tombstones it leaves for blocks the base has, and the managed blocks it
//!   drops that were never deployed. The browser shows the result as a
//!   revision and lists the blocks.
//! - **Without consent** Apply refuses when lox would refuse the module, or
//!   when the push would delete a block: a tombstone in the project's
//!   lockfile, or one a compile of the module as it is would leave.
//! - **The consent** names the plan's program (its blob id) and exactly its
//!   blocks. Apply compiles in the project with `--allow-removals` and the
//!   plan's `--time`: `out` must be the confirmed program, and the blocks it
//!   deletes the confirmed ones. Until the push succeeds, a guard
//!   ([`PutBack`]) puts the lockfile and `out` back on any failure, so the
//!   tombstones never outlive the request. The push runs *without*
//!   `--allow-removals`: a block that vanishes after the compile is refused
//!   by lox again, not deleted.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Says an Apply was refused for blocks it would delete without consent;
/// [`apply_hints`] reads it back.
pub const NEEDS_CONSENT: &str = "lox deletes them only when you allow it";

/// The managed block lox names when it refuses a compile because the module
/// lost it: "managed block `x` is in the lockfile but not in the source".
pub fn refused_slug(msg: &str) -> Option<String> {
    msg.split("managed block `").skip(1).find_map(|rest| {
        let (slug, after) = rest.split_once('`')?;
        after
            .trim_start()
            .starts_with("is in the lockfile but not in the source")
            .then(|| slug.to_string())
    })
}

/// A block a push would delete, or a managed block the lockfile forgets.
#[derive(Clone, Serialize, PartialEq, Debug)]
pub struct Removed {
    pub slug: String,
    pub uuid: String,
    /// The block type the lockfile records.
    #[serde(rename = "type")]
    pub ty: String,
    /// The base has it, so the Miniserver runs it: the lockfile keeps a
    /// tombstone and the push deletes it there. Else it was never deployed,
    /// and only the lockfile forgets it.
    pub deployed: bool,
}

/// What a compile that allows removals deploys ([`Actions::removal_plan`]).
#[derive(Clone)]
pub struct RemovalPlan {
    pub blocks: Vec<Removed>,
    /// The module compiled with the removals allowed.
    pub compiled: Vec<u8>,
    /// Its blob id.
    pub blob: String,
    /// The `--time` it was compiled with, so that the deploy's compile mints
    /// the same UUIDs for blocks the module adds.
    time: i64,
    /// When it was worked out.
    at: i64,
}

impl RemovalPlan {
    /// The revision that shows it (`removals:<blob>`).
    pub fn id(&self) -> String {
        plan_id(&self.blob)
    }
}

pub fn plan_id(blob: &str) -> String {
    format!("removals:{}", &blob[..12.min(blob.len())])
}

/// The user's consent to the removals of a plan, with Apply.
#[derive(serde::Deserialize, Clone, Debug, Default)]
pub struct RemovalConsent {
    /// The blob id of the plan's program the user reviewed.
    pub reviewed: String,
    /// The UUIDs of the blocks the user confirmed, exactly the plan's.
    pub blocks: Vec<String>,
}

/// A lockfile's managed objects (slug → UUID and type) and tombstones
/// (UUID → slug and type).
type Objects = BTreeMap<String, (String, String)>;
fn lock_objects(bytes: Option<&[u8]>) -> (Objects, Objects) {
    let v: Value = bytes
        .and_then(|b| serde_json::from_slice(b).ok())
        .unwrap_or(Value::Null);
    let read = |o: &Value, key: &str| -> Objects {
        o.as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, x)| {
                        let v = x[key].as_str()?.to_string();
                        let ty = x["type"].as_str().unwrap_or("").to_string();
                        Some((k.clone(), (v, ty)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    (read(&v["objects"], "uuid"), read(&v["removed"], "slug"))
}

/// The tombstones of lockfile `lock` whose object `base` still has: the
/// blocks a push of its compile deletes.
pub fn pending(lock: Option<&[u8]>, base: &[u8]) -> Vec<Removed> {
    let base = String::from_utf8_lossy(base);
    let (_, tombs) = lock_objects(lock);
    tombs
        .into_iter()
        .filter(|(uuid, _)| base.contains(&format!("U=\"{uuid}\"")))
        .map(|(uuid, (slug, ty))| Removed {
            slug,
            uuid,
            ty,
            deployed: true,
        })
        .collect()
}

/// What a compile from lockfile `before` to `after` against `base`
/// removes: the blocks its push deletes ([`pending`]), and the managed
/// blocks it drops that were never deployed. By slug.
fn removals(before: Option<&[u8]>, after: Option<&[u8]>, base: &[u8]) -> Vec<Removed> {
    let mut out = pending(after, base);
    let (was, _) = lock_objects(before);
    let (now, _) = lock_objects(after);
    for (slug, (uuid, ty)) in was {
        if !now.contains_key(&slug) && !out.iter().any(|b| b.uuid == uuid) {
            out.push(Removed {
                slug,
                uuid,
                ty,
                deployed: false,
            });
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// A new, empty directory of its own: two never share one, even when
/// started in the same instant.
pub(super) fn scratch_dir(prefix: &str) -> Result<PathBuf, String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}-{n}",
        std::process::id(),
        now_nanos()
    ));
    std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    Ok(d)
}

/// A compile of the module into a scratch directory ([`Actions::scratch_compile`]):
/// its lockfile and out, removed with it.
pub(super) struct Scratch {
    pub lock: PathBuf,
    pub out: PathBuf,
    _gone: RemoveDir,
}

fn uuids(blocks: &[Removed]) -> BTreeSet<&str> {
    blocks.iter().map(|b| b.uuid.as_str()).collect()
}

/// The lockfile and out as they were before Apply compiled the removals in:
/// put back on drop, and the compile forgotten, unless [`PutBack::keep`] was
/// called (once the push went through). So a consent never outlives its
/// request: a failure at any step before the push leaves no tombstone.
pub(super) struct PutBack<'a> {
    a: &'a Actions,
    saved: Vec<(PathBuf, Option<Vec<u8>>)>,
    keep: bool,
}

impl PutBack<'_> {
    pub(super) fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for PutBack<'_> {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        for (path, bytes) in &self.saved {
            let _ = match bytes {
                Some(b) => std::fs::write(path, b),
                None => std::fs::remove_file(path),
            };
        }
        *self.a.compiled.lock().unwrap() = None;
    }
}

impl Actions {
    /// `lox logic compile` of the module as it is, onto a scratch copy of
    /// the lockfile, with `extra` flags. `Err`: no scratch directory; the
    /// inner `Err` is lox's message.
    pub(super) async fn scratch_compile(
        &self,
        p: &loxone_config::Project,
        extra: &[&str],
    ) -> Result<(Scratch, Result<(), String>), String> {
        let dir = self.s.project.as_deref().unwrap();
        let d = scratch_dir("vislox-compile")?;
        let s = Scratch {
            lock: d.join("lock.json"),
            out: d.join("out.Loxone"),
            _gone: RemoveDir(d),
        };
        if p.lock.exists() {
            std::fs::copy(&p.lock, &s.lock).map_err(|e| format!("cannot copy the lock: {e}"))?;
        }
        let (l, o) = (s.lock.to_string_lossy(), s.out.to_string_lossy());
        let mut args = vec!["logic", "compile", "--lock", &l, "--out", &o];
        args.extend(extra);
        let res = run(&self.s.lox, &args, dir, COMPILE_TIMEOUT)
            .await
            .map(|_| ());
        Ok((s, res))
    }

    /// What Apply would deploy if the removals were allowed (see the
    /// module's documentation). Runs while no action runs, holding the
    /// compile lock; changes nothing in the project. Kept for
    /// [`crate::history::PREVIEW_TTL`], for the consent to name; a newer plan
    /// replaces it.
    pub async fn removal_plan(&self) -> Result<RemovalPlan, String> {
        let _one = self
            .busy
            .try_lock()
            .map_err(|_| "already running".to_string())?;
        if let Some(why) = self.why_not_apply() {
            return Err(why.to_string());
        }
        let _compiling = self.compile_lock.lock().await;
        let p = self.project()?;
        let time = now();
        let t = time.to_string();
        let (s, res) = self
            .scratch_compile(&p, &["--time", &t, "--allow-removals"])
            .await?;
        res.map_err(|e| {
            format!(
                "The module does not compile even with its removals allowed:\n{}",
                last_lines(&e, 6)
            )
        })?;
        let compiled =
            std::fs::read(&s.out).map_err(|e| format!("lox logic compile wrote no config: {e}"))?;
        let base = std::fs::read(&p.base).map_err(|e| format!("cannot read the base: {e}"))?;
        let blocks = removals(
            std::fs::read(&p.lock).ok().as_deref(),
            std::fs::read(&s.lock).ok().as_deref(),
            &base,
        );
        if blocks.is_empty() {
            return Err(
                "The module removes no managed block: there is nothing to allow.".to_string(),
            );
        }
        let plan = RemovalPlan {
            blob: crate::history::blob_id(&compiled),
            blocks,
            compiled,
            time,
            at: now(),
        };
        *self.removals.lock().unwrap() = Some(plan.clone());
        Ok(plan)
    }

    /// The plan the server keeps, for the browsers: its revision id.
    pub fn removal_plan_id(&self) -> Option<String> {
        self.removals
            .lock()
            .unwrap()
            .as_ref()
            .filter(|k| now() - k.at < crate::history::PREVIEW_TTL)
            .map(RemovalPlan::id)
    }

    /// The browser cancelled plan `id`: forget it.
    pub fn drop_removal_plan(&self, id: &str) {
        let mut k = self.removals.lock().unwrap();
        if k.as_ref().is_some_and(|k| k.id() == id) {
            *k = None;
        }
    }

    /// What the push of the project as it is would remove without consent:
    /// lox's refusal for a block the module lost, or the blocks it deletes.
    /// From the check when it is of the files as they are, else a scratch
    /// compile (as the check's). The project's own lockfile is read anyway:
    /// its tombstones are deleted by any push.
    async fn unasked_removals(
        &self,
        p: &loxone_config::Project,
    ) -> Result<(Option<String>, Vec<Removed>), String> {
        let base = std::fs::read(&p.base).map_err(|e| format!("cannot read the base: {e}"))?;
        let mut blocks = pending(std::fs::read(&p.lock).ok().as_deref(), &base);
        let mut refused = None;
        let cached = {
            let k = self.check_key();
            let files = self.edit_state.lock().unwrap().files.clone();
            let c = self.check_cache.lock().unwrap();
            c.as_ref()
                .filter(|c| c.key == k && c.v["available"] == true && c.v["ok"] == true)
                .map(|c| (c.v["removals"].clone(), files))
        };
        let from_check = match cached {
            Some((v, files)) if self.files_key_now().await == files => Some(v),
            _ => None,
        };
        match from_check {
            Some(v) => {
                refused = v["refused"].as_str().map(String::from);
                for b in v["blocks"].as_array().into_iter().flatten() {
                    if let Ok(b) = serde_json::from_value::<RemovedIn>(b.clone()) {
                        blocks.push(Removed {
                            slug: b.slug,
                            uuid: b.uuid,
                            ty: b.ty,
                            deployed: true,
                        });
                    }
                }
            }
            None => match self.freshness(p).await {
                Ok(Fresh::Broken { error, .. }) => refused = refused_slug(&error),
                Ok(Fresh::Yes { removals } | Fresh::Stale { removals, .. }) => {
                    blocks.extend(removals)
                }
                Err(_) => {}
            },
        }
        blocks.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        blocks.dedup_by(|a, b| a.uuid == b.uuid);
        blocks.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok((refused, blocks))
    }

    /// Apply's *Removals* step. Without consent: refuse when lox would refuse
    /// the module for a block it lost, or the push would delete a block
    /// ([`Self::unasked_removals`]), so that the browser asks. With it: the
    /// plan the consent names is compiled into the project, and must be it
    /// (see the module's documentation); the [`PutBack`] it returns puts the
    /// project back unless the push goes through.
    pub(super) async fn removals_step(
        &self,
        consent: Option<&RemovalConsent>,
    ) -> Result<(&'static str, String, Option<PutBack<'_>>), String> {
        let dir = self.s.project.as_deref().unwrap();
        let p = self.project()?;
        let Some(c) = consent else {
            let (refused, blocks) = self.unasked_removals(&p).await?;
            if refused.is_none() && blocks.is_empty() {
                return Ok(("skip", "the push removes no block".into(), None));
            }
            let mut msg = format!(
                "The push would delete blocks the Miniserver runs, and {NEEDS_CONSENT}. Nothing \
                 was deployed: review the removals, then apply."
            );
            for b in &blocks {
                msg.push_str(&format!("\n  {} ({})", b.slug, b.ty));
            }
            if let Some(slug) = refused {
                msg.push_str(&format!(
                    "\ncompile error: managed block `{slug}` is in the lockfile but not in the \
                     source."
                ));
            }
            return Err(msg);
        };
        let kept = self.removals.lock().unwrap().clone();
        let Some(kept) =
            kept.filter(|k| k.blob == c.reviewed && now() - k.at < crate::history::PREVIEW_TTL)
        else {
            return Err(
                "The removals you confirmed are not the ones vislox worked out last, or are too \
                 old. Nothing was deployed: review the removals again."
                    .into(),
            );
        };
        let confirmed: BTreeSet<&str> = c.blocks.iter().map(String::as_str).collect();
        if confirmed != uuids(&kept.blocks) {
            return Err(
                "The blocks you confirmed are not the ones the plan removes. Nothing was \
                 deployed: review the removals again."
                    .into(),
            );
        }
        // In the project, with the plan's time: out must be what was
        // confirmed, which binds the module, lockfile and base to the plan's.
        snapshot_before_edit(dir, &p).await;
        let module = module_blobs(&p);
        let base = std::fs::read(&p.base).map_err(|e| format!("cannot read the base: {e}"))?;
        let lock_was = std::fs::read(&p.lock).ok();
        let guard = PutBack {
            a: self,
            saved: vec![
                (p.lock.clone(), lock_was.clone()),
                (p.out.clone(), std::fs::read(&p.out).ok()),
            ],
            keep: false,
        };
        let t = kept.time.to_string();
        let args = ["logic", "compile", "--time", &t, "--allow-removals"];
        if let Err(e) = run(&self.s.lox, &args, dir, COMPILE_TIMEOUT).await {
            return Err(format!(
                "lox logic compile --allow-removals failed, so nothing was deployed:\n{}",
                last_lines(&e, 6)
            ));
        }
        let out = file_blob(&p.out);
        let gone = removals(
            lock_was.as_deref(),
            std::fs::read(&p.lock).ok().as_deref(),
            &base,
        );
        if out.as_deref() != Some(c.reviewed.as_str()) || uuids(&gone) != confirmed {
            return Err(format!(
                "The module, its lockfile or the base changed since you confirmed the removals: \
                 the compile is not the one you reviewed (out {}, {} removed). The lockfile and \
                 out were put back; nothing was deployed. Review the removals again.",
                out.as_deref().map(|o| &o[..10]).unwrap_or("missing"),
                plural(gone.len(), "block")
            ));
        }
        if module_blobs(&p) == module
            && std::fs::read(&p.base).ok().as_deref() == Some(base.as_slice())
            && let Some(out) = out
        {
            *self.compiled.lock().unwrap() = Some(Compiled {
                module,
                out,
                base: Some(crate::history::blob_id(&base)),
                lock: file_blob(&p.lock),
            });
        }
        *self.removals.lock().unwrap() = None;
        let on_ms = gone.iter().filter(|b| b.deployed).count();
        Ok((
            "ok",
            format!(
                "{} removed in the compile you confirmed ({on_ms} from the Miniserver)",
                plural(gone.len(), "block")
            ),
            Some(guard),
        ))
    }
}

/// A removed block as the check lists it.
#[derive(serde::Deserialize)]
struct RemovedIn {
    slug: String,
    uuid: String,
    #[serde(rename = "type", default)]
    ty: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn sh(dir: &Path, cmd: &str) {
        let o = std::process::Command::new("sh")
            .args(["-c", cmd])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{cmd}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    /// The base: the Miniserver runs `licht` (U1).
    const BASE: &str = "<C U=\"U1\"/>\n";
    /// The lockfile before: `licht` (on the Miniserver) and `bleibt`.
    const LOCK: &str =
        r#"{"objects":{"licht":{"uuid":"U1","type":"Or"},"bleibt":{"uuid":"U3","type":"And"}}}"#;

    /// A project in git whose module lost the managed block `licht`, and a
    /// `lox` that refuses to compile it unless removals are allowed (then it
    /// tombstones `licht` in the lockfile it is given), writes out as the
    /// module and base, and pushes as lox does: refusing while the project's
    /// lockfile still manages `licht`. It logs every call.
    #[cfg(unix)]
    fn project() -> (tempfile::TempDir, PathBuf, Actions) {
        project_with(LOCK)
    }

    /// [`project`] with this lockfile. `lox logic test` fails while the
    /// file `failtest` exists, and `lox config push` while `pushfail` does
    /// (before its upload) or `uploadfail` does (after it). `lox config
    /// download` copies `runs.zip`, or fails while `download-fails` exists.
    #[cfg(unix)]
    fn project_with(lock: &str) -> (tempfile::TempDir, PathBuf, Actions) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("project");
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::write(d.join("base.Loxone"), BASE).unwrap();
        std::fs::write(d.join("out.Loxone"), "compiled before\n").unwrap();
        std::fs::write(d.join("logic.lock.json"), lock).unwrap();
        std::fs::write(d.join("pages/main.loxl"), "bleibt = And()\n").unwrap();
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
        let calls = tmp.path().join("calls");
        let refusal = "managed block `licht` is in the lockfile but not in the source. To delete \
                       it, add `removed licht` (or pass --allow-removals); to rename it, add \
                       `moved licht -> <new_slug>`.";
        std::fs::write(
            &lox,
            format!(
                r#"#!/bin/sh
echo "$*" >> '{calls}'
lost() {{ grep -q '"licht":{{' "$1" && ! grep -q licht pages/main.loxl; }}
case "$1 $2" in
  'logic compile')
    lock=logic.lock.json; out=out.Loxone; allow=; shift 2
    while [ $# -gt 0 ]; do
      case "$1" in --lock) lock=$2; shift ;; --out) out=$2; shift ;; --time) shift ;; --allow-removals) allow=1 ;; esac
      shift
    done
    if lost "$lock"; then
      [ -n "$allow" ] || {{ echo 'error: compile error: {refusal}' >&2; exit 1; }}
      printf '%s' '{{"objects":{{"bleibt":{{"uuid":"U3","type":"And"}}}},"removed":{{"U1":{{"slug":"licht","type":"Or"}}}}}}' > "$lock"
    fi
    cat pages/main.loxl base.Loxone > "$out" ;;
  'logic test')
    if [ -f '{dir}/failtest' ]; then echo 'test "t" ... FAILED'; echo '1 tests: 0 passed, 1 failed'; exit 1; fi
    echo 'no tests in pages' ;;
  'logic diff') echo 'Remove the light' ;;
  'config push')
    [ -f '{dir}/pushfail' ] && {{ echo 'Downloading the live config...' >&2; echo 'Error: no answer' >&2; exit 1; }}
    [ -f '{dir}/uploadfail' ] && {{ echo 'Downloading the live config...' >&2; echo 'Uploading as sps_new.zip...' >&2; echo 'Restarting the program...' >&2; echo 'Error: the Miniserver did not load the upload' >&2; exit 1; }}
    lost logic.lock.json && {{ printf 'Error: Compiling the module failed

Caused by:
    compile error: {refusal}
' >&2; exit 1; }}
    echo 'Downloading the live config...' >&2
    echo 'Uploading as sps_new.zip...' >&2
    echo 'Restarting the program...' >&2
    echo 'Downloading the config again to verify...' >&2
    cp out.Loxone base.Loxone
    echo 'Pushed and verified.' >&2 ;;
  'config download')
    [ -f '{dir}/download-fails' ] && {{ echo 'Error: FTP: connection refused' >&2; exit 1; }}
    cp '{dir}/runs.zip' "$4" ;;
  *) exit 2 ;;
esac
"#,
                calls = calls.display(),
                dir = tmp.path().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = Actions::new(Settings {
            token: Some("t".into()),
            project: Some(d.clone()),
            lox,
            running: Arc::new(crate::running::Running::new(Err("test".into()))),
            probe: None,
        });
        (tmp, d, a)
    }

    fn calls(tmp: &tempfile::TempDir) -> String {
        std::fs::read_to_string(tmp.path().join("calls")).unwrap_or_default()
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    fn step(a: &Actions, id: &str) -> (String, String) {
        let info = a.info();
        let s = info["progress"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()
            .clone();
        (
            s["state"].as_str().unwrap().to_string(),
            s["detail"].as_str().unwrap_or("").to_string(),
        )
    }

    fn consent(plan: &RemovalPlan) -> ApplyOpts {
        ApplyOpts {
            allow_removals: Some(RemovalConsent {
                reviewed: plan.blob.clone(),
                blocks: plan.blocks.iter().map(|b| b.uuid.clone()).collect(),
            }),
            ..Default::default()
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn apply_never_removes_a_block_unasked() {
        let (tmp, d, a) = project();
        let (status, e) = a.apply(&ApplyOpts::default()).await.err().unwrap();
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(e.contains("lox deletes them only when you allow it"), "{e}");
        // The browser learns which block, to ask for the plan.
        assert_eq!(apply_hints(&e)["removals"], "licht");
        assert_eq!(step(&a, "removals").0, "fail");
        assert_eq!(step(&a, "tests").0, "wait");
        // Nothing was pushed, and nothing compiled into the project.
        let c = calls(&tmp);
        assert!(
            !c.contains("config push") && !c.contains("--allow-removals"),
            "{c}"
        );
        assert_eq!(read(&d.join("logic.lock.json")), LOCK);
        assert_eq!(read(&d.join("out.Loxone")), "compiled before\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removals_are_deployed_only_as_confirmed() {
        let (tmp, d, a) = project();
        let lock = d.join("logic.lock.json");
        // The plan names the block, and changes nothing.
        let plan = a.removal_plan().await.unwrap();
        assert_eq!(
            plan.blocks,
            vec![Removed {
                slug: "licht".into(),
                uuid: "U1".into(),
                ty: "Or".into(),
                deployed: true
            }]
        );
        assert_eq!(plan.compiled, format!("bleibt = And()\n{BASE}").as_bytes());
        assert_eq!(read(&lock), LOCK);
        assert_eq!(read(&d.join("out.Loxone")), "compiled before\n");
        let unchanged = |what: &str| {
            assert_eq!(read(&lock), LOCK, "{what}");
            assert_eq!(read(&d.join("base.Loxone")), BASE, "{what}");
            assert!(!calls(&tmp).contains("config push"), "{what}");
        };
        // Other blocks than the plan's: refused.
        let mut other = consent(&plan);
        other.allow_removals.as_mut().unwrap().blocks = vec!["U1".into(), "U3".into()];
        let e = a.apply(&other).await.err().unwrap().1;
        assert!(e.contains("not the ones the plan removes"), "{e}");
        unchanged("other blocks");
        // Another program than the plan's: refused.
        let mut other = consent(&plan);
        other.allow_removals.as_mut().unwrap().reviewed = "0".repeat(40);
        let e = a.apply(&other).await.err().unwrap().1;
        assert!(e.contains("not the ones vislox worked out last"), "{e}");
        unchanged("another program");
        // The module changed since the plan: refused, and nothing written.
        std::fs::write(d.join("pages/main.loxl"), "bleibt = And()\nneu = Or()\n").unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(e.contains("changed since you confirmed"), "{e}");
        assert_eq!(read(&d.join("out.Loxone")), "compiled before\n");
        unchanged("module changed");
        // Confirmed as planned: compiled in, then pushed without the flag.
        let plan = a.removal_plan().await.unwrap();
        let msg = a.apply(&consent(&plan)).await.unwrap();
        assert!(msg.contains("Committed: Remove the light"), "{msg}");
        let (state, detail) = step(&a, "removals");
        assert_eq!(state, "ok");
        assert!(detail.contains("1 block removed"), "{detail}");
        let c = calls(&tmp);
        let push: Vec<&str> = c.lines().filter(|l| l.starts_with("config push")).collect();
        assert_eq!(push.len(), 1, "{c}");
        assert!(!push[0].contains("--allow-removals"), "{c}");
        let t = plan.time.to_string();
        assert!(
            c.lines()
                .any(|l| l == format!("logic compile --time {t} --allow-removals")),
            "{c}"
        );
        // The lockfile keeps the removal; the Miniserver runs the plan.
        assert!(
            read(&lock).contains(r#""removed":{"U1""#),
            "{}",
            read(&lock)
        );
        assert_eq!(std::fs::read(d.join("base.Loxone")).unwrap(), plan.compiled);
        // The consent was for that plan only: sent again, it is refused.
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(e.contains("not the ones vislox worked out last"), "{e}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_compile_that_is_not_the_plan_is_put_back() {
        let (tmp, d, a) = project();
        let plan = a.removal_plan().await.unwrap();
        // lox, now, writes another out in the project than in the plan.
        let lox = tmp.path().join("lox");
        let s = read(&lox).replace(
            "cat pages/main.loxl base.Loxone > \"$out\" ;;",
            "cat pages/main.loxl base.Loxone > \"$out\"; if [ \"$out\" = out.Loxone ]; then echo x >> out.Loxone; fi ;;",
        );
        std::fs::write(&lox, s).unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(e.contains("not the one you reviewed"), "{e}");
        assert_eq!(read(&d.join("logic.lock.json")), LOCK);
        assert_eq!(read(&d.join("out.Loxone")), "compiled before\n");
        assert!(!calls(&tmp).contains("config push"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_plan_needs_a_removal_and_a_roll_back_takes_no_consent() {
        let (_tmp, d, a) = project();
        let plan = a.removal_plan().await.unwrap();
        let (status, e) = a
            .rollback("HEAD", None, &consent(&plan))
            .await
            .err()
            .unwrap();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(e.contains("never removes blocks"), "{e}");
        std::fs::write(d.join("pages/main.loxl"), "bleibt = And()\nlicht = Or()\n").unwrap();
        let e = a.removal_plan().await.err().unwrap();
        assert!(e.contains("removes no managed block"), "{e}");
    }

    #[test]
    fn lox_refusal_names_the_block() {
        let m = "lox config push failed:\nError: Compiling the module failed\n\nCaused by:\n    \
                 compile error: managed block `licht` is in the lockfile but not in the source. \
                 To delete it, add `removed licht` (or pass --allow-removals); to rename it, add \
                 `moved licht -> <new_slug>`.";
        assert_eq!(refused_slug(m).as_deref(), Some("licht"));
        assert_eq!(refused_slug("managed block `x` moved"), None);
        assert_eq!(refused_slug("nothing"), None);
    }

    #[test]
    fn the_blocks_removed_are_read_from_the_lockfiles() {
        let a = br#"{"objects":{"licht":{"uuid":"u1","type":"Or"},"neu":{"uuid":"u2","type":"And"},"bleibt":{"uuid":"u3","type":"And"}},"removed":{"u9":{"slug":"alt","type":"Or"}}}"#;
        let b = br#"{"objects":{"bleibt":{"uuid":"u3","type":"And"}},"removed":{"u1":{"slug":"licht","type":"Or"},"u9":{"slug":"alt","type":"Or"},"u8":{"slug":"weg","type":"Or"}}}"#;
        // The base runs u1, u3 and u9; u8 is gone already.
        let base = br#"<C U="u1"/><C U="u3"/><C U="u9"/>"#;
        let r = removals(Some(a), Some(b), base);
        let got: Vec<(&str, bool)> = r.iter().map(|b| (b.slug.as_str(), b.deployed)).collect();
        // A tombstone from before is deleted by the push too: it is listed.
        assert_eq!(got, [("alt", true), ("licht", true), ("neu", false)]);
        assert_eq!(pending(Some(a), base).len(), 1);
        // No lockfile: nothing managed, nothing removed.
        assert!(removals(None, None, base).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_consent_never_outlives_a_failed_apply() {
        let (tmp, d, a) = project();
        let lock = d.join("logic.lock.json");
        let as_before = |what: &str| {
            assert_eq!(read(&lock), LOCK, "{what}: no tombstone left");
            assert_eq!(read(&d.join("out.Loxone")), "compiled before\n", "{what}");
            assert_eq!(read(&d.join("base.Loxone")), BASE, "{what}");
        };
        // The tests fail after the removals were compiled in: put back.
        std::fs::write(tmp.path().join("failtest"), "").unwrap();
        let plan = a.removal_plan().await.unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(e.contains("1 of 1 test fails"), "{e}");
        assert_eq!(step(&a, "removals").0, "ok");
        as_before("tests failed");
        assert!(!calls(&tmp).contains("config push"));
        std::fs::remove_file(tmp.path().join("failtest")).unwrap();
        // The push fails: put back too.
        std::fs::write(tmp.path().join("pushfail"), "").unwrap();
        let plan = a.removal_plan().await.unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(e.contains("lox config push failed"), "{e}");
        as_before("push failed");
        std::fs::remove_file(tmp.path().join("pushfail")).unwrap();
        // A later Apply without consent removes nothing: it asks again.
        let e = a.apply(&ApplyOpts::default()).await.err().unwrap().1;
        assert_eq!(apply_hints(&e)["removals"], "licht");
        as_before("plain Apply");
        let pushes = calls(&tmp)
            .lines()
            .filter(|l| l.starts_with("config push"))
            .count();
        assert_eq!(pushes, 1, "only the push that failed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_tombstone_in_the_lockfile_is_deleted_only_with_consent() {
        // The lockfile already deletes `licht` (an earlier compile), the
        // module compiles: a plain push would delete it.
        let lock = r#"{"objects":{"bleibt":{"uuid":"U3","type":"And"}},"removed":{"U1":{"slug":"licht","type":"Or"}}}"#;
        let (tmp, d, a) = project_with(lock);
        let e = a.apply(&ApplyOpts::default()).await.err().unwrap().1;
        assert!(
            e.contains(NEEDS_CONSENT) && e.contains("\n  licht (Or)"),
            "{e}"
        );
        assert_eq!(apply_hints(&e)["removals"], "pending");
        assert_eq!(step(&a, "removals").0, "fail");
        assert!(!calls(&tmp).contains("config push"));
        // The plan lists it, and with consent it goes.
        let plan = a.removal_plan().await.unwrap();
        assert_eq!(
            plan.blocks
                .iter()
                .map(|b| (b.slug.as_str(), b.deployed))
                .collect::<Vec<_>>(),
            [("licht", true)]
        );
        a.apply(&consent(&plan)).await.unwrap();
        let c = calls(&tmp);
        assert!(
            c.contains("config push") && !c.contains("config push --force --allow"),
            "{c}"
        );
        assert_eq!(std::fs::read(d.join("base.Loxone")).unwrap(), plan.compiled);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_push_with_removals_that_fails_after_its_upload_keeps_what_runs() {
        let (tmp, d, a) = project();
        let t = tmp.path();
        let lock = d.join("logic.lock.json");
        let as_before = |what: &str| {
            assert_eq!(read(&lock), LOCK, "{what}: no tombstone left");
            assert_eq!(read(&d.join("out.Loxone")), "compiled before\n", "{what}");
            assert_eq!(read(&d.join("base.Loxone")), BASE, "{what}");
        };
        std::fs::write(t.join("uploadfail"), "").unwrap();
        // vislox cannot tell what runs: it says so, and puts the project back.
        std::fs::write(t.join("download-fails"), "").unwrap();
        let plan = a.removal_plan().await.unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(e.contains("did not load the upload"), "{e}");
        assert!(
            e.contains("cannot tell which config the Miniserver runs now (lox config download: Error: FTP: connection refused)"),
            "{e}"
        );
        assert!(
            e.contains("\"Which config the Miniserver runs\" in the History, then Pull or Sync"),
            "{e}"
        );
        as_before("download failed");
        std::fs::remove_file(t.join("download-fails")).unwrap();
        // It runs the previous program, but an upload waits to load: the same.
        std::fs::write(
            t.join("runs.zip"),
            crate::actions::tests::zip_of(BASE.as_bytes()),
        )
        .unwrap();
        *a.waiting.lock().unwrap() = Some(Ok(Some("sps_new.zip".into())));
        let plan = a.removal_plan().await.unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(
            e.contains("it runs the previous program, but sps_new.zip waits to be loaded"),
            "{e}"
        );
        as_before("upload waiting");
        // It runs the previous program, nothing waits: nothing changed, put back.
        *a.waiting.lock().unwrap() = Some(Ok(None));
        let plan = a.removal_plan().await.unwrap();
        let e = a.apply(&consent(&plan)).await.err().unwrap().1;
        assert!(
            e.contains("still runs the previous config, and no upload is waiting"),
            "{e}"
        );
        as_before("nothing changed");
        assert!(a.info()["last"]["ok"] != true);
        // It runs the program confirmed: kept, recorded as deployed, not verified.
        let plan = a.removal_plan().await.unwrap();
        std::fs::write(
            t.join("runs.zip"),
            crate::actions::tests::zip_of(&plan.compiled),
        )
        .unwrap();
        let msg = a.apply(&consent(&plan)).await.unwrap();
        assert!(
            msg.contains("Not verified: lox failed after the upload, but the Miniserver runs the program you confirmed"),
            "{msg}"
        );
        assert_eq!(a.info()["last"]["warn"], true);
        assert_eq!(std::fs::read(d.join("base.Loxone")).unwrap(), plan.compiled);
        assert!(
            read(&lock).contains(r#""removed":{"U1""#),
            "{}",
            read(&lock)
        );
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .args(args)
                .current_dir(&d)
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        assert_eq!(git(&["status", "--porcelain"]), "");
        assert_eq!(git(&["log", "-1", "--format=%s"]), "Remove the light");
    }
}
