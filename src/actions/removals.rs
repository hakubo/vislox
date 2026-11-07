//! Managed blocks the module no longer has (#70). lox refuses to compile a
//! module that lost a block its lockfile manages, unless the removal is
//! allowed: `removed <slug>` in the module, or `--allow-removals`, which
//! deletes *every* vanished block. Apply never passes that flag by default,
//! nor again on a retry by itself:
//!
//! - **The plan** ([`Actions::removal_plan`]) changes nothing: the module
//!   compiled in a scratch directory with `--allow-removals` and a fixed
//!   `--time`, onto a copy of the lockfile. The blocks it removes are the
//!   lockfile's objects the compile drops, each with the UUID the lockfile
//!   pins. The browser shows the result as a revision and lists the blocks.
//! - **The consent** names the plan's program (its blob id) and exactly its
//!   blocks. Apply works the plan out again from the files as they are now,
//!   and refuses unless both are the same.
//! - **What is deployed is what was confirmed.** Apply then compiles in the
//!   project with `--allow-removals` and the plan's time: `out` must be the
//!   confirmed program, and the lockfile must have dropped exactly the
//!   confirmed blocks (lox keeps a tombstone for each one the Miniserver
//!   runs). Otherwise the lockfile and `out` are put back. The tests then run
//!   on it, and `lox config push` deletes the tombstoned blocks *without*
//!   `--allow-removals`: a block that vanishes after the confirmation is
//!   refused by lox again, not deleted.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// How long a plan can be confirmed (seconds): then it is worked out again.
pub const PLAN_TTL: i64 = 15 * 60;

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

/// A managed block a compile with `--allow-removals` drops.
#[derive(Clone, Serialize, PartialEq, Debug)]
pub struct Removed {
    pub slug: String,
    pub uuid: String,
    /// The block type the lockfile records.
    #[serde(rename = "type")]
    pub ty: String,
    /// The base has it, so the Miniserver runs it: lox keeps a tombstone and
    /// deletes it there. Else it was never deployed, and only the lockfile
    /// forgets it.
    pub deployed: bool,
}

/// What a compile that allows removals deploys ([`Actions::removal_plan`]).
#[derive(Clone)]
pub struct RemovalPlan {
    pub blocks: Vec<Removed>,
    /// The module compiled with the removals allowed.
    pub compiled: Vec<u8>,
    /// The `--time` it was compiled with, so that the deploy's compile mints
    /// the same UUIDs for blocks the module adds.
    time: i64,
    /// When it was worked out.
    at: i64,
}

impl RemovalPlan {
    pub fn blob(&self) -> String {
        crate::history::blob_id(&self.compiled)
    }
}

/// The user's consent to the removals of a plan, with Apply.
#[derive(serde::Deserialize, Clone, Debug, Default)]
pub struct RemovalConsent {
    /// The blob id of the plan's program the user reviewed.
    pub reviewed: String,
    /// The UUIDs of the blocks the user confirmed, exactly the plan's.
    pub blocks: Vec<String>,
}

/// A lockfile's managed objects (slug → UUID and type) and tombstones (UUIDs).
fn lock_objects(path: &Path) -> (BTreeMap<String, (String, String)>, BTreeSet<String>) {
    let v: Value = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let objects = v["objects"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(s, x)| {
                    let uuid = x["uuid"].as_str()?.to_string();
                    let ty = x["type"].as_str().unwrap_or("").to_string();
                    Some((s.clone(), (uuid, ty)))
                })
                .collect()
        })
        .unwrap_or_default();
    let removed = v["removed"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    (objects, removed)
}

