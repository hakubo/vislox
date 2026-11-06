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

#[derive(Default)]
struct Inner {
    listing: Option<Listing>,
    /// File name to blob id of the cached program.
    index: BTreeMap<String, String>,
    /// Downloaded so far, to download, in the read running now.
    progress: Option<(usize, usize)>,
    error: Option<String>,
}

pub struct Backups {
    port: Result<Arc<dyn Port>, String>,
    /// `<cache>/<miniserver>/backups`; none: nothing is cached.
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    running: Arc<tokio::sync::Mutex<()>>,
}

impl Backups {
    /// `key` names the Miniserver (its serial, else its address), as for the
    /// structure file's cache.
    pub fn new(port: Result<Arc<dyn Port>, String>, key: Option<&str>) -> Backups {
        let dir = key.and_then(|k| Some(cache_root()?.join(file_name(k)).join("backups")));
        Self::with_dir(port, dir)
    }

    fn with_dir(port: Result<Arc<dyn Port>, String>, dir: Option<PathBuf>) -> Backups {
        let mut inner = Inner::default();
        if port.is_ok()
            && let Some(d) = &dir
        {
            let read = |name: &str| std::fs::read(d.join(name)).ok();
            inner.listing = read("list.json").and_then(|b| serde_json::from_slice(&b).ok());
            inner.index = read("index.json")
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
        }
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
        if let Some(why) = self.why_not() {
            return json!({ "available": false, "reason": why });
        }
        let i = self.inner.lock().unwrap();
        json!({
            "available": true,
            "busy": self.running.try_lock().is_err(),
            "listed": i.listing.as_ref().map(|l| l.at),
            "on_miniserver": i.listing.as_ref().map(|l| l.files.len()),
            "cached": self.entries_of(&i).len(),
            "progress": i.progress,
            "error": i.error,
        })
    }

    /// The downloaded backups, for the History.
    pub fn entries(&self) -> Vec<Backup> {
        self.entries_of(&self.inner.lock().unwrap())
    }

    fn entries_of(&self, i: &Inner) -> Vec<Backup> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let listed = |f: &str| {
            i.listing
                .as_ref()
                .is_some_and(|l| l.files.iter().any(|x| x.file == f))
        };
        i.index
            .iter()
            .filter_map(|(file, blob)| {
                let b = loxone_client::ftp::parse_backup_name(file, 0)?;
                let path = dir.join(format!("{blob}.Loxone"));
                path.is_file().then(|| Backup {
                    file: file.clone(),
                    version: b.version,
                    stamp: b.timestamp,
                    blob: blob.clone(),
                    path,
                    on_miniserver: listed(file),
                })
            })
            .collect()
    }

    /// List the backups, then download the new ones, in the background;
    /// `changed` is called after each step. One read at a time.
    pub fn start(self: &Arc<Self>, changed: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        let port = self.port.clone()?;
        let one = self
            .running
            .clone()
            .try_lock_owned()
            .map_err(|_| "the backups are being read already".to_string())?;
        let me = self.clone();
        tokio::spawn(async move {
            let _one = one;
            let res = me.read(port, &*changed).await;
            let mut i = me.inner.lock().unwrap();
            i.progress = None;
            i.error = res.err();
            drop(i);
            changed();
        });
        Ok(())
    }

    async fn read(
        &self,
        port: Arc<dyn Port>,
        changed: &(dyn Fn() + Send + Sync),
    ) -> Result<(), String> {
        self.inner.lock().unwrap().error = None;
        changed();
        let p = port.clone();
        let files = blocking(LIST_TIMEOUT, "listing the backups", move || p.list()).await?;
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
                .filter(|f| !i.index.contains_key(&f.file) || !self.cached(&i.index[&f.file]))
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
        for (n, f) in todo.into_iter().enumerate() {
            if n > 0 {
                tokio::time::sleep(PAUSE).await;
            }
            let p = port.clone();
            let file = f.file.clone();
            let what = format!("downloading {}", f.file);
            let zip = blocking(DOWNLOAD_TIMEOUT, &what, move || p.download(&file)).await?;
            let xml = loxone_config::archive::extract_and_decompress(&zip)
                .map_err(|e| format!("{}: {e}", f.file))?;
            let blob = crate::history::blob_id(&xml);
            if let Some(d) = &self.dir {
                write_file(&d.join(format!("{blob}.Loxone")), &xml)
                    .map_err(|e| format!("cannot cache {}: {e}", f.file))?;
            }
            let index = {
                let mut i = self.inner.lock().unwrap();
                i.index.insert(f.file.clone(), blob);
                i.progress = Some((n + 1, total));
                i.index.clone()
            };
            self.save("index.json", &index);
            changed();
        }
        Ok(())
    }

    fn cached(&self, blob: &str) -> bool {
        self.dir
            .as_ref()
            .is_some_and(|d| d.join(format!("{blob}.Loxone")).is_file())
    }

    fn save(&self, name: &str, v: &impl Serialize) {
        if let Some(d) = &self.dir
            && let Ok(bytes) = serde_json::to_vec_pretty(v)
        {
            let _ = write_file(&d.join(name), &bytes);
        }
    }
}

