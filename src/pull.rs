//! Pull: fetch the Miniserver's current config from the UI, with
//! `lox config pull` (FTP download, decompress, git commit).
//!
//! vislox itself still writes nothing: lox does the download and the commit,
//! and vislox picks the new commit up like any other. A pull is offered only
//! when lox would commit to the repository vislox shows, into the folder of
//! the Miniserver vislox shows: it reads from the Miniserver and never sends
//! anything to it.

use crate::live::Found;
use crate::source::Source;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A download that takes longer than this has hung.
const TIMEOUT: Duration = Duration::from_secs(300);

pub struct Pull {
    cmd: Result<Cmd, String>,
    /// Offer the button (perhaps disabled): a config repository, pulls not
    /// turned off.
    shown: bool,
    running: tokio::sync::Mutex<()>,
}

struct Cmd {
    lox: PathBuf,
    /// The context vislox picked, so lox uses the same one.
    ctx: Option<String>,
    /// lox looks for a project-local config from here, as vislox did.
    cwd: PathBuf,
}

/// What a pull did.
pub struct Pulled {
    /// A new config was committed.
    pub committed: bool,
}

impl Pull {
    pub fn new(
        lox: &Path,
        off: Option<&str>,
        src: &Source,
        found: Result<&Found, &String>,
        start: &Path,
    ) -> Pull {
        Pull {
            cmd: check(lox, off, src, found, start),
            shown: off.is_none() && src.miniserver.is_some(),
            running: tokio::sync::Mutex::new(()),
        }
    }

    /// For `/api/history`: whether pulls can be had, and why not.
    pub fn info(&self) -> Value {
        match &self.cmd {
            Ok(_) => json!({ "available": true, "shown": true }),
            Err(why) => json!({ "available": false, "shown": self.shown, "reason": why }),
        }
    }

    /// A pull is running.
    pub fn busy(&self) -> bool {
        self.running.try_lock().is_err()
    }

    pub fn why_not(&self) -> Option<&str> {
        self.cmd.as_ref().err().map(String::as_str)
    }

