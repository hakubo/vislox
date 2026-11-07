//! Rolling back an Apply (#69): the downloads `lox config push` leaves in the
//! project's `backups/` (git-ignored, since configs carry personal data).
//!
//! Each push writes, under one stamp (`YYYYMMDD-HHMMSS`, local time):
//! `<stamp>-pre-push.zip` and `.Loxone`, what the Miniserver ran before the
//! upload, and, once the push is verified, `<stamp>-post-push.zip` and
//! `.Loxone`, what it runs after. Apply copies the post-push program over the
//! project's base and commits it, so a commit of the base whose blob is a
//! post-push program is a push, and its pre-push program is what the
//! Miniserver ran before it.
//!
//! This module only finds and reads those files, and refuses whatever does
//! not add up. Deploying the program again is Apply's own path
//! (`Actions::rollback`): `lox config push --patched-from`, with its drift
//! check, upload, program restart and verification.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// One verified push, by its downloads.
#[derive(Debug, Clone, PartialEq)]
pub struct Push {
    /// `YYYYMMDD-HHMMSS`, lox's local time when the push started.
    pub stamp: String,
    /// What the Miniserver ran before the upload.
    pub pre_zip: PathBuf,
    pub pre_xml: PathBuf,
    /// What it ran after: the program the commit holds as its base.
    pub post_xml: PathBuf,
}

impl Push {
    fn new(dir: &Path, stamp: &str) -> Push {
        Push {
            stamp: stamp.to_string(),
            pre_zip: dir.join(format!("{stamp}-pre-push.zip")),
            pre_xml: dir.join(format!("{stamp}-pre-push.Loxone")),
            post_xml: dir.join(format!("{stamp}-post-push.Loxone")),
        }
    }

    /// `backups/<stamp>-pre-push.zip`, for messages.
    pub fn shown(&self, root: &Path) -> String {
        let p = if self.pre_zip.exists() || !self.pre_xml.exists() {
            &self.pre_zip
        } else {
            &self.pre_xml
        };
        p.strip_prefix(root).unwrap_or(p).display().to_string()
    }
}

/// The stamp of a `<stamp>-post-push.Loxone` file name.
fn post_stamp(name: &str) -> Option<&str> {
    let stamp = name.strip_suffix("-post-push.Loxone")?;
    let ok = stamp.len() == 15
        && stamp.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    ok.then_some(stamp)
}

/// The blob ids of the post-push programs in a backups directory, read again
/// only when a file's size or modification time changed. The time is only a
/// cache key here: what a rollback deploys is read and checked again then
/// ([`Pushes::find_now`]).
#[derive(Default)]
pub struct Pushes {
    seen: Mutex<HashMap<PathBuf, Seen>>,
}

/// A post-push file as last read: its size and time, and blob id.
type Seen = (u64, Option<SystemTime>, String);

impl Pushes {
    /// Post-push blob id → the newest push that left it (a later push can
    /// leave the same program: a rollback of a rollback).
    pub fn list(&self, dir: &Path) -> HashMap<String, Push> {
        let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                Some((post_stamp(&name)?.to_string(), e.path()))
            })
            .collect();
        found.sort();
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|p, _| found.iter().any(|(_, q)| q == p));
        let mut out = HashMap::new();
        for (stamp, path) in found {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let (len, time) = (meta.len(), meta.modified().ok());
            let blob = match seen.get(&path) {
                Some((l, t, b)) if *l == len && *t == time => b.clone(),
                _ => {
                    let Ok(bytes) = std::fs::read(&path) else {
                        continue;
                    };
                    let b = crate::history::blob_id(&bytes);
                    seen.insert(path.clone(), (len, time, b.clone()));
                    b
                }
            };
            // Sorted by stamp: a later push of the same program wins.
            out.insert(blob, Push::new(dir, &stamp));
        }
        out
    }

    /// The push whose post-push program is `blob`, read again now (no
    /// cache): the newest one.
    pub fn find_now(dir: &Path, blob: &str) -> Option<Push> {
        let mut stamps: Vec<String> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|e| post_stamp(&e.file_name().to_string_lossy()).map(String::from))
            .collect();
        stamps.sort();
        stamps
            .into_iter()
            .rev()
            .map(|s| Push::new(dir, &s))
            .find(|p| std::fs::read(&p.post_xml).is_ok_and(|b| crate::history::blob_id(&b) == blob))
    }
}

