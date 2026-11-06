//! The Miniserver's own config backups: every program it loads leaves
//! `/prog/sps_<ver>_<YYYYMMDDHHMMSS>.zip` behind, and older ones stay on its
//! SD card. They fill the History between two Pulls: each save from Loxone
//! Config, made by anyone, can be looked at on its own.
//!
//! Reading them is read-only and only happens when a user asks (the
//! History's *Read backups*, or a Pull): lox's `ftp::list_backups` lists
//! them, and each one not seen before is downloaded once with
//! `ftp::download_backup`, one at a time, and turned into its program with
//! `archive::extract_and_decompress` (which checks the LoxCC size and
//! CRC-32): the bytes `lox config pull` would commit. The program is cached
//! by its git blob id (`<cache>/<miniserver>/backups/<blob>.Loxone`, with an
//! `index.json` from file name to blob id), so it is never downloaded again
//! and stays in the History after the Miniserver has deleted the backup.
//! A backup that cannot be downloaded or read is noted with the reason and
//! tried again on the next read; the others go on.
//!
//! The stamp in a backup's name is the Miniserver's local time. Its zone is
//! the `Timezone` of the config (`Europe/Warsaw`), kept with each backup.

use crate::history::Backup;
use crate::structure::{cache_root, file_name, write_file};
use loxone_client::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A listing that takes longer than this has hung.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// A download that takes longer than this has hung.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// A pause between two downloads, to keep the load on the Miniserver low.
const PAUSE: Duration = Duration::from_millis(500);
/// At most this many (the newest) are downloaded in one read.
const MAX_DOWNLOADS: usize = 60;

/// One backup the Miniserver listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    pub file: String,
    pub version: u32,
    /// `YYYYMMDDHHMMSS`, the Miniserver's local time.
    pub stamp: String,
    pub size: u64,
}

/// How backups are read from the Miniserver (lox's FTP client, or a stand-in
/// in tests). Blocking.
pub trait Port: Send + Sync {
    fn list(&self) -> Result<Vec<Listed>, String>;
    fn download(&self, file: &str) -> Result<Vec<u8>, String>;
}

/// lox's FTP client, with the user lox logs in with.
pub struct Ftp(pub Connection);

impl Port for Ftp {
    fn list(&self) -> Result<Vec<Listed>, String> {
        let list = loxone_client::ftp::list_backups(&self.0).map_err(|e| format!("{e:#}"))?;
        Ok(list
            .into_iter()
            .map(|b| Listed {
                file: b.filename,
                version: b.version,
                stamp: b.timestamp,
                size: b.size,
            })
            .collect())
    }

    fn download(&self, file: &str) -> Result<Vec<u8>, String> {
        loxone_client::ftp::download_backup(&self.0, file).map_err(|e| format!("{e:#}"))
    }
}

/// The last listing, as kept on disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Listing {
    /// When it was read, Unix seconds.
    at: i64,
    files: Vec<Listed>,
}

/// What became of a backup vislox downloaded, as kept in `index.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// The blob id of its program, cached as `<blob>.Loxone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    /// The `Timezone` of its config (`Europe/Warsaw`): its stamp is in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zone: Option<String>,
    /// Why it could not be read (it is tried again on the next read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Default)]
struct Inner {
    listing: Option<Listing>,
    /// File name to what became of it.
    index: BTreeMap<String, Entry>,
    /// Downloaded so far, to download, in the read running now.
    progress: Option<(usize, usize)>,
    error: Option<String>,
    /// A listing or download took too long: the next read waits until its
    /// FTP session has ended.
    stuck: bool,
}

pub struct Backups {
    /// How to reach the Miniserver; an error: why backups cannot be read.
    port: Result<Arc<dyn Port>, String>,
    /// `<cache>/<miniserver>/backups`.
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    running: Arc<tokio::sync::Mutex<()>>,
}

/// What a download made of a backup (on the blocking thread).
type Fetched = Result<(String, Option<String>), String>;

impl Backups {
    /// `key` names the Miniserver (its serial, else its address), as for the
    /// structure file's cache.
    pub fn new(port: Result<Arc<dyn Port>, String>, key: Option<&str>) -> Backups {
        let dir = key.and_then(|k| Some(cache_root()?.join(file_name(k)).join("backups")));
        Self::with_dir(port, dir)
    }