/// The blocks a compile dropped: in the lockfile `before`, not in `after`.
fn dropped(before: &Path, after: &Path) -> Vec<Removed> {
    let (was, _) = lock_objects(before);
    let (now, tombs) = lock_objects(after);
    was.into_iter()
        .filter(|(slug, _)| !now.contains_key(slug))
        .map(|(slug, (uuid, ty))| Removed {
            deployed: tombs.contains(&uuid),
            slug,
            uuid,
            ty,
        })
        .collect()
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
    std::fs::create_dir(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    Ok(d)
}

fn uuids(blocks: &[Removed]) -> BTreeSet<&str> {
    blocks.iter().map(|b| b.uuid.as_str()).collect()
}

impl Actions {
    /// What Apply would deploy if the removals were allowed (see the
    /// module's documentation). Runs while no action runs, holding the
    /// compile lock; changes nothing in the project. Kept for
    /// [`PLAN_TTL`], for the consent to name.
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
        let plan = self.plan_removals(&p, now()).await?;
        if plan.blocks.is_empty() {
            return Err(
                "The module removes no managed block: there is nothing to allow.".to_string(),
            );
        }
        *self.removals.lock().unwrap() = Some(plan.clone());
        Ok(plan)
    }

    /// The module compiled with `--allow-removals --time <time>` onto a
    /// scratch copy of the lockfile, and the blocks it drops.
    async fn plan_removals(
        &self,
        p: &loxone_config::Project,
        time: i64,
    ) -> Result<RemovalPlan, String> {
        let dir = self.s.project.as_deref().unwrap();
        let scratch = scratch_dir("vislox-removals")?;
        let _gone = RemoveDir(scratch.clone());
        let (lock, out) = (scratch.join("lock.json"), scratch.join("out.Loxone"));
        if p.lock.exists() {
            std::fs::copy(&p.lock, &lock).map_err(|e| format!("cannot copy the lock: {e}"))?;
        }
        let (l, o, t) = (
            lock.to_string_lossy(),
            out.to_string_lossy(),
            time.to_string(),
        );
        let args = [
            "logic",
            "compile",
            "--lock",
            &l,
            "--out",
            &o,
            "--time",
            &t,
            "--allow-removals",
        ];
        run(&self.s.lox, &args, dir, COMPILE_TIMEOUT)
            .await
            .map_err(|e| {
                format!(
                    "The module does not compile even with its removals allowed:\n{}",
                    last_lines(&e, 6)
                )
            })?;
        let compiled =
            std::fs::read(&out).map_err(|e| format!("lox logic compile wrote no config: {e}"))?;
        Ok(RemovalPlan {
            blocks: dropped(&p.lock, &lock),
            compiled,
            time,
            at: now(),
        })
    }

    /// Apply's *Removals* step. Without consent: refuse when lox would refuse
    /// the module for a block it lost (a scratch compile, as the check's),
    /// with lox's words, so that the browser asks. With it: the plan worked
    /// out again must be the confirmed one, and is compiled into the project
    /// (see the module's documentation). Returns the step's state and detail.
    pub(super) async fn removals_step(
        &self,
        consent: Option<&RemovalConsent>,
    ) -> Result<(&'static str, String), String> {
        let dir = self.s.project.as_deref().unwrap();
        let p = self.project()?;
        let Some(c) = consent else {
            return match self.freshness(&p, dir).await {
                Ok(Fresh::Broken { error, .. }) if refused_slug(&error).is_some() => Err(format!(
                    "The module no longer has blocks the Miniserver runs, and lox deletes them \
                     only when you allow it. Nothing was deployed: review the removals, then \
                     apply.\n{}",
                    last_lines(&error, 3)
                )),
                _ => Ok(("skip", "the module removes no managed block".into())),
            };
        };
        let kept = self.removals.lock().unwrap().clone();
        let Some(kept) = kept.filter(|k| k.blob() == c.reviewed && now() - k.at < PLAN_TTL) else {
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
        // Worked out again from the files as they are now.
        let again = self.plan_removals(&p, kept.time).await?;
        if again.blob() != c.reviewed || uuids(&again.blocks) != confirmed {
            return Err(format!(
                "The module, its lockfile or the base changed since you confirmed the removals \
                 (the plan is now {}, you reviewed {}). Nothing was deployed: review the removals \
                 again.",
                &again.blob()[..10],
                &c.reviewed[..10.min(c.reviewed.len())]
            ));
        }
        // In the project, with the plan's time: out must be what was confirmed.
        snapshot_before_edit(dir, &p).await;
        let module = module_blobs(&p);
        let base = file_blob(&p.base);
        let (lock_was, out_was) = (std::fs::read(&p.lock).ok(), std::fs::read(&p.out).ok());
        let put_back = || {
            for (path, was) in [(&p.lock, &lock_was), (&p.out, &out_was)] {
                let _ = match was {
                    Some(b) => std::fs::write(path, b),
                    None => std::fs::remove_file(path),
                };
            }
        };
        let scratch = scratch_dir("vislox-removals-lock")?;
        let _gone = RemoveDir(scratch.clone());
        let before = scratch.join("lock.json");
        if let Some(b) = &lock_was {
            std::fs::write(&before, b).map_err(|e| format!("cannot copy the lock: {e}"))?;
        }
        let t = kept.time.to_string();
        let args = ["logic", "compile", "--time", &t, "--allow-removals"];
        if let Err(e) = run(&self.s.lox, &args, dir, COMPILE_TIMEOUT).await {
            put_back();
            return Err(format!(
                "lox logic compile --allow-removals failed, so nothing was deployed:\n{}",
                last_lines(&e, 6)
            ));
        }
        let out = file_blob(&p.out);
        let gone = dropped(&before, &p.lock);
        if out.as_deref() != Some(c.reviewed.as_str()) || uuids(&gone) != confirmed {
            put_back();
            return Err(format!(
                "The compile with the removals allowed is not the one you confirmed (out {}, \
                 {} blocks dropped). The lockfile and out were put back; nothing was deployed.",
                out.as_deref().map(|o| &o[..10]).unwrap_or("missing"),
                gone.len()
            ));
        }
        if module_blobs(&p) == module
            && file_blob(&p.base) == base
            && let Some(out) = out
        {
            let lock = file_blob(&p.lock);
            *self.compiled.lock().unwrap() = Some(Compiled {
                module,
                out,
                base,
                lock,
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
        ))
    }
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
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
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("project");
        std::fs::create_dir_all(d.join("pages")).unwrap();
        std::fs::write(d.join("base.Loxone"), "base\n").unwrap();
        std::fs::write(d.join("out.Loxone"), "compiled before\n").unwrap();
        std::fs::write(d.join("logic.lock.json"), LOCK).unwrap();
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
  'logic test') echo 'no tests in pages' ;;
  'logic diff') echo 'Remove the light' ;;
  'config push')
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
  *) exit 2 ;;
