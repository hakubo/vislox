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

use loxone_client::Connection;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Semaphore, broadcast};

/// The structure file as the browser receives it, and the icon names in it.
struct Summary {
    json: Arc<String>,
    icons: HashSet<String>,
}

#[derive(Default)]
struct Inner {
    raw: Option<Value>,
    /// When it was read (Unix ms), and from where.
    fetched: Option<i64>,
    from: Option<&'static str>,
    /// The last load's error.
    error: Option<String>,
}

pub struct Structure {
    conn: Result<Connection, String>,
    /// This Miniserver's cache directory.
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    summary: Mutex<Option<Summary>>,
    tx: broadcast::Sender<()>,
    /// One load at a time.
    loading: tokio::sync::Mutex<()>,
    /// The Miniserver serves few requests at once.
    icon_gate: Semaphore,
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
fn file_name(key: &str) -> String {
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Write `bytes` to `path` whole: a reader never sees half a file.
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// What the browser needs from the structure file: the controls (with
/// their sub-controls), the rooms and the categories, each with its icon.
fn summarize(raw: &Value) -> (Map<String, Value>, HashSet<String>) {
    let mut icons = HashSet::new();
    let mut icon = |v: &Value| -> Value {
        match v.as_str().filter(|s| !s.is_empty()) {
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

impl Structure {
    /// `conn`: the Miniserver, or why there is none. `key`: what names it in
    /// the cache (its serial, else its address).
    pub fn new(conn: Result<Connection, String>, key: Option<&str>) -> Arc<Structure> {
        let dir = key.and_then(|k| Some(cache_root()?.join(file_name(k))));
        Self::with_dir(conn, dir)
    }

    fn with_dir(conn: Result<Connection, String>, dir: Option<PathBuf>) -> Arc<Structure> {
        let (tx, _) = broadcast::channel(8);
        let s = Arc::new(Structure {
            conn,
            dir,
            inner: Mutex::new(Inner::default()),
            summary: Mutex::new(None),
            tx,
            loading: tokio::sync::Mutex::new(()),
            icon_gate: Semaphore::new(4),
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
            s.put(raw, when.unwrap_or_else(now_ms), "cache", false);
        }
        s
    }

    fn file(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("LoxAPP3.json"))
    }

    /// Told when the structure changes.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    fn put(&self, raw: Value, when: i64, from: &'static str, save: bool) {
        if save
            && let Some(path) = self.file()
            && let Ok(bytes) = serde_json::to_vec(&raw)
            && let Err(e) = write_file(&path, &bytes)
        {
            eprintln!(
                "vislox: cannot cache the structure file in {}: {e}",
                path.display()
            );
        }
        {
            let mut i = self.inner.lock().unwrap();
            i.raw = Some(raw);
            i.fetched = Some(when);
            i.from = Some(from);
            i.error = None;
        }
        *self.summary.lock().unwrap() = None;
        let _ = self.tx.send(());
    }

    /// A structure file the live session read.
    pub fn set(&self, raw: &Value) {
        let same = self.inner.lock().unwrap().raw.as_ref() == Some(raw);
        if same {
            // Unchanged: only the time it was read.
            let mut i = self.inner.lock().unwrap();
            i.fetched = Some(now_ms());
            i.from = Some("live values");
            drop(i);
            *self.summary.lock().unwrap() = None;
            let _ = self.tx.send(());
            return;
        }
        self.put(raw.clone(), now_ms(), "live values", true);
    }

    /// Read the structure file from the Miniserver now.
    pub async fn load(&self) -> Result<(), String> {
        let conn = self.conn.clone()?;
        let _one = self.loading.lock().await;
        let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
            let mut client = loxone_client::LoxClient::new(conn).map_err(|e| e.to_string())?;
            client.get_structure().cloned().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match res {
            Ok(raw) => {
                self.put(raw, now_ms(), "Miniserver", true);
                Ok(())
            }
            Err(e) => {
                let e = format!("cannot read the structure file: {e}");
                self.inner.lock().unwrap().error = Some(e.clone());
                *self.summary.lock().unwrap() = None;
                let _ = self.tx.send(());
                Err(e)
            }
        }
    }

    /// `GET /api/app`: `{available, reason?, loaded, fetched, from, error,
    /// lastModified, serial, rooms, cats, controls}`.
    pub fn json(&self) -> Arc<String> {
        if let Some(s) = &*self.summary.lock().unwrap() {
            return s.json.clone();
        }
        let i = self.inner.lock().unwrap();
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
        out.insert("error".into(), json!(i.error));
        drop(i);
        let json = Arc::new(Value::Object(out).to_string());
        *self.summary.lock().unwrap() = Some(Summary {
            json: json.clone(),
            icons,
        });
        json
    }

    fn knows_icon(&self, name: &str) -> bool {
        self.json();
        self.summary
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.icons.contains(name))
    }

    /// An icon the structure file names: from the cache, else from the
    /// Miniserver (then cached). `Err(None)`: not a name it lists.
    pub async fn icon(&self, name: &str) -> Result<Vec<u8>, Option<String>> {
        if !self.knows_icon(name) {
            return Err(None);
        }
        let path = self
            .dir
            .as_ref()
            .map(|d| d.join("icons").join(file_name(name)));
        if let Some(p) = &path
            && let Ok(b) = std::fs::read(p)
        {
            return Ok(b);
        }
        let conn = self.conn.clone().map_err(Some)?;
        let _gate = self
            .icon_gate
            .acquire()
            .await
            .map_err(|e| Some(e.to_string()))?;
        // Another request may have fetched it meanwhile.
        if let Some(p) = &path
            && let Ok(b) = std::fs::read(p)
        {
            return Ok(b);
        }
        let file = if name.rsplit('/').next().is_some_and(|f| f.contains('.')) {
            name.to_string()
        } else {
            format!("{name}.svg")
        };
        let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
            let client = loxone_client::LoxClient::new(conn).map_err(|e| e.to_string())?;
            client
                .get_bytes(&format!("/{}", file.trim_start_matches('/')))
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| Some(e.to_string()))?
        .map_err(Some)?;
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
    fn cache_names_stay_in_their_folder() {
        assert_eq!(file_name("IconsFilled/a.svg"), "IconsFilled_a.svg");
        assert_eq!(file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(file_name("192.168.1.77"), "192.168.1.77");
    }

    #[tokio::test]
    async fn the_last_structure_is_cached_and_unknown_icons_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = Ok(Connection::new("http://127.0.0.1:9", "u", "p"));
        let s = Structure::with_dir(conn.clone(), Some(tmp.path().to_path_buf()));
        let v: Value = serde_json::from_str(&s.json()).unwrap();
        assert_eq!(
            (v["available"].clone(), v["loaded"].clone()),
            (json!(true), json!(false))
        );

        let mut rx = s.subscribe();
        s.set(&sample());
        rx.try_recv().unwrap();
        let v: Value = serde_json::from_str(&s.json()).unwrap();
        assert_eq!(v["from"], "live values");
        assert_eq!(v["controls"]["c1"]["name"], "Deckenlicht");

        // A later run starts with it.
        let again = Structure::with_dir(conn, Some(tmp.path().to_path_buf()));
        let v: Value = serde_json::from_str(&again.json()).unwrap();
        assert_eq!(
            (v["loaded"].clone(), v["from"].clone()),
            (json!(true), json!("cache"))
        );

        // A cached icon is served without the Miniserver; others are not fetched.
        std::fs::create_dir_all(tmp.path().join("icons")).unwrap();
        std::fs::write(tmp.path().join("icons/IconsFilled_kitchen.svg"), "<svg/>").unwrap();
        assert_eq!(
            again.icon("IconsFilled/kitchen.svg").await.unwrap(),
            b"<svg/>"
        );
        assert_eq!(again.icon("jdev/sps/io/x/on").await, Err(None));
    }

    #[test]
    fn without_a_miniserver_it_says_why() {
        let s = Structure::with_dir(Err("turned off (--no-live)".into()), None);
        let v: Value = serde_json::from_str(&s.json()).unwrap();
        assert_eq!(v["available"], false);
        assert_eq!(v["reason"], "turned off (--no-live)");
    }
}
