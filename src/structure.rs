//! What the app shows: the Miniserver's structure file (`LoxAPP3.json`).
//!
//! It lists the controls the app shows (block UUID → name, control type,
//! room, category, icon), the rooms and the categories. vislox reads it when
//! live values start (each live session fetches it) and when a user asks for
//! it (`POST /api/app/load`); it never fetches it on its own. The last one
//! is kept on disk per Miniserver, so the In the app section has it after a
//! restart, with the time it was read.
//!
//! Icons are downloaded from the Miniserver by the name the structure file
//! gives (`IconsFilled/lightbulb.svg`, or an older `<uuid>`, fetched as
//! `<uuid>.svg`) and cached on disk: a name keeps its icon. Only names the
//! structure file lists are fetched, so `/api/app/icon` cannot be used to
//! read anything else from the Miniserver.

use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Semaphore, broadcast};

/// The structure file as the browser receives it, and the icon names in it.
struct Summary {
    json: Arc<String>,
    icons: Arc<HashSet<String>>,
}

#[derive(Default)]
struct Inner {
    raw: Option<Arc<Value>>,
    /// When it was last read (Unix ms), and from where.
    fetched: Option<i64>,
    from: Option<&'static str>,
    /// Successful reads since start: after an Apply, the browser waits for
    /// one more than it saw when Apply started (no clocks compared).
    reads: u64,
    /// The last load's error.
    error: Option<String>,
    /// The summary of exactly this state; dropped on every change, and
    /// built under the same lock, so it can never describe an older one.
    summary: Option<Summary>,
}

/// What the structure changed: the browser re-reads it only when it did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A new structure file, or a new error.
    Changed,
    /// Read again, unchanged.
    Read,
}

pub struct Structure {
    conn: Result<Arc<crate::route::Handle>, String>,
    /// This Miniserver's cache directory.
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    /// Cache writes, one at a time, in the order the states were taken.
    io: Mutex<()>,
    tx: broadcast::Sender<Event>,
    /// One load at a time.
    loading: tokio::sync::Mutex<()>,
    /// The Miniserver serves few requests at once.
    icon_gate: Semaphore,

    /// One download per icon name at a time.
    icon_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// Where vislox keeps what it caches: `VISLOX_CACHE`, else
/// `$XDG_CACHE_HOME/vislox`, else `~/.cache/vislox`.
pub fn cache_root() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    env("VISLOX_CACHE")
        .or_else(|| env("XDG_CACHE_HOME").map(|d| d.join("vislox")))
        .or_else(|| env("HOME").map(|h| h.join(".cache").join("vislox")))
}

/// A file name for a key: letters, digits, `.`, `-` and `_` only.
pub(crate) fn file_name(key: &str) -> String {
    let s: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.trim_start_matches('.').to_string()
}