esac
"#,
                calls = calls.display()
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
                reviewed: plan.blob(),
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
        assert_eq!(plan.compiled, b"bleibt = And()\nbase\n");
        assert_eq!(read(&lock), LOCK);
        assert_eq!(read(&d.join("out.Loxone")), "compiled before\n");
        let unchanged = |what: &str| {
            assert_eq!(read(&lock), LOCK, "{what}");
            assert_eq!(read(&d.join("base.Loxone")), "base\n", "{what}");
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
        assert!(e.contains("not the one you confirmed"), "{e}");
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
    fn the_blocks_dropped_are_read_from_the_lockfiles() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a.json"), t.path().join("b.json"));
        std::fs::write(
            &a,
            r#"{"objects":{"licht":{"uuid":"u1","type":"Or"},"neu":{"uuid":"u2","type":"And"},"bleibt":{"uuid":"u3","type":"And"}}}"#,
        )
        .unwrap();
        std::fs::write(
            &b,
            r#"{"objects":{"bleibt":{"uuid":"u3","type":"And"}},"removed":{"u1":{"slug":"licht","type":"Or"}}}"#,
        )
        .unwrap();
        let d = dropped(&a, &b);
        assert_eq!(
            d,
            vec![
                Removed {
                    slug: "licht".into(),
                    uuid: "u1".into(),
                    ty: "Or".into(),
                    deployed: true
                },
                Removed {
                    slug: "neu".into(),
                    uuid: "u2".into(),
                    ty: "And".into(),
                    deployed: false
                },
            ]
        );
        // No lockfile: nothing managed, nothing dropped.
        assert!(dropped(&t.path().join("none"), &b).is_empty());
    }
}
