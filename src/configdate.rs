//! When the base config was taken from the Miniserver, in the Miniserver's
//! own terms: the `lastModified` of its structure file (`LoxAPP3.json`).
//!
//! The Miniserver tells its current `lastModified` with
//! `jdev/sps/LoxAPPversion3` (see `miniserver.rs`). Every download `lox`
//! keeps carries `LoxAPP3.json` too: `<serial>/backup.zip` next to a config
//! repository's `config.Loxone` (`lox config pull`), and the zips in a
//! project's `backups/`, each beside the `.Loxone` lox extracted from it
//! (`lox config push`, `merge`, vislox's Sync). So the base's date is read
//! from the download it came from, and vislox keeps no state of its own.
//!
//! When the Miniserver's is later, its config changed after the base was
//! taken (a save from Loxone Config, or the like). The date is coarse: a push
//! that changes only logic can leave it as it was, so equal dates do not
//! prove equal configs.
//!
//! A backup `.zip` opened on its own has no date here: it is a look at an
//! old config, not a base anyone works on, so it is never "out of date".

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// What the date found depends on: the base file (by content: a rewrite of
/// the same size can keep its time), and the downloads it is looked for in
/// (a file added to `backups/` changes the directory's time).
#[derive(Clone, PartialEq, Debug)]
struct Stamp {
    base: String,
    backup_zip: Option<SystemTime>,
    backups: Option<SystemTime>,
}

/// The base config, and where lox keeps the downloads it may come from.
pub struct BaseDate {
    /// The config file whose history is shown (a project's `base`).
    file: PathBuf,
    /// A project's `backups/` directory.
    backups: Option<PathBuf>,
    /// The date found (or not) for this stamp.
    cache: Mutex<Option<(Stamp, Option<String>)>>,
}

impl BaseDate {
    pub fn new(file: PathBuf, backups: Option<PathBuf>) -> BaseDate {
        BaseDate {
            file,
            backups,
            cache: Mutex::new(None),
        }
    }

    /// The `lastModified` of the download the base file came from, if one
    /// is found. Blocking (reads files, when the base or the downloads
    /// changed since the last time; a date not found is remembered too).
    pub fn get(&self) -> Option<String> {
        let bytes = std::fs::read(&self.file).ok()?;
        let stamp = self.stamp(&bytes);
        if let Some((s, v)) = self.cache.lock().unwrap().as_ref()
            && *s == stamp
        {
            return v.clone();
        }
        let v = find(&self.file, self.backups.as_deref(), &bytes);
        *self.cache.lock().unwrap() = Some((stamp, v.clone()));
        v
    }

    /// Look again next time (after Pull, Apply or Sync).
    pub fn forget(&self) {
        *self.cache.lock().unwrap() = None;
    }

    fn stamp(&self, bytes: &[u8]) -> Stamp {
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        Stamp {
            base: crate::history::blob_id(bytes),
            backup_zip: self
                .file
                .parent()
                .and_then(|d| modified(&d.join("backup.zip"))),
            backups: self.backups.as_deref().and_then(modified),
        }
    }
}

/// The download whose config is `bytes`, and its `lastModified`:
/// - none for a base that is itself a download (`.zip`), see above;
/// - `backup.zip` next to a config repository's `config.Loxone` (both
///   written by `lox config pull`);
/// - in a project's `backups/`, the zip beside the newest `.Loxone` that
///   equals the base byte for byte.
fn find(file: &Path, backups: Option<&Path>, bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"PK") {
        return None;
    }
    if file.file_name().is_some_and(|n| n == "config.Loxone")
        && let Some(dir) = file.parent()
        && let Ok(zip) = std::fs::read(dir.join("backup.zip"))
        && let Some(v) = last_modified_in(&zip)
    {
        return Some(v);
    }
    for loxone in newest_first(backups?, bytes.len() as u64) {
        if std::fs::read(&loxone).ok().as_deref() != Some(bytes) {
            continue;
        }
        if let Ok(zip) = std::fs::read(loxone.with_extension("zip"))
            && let Some(v) = last_modified_in(&zip)
        {
            return Some(v);
        }
    }
    None
}

/// The `.Loxone` files in `dir` of this size, newest first (lox names them
/// `<YYYYMMDD-HHMMSS>-<step>`).
fn newest_first(dir: &Path, len: u64) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.metadata().is_ok_and(|m| m.len() == len))
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "Loxone"))
        .collect();
    v.sort();
    v.reverse();
    v
}

