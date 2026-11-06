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
//! When the two differ, the Miniserver's config changed after the base was
//! taken (a save from Loxone Config, or the like). The date is coarse: a push
//! that changes only logic can leave it as it was, so equal dates do not
//! prove equal configs.

use crate::history::blob_id;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The base config, and where lox keeps the downloads it may come from.
pub struct BaseDate {
    /// The config file whose history is shown (a project's `base`).
    file: PathBuf,
    /// A project's `backups/` directory.
    backups: Option<PathBuf>,
    /// The base's blob and the date found for it.
    cache: Mutex<Option<(String, Option<String>)>>,
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
    /// is found. Blocking (reads files).
    pub fn get(&self) -> Option<String> {
        let bytes = std::fs::read(&self.file).ok()?;
        let blob = blob_id(&bytes);
        if let Some((b, v)) = self.cache.lock().unwrap().as_ref()
            && *b == blob
            && v.is_some()
        {
            return v.clone();
        }
        // Not found yet: look again, a Sync or Apply may have written it.
        let v = find(&self.file, self.backups.as_deref(), &bytes);
        *self.cache.lock().unwrap() = Some((blob, v.clone()));
        v
    }
}

/// The download whose config is `bytes`, and its `lastModified`:
/// - the base file itself, when it is a download (`.zip`);
/// - `backup.zip` next to a config repository's `config.Loxone` (both
///   written by `lox config pull`);
/// - in a project's `backups/`, the zip beside the newest `.Loxone` that
///   equals the base byte for byte.
fn find(file: &Path, backups: Option<&Path>, bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"PK") {
        return last_modified_in(bytes);
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
    fn a_zip_base_carries_its_own_date() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("x.zip");
        std::fs::write(&file, download("2026-01-01 00:00:00")).unwrap();
        assert_eq!(
            BaseDate::new(file, None).get().as_deref(),
            Some("2026-01-01 00:00:00")
        );
    }
}
