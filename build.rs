//! The commit vislox is built from, for `--version` and the UI:
//! `VISLOX_COMMIT` from the environment (the Docker build, which has no
//! .git), else `git rev-parse`. None outside a git checkout.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=VISLOX_COMMIT");
    let commit = match std::env::var("VISLOX_COMMIT") {
        Ok(c) if !c.trim().is_empty() => Some(c.trim().chars().take(7).collect::<String>()),
        _ => {
            // Rebuild when HEAD moves (a commit, a checkout).
            for p in [
                git(&["rev-parse", "--git-path", "HEAD"]),
                git(&["rev-parse", "--git-path", "refs/heads"]),
                git(&["rev-parse", "--git-path", "packed-refs"]),
            ]
            .into_iter()
            .flatten()
            {
                if std::path::Path::new(&p).exists() {
                    println!("cargo:rerun-if-changed={p}");
                }
            }
            git(&["rev-parse", "--short=7", "HEAD"])
        }
    };
    match commit {
        Some(c) => {
            println!("cargo:rustc-env=VISLOX_COMMIT={c}");
            println!("cargo:rustc-env=VISLOX_COMMIT_SUFFIX= ({c})");
        }
        None => println!("cargo:rustc-env=VISLOX_COMMIT_SUFFIX="),
    }
}