/// The cache file of an icon: its name as a file name, plus a hash when
/// characters had to be replaced, so `a/b.svg` and `a_b.svg` never share one.
fn icon_file(name: &str) -> String {
    use sha1::Digest;
    let f = file_name(name);
    if f == name {
        f
    } else {
        format!("{f}.{}", &hex::encode(sha1::Sha1::digest(name))[..12])
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Write `bytes` to `path` whole: a reader never sees half a file. Each
/// write has a temporary file of its own, so two writers cannot mix.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    let res = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// What the browser needs from the structure file: the controls (with
/// their sub-controls), the rooms and the categories, each with its icon.
fn summarize(raw: &Value) -> (Map<String, Value>, HashSet<String>) {
    let mut icons = HashSet::new();
    let mut icon = |v: &Value| -> Value {
        // Only names of the icon shape: anything else is never fetched.
        match v.as_str().filter(|s| icon_path(s).is_some()) {
            Some(s) => {
                icons.insert(s.to_string());
                Value::from(s)
            }
            None => Value::Null,
        }
    };
    let mut places = |key: &str| -> Value {
        let mut out = Map::new();
        for (uuid, r) in raw[key].as_object().into_iter().flatten() {
            out.insert(
                uuid.clone(),
                json!({ "name": r["name"], "image": icon(&r["image"]), "color": r["color"] }),
            );
        }
        Value::Object(out)
    };
    let rooms = places("rooms");
    let cats = places("cats");
    let mut controls = Map::new();
    fn add(
        controls: &mut Map<String, Value>,
        icon: &mut dyn FnMut(&Value) -> Value,
        uuid: &str,
        c: &Value,
        parent: Option<&str>,
    ) {
        controls.insert(
            uuid.to_string(),
            json!({
                "name": c["name"],
                "type": c["type"],
                "room": c["room"],
                "cat": c["cat"],
                "image": icon(&c["defaultIcon"]),
                "parent": parent,
            }),
        );
        for (su, sub) in c["subControls"].as_object().into_iter().flatten() {
            add(controls, icon, su, sub, Some(uuid));
        }
    }
    for (uuid, c) in raw["controls"].as_object().into_iter().flatten() {
        add(&mut controls, &mut icon, uuid, c, None);
    }
    let ms = &raw["msInfo"];
    let mut out = Map::new();
    out.insert("lastModified".into(), raw["lastModified"].clone());
    out.insert("serial".into(), ms["serialNr"].clone());
    out.insert("msName".into(), ms["msName"].clone());
    out.insert("rooms".into(), rooms);
    out.insert("cats".into(), cats);
    out.insert("controls".into(), Value::Object(controls));
    (out, icons)
}

/// Folders the Miniserver serves commands, data or files under: an icon
/// name never starts with one.
const NOT_ICONS: &[&str] = &[
    "jdev", "dev", "data", "sys", "prog", "stats", "log", "temp", "update", "web",
];

/// The request path of an icon name, `None` when the name does not have an
/// icon's shape, so that no name can be a command (`jdev/sps/io/<uuid>/1.0`):
/// - `<folder>/<file>.svg` or `.png` (`IconsFilled/lightbulb.svg`): one
///   folder, letters, digits, `-` and `_` only, and not a folder of
///   [`NOT_ICONS`];
/// - an icon UUID (`00000000-0000-0020-2000000000000000`), fetched as
///   `<uuid>.svg`, or with its `.svg`/`.png`.
fn icon_path(name: &str) -> Option<String> {
    let word = |w: &str| {
        !w.is_empty()
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    let uuid = |u: &str| {
        let parts: Vec<&str> = u.split('-').collect();
        parts.iter().map(|p| p.len()).eq([8, 4, 4, 16])
            && parts
                .iter()
                .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
    };
    if name.len() > 128 {
        return None;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e @ ("svg" | "png"))) => (s, Some(e)),
        Some(_) => return None,
        None => (name, None),
    };
    match stem.split_once('/') {
        Some((folder, file))
            if ext.is_some()
                && word(folder)
                && word(file)
                && !NOT_ICONS.iter().any(|n| folder.eq_ignore_ascii_case(n)) =>
        {
            Some(format!("/{name}"))
        }
        None if uuid(stem) => Some(format!("/{stem}.{}", ext.unwrap_or("svg"))),
        _ => None,
    }
}

/// The type of an icon the Miniserver sent: SVG comes as text (and is
/// often gzipped), PNG as binary.
pub fn icon_type(bytes: &[u8]) -> (&'static str, Option<&'static str>) {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        ("image/svg+xml", Some("gzip"))
    } else if bytes.starts_with(b"\x89PNG") {
        ("image/png", None)
    } else {
        ("image/svg+xml", None)
    }
}

/// Whether an icon arrived whole: a PNG up to its end chunk, an SVG (also
/// gzipped) up to its closing tag. Only a whole icon is cached.
pub fn icon_whole(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"\x89PNG") {
        return bytes.ends_with(b"IEND\xaeB`\x82");
    }
    let text;
    let svg: &[u8] = match gunzip(bytes) {
        Some(t) => {
            text = t;
            &text
        }
        None => bytes,
    };
    let s = String::from_utf8_lossy(svg);
    s.contains("<svg") && s.trim_end().ends_with("</svg>")
}