/// The program the Miniserver ran before `push`: the pre-push download,
/// as `.Loxone` and as the ZIP lox downloaded. Both must be readable where
/// they exist and hold the same program, and at least one must exist.
pub fn read_pre_push(push: &Push, root: &Path) -> Result<Vec<u8>, String> {
    let show = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
    let xml = match std::fs::read(&push.pre_xml) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", show(&push.pre_xml))),
    };
    let zip = match std::fs::read(&push.pre_zip) {
        Ok(z) => Some(
            loxone_config::archive::extract_and_decompress(&z)
                .map_err(|e| format!("cannot read {}: {e}", show(&push.pre_zip)))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", show(&push.pre_zip))),
    };
    match (xml, zip) {
        (None, None) => Err(format!(
            "the download from before this Apply is gone ({} and {} are missing)",
            show(&push.pre_zip),
            show(&push.pre_xml)
        )),
        (Some(x), Some(z)) if x != z => Err(format!(
            "{} and {} hold different programs (blob {} and {})",
            show(&push.pre_xml),
            show(&push.pre_zip),
            &crate::history::blob_id(&x)[..10],
            &crate::history::blob_id(&z)[..10]
        )),
        (Some(b), _) | (None, Some(b)) => {
            if b.is_empty() {
                return Err(format!("{} is empty", show(&push.pre_xml)));
            }
            loxone_config::LoxoneDoc::parse(&b)
                .map_err(|e| format!("cannot read the program from before this Apply: {e}"))?;
            Ok(b)
        }
    }
}

