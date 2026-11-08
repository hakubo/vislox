//! Discard (put the module, lock and compiled config back to the last
//! commit), and what it reads first to put back when a step fails: the
//! snapshot of the lock and out from before vislox's first edit, every path
//! it may change ([`Was`], [`Rollback`], shared with Roll back) and the git
//! index ([`GitIndex`]). See `docs/editing.md`.

use super::*;

/// The lock and out as they were before vislox's first edit since the last
/// commit. Discard puts them back where the commit does not have them: a
/// lock nobody committed (or that git ignores) is never deleted unless
/// vislox's edit made it, since a lost lock re-mints every managed block.
struct Snapshot {
    lock: Option<Vec<u8>>,
    out: Option<Vec<u8>>,
}

/// vislox's directory `name` in the git directory, one per project (by its
/// path): two projects of one repository never share it.
pub(super) async fn project_git_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let gp = git(dir, &["rev-parse", "--git-path", name]).await.ok()?;
    let id = crate::history::blob_id(dir.canonicalize().ok()?.to_string_lossy().as_bytes());
    Some(dir.join(gp.trim()).join(&id[..16]))
}

/// Where the snapshot is kept: in the git directory, one per project, with
/// the commit it was taken on (`head`).
async fn snapshot_dir(dir: &Path) -> Option<PathBuf> {
    project_git_path(dir, "vislox-discard").await
}

async fn head_commit(dir: &Path) -> String {
    git(dir, &["rev-parse", "-q", "--verify", "HEAD"])
        .await
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| "none".into())
}

/// Take the snapshot, unless one was taken on this commit already.
pub(super) async fn snapshot_before_edit(dir: &Path, p: &loxone_config::Project) {
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

/// A path as Discard or Roll back found it, read without following a link.
#[derive(PartialEq)]
enum Was {
    Missing,
    Dir,
    File(Vec<u8>, std::fs::Permissions),
    Link(PathBuf),
}

impl Was {
    fn read(path: &Path) -> Result<Was, String> {
        let m = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Was::Missing),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let t = m.file_type();
        if t.is_symlink() {
            std::fs::read_link(path)
                .map(Was::Link)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))
        } else if t.is_dir() {
            Ok(Was::Dir)
        } else if t.is_file() {
            std::fs::read(path)
                .map(|b| Was::File(b, m.permissions()))
                .map_err(|e| format!("cannot read {}: {e}", path.display()))
        } else {
            Err(format!("{} is no file, directory or link", path.display()))
        }
    }

    /// Put `path` back as it was. A file is written in place (its owner and
    /// hard links stay), opened without following a link: a link there now
    /// (git checked one out) is removed first, never written through. The
    /// error does not name `path`: the report does.
    fn put(&self, path: &Path) -> Result<(), String> {
        let now = std::fs::symlink_metadata(path).ok();
        if now.is_some() && Was::read(path).ok().as_ref() == Some(self) {
            return Ok(());
        }
        let clear = || -> Result<(), String> {
            match &now {
                Some(m) if m.is_dir() => std::fs::remove_dir(path),
                Some(_) => std::fs::remove_file(path),
                None => Ok(()),
            }
            .map_err(|e| format!("cannot remove what is there: {e}"))
        };
        match self {
            Was::Missing => clear(),
            Was::Dir => {
                clear()?;
                std::fs::create_dir(path).map_err(|e| format!("cannot create it: {e}"))
            }
            Was::File(bytes, perm) => {
                if now.as_ref().is_some_and(|m| !m.is_file()) {
                    clear()?;
                }
                write_in_place(path, bytes, perm).map_err(|e| format!("cannot write it: {e}"))
            }
            Was::Link(target) => {
                clear()?;
                #[cfg(unix)]
                let res = std::os::unix::fs::symlink(target, path);
                #[cfg(not(unix))]
                let res: std::io::Result<()> = Err(std::io::Error::other(format!(
                    "a link to {}",
                    target.display()
                )));
                res.map_err(|e| format!("cannot create it: {e}"))
            }
        }
    }
}

/// Write `bytes` over the plain file `path` (or create it), never through a
/// link (O_NOFOLLOW), and give it `perm`. A read-only file is made writable
/// for the write.
fn write_in_place(path: &Path, bytes: &[u8], perm: &std::fs::Permissions) -> std::io::Result<()> {
    use std::io::Write;
    let open = || {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.custom_flags(libc::O_NOFOLLOW);
        }
        o.open(path)
    };
    let mut f = match open() {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let m = std::fs::symlink_metadata(path)?;
            if !m.is_file() {
                return Err(e);
            }
            let mut w = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            w.set_readonly(false);
            std::fs::set_permissions(path, w)?;
            open()?
        }
        r => r?,
    };
    f.write_all(bytes)?;
    f.set_permissions(perm.clone())
}