    /// Run `lox config pull`; one at a time.
    pub async fn run(&self) -> Result<Pulled, String> {
        let cmd = self.cmd.as_ref().map_err(Clone::clone)?;
        let _one = self
            .running
            .try_lock()
            .map_err(|_| "a pull is already running".to_string())?;
        let mut c = tokio::process::Command::new(&cmd.lox);
        if let Some(ctx) = &cmd.ctx {
            c.args(["--ctx", ctx]);
        }
        c.args(["config", "pull", "--quiet"])
            // The Miniserver's folder may be gone (a reset): its nearest
            // existing parent finds the same project-local lox config.
            .current_dir(cmd.cwd.ancestors().find(|d| d.is_dir()).unwrap_or(&cmd.cwd))
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let out = tokio::time::timeout(TIMEOUT, output_retrying(&mut c))
            .await
            .map_err(|_| format!("lox config pull took longer than {} s", TIMEOUT.as_secs()))?
            .map_err(|e| format!("cannot run {}: {e}", cmd.lox.display()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = [stderr.trim(), stdout.trim()]
                .into_iter()
                .find(|m| !m.is_empty())
                .unwrap_or("no output")
                .to_string();
            return Err(format!("lox config pull failed: {msg}"));
        }
        // With --quiet, lox prints `committed` when it made a commit.
        Ok(Pulled {
            committed: stdout.lines().any(|l| l.trim() == "committed"),
        })
    }
}

fn check(
    lox: &Path,
    off: Option<&str>,
    src: &Source,
    found: Result<&Found, &String>,
    start: &Path,
) -> Result<Cmd, String> {
    if let Some(why) = off {
        return Err(why.to_string());
    }
    let ms = src.miniserver.as_deref().ok_or(
        "only for a `lox config init` repository (this is a single file or a lox.toml project)",
    )?;
    let f = found.map_err(|e| format!("no lox configuration: {e}"))?;
    let repo = f
        .config_repo
        .as_ref()
        .ok_or("the lox configuration names no config repository (run `lox config init <path>`)")?;
    let here = src
        .current
        .parent()
        .and_then(Path::parent)
        .ok_or("cannot tell the repository of the config")?;
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same(repo, here) {
        return Err(format!(
            "lox pulls into {}, not into {}",
            repo.display(),
            here.display()
        ));
    }
    if !f.ms_dir.eq_ignore_ascii_case(ms) {
        return Err(format!(
            "lox pulls Miniserver {} ({}), not {ms}",
            f.ms_dir, f.conn.host
        ));
    }
    Ok(Cmd {
        lox: lox.to_path_buf(),
        ctx: f.ctx.clone(),
        cwd: start.to_path_buf(),
    })
}

/// Run `c`, retrying briefly while the binary is "busy" (ETXTBSY): on
/// Linux, a file just written can still be held open for writing by a child
/// another thread forked at that moment, and executing it fails until that
/// child execs.
pub(crate) async fn output_retrying(
    c: &mut tokio::process::Command,
) -> std::io::Result<std::process::Output> {
    let mut tries = 0;
    loop {
        match c.output().await {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            res => return res,
        }
    }
}

/// Make an explicit relative executable path absolute before commands change
/// directories. Bare names stay bare so each spawn resolves them on `PATH`.
pub(crate) fn resolve_lox(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() || path.components().count() == 1 {
        return Ok(path.to_path_buf());
    }
    resolve_lox_from(path, &std::env::current_dir()?)
}

fn resolve_lox_from(path: &Path, cwd: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() || path.components().count() == 1 {
        Ok(path.to_path_buf())
    } else {
        Ok(cwd.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loxone_client::Connection;

    #[cfg(unix)]
    #[test]
    fn explicit_relative_executable_runs_after_changing_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let startup = tmp.path().join("startup");
        let elsewhere = tmp.path().join("project");
        std::fs::create_dir_all(startup.join("bin")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let explicit = startup.join("bin/lox");
        std::fs::write(&explicit, "#!/bin/sh\nprintf ran").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&explicit, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let resolved = resolve_lox_from(Path::new("./bin/lox"), &startup).unwrap();
        let child = std::process::Command::new(resolved)
            .current_dir(&elsewhere)
            .output()
            .unwrap();
        assert!(child.status.success());
        assert_eq!(child.stdout, b"ran");
    }

    #[test]
    fn bare_executable_name_stays_bare() {
        assert_eq!(
            resolve_lox_from(Path::new("lox"), Path::new("/startup")).unwrap(),
            PathBuf::from("lox")
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_executable_does_not_need_path_or_current_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let executable = tmp.path().join("lox");
        std::fs::write(&executable, "#!/bin/sh\nprintf ran").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let resolved = resolve_lox(&executable).unwrap();
        let child = std::process::Command::new(resolved)
            .env_clear()
            .output()
            .unwrap();
        assert!(child.status.success());
        assert_eq!(child.stdout, b"ran");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_symlink_path_is_not_canonicalized() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let target = dir.join("real-lox");
        let link = dir.join("lox-link");
        std::fs::write(&target, "target").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let resolved = resolve_lox_from(Path::new("./lox-link"), dir).unwrap();
        assert_eq!(resolved, dir.join("./lox-link"));
        assert_ne!(resolved, target);
    }

    fn setup() -> (tempfile::TempDir, Source, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("cfg");
        std::fs::create_dir_all(repo.join("192_168_1_120")).unwrap();
        std::fs::write(repo.join("192_168_1_120/config.Loxone"), "x").unwrap();
        let src = Source::resolve(&repo, None).unwrap();
        // A stand-in for lox: any existing file will do for the checks.
        let lox = tmp.path().join("lox");
        std::fs::write(&lox, "").unwrap();
        (tmp, src, lox)
    }

    fn found(repo: Option<&Path>, ms_dir: &str) -> Found {
        Found {
            conn: Connection::new("https://192.168.1.120", "u", "p"),
            from: "test".into(),
            ctx: Some("home".into()),
            config_repo: repo.map(Path::to_path_buf),
            ms_dir: ms_dir.into(),
            serial: None,
        }
    }

    #[test]
    fn a_pull_needs_lox_to_commit_where_vislox_looks() {
        let (tmp, src, lox) = setup();
        let repo = tmp.path().join("cfg");
        let start = src.current.parent().unwrap().to_path_buf();
        let ok = found(Some(&repo), "192_168_1_120");
        let cmd = check(&lox, None, &src, Ok(&ok), &start).unwrap();
        assert_eq!(cmd.ctx.as_deref(), Some("home"));

        let why = |f: &Found| check(&lox, None, &src, Ok(f), &start).err().unwrap();
        assert!(why(&found(None, "192_168_1_120")).contains("lox config init"));
        assert!(why(&found(Some(tmp.path()), "192_168_1_120")).contains("not into"));
        assert!(why(&found(Some(&repo), "504F94000001")).contains("not 192_168_1_120"));
        // Existence is checked when the command runs. In particular, bare
        // names must remain available for PATH lookup at each spawn.
        let missing = Path::new("not-installed-yet");
        assert!(check(missing, None, &src, Ok(&ok), &start).is_ok());
        assert!(
            check(&lox, Some("off"), &src, Ok(&ok), &start)
                .err()
                .unwrap()
                .contains("off")
        );
        let err = "no config".to_string();
        assert!(
            check(&lox, None, &src, Err(&err), &start)
                .err()
                .unwrap()
                .contains("no config")
        );
    }

    #[test]
    fn a_single_file_cannot_be_pulled() {
        let (tmp, _, lox) = setup();
        let file =
            Source::resolve(&tmp.path().join("cfg/192_168_1_120/config.Loxone"), None).unwrap();
        let f = found(Some(&tmp.path().join("cfg")), "192_168_1_120");
        assert!(
            check(&lox, None, &file, Ok(&f), tmp.path())
                .err()
                .unwrap()
                .contains("only for")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_pull_reports_a_commit_and_failures() {
        use std::os::unix::fs::PermissionsExt;
        let (tmp, src, lox) = setup();
        let start = src.current.parent().unwrap().to_path_buf();
        let f = found(Some(&tmp.path().join("cfg")), "192_168_1_120");
        let script = |body: &str| {
            std::fs::write(&lox, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
            Pull::new(&lox, None, &src, Ok(&f), &start)
        };
        let p =
            script(r#"[ "$1 $2 $3 $4 $5" = "--ctx home config pull --quiet" ] && echo committed"#);
        assert!(p.run().await.unwrap().committed);
        let p = script("exit 0");
        assert!(!p.run().await.unwrap().committed);
        let p = script("echo 'FTP login failed' >&2; exit 1");
        assert!(p.run().await.err().unwrap().contains("FTP login failed"));
    }
}