    /// The cache is read whether or not the Miniserver can be reached: the
    /// backups downloaded before (and links to them) stay.
    fn with_dir(port: Result<Arc<dyn Port>, String>, dir: Option<PathBuf>) -> Backups {
        let mut inner = Inner::default();
        if let Some(d) = &dir {
            let read = |name: &str| std::fs::read(d.join(name)).ok();
            inner.listing = read("list.json").and_then(|b| serde_json::from_slice(&b).ok());
            inner.index = read("index.json")
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
        }
        // Without a cache, every read would download them all again.
        let port = match (port, &dir) {
            (Ok(_), None) => Err(
                "no cache directory (set VISLOX_CACHE, XDG_CACHE_HOME or HOME), so downloaded \
                 backups could not be kept"
                    .to_string(),
            ),
            (p, _) => p,
        };
        Backups {
            port,
            dir,
            inner: Mutex::new(inner),
            running: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn why_not(&self) -> Option<&str> {
        self.port.as_ref().err().map(String::as_str)
    }

    /// The backups were listed before (in this run or an earlier one).
    pub fn listed(&self) -> bool {
        self.inner.lock().unwrap().listing.is_some()
    }

    /// For `/api/history`.
    pub fn info(&self) -> Value {
        let i = self.inner.lock().unwrap();
        let broken: Vec<Value> = i
            .index
            .iter()
            .filter_map(|(file, e)| {
                let why = e.error.as_ref()?;
                let b = loxone_client::ftp::parse_backup_name(file, 0)?;
                Some(json!({
                    "file": file,
                    "version": b.version,
                    "stamp": b.timestamp,
                    "error": why,
                }))
            })
            .collect();
        json!({
            "available": self.port.is_ok(),
            "reason": self.why_not(),
            "busy": self.running.try_lock().is_err(),
            "listed": i.listing.as_ref().map(|l| l.at),
            "on_miniserver": i.listing.as_ref().map(|l| l.files.len()),
            "cached": i.index.values().filter(|e| e.blob.is_some()).count(),
            "broken": broken,
            "progress": i.progress,
            "error": i.error,
            "stuck": i.stuck,
        })
    }

    /// The downloaded backups and the Miniserver's time zone (the one the
    /// newest backup's config names), for the History. From the index alone:
    /// a program is indexed only once it was written.
    pub fn entries(&self) -> (Vec<Backup>, Option<chrono_tz::Tz>) {
        let i = self.inner.lock().unwrap();
        let Some(dir) = &self.dir else {
            return (Vec::new(), None);
        };
        let listed: std::collections::HashSet<&str> = i
            .listing
            .iter()
            .flat_map(|l| l.files.iter().map(|f| f.file.as_str()))
            .collect();
        let mut zone = None;
        let mut out = Vec::new();
        for (file, e) in &i.index {
            let (Some(blob), Some(b)) = (&e.blob, loxone_client::ftp::parse_backup_name(file, 0))
            else {
                continue;
            };
            if let Some(z) = e.zone.as_deref().and_then(|z| z.parse().ok())
                && zone.as_ref().is_none_or(|(s, _)| *s < b.timestamp)
            {
                zone = Some((b.timestamp.clone(), z));
            }
            out.push(Backup {
                file: file.clone(),
                version: b.version,
                stamp: b.timestamp,
                blob: blob.clone(),
                path: dir.join(format!("{blob}.Loxone")),
                on_miniserver: listed.contains(file.as_str()),
            });
        }
        (out, zone.map(|(_, z)| z))
    }

    /// List the backups, then download the new ones, in the background;
    /// `changed` is called after each step. One read at a time, and at most
    /// one FTP session: a read that gave up on a session that hangs holds on
    /// until it has ended.
    pub fn start(self: &Arc<Self>, changed: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        let port = self.port.clone()?;
        let one = self.running.clone().try_lock_owned().map_err(|_| {
            if self.inner.lock().unwrap().stuck {
                "the last read is still waiting for an FTP session that hangs to end".to_string()
            } else {
                "the backups are being read already".to_string()
            }
        })?;
        let me = self.clone();
        tokio::spawn(async move {
            let _one = one;
            let mut stray = None;
            let res = me.read(port, &*changed, &mut stray).await;
            {
                let mut i = me.inner.lock().unwrap();
                i.progress = None;
                i.error = res.err();
                i.stuck = stray.is_some();
            }
            changed();
            if let Some(h) = stray {
                let _ = h.await;
                me.inner.lock().unwrap().stuck = false;
                changed();
            }
        });
        Ok(())
    }

    async fn read(
        &self,
        port: Arc<dyn Port>,
        changed: &(dyn Fn() + Send + Sync),
        stray: &mut Option<tokio::task::JoinHandle<()>>,
    ) -> Result<(), String> {
        self.inner.lock().unwrap().error = None;
        changed();
        let p = port.clone();
        let files = blocking(LIST_TIMEOUT, "listing the backups", stray, move || p.list()).await?;
        let listing = Listing {
            at: chrono::Utc::now().timestamp(),
            files,
        };
        self.save("list.json", &listing);
        let mut todo: Vec<Listed> = {
            let mut i = self.inner.lock().unwrap();
            let todo = listing
                .files
                .iter()
                .filter(|f| i.index.get(&f.file).is_none_or(|e| e.blob.is_none()))
                .cloned()
                .collect();
            i.listing = Some(listing);
            todo
        };
        todo.sort_by(|a, b| b.stamp.cmp(&a.stamp));
        todo.truncate(MAX_DOWNLOADS);
        let total = todo.len();
        self.inner.lock().unwrap().progress = Some((0, total));
        changed();
        let dir = self.dir.clone().ok_or("no cache directory")?;
        for (n, f) in todo.into_iter().enumerate() {
            if n > 0 {
                tokio::time::sleep(PAUSE).await;
            }
            let p = port.clone();
            let file = f.file.clone();
            let d = dir.clone();
            let what = format!("downloading {}", f.file);
            // A download that does not end stops the read; one that fails,
            // or a backup that cannot be read, is noted and the read goes on.
            let fetched: Fetched = blocking(DOWNLOAD_TIMEOUT, &what, stray, move || {
                Ok(fetch(&*p, &file, &d))
            })
            .await?;
            let index = {
                let mut i = self.inner.lock().unwrap();
                let e = match fetched {
                    Ok((blob, zone)) => Entry {
                        blob: Some(blob),
                        zone,
                        error: None,
                    },
                    Err(why) => Entry {
                        error: Some(why),
                        ..Entry::default()
                    },
                };
                i.index.insert(f.file.clone(), e);
                i.progress = Some((n + 1, total));
                i.index.clone()
            };
            self.save("index.json", &index);
            changed();
        }
        Ok(())
    }

    fn save(&self, name: &str, v: &impl Serialize) {
        if let Some(d) = &self.dir
            && let Ok(bytes) = serde_json::to_vec_pretty(v)
        {
            let _ = write_file(&d.join(name), &bytes);
        }
    }
}

/// Download one backup, check and decompress it, and cache its program by
/// blob id: (blob id, its config's time zone). Blocking.
fn fetch(port: &dyn Port, file: &str, dir: &std::path::Path) -> Fetched {
    let zip = port.download(file)?;
    let xml = loxone_config::archive::extract_and_decompress(&zip).map_err(|e| e.to_string())?;
    let blob = crate::history::blob_id(&xml);
    write_file(&dir.join(format!("{blob}.Loxone")), &xml)
        .map_err(|e| format!("cannot cache it: {e}"))?;
    Ok((blob, time_zone(&xml)))
}

/// The `Timezone` of a config's `Document` (`Europe/Warsaw`), if it names one
/// vislox knows.
pub fn time_zone(xml: &[u8]) -> Option<String> {
    let head = &xml[..xml.len().min(256 * 1024)];
    let text = String::from_utf8_lossy(head);
    let doc = &text[text.find("<C Type=\"Document\"")?..];
    let doc = &doc[..doc.find('>')?];
    let v = doc.split(" Timezone=\"").nth(1)?.split('"').next()?;
    v.parse::<chrono_tz::Tz>().ok().map(|_| v.to_string())
}

/// Run a blocking FTP call on its own thread, with a time limit. When the
/// limit is hit, the call goes on: its handle is left in `stray`, so the read
/// can wait for it before another one starts.
async fn blocking<T: Send + 'static>(
    limit: Duration,
    what: &str,
    stray: &mut Option<tokio::task::JoinHandle<()>>,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let mut h = tokio::task::spawn_blocking(f);
    match tokio::time::timeout(limit, &mut h).await {
        Err(_) => {
            *stray = Some(tokio::spawn(async move {
                let _ = h.await;
            }));
            Err(format!("{what} took longer than {} s", limit.as_secs()))
        }
        Ok(Err(e)) => Err(format!("{what}: {e}")),
        Ok(Ok(r)) => r.map_err(|e| format!("{what}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A backup ZIP holding `xml`, as the Miniserver writes it.
    fn backup_zip(xml: &[u8]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("sps0.LoxCC", o).unwrap();
        w.write_all(&loxone_config::archive::compress_loxcc(xml).unwrap())
            .unwrap();
        w.finish().unwrap().into_inner()
    }

    struct Fake {
        files: Mutex<Vec<(Listed, Vec<u8>)>>,
        downloads: AtomicUsize,
    }

    impl Fake {
        fn new(files: &[(&str, &[u8])]) -> Arc<Fake> {
            Arc::new(Fake {
                files: Mutex::new(
                    files
                        .iter()
                        .map(|(name, xml)| {
                            let b = loxone_client::ftp::parse_backup_name(name, 0).unwrap();
                            (
                                Listed {
                                    file: b.filename,
                                    version: b.version,
                                    stamp: b.timestamp,
                                    size: 0,
                                },
                                backup_zip(xml),
                            )
                        })
                        .collect(),
                ),
                downloads: AtomicUsize::new(0),
            })
        }
    }

    impl Port for Fake {
        fn list(&self) -> Result<Vec<Listed>, String> {
            Ok(self
                .files
                .lock()
                .unwrap()
                .iter()
                .map(|f| f.0.clone())
                .collect())
        }
        fn download(&self, file: &str) -> Result<Vec<u8>, String> {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            self.files
                .lock()
                .unwrap()
                .iter()
                .find(|f| f.0.file == file)
                .map(|f| f.1.clone())
                .ok_or_else(|| format!("550 {file}: no such file"))
        }
    }

    async fn read_all(b: &Arc<Backups>) {
        b.start(Arc::new(|| {})).unwrap();
        // Wait for the read to finish, with a time limit.
        let _done = tokio::time::timeout(Duration::from_secs(10), b.running.lock())
            .await
            .expect("the read finishes");
    }

    fn with(fake: &Arc<Fake>, dir: &std::path::Path) -> Arc<Backups> {
        Arc::new(Backups::with_dir(
            Ok(fake.clone() as Arc<dyn Port>),
            Some(dir.to_path_buf()),
        ))
    }

    #[tokio::test]
    async fn backups_are_downloaded_once_and_cached_by_blob_id() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::new(&[
            (
                "sps_273_20261101100000.zip",
                b"<C Type=\"Document\" Timezone=\"Europe/Warsaw\" V=\"273\"/>",
            ),
            ("sps_274_20261102100000.zip", b"<C v=\"274\"/>"),
        ]);
        let b = with(&fake, d.path());
        assert!(!b.listed());
        read_all(&b).await;
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
        let (e, zone) = b.entries();
        assert_eq!(e.len(), 2);
        // The zone of the newest backup that names one.
        assert_eq!(zone, Some(chrono_tz::Europe::Warsaw));
        let first = e.iter().find(|x| x.version == 273).unwrap();
        assert_eq!(first.stamp, "20261101100000");
        assert!(first.on_miniserver);
        assert_eq!(
            std::fs::read(&first.path).unwrap(),
            b"<C Type=\"Document\" Timezone=\"Europe/Warsaw\" V=\"273\"/>"
        );
        assert_eq!(b.info()["cached"], 2);
        assert_eq!(b.info()["on_miniserver"], 2);
        assert!(b.info()["error"].is_null());

        // Read again: nothing new is downloaded.
        read_all(&b).await;
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);

        // A new start finds the cache; a backup the Miniserver deleted stays.
        fake.files.lock().unwrap().remove(0);
        let b = with(&fake, d.path());
        assert!(b.listed());
        assert_eq!(b.entries().0.len(), 2);
        read_all(&b).await;
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
        let (e, _) = b.entries();
        assert!(!e.iter().find(|x| x.version == 273).unwrap().on_miniserver);
        assert!(e.iter().find(|x| x.version == 274).unwrap().on_miniserver);
    }

    #[tokio::test]
    async fn a_broken_backup_is_noted_and_the_others_are_still_read() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::new(&[
            ("sps_1_20261101100000.zip", b"<C n=\"1\"/>"),
            ("sps_2_20261102100000.zip", b"<C n=\"2\"/>"),
        ]);
        // The newest, read first, is not a whole backup (still being written).
        fake.files.lock().unwrap()[1].1 = b"PK not a zip".to_vec();
        let b = with(&fake, d.path());
        read_all(&b).await;
        let info = b.info();
        assert!(info["error"].is_null(), "{info}");
        assert_eq!(info["broken"][0]["file"], "sps_2_20261102100000.zip");
        assert_eq!(info["broken"][0]["version"], 2);
        assert!(info["broken"][0]["error"].as_str().unwrap().len() > 3);
        let (e, _) = b.entries();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].version, 1);
        // Whole now: the next read takes it, and it is no longer broken.
        fake.files.lock().unwrap()[1].1 = backup_zip(b"<C n=\"2\"/>");
        read_all(&b).await;
        assert_eq!(b.entries().0.len(), 2);
        assert_eq!(b.info()["broken"], json!([]));
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn the_cache_shows_without_a_miniserver() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::new(&[("sps_1_20261101100000.zip", b"<C/>")]);
        read_all(&with(&fake, d.path())).await;
        // Started with --no-live (or no lox setup): nothing is read, but the
        // backups downloaded before are there.
        let b = Arc::new(Backups::with_dir(
            Err("turned off (--no-live)".into()),
            Some(d.path().to_path_buf()),
        ));
        assert_eq!(b.entries().0.len(), 1);
        assert_eq!(b.info()["available"], false);
        assert_eq!(b.info()["reason"], "turned off (--no-live)");
        assert_eq!(b.info()["cached"], 1);
        assert!(b.start(Arc::new(|| {})).is_err());
    }

    #[tokio::test]
    async fn no_read_without_a_cache_and_one_read_at_a_time() {
        let fake = Fake::new(&[("sps_1_20261101100000.zip", b"<C/>")]);
        let b = Arc::new(Backups::with_dir(Ok(fake.clone() as Arc<dyn Port>), None));
        let why = b.start(Arc::new(|| {})).unwrap_err();
        assert!(why.starts_with("no cache directory"), "{why}");
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);

        let d = tempfile::tempdir().unwrap();
        let b = with(&fake, d.path());
        let _held = b.running.clone().try_lock_owned().unwrap();
        assert_eq!(
            b.start(Arc::new(|| {})).unwrap_err(),
            "the backups are being read already"
        );
    }

    #[tokio::test]
    async fn a_hung_ftp_call_holds_the_next_read_until_it_ends() {
        struct Hang(Mutex<Option<std::sync::mpsc::Receiver<()>>>);
        impl Port for Hang {
            fn list(&self) -> Result<Vec<Listed>, String> {
                if let Some(rx) = self.0.lock().unwrap().take() {
                    let _ = rx.recv();
                }
                Ok(Vec::new())
            }
            fn download(&self, _: &str) -> Result<Vec<u8>, String> {
                Err("no".into())
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let d = tempfile::tempdir().unwrap();
        let b = Arc::new(Backups::with_dir(
            Ok(Arc::new(Hang(Mutex::new(Some(rx)))) as Arc<dyn Port>),
            Some(d.path().to_path_buf()),
        ));
        let mut stray = None;
        let p = b.port.clone().unwrap();
        let r = blocking(
            Duration::from_millis(50),
            "listing",
            &mut stray,
            move || p.list(),
        )
        .await;
        assert_eq!(r.unwrap_err(), "listing took longer than 0 s");
        let h = stray.expect("the call goes on");
        assert!(!h.is_finished());
        tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .expect("it ends once the call does")
            .unwrap();
    }
}