/// Run a blocking FTP call on its own thread, with a time limit.
async fn blocking<T: Send + 'static>(
    limit: Duration,
    what: &str,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(f)).await {
        Err(_) => Err(format!("{what} took longer than {} s", limit.as_secs())),
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

    #[tokio::test]
    async fn backups_are_downloaded_once_and_cached_by_blob_id() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::new(&[
            ("sps_273_20261101100000.zip", b"<C v=\"273\"/>"),
            ("sps_274_20261102100000.zip", b"<C v=\"274\"/>"),
        ]);
        let b = Arc::new(Backups::with_dir(
            Ok(fake.clone() as Arc<dyn Port>),
            Some(d.path().to_path_buf()),
        ));
        assert!(!b.listed());
        read_all(&b).await;
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
        let e = b.entries();
        assert_eq!(e.len(), 2);
        let first = e.iter().find(|x| x.version == 273).unwrap();
        assert_eq!(first.stamp, "20261101100000");
        assert_eq!(first.blob, crate::history::blob_id(b"<C v=\"273\"/>"));
        assert_eq!(std::fs::read(&first.path).unwrap(), b"<C v=\"273\"/>");
        assert!(first.on_miniserver);
        assert_eq!(b.info()["cached"], 2);
        assert_eq!(b.info()["on_miniserver"], 2);
        assert!(b.info()["error"].is_null());

        // Read again: nothing new is downloaded.
        read_all(&b).await;
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);

        // A new start finds the cache; a backup the Miniserver deleted stays.
        fake.files.lock().unwrap().remove(0);
        let b = Arc::new(Backups::with_dir(
            Ok(fake.clone() as Arc<dyn Port>),
            Some(d.path().to_path_buf()),
        ));
        assert!(b.listed());
        assert_eq!(b.entries().len(), 2);
        read_all(&b).await;
        assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
        let e = b.entries();
        assert!(!e.iter().find(|x| x.version == 273).unwrap().on_miniserver);
        assert!(e.iter().find(|x| x.version == 274).unwrap().on_miniserver);
    }

    #[tokio::test]
    async fn a_broken_backup_stops_the_read_with_the_reason() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::new(&[("sps_1_20261101100000.zip", b"<C/>")]);
        fake.files.lock().unwrap()[0].1 = b"PK not a zip".to_vec();
        let b = Arc::new(Backups::with_dir(
            Ok(fake as Arc<dyn Port>),
            Some(d.path().to_path_buf()),
        ));
        read_all(&b).await;
        let why = b.info()["error"].as_str().unwrap().to_string();
        assert!(why.starts_with("sps_1_20261101100000.zip: "), "{why}");
        assert!(b.entries().is_empty());
    }

    #[tokio::test]
    async fn off_without_a_miniserver_and_one_read_at_a_time() {
        let b = Arc::new(Backups::with_dir(
            Err("turned off (--no-live)".into()),
            None,
        ));
        assert_eq!(b.info()["available"], false);
        assert_eq!(b.info()["reason"], "turned off (--no-live)");
        assert!(b.start(Arc::new(|| {})).is_err());

        let fake = Fake::new(&[("sps_1_20261101100000.zip", b"<C/>")]);
        let b = Arc::new(Backups::with_dir(Ok(fake as Arc<dyn Port>), None));
        let _held = b.running.clone().try_lock_owned().unwrap();
        assert_eq!(
            b.start(Arc::new(|| {})).unwrap_err(),
            "the backups are being read already"
        );
    }
}