/// `root` and every path under it, parents before children, hidden ones
/// included; a link is listed, not followed.
fn paths_under(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let m = match std::fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot read {}: {e}", root.display())),
    };
    out.push(root.to_path_buf());
    if m.is_dir() {
        let mut names = std::fs::read_dir(root)
            .and_then(|d| {
                d.map(|e| e.map(|e| e.path()))
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| format!("cannot read {}: {e}", root.display()))?;
        names.sort();
        for p in names {
            paths_under(&p, out)?;
        }
    }
    Ok(())
}

/// Refused when `path` is a symbolic link, or reached through one, below
/// the project directory `root` (#81): `what` would write through it, and
/// what it reaches would be changed for good. A path outside the project is
/// refused too.
pub(super) fn no_link(root: &Path, path: &Path, what: &str) -> Result<(), String> {
    let shown = path.strip_prefix(root).unwrap_or(path).display();
    let rel = path
        .strip_prefix(root)
        .map_err(|_| format!("{shown} is outside the project: {what} writes only inside it"))?;
    let mut at = root.to_path_buf();
    for c in rel.components() {
        match c {
            std::path::Component::Normal(n) => at.push(n),
            std::path::Component::CurDir => continue,
            _ => {
                return Err(format!(
                    "{shown} is outside the project: {what} writes only inside it"
                ));
            }
        }
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = at.strip_prefix(root).unwrap_or(&at).display();
                return Err(if at == path {
                    format!(
                        "{shown} is a symbolic link: {what} never writes through one. Put the \
                         file itself there, then try again."
                    )
                } else {
                    format!(
                        "{shown} is reached through a symbolic link ({link}): {what} never \
                         writes through one. Put the directory itself there, then try again."
                    )
                });
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("cannot read {}: {e}", at.display())),
        }
    }
    Ok(())
}