/// A gzipped icon, unpacked; `None` when not gzipped. The Miniserver may
/// leave out the gzip trailer (its checksum and length) after a whole SVG,
/// so what was unpacked counts even when the stream ends early; whether
/// the SVG is whole is checked on the text.
fn gunzip(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    let mut text = Vec::new();
    // Bounded: an icon is small. On an error, `text` keeps what was read.
    let _ = flate2::read::GzDecoder::new(bytes)
        .take(4 << 20)
        .read_to_end(&mut text);
    Some(text)
}

impl Structure {
    /// `conn`: the Miniserver, or why there is none. `key`: what names it in
    /// the cache (its serial, else its address).
    pub fn new(
        conn: Result<Arc<crate::route::Handle>, String>,
        key: Option<&str>,
    ) -> Arc<Structure> {
        let dir = key.and_then(|k| Some(cache_root()?.join(file_name(k))));
        Self::with_dir(conn, dir)
    }

    fn with_dir(
        conn: Result<Arc<crate::route::Handle>, String>,
        dir: Option<PathBuf>,
    ) -> Arc<Structure> {
        let (tx, _) = broadcast::channel(8);
        let s = Arc::new(Structure {
            conn,
            dir,
            inner: Mutex::new(Inner::default()),
            io: Mutex::new(()),
            tx,
            loading: tokio::sync::Mutex::new(()),
            icon_gate: Semaphore::new(4),

            icon_locks: Mutex::new(HashMap::new()),
        });
        // The last one read, from an earlier run.
        if s.conn.is_ok()
            && let Some(path) = s.file()
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(raw) = serde_json::from_slice::<Value>(&bytes)
        {
            let when = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            let mut i = s.inner.lock().unwrap();
            i.raw = Some(Arc::new(raw));
            i.fetched = Some(when.unwrap_or_else(now_ms));
            i.from = Some("cache");
        }
        s
    }