/// Whether `compiled` is the program `backup` holds: the same bytes, else no
/// difference lox's diff sees (`Ok(false)`). Anything else would not put back
/// what ran before: the differences, briefly.
pub fn same_program(backup: &[u8], compiled: &[u8]) -> Result<bool, String> {
    if backup == compiled {
        return Ok(true);
    }
    let parse = |b: &[u8]| loxone_config::LoxoneDoc::parse(b).map_err(|e| e.to_string());
    let d = loxone_config::diff::diff(&parse(backup)?, &parse(compiled)?);
    if d.is_empty() {
        return Ok(false);
    }
    let title = |t: &Option<String>, u: &str| t.clone().unwrap_or_else(|| u.to_string());
    let mut what: Vec<String> = Vec::new();
    what.extend(
        d.added
            .iter()
            .map(|o| format!("adds {}", title(&o.title, &o.uuid))),
    );
    what.extend(
        d.removed
            .iter()
            .map(|o| format!("removes {}", title(&o.title, &o.uuid))),
    );
    what.extend(d.renamed.iter().map(|r| {
        format!(
            "renames {} to {}",
            r.from.as_deref().unwrap_or("?"),
            r.to.as_deref().unwrap_or("?")
        )
    }));
    what.extend(
        d.param_changes
            .iter()
            .map(|p| format!("changes {} of {}", p.port_key, p.object_uuid)),
    );
    let wires = d.wires_added.len() + d.wires_removed.len();
    if wires > 0 {
        what.push(format!("changes {wires} wire(s)"));
    }
    let others = d.placements.len() + d.access.len();
    if others > 0 {
        what.push(format!("{others} other change(s)"));
    }
    let n = what.len();
    what.truncate(5);
    if n > 5 {
        what.push(format!("and {} more", n - 5));
    }
    Err(what.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_post_push_programs_with_a_stamp_count() {
        assert_eq!(
            post_stamp("20261107-101500-post-push.Loxone"),
            Some("20261107-101500")
        );
        for n in [
            "20261107-101500-pre-push.Loxone",
            "20261107-101500-post-push.zip",
            "x-post-push.Loxone",
            "2026110-7101500-post-push.Loxone",
        ] {
            assert_eq!(post_stamp(n), None, "{n}");
        }
    }

    #[test]
    fn the_newest_push_of_a_program_is_found_and_cached_by_content() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::write(d.join("20261101-100000-post-push.Loxone"), "a").unwrap();
        std::fs::write(d.join("20261102-100000-post-push.Loxone"), "b").unwrap();
        std::fs::write(d.join("20261103-100000-post-push.Loxone"), "a").unwrap();
        std::fs::write(d.join("20261104-100000-pre-push.Loxone"), "c").unwrap();
        let p = Pushes::default();
        let l = p.list(d);
        assert_eq!(l.len(), 2);
        let a = crate::history::blob_id(b"a");
        assert_eq!(l[&a].stamp, "20261103-100000");
        assert_eq!(
            l[&a].pre_zip,
            d.join("20261103-100000-pre-push.zip"),
            "{l:?}"
        );
        assert_eq!(Pushes::find_now(d, &a).unwrap().stamp, "20261103-100000");
        assert_eq!(Pushes::find_now(d, &crate::history::blob_id(b"c")), None);
        // A file that changed is read again.
        std::fs::write(d.join("20261103-100000-post-push.Loxone"), "bb").unwrap();
        let l = p.list(d);
        assert_eq!(l[&a].stamp, "20261101-100000");
        assert_eq!(l[&crate::history::blob_id(b"bb")].stamp, "20261103-100000");
    }

    fn zip_of(xml: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("sps0.LoxCC", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(&loxone_config::archive::compress_loxcc(xml).unwrap())
            .unwrap();
        w.finish().unwrap().into_inner()
    }

    const DOC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ControlList Version="1" NextObj="100">
<C Type="Document" V="178" U="00000000-0000-0001-ffff504f94000001" Title="Haus">
</C>
</ControlList>
"#;

    #[test]
    fn the_backup_must_be_there_readable_and_one_program() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let p = Push::new(d, "20261107-101500");
        let missing = read_pre_push(&p, d).unwrap_err();
        assert!(missing.contains("is gone"), "{missing}");
        // Either file alone will do.
        std::fs::write(&p.pre_zip, zip_of(DOC.as_bytes())).unwrap();
        assert_eq!(read_pre_push(&p, d).unwrap(), DOC.as_bytes());
        assert_eq!(p.shown(d), "20261107-101500-pre-push.zip");
        std::fs::write(&p.pre_xml, DOC).unwrap();
        assert_eq!(read_pre_push(&p, d).unwrap(), DOC.as_bytes());
        // Two programs: refused.
        std::fs::write(&p.pre_xml, DOC.replace("Haus", "Halle")).unwrap();
        let two = read_pre_push(&p, d).unwrap_err();
        assert!(two.contains("hold different programs"), "{two}");
        // A ZIP that cannot be read: refused, whatever the .Loxone says.
        std::fs::write(&p.pre_xml, DOC).unwrap();
        std::fs::write(&p.pre_zip, b"PK\x03\x04 broken").unwrap();
        let bad = read_pre_push(&p, d).unwrap_err();
        assert!(
            bad.starts_with("cannot read 20261107-101500-pre-push.zip"),
            "{bad}"
        );
        // Not a config at all.
        std::fs::remove_file(&p.pre_zip).unwrap();
        std::fs::write(&p.pre_xml, "").unwrap();
        assert!(read_pre_push(&p, d).is_err());
    }

    #[test]
    fn a_compile_must_give_back_the_backup() {
        let doc = DOC.as_bytes();
        assert_eq!(same_program(doc, doc), Ok(true));
        let fixture = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/house.Loxone"),
        )
        .unwrap();
        let text = String::from_utf8(fixture.clone()).unwrap();
        // Only formatting differs: the same program.
        let spaced = text.replacen("\n", "\n\n", 1);
        assert_eq!(same_program(&fixture, spaced.as_bytes()), Ok(false));
        let renamed = text.replace("Title=\"Licht OG\"", "Title=\"Licht oben\"");
        let why = same_program(&fixture, renamed.as_bytes()).unwrap_err();
        assert!(why.contains("Licht"), "{why}");
    }
}