/// `lastModified` of the `LoxAPP3.json` in a download.
fn last_modified_in(zip: &[u8]) -> Option<String> {
    let json = loxone_config::archive::read_entry(zip, "LoxAPP3.json").ok()??;
    let json = json.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&json);
    let v: Value = serde_json::from_slice(json).ok()?;
    v["lastModified"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn download(last_modified: &str) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("sps0.LoxCC", o).unwrap();
        w.write_all(b"program").unwrap();
        w.start_file("LoxAPP3.json", o).unwrap();
        w.write_all(
            format!("\u{feff}{{\"lastModified\":\"{last_modified}\",\"msInfo\":{{}}}}").as_bytes(),
        )
        .unwrap();
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn a_repository_base_is_dated_by_its_backup_zip() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("config.Loxone");
        std::fs::write(&file, b"<ControlList/>").unwrap();
        let b = BaseDate::new(file, None);
        assert_eq!(b.get(), None);
        std::fs::write(d.path().join("backup.zip"), download("2026-11-05 23:55:01")).unwrap();
        // Looked for again: nothing was found before.
        assert_eq!(b.get().as_deref(), Some("2026-11-05 23:55:01"));
    }

    #[test]
    fn a_project_base_is_dated_by_the_newest_download_equal_to_it() {
        let d = tempfile::tempdir().unwrap();
        let backups = d.path().join("backups");
        std::fs::create_dir(&backups).unwrap();
        let file = d.path().join("base.Loxone");
        let cfg = b"<ControlList V=\"2\"/>";
        std::fs::write(&file, cfg).unwrap();
        // An older download of the same config, a newer one of another, and
        // the newest of the same one.
        for (stem, content, at) in [
            ("20261105-225445-pre-push", &cfg[..], "2026-10-31 20:00:00"),
            (
                "20261106-001253-pre-push",
                b"<ControlList V=\"1\"/>",
                "2026-11-06 00:10:00",
            ),
            ("20261106-003332-post-push", &cfg[..], "2026-11-06 00:23:31"),
        ] {
            std::fs::write(backups.join(format!("{stem}.Loxone")), content).unwrap();
            std::fs::write(backups.join(format!("{stem}.zip")), download(at)).unwrap();
        }
        let b = BaseDate::new(file.clone(), Some(backups));
        assert_eq!(b.get().as_deref(), Some("2026-11-06 00:23:31"));
        // A base no download equals (e.g. edited by hand) has no date.
        std::fs::write(&file, b"<ControlList V=\"3\"/>").unwrap();
        assert_eq!(b.get(), None);
    }

    #[test]
    fn a_backup_zip_looked_at_on_its_own_has_no_date() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("x.zip");
        std::fs::write(&file, download("2026-01-01 00:00:00")).unwrap();
        assert_eq!(BaseDate::new(file, None).get(), None);
    }

    /// Move a file's or directory's modification time on, as a later write
    /// would (file systems with coarse times may not have moved it yet).
    fn touch(p: &Path, secs: u64) {
        let t = SystemTime::now() + std::time::Duration::from_secs(secs);
        std::fs::File::open(p).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn a_newer_download_of_the_same_config_moves_the_date() {
        // A Sync after a re-save (or a change in the app) downloads the same
        // program again, with a newer date: the base's bytes do not change.
        let d = tempfile::tempdir().unwrap();
        let backups = d.path().join("backups");
        std::fs::create_dir(&backups).unwrap();
        let file = d.path().join("base.Loxone");
        let cfg = b"<ControlList/>";
        std::fs::write(&file, cfg).unwrap();
        std::fs::write(backups.join("20261106-010155-sync.Loxone"), cfg).unwrap();
        std::fs::write(
            backups.join("20261106-010155-sync.zip"),
            download("2026-11-06 00:23:31"),
        )
        .unwrap();
        let b = BaseDate::new(file, Some(backups.clone()));
        assert_eq!(b.get().as_deref(), Some("2026-11-06 00:23:31"));
        std::fs::write(backups.join("20261106-143300-sync.Loxone"), cfg).unwrap();
        std::fs::write(
            backups.join("20261106-143300-sync.zip"),
            download("2026-11-06 14:32:10"),
        )
        .unwrap();
        touch(&backups, 5);
        assert_eq!(b.get().as_deref(), Some("2026-11-06 14:32:10"));
    }

    #[test]
    fn a_repository_backup_rewritten_by_a_pull_moves_the_date() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("config.Loxone");
        std::fs::write(&file, b"<ControlList/>").unwrap();
        let zip = d.path().join("backup.zip");
        std::fs::write(&zip, download("2026-11-05 23:55:01")).unwrap();
        let b = BaseDate::new(file, None);
        assert_eq!(b.get().as_deref(), Some("2026-11-05 23:55:01"));
        // `lox config pull` with an unchanged program: same config.Loxone.
        std::fs::write(&zip, download("2026-11-06 00:23:31")).unwrap();
        touch(&zip, 5);
        assert_eq!(b.get().as_deref(), Some("2026-11-06 00:23:31"));
        // And after forget(), it is read again whatever the times say.
        std::fs::write(&zip, download("2026-11-06 09:00:00")).unwrap();
        touch(&zip, 5);
        b.forget();
        assert_eq!(b.get().as_deref(), Some("2026-11-06 09:00:00"));
    }

    #[test]
    fn a_base_without_a_date_is_not_looked_for_on_every_check() {
        let d = tempfile::tempdir().unwrap();
        let backups = d.path().join("backups");
        std::fs::create_dir(&backups).unwrap();
        let file = d.path().join("base.Loxone");
        std::fs::write(&file, b"<ControlList/>").unwrap();
        let b = BaseDate::new(file, Some(backups.clone()));
        assert_eq!(b.get(), None);
        // A download that would match, but written without the directory's
        // time moving (as within one tick of a coarse clock): the remembered
        // answer stands until the directory changes.
        let t = std::fs::metadata(&backups).unwrap().modified().unwrap();
        std::fs::write(backups.join("1-sync.Loxone"), b"<ControlList/>").unwrap();
        std::fs::write(backups.join("1-sync.zip"), download("2026-11-06 00:23:31")).unwrap();
        std::fs::File::open(&backups)
            .unwrap()
            .set_modified(t)
            .unwrap();
        assert_eq!(b.get(), None);
        touch(&backups, 5);
        assert_eq!(b.get().as_deref(), Some("2026-11-06 00:23:31"));
    }
}