    fn file(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("LoxAPP3.json"))
    }

    /// Successful reads since start; a failed load does not count.
    pub fn reads(&self) -> u64 {
        self.inner.lock().unwrap().reads
    }

    /// Told when the structure changes, or is read again unchanged.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// A structure file was read. Blocking (compares, and writes the cache
    /// when it changed): call it off the async runtime.
    fn put(&self, raw: Value, from: &'static str, clear_error: bool) {
        let _io = self.io.lock().unwrap();
        let changed = {
            let mut i = self.inner.lock().unwrap();
            let changed = i.raw.as_deref() != Some(&raw);
            if changed {
                i.raw = Some(Arc::new(raw));
            }
            i.fetched = Some(now_ms());
            i.from = Some(from);
            i.reads += 1;
            i.summary = None;
            let cleared = clear_error && i.error.take().is_some();
            (changed.then(|| i.raw.clone()).flatten(), cleared)
        };
        let (changed, cleared) = changed;
        if let Some(raw) = &changed
            && let Some(path) = self.file()
            && let Err(e) = serde_json::to_vec(&**raw)
                .map_err(std::io::Error::other)
                .and_then(|b| write_file(&path, &b))
        {
            eprintln!(
                "vislox: cannot cache the structure file in {}: {e}",
                path.display()
            );
        }
        let ev = if changed.is_some() || cleared {
            Event::Changed
        } else {
            Event::Read
        };
        let _ = self.tx.send(ev);
    }

    /// A structure file the live session read (blocking, see [`Self::put`]).
    /// It leaves an earlier load's error as it is: that was another read.
    pub fn set(&self, raw: Value) {
        self.put(raw, "live values", false);
    }

    /// Read the structure file from the Miniserver now.
    pub async fn load(self: &Arc<Self>) -> Result<(), String> {
        let conn = self.conn.clone()?.conn();
        let _one = self.loading.lock().await;
        let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
            let mut client = crate::tls::lox_client(&conn)?;
            client.get_structure().cloned().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match res {
            Ok(raw) => {
                let me = self.clone();
                tokio::task::spawn_blocking(move || me.put(raw, "Miniserver", true))
                    .await
                    .map_err(|e| e.to_string())
            }
            Err(e) => {
                let e = format!("cannot read the structure file: {e}");
                {
                    let _io = self.io.lock().unwrap();
                    let mut i = self.inner.lock().unwrap();
                    i.error = Some(e.clone());
                    i.summary = None;
                }
                let _ = self.tx.send(Event::Changed);
                Err(e)
            }
        }
    }

    /// The summary of the current state, built once per state.
    fn summary(&self) -> (Arc<String>, Arc<HashSet<String>>) {
        let mut i = self.inner.lock().unwrap();
        if let Some(s) = &i.summary {
            return (s.json.clone(), s.icons.clone());
        }
        let (mut out, icons) = match &i.raw {
            Some(raw) => summarize(raw),
            None => (Map::new(), HashSet::new()),
        };
        match &self.conn {
            Ok(_) => out.insert("available".into(), true.into()),
            Err(why) => {
                out.insert("reason".into(), why.clone().into());
                out.insert("available".into(), false.into())
            }
        };
        out.insert("loaded".into(), i.raw.is_some().into());
        out.insert("fetched".into(), json!(i.fetched));
        out.insert("from".into(), json!(i.from));
        out.insert("reads".into(), json!(i.reads));
        out.insert("error".into(), json!(i.error));
        let s = Summary {
            json: Arc::new(Value::Object(out).to_string()),
            icons: Arc::new(icons),
        };
        let r = (s.json.clone(), s.icons.clone());
        i.summary = Some(s);
        r
    }

    /// `GET /api/app`: `{available, reason?, loaded, fetched, from, reads,
    /// error, lastModified, serial, rooms, cats, controls}`.
    pub fn json(&self) -> Arc<String> {
        self.summary().0
    }

    /// An icon the structure file names: from the cache, else from the
    /// Miniserver (then cached, if it arrived whole). `Err(None)`: not a
    /// name it lists.
    pub async fn icon(&self, name: &str) -> Result<Vec<u8>, Option<String>> {
        // Only names the structure file lists, and only of an icon's shape.
        let Some(file) = icon_path(name).filter(|_| self.summary().1.contains(name)) else {
            return Err(None);
        };
        // Names that differ only in characters `file_name` replaces get
        // a hash of their own, so they never share a file.
        let path = self
            .dir
            .as_ref()
            .map(|d| d.join("icons").join(icon_file(name)));
        let cached = || path.as_ref().and_then(|p| std::fs::read(p).ok());
        if let Some(b) = cached() {
            return Ok(b);
        }
        let conn = self.conn.clone().map_err(Some)?.conn();
        // One download per name: the others wait, then find it cached.
        let lock = self
            .icon_locks
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_default()
            .clone();
        let _one = lock.lock().await;
        if let Some(b) = cached() {
            return Ok(b);
        }
        let _gate = self
            .icon_gate
            .acquire()
            .await
            .map_err(|e| Some(e.to_string()))?;
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
            let client = crate::tls::lox_client(&conn)?;
            client.get_bytes(&file).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| Some(e.to_string()))?
        .map_err(Some)?;
        if !icon_whole(&bytes) {
            return Err(Some(format!(
                "the Miniserver sent an incomplete icon {name} ({} bytes)",
                bytes.len()
            )));
        }
        // Kept and served unpacked: a browser refuses gzip without its trailer.
        let bytes = gunzip(&bytes).unwrap_or(bytes);
        if let Some(p) = &path {
            let _ = write_file(p, &bytes);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "lastModified": "2026-10-31 20:00:00",
            "msInfo": { "serialNr": "504F94000001", "msName": "Haus" },
            "rooms": { "r1": { "uuid": "r1", "name": "Küche", "image": "IconsFilled/kitchen.svg", "color": "#8DFF70" } },
            "cats": { "k1": { "uuid": "k1", "name": "Licht", "image": "00000000-0000-0020-2000000000000000" } },
            "controls": {
                "c1": { "name": "Deckenlicht", "type": "LightControllerV2", "room": "r1", "cat": "k1",
                        "defaultIcon": null, "states": {},
                        "subControls": { "c1/s": { "name": "Spot", "type": "Switch", "defaultIcon": "IconsFilled/spot.svg" } } }
            }
        })
    }

    #[test]
    fn the_summary_has_controls_rooms_categories_and_icons() {
        let (out, icons) = summarize(&sample());
        assert_eq!(out["controls"]["c1"]["room"], "r1");
        assert_eq!(out["controls"]["c1"]["type"], "LightControllerV2");
        assert_eq!(out["controls"]["c1"]["image"], Value::Null);
        assert_eq!(out["controls"]["c1/s"]["parent"], "c1");
        assert_eq!(out["rooms"]["r1"]["name"], "Küche");
        assert_eq!(out["serial"], "504F94000001");
        let mut icons: Vec<_> = icons.into_iter().collect();
        icons.sort();
        assert_eq!(
            icons,
            [
                "00000000-0000-0020-2000000000000000",
                "IconsFilled/kitchen.svg",
                "IconsFilled/spot.svg"
            ]
        );
    }

    #[test]
    fn icons_are_told_apart_by_their_bytes() {
        assert_eq!(icon_type(&[0x1f, 0x8b, 8]), ("image/svg+xml", Some("gzip")));
        assert_eq!(icon_type(b"\x89PNG\r\n"), ("image/png", None));
        assert_eq!(icon_type(b"<svg/>"), ("image/svg+xml", None));
    }

    #[test]
    fn a_gzipped_icon_without_its_trailer_is_whole_when_its_svg_is() {
        use std::io::Write;
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>\n";
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(svg).unwrap();
        let gz = e.finish().unwrap();
        assert!(icon_whole(&gz));
        // The Miniserver's way: the 8-byte trailer left out.
        let cut = &gz[..gz.len() - 8];
        assert!(icon_whole(cut));
        assert_eq!(gunzip(cut).unwrap(), svg);
        // Cut inside the SVG: not whole.
        assert!(!icon_whole(&gz[..gz.len() / 2]));
    }

    #[test]
    fn only_names_of_an_icons_shape_are_fetched() {
        assert_eq!(
            icon_path("IconsFilled/battery-full.svg").as_deref(),
            Some("/IconsFilled/battery-full.svg")
        );
        assert_eq!(
            icon_path("00000000-0000-0020-2000000000000000").as_deref(),
            Some("/00000000-0000-0020-2000000000000000.svg")
        );
        assert_eq!(
            icon_path("00000000-0000-0020-2000000000000000.png").as_deref(),
            Some("/00000000-0000-0020-2000000000000000.png")
        );
        for bad in [
            "jdev/sps/io/1b35ac02-03a3-4402-ffff450b2297409e/1.0",
            "dev/sps/io/x/On",
            "jdev/sps/io/x.svg",
            "dev/x.svg",
            "data/LoxAPP3.json",
            "sys/x.svg",
            "prog/sps.svg",
            "JDEV/x.svg",
            "IconsFilled/../jdev/x.svg",
            "IconsFilled//x.svg",
            "IconsFilled/x.svg?y=1",
            "IconsFilled/x%2F.svg",
            "IconsFilled/a/b.svg",
            "/IconsFilled/x.svg",
            "IconsFilled/x",
            "IconsFilled/x.json",
            "1.0",
            "",
        ] {
            assert_eq!(icon_path(bad), None, "{bad}");
        }
    }

    #[tokio::test]
    async fn a_command_in_the_structure_file_is_never_requested() {
        let (host, hits) = icon_server(HashMap::new()).await;
        let s = Structure::with_dir(
            Ok(crate::route::Handle::fixed(loxone_client::Connection::new(
                host, "u", "p",
            ))),
            None,
        );
        s.set(json!({ "controls": {
            "a": { "name": "A", "type": "Switch", "defaultIcon": "jdev/sps/io/a/1.0" },
            "b": { "name": "B", "type": "Switch", "defaultIcon": "dev/sps/io/b/On" },
            "c": { "name": "C", "type": "Switch", "defaultIcon": "IconsFilled/../jdev/sps/io/c/On" },
        }}));
        for name in [
            "jdev/sps/io/a/1.0",
            "dev/sps/io/b/On",
            "IconsFilled/../jdev/sps/io/c/On",
        ] {
            assert_eq!(s.icon(name).await, Err(None), "{name}");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // Not offered to the browser either.
        assert!(s.summary().1.is_empty());
    }

    #[test]
    fn cache_names_stay_in_their_folder() {
        assert_eq!(file_name("IconsFilled/a.svg"), "IconsFilled_a.svg");
        assert_eq!(file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(file_name("192.168.1.77"), "192.168.1.77");
    }

    fn read(s: &Structure) -> Value {
        serde_json::from_str(&s.json()).unwrap()
    }

    /// A stand-in Miniserver for icons: `path → body`, counting requests.
    async fn icon_server(icons: HashMap<&'static str, &'static [u8]>) -> (String, Arc<AtomicU64>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let hits = Arc::new(AtomicU64::new(0));
        let h = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut c, _)) = l.accept().await else {
                    return;
                };
                let icons = icons.clone();
                let h = h.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut b = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match c.read(&mut b).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&b[..n]),
                        }
                    }
                    let req = String::from_utf8_lossy(&buf);
                    let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                    h.fetch_add(1, Ordering::SeqCst);
                    // Slow enough for concurrent requests to overlap.
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    let (status, body): (&str, &[u8]) = match icons.get(path.as_str()) {
                        Some(b) => ("200 OK", b),
                        None => ("404 Not Found", b""),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = c.write_all(head.as_bytes()).await;
                    let _ = c.write_all(body).await;
                });
            }
        });
        (format!("http://{addr}"), hits)
    }

    #[test]
    fn a_new_structure_is_served_at_once_with_its_icons() {
        let s = Structure::with_dir(
            Ok(crate::route::Handle::fixed(loxone_client::Connection::new(
                "http://127.0.0.1:9",
                "u",
                "p",
            ))),
            None,
        );
        let mut rx = s.subscribe();
        s.set(sample());
        assert_eq!(rx.try_recv().unwrap(), Event::Changed);
        // The summary is built for this state…
        assert_eq!(read(&s)["controls"]["c1"]["name"], "Deckenlicht");
        assert!(s.summary().1.contains("IconsFilled/spot.svg"));
        // …and a new structure replaces it, icons and all.
        let mut next = sample();
        next["controls"]["c1"]["name"] = json!("Wandlicht");
        next["controls"]["c1"]["subControls"]["c1/s"]["defaultIcon"] = json!("IconsFilled/new.svg");
        s.set(next);
        assert_eq!(rx.try_recv().unwrap(), Event::Changed);
        let v = read(&s);
        assert_eq!(v["controls"]["c1"]["name"], "Wandlicht");
        assert_eq!(v["reads"], 2);
        let icons = s.summary().1;
        assert!(icons.contains("IconsFilled/new.svg") && !icons.contains("IconsFilled/spot.svg"));
    }

    #[test]
    fn reading_the_same_structure_again_only_counts_the_read() {
        let s = Structure::with_dir(
            Ok(crate::route::Handle::fixed(loxone_client::Connection::new(
                "http://127.0.0.1:9",
                "u",
                "p",
            ))),
            None,
        );
        s.set(sample());
        let mut rx = s.subscribe();
        s.set(sample());
        assert_eq!(rx.try_recv().unwrap(), Event::Read);
        assert!(rx.try_recv().is_err());
        assert_eq!(read(&s)["reads"], 2);
    }

    #[tokio::test]
    async fn writes_never_mix_and_a_failed_load_stays_reported() {
        let tmp = tempfile::tempdir().unwrap();
        // Nothing listens on port 9: loading fails.
        let s = Structure::with_dir(
            Ok(crate::route::Handle::fixed(loxone_client::Connection::new(
                "http://127.0.0.1:9",
                "u",
                "p",
            ))),
            Some(tmp.path().to_path_buf()),
        );
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let s = s.clone();
                std::thread::spawn(move || {
                    for k in 0..5 {
                        let mut v = sample();
                        v["lastModified"] = json!(format!("{n}.{k}"));
                        s.set(v);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        // The file on disk is whole, and is the state served.
        let disk: Value =
            serde_json::from_slice(&std::fs::read(tmp.path().join("LoxAPP3.json")).unwrap())
                .unwrap();
        assert_eq!(disk["lastModified"], read(&s)["lastModified"]);
        assert_eq!(read(&s)["reads"], 40);
        let left: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["LoxAPP3.json"], "temporary files left behind");

        assert!(s.load().await.is_err());
        assert!(read(&s)["error"].is_string());
        // The live session reading it again does not hide that the load failed.
        let same = s.inner.lock().unwrap().raw.as_deref().cloned().unwrap();
        tokio::task::spawn_blocking({
            let s = s.clone();
            move || s.set(same)
        })
        .await
        .unwrap();
        assert!(read(&s)["error"].is_string());

        // A later run starts with the cached one.
        let again = Structure::with_dir(
            Ok(crate::route::Handle::fixed(loxone_client::Connection::new(
                "http://127.0.0.1:9",
                "u",
                "p",
            ))),
            Some(tmp.path().to_path_buf()),
        );
        let v = read(&again);
        assert_eq!(
            (v["loaded"].clone(), v["from"].clone()),
            (json!(true), json!("cache"))
        );
    }

    #[tokio::test]
    async fn an_icon_is_fetched_once_and_cached_only_when_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let svg: &'static [u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>";
        let (host, hits) = icon_server(HashMap::from([
            ("/IconsFilled/kitchen.svg", svg),
            (
                "/IconsFilled/spot.svg",
                &b"<svg xmlns=\"http://www.w3.org/2000/svg\"><re"[..],
            ),
        ]))
        .await;
        let s = Structure::with_dir(
            Ok(crate::route::Handle::fixed(loxone_client::Connection::new(
                host, "u", "p",
            ))),
            Some(tmp.path().to_path_buf()),
        );
        s.set(sample());

        // Five at once: one download, the others find it cached.
        let all =
            futures_util::future::join_all((0..5).map(|_| s.icon("IconsFilled/kitchen.svg"))).await;
        assert!(all.iter().all(|r| r.as_deref() == Ok(svg)));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read(
                tmp.path()
                    .join("icons")
                    .join(icon_file("IconsFilled/kitchen.svg"))
            )
            .unwrap(),
            svg
        );

        // A cut-off icon is refused and not cached, so the next request tries again.
        assert!(matches!(s.icon("IconsFilled/spot.svg").await, Err(Some(_))));
        assert!(
            !tmp.path()
                .join("icons")
                .join(icon_file("IconsFilled/spot.svg"))
                .exists()
        );
        assert!(s.icon("IconsFilled/spot.svg").await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 3);

        // Names the structure file does not list are never fetched.
        assert_eq!(s.icon("jdev/sps/io/x/on").await, Err(None));
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn only_whole_icons_count() {
        use std::io::Write;
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>\n";
        assert!(icon_whole(svg));
        assert!(!icon_whole(&svg[..30]));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(svg).unwrap();
        let gz = gz.finish().unwrap();
        assert!(icon_whole(&gz));
        // Cut inside the compressed SVG (a missing trailer alone is fine: see below).
        assert!(!icon_whole(&gz[..gz.len() - 20]));
        assert!(icon_whole(b"\x89PNG\r\n\x1a\n....IEND\xaeB`\x82"));
        assert!(!icon_whole(b"\x89PNG\r\n\x1a\n...."));
    }

    #[test]
    fn names_that_differ_get_files_of_their_own() {
        assert_eq!(icon_file("IconsFilled_a.svg"), "IconsFilled_a.svg");
        assert_ne!(
            icon_file("IconsFilled/a.svg"),
            icon_file("IconsFilled_a.svg")
        );
        assert!(icon_file("IconsFilled/a.svg").starts_with("IconsFilled_a.svg."));
    }

    #[test]
    fn without_a_miniserver_it_says_why() {
        let s = Structure::with_dir(Err("turned off (--no-live)".into()), None);
        let v: Value = serde_json::from_str(&s.json()).unwrap();
        assert_eq!(v["available"], false);
        assert_eq!(v["reason"], "turned off (--no-live)");
    }
}