/// Git's lock beside a file of its own (`index.lock`).
fn git_lock(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

/// The git index as Discard found it, byte for byte, so that what is
/// staged anywhere in the repository comes back as it was; with split
/// index on, the shared index file it names too (git may expire that file
/// while Discard runs).
struct GitIndex {
    path: PathBuf,
    was: Option<Vec<u8>>,
    shared: Option<(PathBuf, Vec<u8>)>,
    /// `git ls-files --stage` of it: what is staged, by path.
    staged: String,
    /// The index after Discard's last git command, and what it staged: only
    /// that is Discard's to put back. Anything else is someone else's.
    ours: std::sync::Mutex<(Option<Vec<u8>>, String)>,
}

fn read_opt(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// What is staged, by path (`git ls-files --stage`, every stage).
async fn staged_list(dir: &Path) -> String {
    git(
        dir,
        &["ls-files", "--stage", "-z", "--full-name", "--", ":/"],
    )
    .await
    .unwrap_or_default()
}

/// The paths whose staged entries differ between two `staged_list`s.
fn staged_changes(a: &str, b: &str) -> Vec<String> {
    let by_path = |s: &str| -> std::collections::BTreeMap<String, Vec<String>> {
        let mut m: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for e in s.split('\0').filter(|e| !e.is_empty()) {
            if let Some((head, path)) = e.split_once('\t') {
                m.entry(path.to_string())
                    .or_default()
                    .push(head.to_string());
            }
        }
        m
    };
    let (a, b) = (by_path(a), by_path(b));
    a.keys()
        .chain(b.keys())
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What Discard or Roll back may change, read before it touches anything:
/// the module's tree (a link as the link), the other files, and for
/// Discard the git index. Kept in memory; when putting it back leaves
/// something not as it was, a copy goes to the git directory
/// (`vislox-recovery/`), and the report says where.
pub(super) struct Rollback {
    root: PathBuf,
    module: PathBuf,
    /// The module and everything in it, parents first; then the others.
    paths: Vec<(PathBuf, Was)>,
    index: Option<GitIndex>,
    /// Where a copy goes when one is needed: this project's directory of
    /// copies in the git directory.
    pub(super) copies: Option<PathBuf>,
}

/// What a failed Discard or Roll back could not put back.
pub(super) struct Left {
    what: String,
    why: String,
    /// Its path in the copy, if it had content.
    copy: Option<PathBuf>,
}

impl Rollback {
    /// Discard's: the module, lock and out, and the index. Refused, with
    /// nothing changed, when one cannot be read, the lock, out or module is
    /// (or is reached through) a symbolic link, or git is at work.
    async fn take(dir: &Path, p: &loxone_config::Project) -> Result<Rollback, String> {
        for path in [&p.module, &p.lock, &p.out] {
            no_link(&p.root, path, "Discard")?;
        }
        let mut rb = Rollback::files(&p.root, &p.module, &[&p.lock, &p.out])?;
        for (path, was) in &rb.paths[rb.paths.len() - 2..] {
            if *was == Was::Dir {
                return Err(format!("{} is a directory", path.display()));
            }
        }
        let index = git(dir, &["rev-parse", "--git-path", "index"])
            .await
            .map(|p| dir.join(p.trim()))?;
        let lock = git_lock(&index);
        if std::fs::symlink_metadata(&lock).is_ok() {
            return Err(format!(
                "git is busy in this repository ({} is there): try again when it is done",
                lock.display()
            ));
        }
        let was = read_opt(&index)?;
        let shared = match git(dir, &["rev-parse", "--shared-index-path"]).await {
            Ok(s) if !s.trim().is_empty() => {
                let path = dir.join(s.trim());
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                Some((path, bytes))
            }
            _ => None,
        };
        let staged = staged_list(dir).await;
        rb.index = Some(GitIndex {
            path: index,
            ours: std::sync::Mutex::new((was.clone(), staged.clone())),
            was,
            shared,
            staged,
        });
        rb.copies = project_git_path(dir, "vislox-recovery").await;
        Ok(rb)
    }

    /// The module's tree and `others` (each once), or why one cannot be read.
    pub(super) fn files(root: &Path, module: &Path, others: &[&Path]) -> Result<Rollback, String> {
        let mut tree = Vec::new();
        paths_under(module, &mut tree)?;
        if tree.is_empty() {
            tree.push(module.to_path_buf());
        }
        let mut paths: Vec<(PathBuf, Was)> = Vec::new();
        for path in tree
            .into_iter()
            .chain(others.iter().map(|p| p.to_path_buf()))
        {
            if paths.iter().any(|(q, _)| *q == path) {
                continue;
            }
            let was = Was::read(&path)?;
            paths.push((path, was));
        }
        Ok(Rollback {
            root: root.to_path_buf(),
            module: module.to_path_buf(),
            paths,
            index: None,
            copies: None,
        })
    }

    /// Discard ran a git command: the index as it left it is Discard's.
    async fn mark_index(&self, dir: &Path) {
        if let Some(ix) = &self.index {
            let bytes = std::fs::read(&ix.path).ok();
            let staged = staged_list(dir).await;
            *ix.ours.lock().unwrap() = (bytes, staged);
        }
    }

    /// Where the copy of `path` goes in `copy`: by its path from the
    /// project directory, under `files`.
    fn copy_of(&self, copy: &Path, n: usize, path: &Path) -> PathBuf {
        let files = copy.join("files");
        match path.strip_prefix(&self.root) {
            Ok(rel)
                if rel
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))) =>
            {
                files.join(rel)
            }
            _ => files.join("outside").join(format!(
                "{n}-{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }

    /// Keep a copy of everything as it was, from memory, in a new directory
    /// of `copies`.
    fn keep_copy(&self, what: &str) -> Result<PathBuf, String> {
        let copies = self
            .copies
            .as_ref()
            .ok_or("no place for it in the git directory")?;
        std::fs::create_dir_all(copies).map_err(|e| e.to_string())?;
        let stamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
        let what = what.to_lowercase().replace(' ', "-");
        let copy = (1..)
            .map(|n| {
                copies.join(match n {
                    1 => format!("{stamp}-{what}"),
                    n => format!("{stamp}-{what}-{n}"),
                })
            })
            .find(|c| match std::fs::create_dir(c) {
                Ok(()) => true,
                Err(e) => e.kind() != std::io::ErrorKind::AlreadyExists,
            })
            .filter(|c| c.is_dir())
            .ok_or_else(|| format!("cannot create a directory in {}", copies.display()))?;
        let res = (|| -> std::io::Result<()> {
            for (n, (path, was)) in self.paths.iter().enumerate() {
                let c = self.copy_of(&copy, n, path);
                match was {
                    Was::Missing => {}
                    Was::Dir => std::fs::create_dir_all(&c)?,
                    Was::File(b, _) => crate::structure::write_file(&c, b)?,
                    #[cfg(unix)]
                    Was::Link(t) => {
                        if let Some(d) = c.parent() {
                            std::fs::create_dir_all(d)?;
                        }
                        std::os::unix::fs::symlink(t, &c)?
                    }
                    #[cfg(not(unix))]
                    Was::Link(t) => {
                        crate::structure::write_file(&c, t.to_string_lossy().as_bytes())?
                    }
                }
            }
            if let Some(ix) = &self.index {
                if let Some(b) = &ix.was {
                    crate::structure::write_file(&copy.join("index"), b)?;
                }
                if let Some((p, b)) = &ix.shared {
                    let name = p.file_name().unwrap_or_default();
                    crate::structure::write_file(&copy.join(name), b)?;
                }
            }
            Ok(())
        })();
        res.map(|_| copy.clone()).map_err(|e| {
            // Half a copy is no copy: it goes, with directories left empty.
            let _ = std::fs::remove_dir_all(&copy);
            let _ = std::fs::remove_dir(copies);
            if let Some(p) = copies.parent() {
                let _ = std::fs::remove_dir(p);
            }
            format!("cannot write it in {}: {e}", copy.display())
        })
    }

    /// Put the index back as it was, as git writes it (into `index.lock`,
    /// then renamed), and the shared index file it names. Only when the
    /// index is what Discard's own git commands left: a change staged
    /// meanwhile by someone else is not overwritten.
    async fn restore_index(&self, dir: &Path) -> Option<Left> {
        let ix = self.index.as_ref()?;
        let what = format!("the git index ({})", ix.path.display());
        let left = |why: String| Left {
            what: what.clone(),
            why,
            copy: ix.was.as_ref().map(|_| PathBuf::from("index")),
        };
        let now = match read_opt(&ix.path) {
            Ok(b) => b,
            Err(e) => return Some(left(e)),
        };
        let shared_there = ix.shared.as_ref().is_none_or(|(p, _)| p.exists());
        if now == ix.was && shared_there {
            return None;
        }
        let (ours, ours_staged) = ix.ours.lock().unwrap().clone();
        if now != ours {
            let changed = staged_changes(&ours_staged, &staged_list(dir).await);
            let names = match changed.len() {
                0 => String::new(),
                n if n > 10 => format!(": {} and {} more", changed[..10].join(", "), n - 10),
                _ => format!(": {}", changed.join(", ")),
            };
            return Some(left(format!(
                "changed while Discard ran, by something other than Discard{names}. Left as it \
                 is, so that change is not lost"
            )));
        }
        let res = (|| -> Result<(), String> {
            use std::io::Write;
            if let Some((p, b)) = &ix.shared
                && !p.exists()
            {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(p)
                    .and_then(|mut f| f.write_all(b))
                    .map_err(|e| format!("cannot write the shared index {}: {e}", p.display()))?;
            }
            if now == ix.was {
                return Ok(());
            }
            let lock = git_lock(&ix.path);
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .map_err(|e| format!("cannot lock it ({}): {e}", lock.display()))?;
            // From here the lock is ours: on failure it goes.
            let res = match &ix.was {
                Some(b) => f
                    .write_all(b)
                    .and_then(|_| f.sync_all())
                    .and_then(|_| std::fs::rename(&lock, &ix.path)),
                None => std::fs::remove_file(&ix.path).and_then(|_| std::fs::remove_file(&lock)),
            };
            if res.is_err() {
                let _ = std::fs::remove_file(&lock);
            }
            res.map_err(|e| format!("cannot write it: {e}"))
        })();
        if let Err(e) = res {
            return Some(left(e));
        }
        // Read back: the bytes, the shared file, and git reading them.
        let back = read_opt(&ix.path).ok().flatten() == ix.was
            && ix.shared.as_ref().is_none_or(|(p, _)| p.exists())
            && staged_list(dir).await == ix.staged;
        (!back).then(|| left("is not as it was".into()))
    }

    /// Put the files back, then read them all again: what is still not as
    /// it was, and why. `what` runs (Discard, Roll back).
    pub(super) fn restore(&self, what: &str) -> Vec<Left> {
        let mut why: std::collections::HashMap<PathBuf, String> = Default::default();
        let was: std::collections::HashMap<&Path, &Was> =
            self.paths.iter().map(|(p, w)| (p.as_path(), w)).collect();
        // What is in the module now and was not (or was something else),
        // deepest first.
        let mut now = Vec::new();
        if let Err(e) = paths_under(&self.module, &mut now) {
            why.insert(self.module.clone(), e);
        }
        for path in now.iter().rev() {
            let Ok(m) = std::fs::symlink_metadata(path) else {
                continue;
            };
            let t = m.file_type();
            let keep = match was.get(path.as_path()) {
                Some(Was::Dir) => t.is_dir(),
                Some(Was::File(..)) => t.is_file(),
                Some(Was::Link(_)) => t.is_symlink(),
                _ => false,
            };
            if !keep {
                let res = if t.is_dir() {
                    std::fs::remove_dir(path)
                } else {
                    std::fs::remove_file(path)
                };
                if let Err(e) = res {
                    why.insert(path.clone(), format!("cannot remove it: {e}"));
                }
            }
        }
        for (path, w) in &self.paths {
            if let Err(e) = w.put(path) {
                why.entry(path.clone()).or_insert(e);
            }
        }
        // Read back: only what is exactly as it was counts as put back.
        let rel = |path: &Path| {
            path.strip_prefix(&self.root)
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let mut left = Vec::new();
        let mut now = Vec::new();
        if let Err(e) = paths_under(&self.module, &mut now) {
            left.push(Left {
                what: rel(&self.module),
                why: e,
                copy: None,
            });
        }
        for path in now.iter().filter(|p| !was.contains_key(p.as_path())) {
            left.push(Left {
                what: rel(path),
                why: why
                    .remove(path)
                    .unwrap_or_else(|| format!("was not there before {what}")),
                copy: None,
            });
        }
        for (n, (path, w)) in self.paths.iter().enumerate() {
            if Was::read(path).ok().as_ref() != Some(w) {
                left.push(Left {
                    what: rel(path),
                    why: why
                        .remove(path)
                        .unwrap_or_else(|| "is not as it was".into()),
                    copy: (*w != Was::Missing).then(|| self.copy_of(Path::new(""), n, path)),
                });
            }
        }
        left
    }

    /// A failed Discard or Roll back that could not put everything back:
    /// what is left where. Keeps the copy the report points at.
    pub(super) fn report(&self, what: &str, left: &[Left]) -> String {
        let copy = self.keep_copy(what);
        let mut msg = format!(
            "{what} stopped halfway and could not put everything back. Not as it was before:"
        );
        for l in left {
            msg.push_str(&format!("\n- {}: {}", l.what, l.why));
            if let (Some(c), Ok(dir)) = (&l.copy, &copy) {
                msg.push_str(&format!("; as it was: {}", dir.join(c).display()));
            }
        }
        match &copy {
            Ok(dir) => msg.push_str(&format!(
                "\nA copy of everything {what} could change, as it was before, is kept in {}. \
                 Copy back what is listed; the index only while no git command runs.",
                dir.display()
            )),
            Err(e) => msg.push_str(&format!(
                "\nNo copy of it could be kept ({e}): what is listed above is lost as it was."
            )),
        }
        msg
    }
}

/// A test's fault, run at each named point of a Discard (see
/// [`discard_fault`]).
#[cfg(test)]
type Fault = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
tokio::task_local! {
    static DISCARD_FAULT: Fault;
}

/// Where a test injects a failure into Discard: `module` after the module
/// is checked out (an error fails the step), `restore` before a failed
/// Discard puts things back. Nothing outside tests.
fn discard_fault(at: &str) -> Result<(), String> {
    #[cfg(test)]
    if let Ok(r) = DISCARD_FAULT.try_with(|f| f(at)) {
        return r;
    }
    let _ = at;
    Ok(())
}

impl Actions {
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
        // All or nothing: a step that fails puts back what the others did,
        // the index (what is staged) with the files. Nothing is touched
        // unless all of it was read and copied.
        let rollback = Rollback::take(dir, &p)
            .await
            .map_err(|e| format!("{e}\nNothing was discarded."))?;
        let mut kept = Vec::new();
        let steps = async {
            // After each git command, success or not, the index it left is
            // Discard's: only that one is put back on a failure.
            if module_in {
                let r = git(dir, &["checkout", "-q", "HEAD", "--", &module]).await;
                rollback.mark_index(dir).await;
                r?;
            }
            let r = git(dir, &["clean", "-fdq", "--", &module]).await;
            rollback.mark_index(dir).await;
            r?;
            discard_fault("module")?;
            let saved = snapshot.as_ref().map(|s| [&s.lock, &s.out]);
            for (i, (path, abs, committed)) in [(&lock, &p.lock, lock_in), (&out, &p.out, out_in)]
                .into_iter()
                .enumerate()
            {
                if committed {
                    let r = git(dir, &["checkout", "-q", "HEAD", "--", path]).await;
                    rollback.mark_index(dir).await;
                    r?;
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
            let _ = discard_fault("restore");
            let mut left = rollback.restore("Discard");
            left.extend(rollback.restore_index(dir).await);
            if left.is_empty() {
                return Err(format!(
                    "{e}\nNothing was discarded: the module, lock, compiled config and git index \
                     are as they were."
                ));
            }
            // Neither the last compile nor the last commit is on disk now.
            *self.compiled.lock().unwrap() = None;
            return Err(format!("{e}\n{}", rollback.report("Discard", &left)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::tests::{blob, git_out, lox_that_adds_blocks, project_for_edits, sh};

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

        // The lock was someone's before the edit; now it is a directory,
        // which Discard cannot copy: refused before anything changes.
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

    /// A project with something pending to discard and, before it, a change
    /// to a committed module file staged and another on top of it unstaged,
    /// and a file outside the module staged. Returns the project directory.
    #[cfg(unix)]
    async fn staged_and_pending(tmp: &Path, a: &Actions) -> PathBuf {
        lox_that_adds_blocks(tmp);
        let d = tmp.join("project");
        std::fs::write(d.join("pages/main.loxl"), "let a = 1\n").unwrap();
        sh(&d, "git add pages/main.loxl && git commit -qm main");
        std::fs::write(d.join("pages/main.loxl"), "let a = 2\n").unwrap();
        std::fs::write(d.join("notes.txt"), "staged\n").unwrap();
        sh(&d, "git add pages/main.loxl notes.txt");
        std::fs::write(d.join("pages/main.loxl"), "let a = 3\n").unwrap();
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
        d
    }

    /// Everything a Discard may change: the module's files, the lock, out,
    /// and the index as git lists it and byte for byte.
    #[cfg(unix)]
    type DiscardState = (Vec<(PathBuf, Vec<u8>)>, String, Vec<u8>);

    #[cfg(unix)]
    fn discard_state(d: &Path) -> DiscardState {
        let mut files = Vec::new();
        let mut paths = Vec::new();
        paths_under(&d.join("pages"), &mut paths).unwrap();
        for p in paths
            .into_iter()
            .chain([d.join("logic.lock.json"), d.join("out.Loxone")])
        {
            if p.is_file() {
                let b = std::fs::read(&p).unwrap();
                files.push((p, b));
            }
        }
        let staged = std::process::Command::new("git")
            .args(["ls-files", "-s"])
            .current_dir(d)
            .output()
            .unwrap();
        (
            files,
            String::from_utf8(staged.stdout).unwrap(),
            std::fs::read(d.join(".git/index")).unwrap(),
        )
    }

    #[cfg(unix)]
    fn staged_text(d: &Path, path: &str) -> String {
        let o = std::process::Command::new("git")
            .args(["show", &format!(":{path}")])
            .current_dir(d)
            .output()
            .unwrap();
        String::from_utf8(o.stdout).unwrap()
    }

    /// The copies failed Discards kept: `.git/vislox-recovery/<project>/*`.
    #[cfg(unix)]
    fn recovery_copies(d: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for p in std::fs::read_dir(d.join(".git/vislox-recovery"))
            .into_iter()
            .flatten()
        {
            for c in std::fs::read_dir(p.unwrap().path()).unwrap() {
                out.push(c.unwrap().path());
            }
        }
        out
    }

    const NOTHING_DISCARDED: &str = "Nothing was discarded: the module, lock, compiled config and \
                                     git index are as they were.";

    /// A Discard that fails after the module was checked out (#122): the
    /// checkout had reset the index too; the staged change, the unstaged
    /// one on top of it, the files vislox wrote, the lock and out all come
    /// back, written in place (the same file, its hard links kept), and no
    /// copy is left. A Discard that works then drops the module's staged
    /// change and leaves the index elsewhere alone.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_discard_puts_back_the_index_with_the_files() {
        use std::os::unix::fs::MetadataExt;
        let (tmp, a) = project_for_edits();
        let d = staged_and_pending(tmp.path(), &a).await;
        let main = d.join("pages/main.loxl");
        // A file it rewrites comes back in place: the same file, and what is
        // linked to it.
        let lock = d.join("logic.lock.json");
        std::fs::hard_link(&lock, tmp.path().join("lock-link")).unwrap();
        let ino = std::fs::metadata(&lock).unwrap().ino();
        let before = discard_state(&d);
        assert!(d.join("pages/vislox").exists());
        let garble = lock.clone();
        let fail: Fault = Box::new(move |at| match at {
            "module" => Err("injected after the checkout".into()),
            "restore" => {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&garble)
                    .unwrap();
                f.write_all(b"garbled").unwrap();
                Ok(())
            }
            _ => Ok(()),
        });
        let (_, msg) = DISCARD_FAULT.scope(fail, a.discard()).await.unwrap_err();
        assert!(msg.contains("injected after the checkout"), "{msg}");
        assert!(msg.contains(NOTHING_DISCARDED), "{msg}");
        assert!(discard_state(&d) == before);
        assert_eq!(staged_text(&d, "pages/main.loxl"), "let a = 2\n");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), "let a = 3\n");
        assert_eq!(std::fs::metadata(&lock).unwrap().ino(), ino);
        assert_eq!(
            std::fs::read(tmp.path().join("lock-link")).unwrap(),
            std::fs::read(&lock).unwrap()
        );
        assert!(recovery_copies(&d).is_empty());
        assert!(!d.join(".git/vislox-recovery").exists());
        // No file of its own is left beside the ones it wrote.
        let mut paths = Vec::new();
        paths_under(&d, &mut paths).unwrap();
        assert!(
            !paths
                .iter()
                .any(|p| p.to_string_lossy().contains("vislox-") && !p.starts_with(d.join(".git"))),
            "{paths:?}"
        );

        a.discard().await.unwrap();
        assert_eq!(staged_text(&d, "pages/main.loxl"), "let a = 1\n");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), "let a = 1\n");
        assert_eq!(staged_text(&d, "notes.txt"), "staged\n");
        assert!(!d.join("pages/vislox").exists());
        assert!(!d.join(".git/vislox-recovery").exists());
    }

    /// When a failed Discard cannot put everything back, it says so, never
    /// "Nothing was discarded": each path not as it was, why, and its copy,
    /// kept for it then (from memory), which copied back restores it all.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_discard_that_cannot_put_everything_back_says_what_is_left_where() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return; // root writes into a read-only directory
        }
        let (tmp, a) = project_for_edits();
        let d = staged_and_pending(tmp.path(), &a).await;
        let (_, _, index) = discard_state(&d);
        let file = std::fs::read(d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        let pages = d.join("pages");
        let lock = d.join(".git/index.lock");
        let fail: Fault = Box::new(move |at| {
            match at {
                "module" => return Err("injected after the checkout".into()),
                // pages/vislox cannot come back, nor the index.
                "restore" => {
                    std::fs::set_permissions(&pages, std::fs::Permissions::from_mode(0o555))
                        .unwrap();
                    std::fs::write(&lock, "").unwrap();
                }
                _ => {}
            }
            Ok(())
        });
        let (_, msg) = DISCARD_FAULT.scope(fail, a.discard()).await.unwrap_err();
        std::fs::set_permissions(d.join("pages"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_file(d.join(".git/index.lock")).unwrap();
        assert!(!msg.contains("Nothing was discarded"), "{msg}");
        assert!(
            msg.contains("Discard stopped halfway and could not put everything back"),
            "{msg}"
        );
        let copies = recovery_copies(&d);
        let [copy] = &copies[..] else {
            panic!("{copies:?}")
        };
        assert!(
            copy.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-discard")
        );
        assert!(msg.contains(&copy.display().to_string()), "{msg}");
        assert!(
            msg.contains("- pages/vislox/obergeschoss.loxl: cannot write it"),
            "{msg}"
        );
        assert!(msg.contains("- the git index ("), "{msg}");
        assert!(msg.contains("cannot lock it"), "{msg}");
        let saved = copy.join("files/pages/vislox/obergeschoss.loxl");
        assert!(msg.contains(&saved.display().to_string()), "{msg}");
        assert_eq!(std::fs::read(&saved).unwrap(), file);
        assert_eq!(std::fs::read(copy.join("index")).unwrap(), index);
        // Written in place, main.loxl came back in the read-only directory.
        assert!(!msg.contains("pages/main.loxl"), "{msg}");
        assert_eq!(
            std::fs::read_to_string(d.join("pages/main.loxl")).unwrap(),
            "let a = 3\n"
        );
        // What it says to do brings back what was staged.
        std::fs::copy(copy.join("index"), d.join(".git/index")).unwrap();
        std::fs::create_dir_all(d.join("pages/vislox")).unwrap();
        std::fs::copy(&saved, d.join("pages/vislox/obergeschoss.loxl")).unwrap();
        assert_eq!(staged_text(&d, "pages/main.loxl"), "let a = 2\n");
        assert_eq!(staged_text(&d, "notes.txt"), "staged\n");
    }

    /// Something else stages a change while Discard runs: a failed Discard
    /// does not put the old index over it, and says what changed and where
    /// the index from before is.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_discard_never_overwrites_what_was_staged_meanwhile() {
        let (tmp, a) = project_for_edits();
        let d = staged_and_pending(tmp.path(), &a).await;
        let (_, _, index) = discard_state(&d);
        let dd = d.clone();
        let fail: Fault = Box::new(move |at| {
            match at {
                "module" => return Err("injected".into()),
                "restore" => {
                    std::fs::write(dd.join("andere.txt"), "theirs\n").unwrap();
                    sh(&dd, "git add andere.txt");
                }
                _ => {}
            }
            Ok(())
        });
        let (_, msg) = DISCARD_FAULT.scope(fail, a.discard()).await.unwrap_err();
        assert!(!msg.contains("Nothing was discarded"), "{msg}");
        assert!(
            msg.contains("changed while Discard ran, by something other than Discard: andere.txt"),
            "{msg}"
        );
        assert_eq!(staged_text(&d, "andere.txt"), "theirs\n");
        // The files are back; the index from before is in the copy.
        assert_eq!(
            std::fs::read_to_string(d.join("pages/main.loxl")).unwrap(),
            "let a = 3\n"
        );
        assert!(d.join("pages/vislox/obergeschoss.loxl").exists());
        let copies = recovery_copies(&d);
        let [copy] = &copies[..] else {
            panic!("{copies:?}")
        };
        assert_eq!(std::fs::read(copy.join("index")).unwrap(), index);
        assert!(
            msg.contains(&copy.join("index").display().to_string()),
            "{msg}"
        );
    }

    /// With split index on, the index names a shared index file, which git
    /// may expire while Discard runs: it comes back with the index.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_discard_puts_back_a_split_index_whole() {
        let (tmp, a) = project_for_edits();
        let d = staged_and_pending(tmp.path(), &a).await;
        sh(
            &d,
            "git config core.splitIndex true && git config splitIndex.sharedIndexExpire now \
             && git update-index --split-index",
        );
        let shared = git_out(&d, &["rev-parse", "--shared-index-path"]);
        assert!(!shared.is_empty());
        let shared = d.join(shared);
        let before = discard_state(&d);
        let gone = shared.clone();
        let fail: Fault = Box::new(move |at| {
            match at {
                "module" => return Err("injected".into()),
                // As git does when it writes a new shared index.
                "restore" => {
                    let _ = std::fs::remove_file(&gone);
                }
                _ => {}
            }
            Ok(())
        });
        let (_, msg) = DISCARD_FAULT.scope(fail, a.discard()).await.unwrap_err();
        assert!(msg.contains(NOTHING_DISCARDED), "{msg}");
        assert!(shared.exists());
        assert!(discard_state(&d) == before);
        assert_eq!(staged_text(&d, "pages/main.loxl"), "let a = 2\n");
        sh(&d, "git status --porcelain >/dev/null");
    }

    /// Discard reads everything before it touches anything; what it cannot
    /// read, a symbolic link where it writes (#81) or git at work refuses
    /// it, and nothing changes.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_discard_that_cannot_read_everything_changes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (tmp, a) = project_for_edits();
        let d = staged_and_pending(tmp.path(), &a).await;
        let before = discard_state(&d);
        let refused = |msg: &str, what: &str| {
            assert!(
                msg.contains(what) && msg.ends_with("Nothing was discarded."),
                "{msg}"
            );
            assert!(discard_state(&d) == before);
        };
        // A file of the module it cannot read.
        if unsafe { libc::geteuid() } != 0 {
            let f = d.join("pages/geheim.loxl");
            std::fs::write(&f, "let g = 1\n").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o000)).unwrap();
            let (_, msg) = a.discard().await.unwrap_err();
            assert_eq!(std::fs::read(d.join(".git/index")).unwrap(), before.2);
            std::fs::remove_file(&f).unwrap();
            refused(&msg, "geheim.loxl");
        }
        // git at work in the repository.
        std::fs::write(d.join(".git/index.lock"), "").unwrap();
        let (_, msg) = a.discard().await.unwrap_err();
        std::fs::remove_file(d.join(".git/index.lock")).unwrap();
        refused(&msg, "git is busy");
        // A linked lock: Discard would write what it reaches.
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let lock = d.join("logic.lock.json");
        std::fs::rename(&lock, outside.join("lock")).unwrap();
        std::os::unix::fs::symlink(outside.join("lock"), &lock).unwrap();
        let theirs = std::fs::read(outside.join("lock")).unwrap();
        let (_, msg) = a.discard().await.unwrap_err();
        assert!(
            msg.starts_with("logic.lock.json is a symbolic link: Discard never writes through one"),
            "{msg}"
        );
        assert!(msg.ends_with("Nothing was discarded."), "{msg}");
        assert_eq!(std::fs::read(outside.join("lock")).unwrap(), theirs);
        std::fs::remove_file(&lock).unwrap();
        std::fs::rename(outside.join("lock"), &lock).unwrap();
        // A linked module directory.
        std::fs::rename(d.join("pages"), outside.join("pages")).unwrap();
        std::os::unix::fs::symlink(outside.join("pages"), d.join("pages")).unwrap();
        let (_, msg) = a.discard().await.unwrap_err();
        assert!(msg.starts_with("pages is a symbolic link"), "{msg}");
        assert!(outside.join("pages/vislox/obergeschoss.loxl").exists());
        std::fs::remove_file(d.join("pages")).unwrap();
        std::fs::rename(outside.join("pages"), d.join("pages")).unwrap();
        assert!(discard_state(&d) == before);
        a.discard().await.unwrap();
        assert!(!d.join("pages/vislox").exists());
    }

    /// A path reached through a linked directory is refused as one.
    #[cfg(unix)]
    #[test]
    fn a_path_through_a_linked_directory_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("p");
        std::fs::create_dir_all(d.join("real")).unwrap();
        std::fs::create_dir_all(tmp.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("elsewhere"), d.join("state")).unwrap();
        no_link(&d, &d.join("real/x"), "Discard").unwrap();
        no_link(&d, &d.join("missing/x"), "Discard").unwrap();
        let e = no_link(&d, &d.join("state/logic.lock.json"), "Roll back").unwrap_err();
        assert!(
            e.starts_with("state/logic.lock.json is reached through a symbolic link (state)"),
            "{e}"
        );
        let e = no_link(&d, &d.join("../x"), "Discard").unwrap_err();
        assert!(e.contains("is outside the project"), "{e}");
    }

    /// Putting the module back never writes through a symbolic link inside
    /// it: a linked directory comes back as the link, and the file it
    /// reaches, changed meanwhile, is not rewritten.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_discard_never_writes_through_a_link() {
        let (tmp, a) = project_for_edits();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("werte.loxl"), "let w = 1\n").unwrap();
        let d = tmp.path().join("project");
        std::os::unix::fs::symlink(&outside, d.join("pages/shared")).unwrap();
        sh(&d, "git add -A && git commit -qm link");
        let d = staged_and_pending(tmp.path(), &a).await;
        let werte = outside.join("werte.loxl");
        let fail: Fault = Box::new(move |at| {
            match at {
                "module" => return Err("injected".into()),
                "restore" => std::fs::write(&werte, "let w = 2\n").unwrap(),
                _ => {}
            }
            Ok(())
        });
        let (_, msg) = DISCARD_FAULT.scope(fail, a.discard()).await.unwrap_err();
        assert!(msg.contains(NOTHING_DISCARDED), "{msg}");
        assert_eq!(
            std::fs::read_to_string(outside.join("werte.loxl")).unwrap(),
            "let w = 2\n"
        );
        assert_eq!(std::fs::read_link(d.join("pages/shared")).unwrap(), outside);
    }
}
