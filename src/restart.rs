//! Following the Miniserver through the program restart of an Apply.
//!
//! `lox config push` uploads the config and restarts the program, then
//! waits until it runs. While it waits, vislox watches the Miniserver on its
//! own, read-only, so the UI can say what happens:
//!
//! - **out of service**: `jdev/cfg/apiKey` (which needs no login) fails or
//!   answers 503 while the Miniserver restarts (*Communicating with the
//!   Miniserver*, p. 7 and p. 30);
//! - **back online** after N s: it answers again;
//! - **new config loaded**: `jdev/sps/LoxAPPversion3`, the structure file's
//!   version, differs from what it was before the upload (p. 23).
//!
//! A program restart can be quick enough that no request ever fails; then
//! the version change alone says the new program runs.

use loxone_client::{Connection, LoxClient};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// How often the Miniserver is asked during a restart.
const EVERY: Duration = Duration::from_millis(500);
/// Give up following after this long; lox has its own deadlines.
const GIVE_UP: Duration = Duration::from_secs(300);

/// Read-only questions to the Miniserver.
pub trait Probe: Send + Sync {
    /// Whether it answers `jdev/cfg/apiKey`; else why not.
    fn answers(&self) -> Result<(), String>;
    /// `jdev/sps/LoxAPPversion3`: the loaded structure's version.
    fn app_version(&self) -> Option<String>;
}

/// The Miniserver over HTTP, with the connection lox uses.
///
/// `LoxClient` is a blocking reqwest client, which must not be built or
/// dropped on the async runtime. It is built on first use (the probe is only
/// asked from `spawn_blocking`), and dropped on a thread of its own.
pub struct Http {
    conn: Connection,
    client: OnceLock<Option<LoxClient>>,
}

impl Http {
    pub fn new(conn: Connection) -> Http {
        Http {
            conn,
            client: OnceLock::new(),
        }
    }

    fn client(&self) -> Result<&LoxClient, String> {
        self.client
            .get_or_init(|| LoxClient::new(self.conn.clone()).ok())
            .as_ref()
            .ok_or_else(|| "no HTTP client".to_string())
    }

    /// `LL.value` of a `jdev/…` answer that has HTTP status 200.
    pub fn value(&self, path: &str) -> Result<String, String> {
        // No retry: a failed request is what is being looked for.
        let (status, body) = self.client()?.get_once(path, None).map_err(|e| {
            let e = e.to_string();
            if e.contains("timed out") || e.contains("timeout") {
                "no answer".to_string()
            } else {
                "unreachable".to_string()
            }
        })?;
        if status != 200 {
            return Err(match status {
                503 => "503, restarting".to_string(),
                s => format!("HTTP {s}"),
            });
        }
        ll_value(&body).ok_or_else(|| "an answer without a value".to_string())
    }
}

impl Http {
    /// A request that only needs HTTP 200 (a `dev/…` command, not `jdev`).
    pub fn call(&self, path: &str) -> Result<(), String> {
        let (status, _) = self
            .client()?
            .get_once(path, None)
            .map_err(|e| format!("no answer: {e}"))?;
        match status {
            200 => Ok(()),
            401 | 403 => Err(format!("HTTP {status}: lox's user may not do this")),
            s => Err(format!("HTTP {s}")),
        }
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        if let Some(Some(c)) = self.client.take() {
            std::thread::spawn(move || drop(c));
        }
    }
}

impl Probe for Http {
    fn answers(&self) -> Result<(), String> {
        self.value("jdev/cfg/apiKey").map(|_| ())
    }

    fn app_version(&self) -> Option<String> {
        self.value("jdev/sps/LoxAPPversion3").ok()
    }
}

/// `LL.value` of a Miniserver answer, when its code is 200.
fn ll_value(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let ll = &v["LL"];
    let code = ll["Code"]
        .as_str()
        .map(String::from)
        .or_else(|| ll["code"].as_str().map(String::from))
        .or_else(|| ll["Code"].as_i64().map(|c| c.to_string()));
    if code.as_deref().is_some_and(|c| c != "200") {
        return None;
    }
    match &ll["value"] {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// What was seen, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// It stopped answering, and why (`503, restarting`, `unreachable`).
    OutOfService(String),
    /// It answers again, this many seconds after following began.
    BackOnline(u64),
    /// The structure version changed: the new config runs. Seconds after
    /// following began, and the new version.
    Loaded(u64, String),
}

/// Turns what the Miniserver answers, poll by poll, into [`Event`]s.
pub struct Watch {
    before: Option<String>,
    down: bool,
    up: bool,
    loaded: bool,
}

impl Watch {
    /// `before`: the structure version before the upload, if known.
    pub fn new(before: Option<String>) -> Watch {
        Watch {
            before,
            down: false,
            up: false,
            loaded: false,
        }
    }

    /// One poll, `secs` after following began: whether it answered, and
    /// the structure version it reports (asked only when it answers).
    pub fn observe(
        &mut self,
        secs: u64,
        answers: &Result<(), String>,
        version: Option<&str>,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        match answers {
            Err(why) => {
                if !self.down {
                    self.down = true;
                    out.push(Event::OutOfService(why.clone()));
                }
                return out;
            }
            Ok(()) => {
                if self.down && !self.up {
                    self.up = true;
                    out.push(Event::BackOnline(secs));
                }
            }
        }
        if let (false, Some(v)) = (self.loaded, version) {
            match &self.before {
                // Not known before: the first version read after an outage
                // is the new program's.
                None if self.up => {
                    self.loaded = true;
                    out.push(Event::Loaded(secs, v.to_string()));
                }
                Some(b) if b != v => {
                    self.loaded = true;
                    out.push(Event::Loaded(secs, v.to_string()));
                }
                _ => {}
            }
        }
        out
    }

    /// Nothing more to learn: the new config runs and the Miniserver
    /// answers.
    pub fn done(&self) -> bool {
        self.loaded && (!self.down || self.up)
    }
}

/// Follow a restart until it is done, `stop` turns true, or [`GIVE_UP`].
pub async fn follow(
    probe: Arc<dyn Probe>,
    before: Option<String>,
    mut stop: watch::Receiver<bool>,
    mut on: impl FnMut(Event),
) {
    let mut w = Watch::new(before);
    let start = Instant::now();
    while !*stop.borrow() && start.elapsed() < GIVE_UP {
        let p = probe.clone();
        let Ok((answers, version)) = tokio::task::spawn_blocking(move || {
            let a = p.answers();
            let v = if a.is_ok() { p.app_version() } else { None };
            (a, v)
        })
        .await
        else {
            return;
        };
        if *stop.borrow() {
            return;
        }
        for e in w.observe(start.elapsed().as_secs(), &answers, version.as_deref()) {
            on(e);
        }
        if w.done() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(EVERY) => {}
            _ = stop.changed() => {}
        }
    }
}

/// Before the upload: whether the Miniserver answers vislox at all, and its
/// structure version. None when it does not answer: an outage seen later
/// would say nothing about the restart.
pub async fn baseline(probe: &Arc<dyn Probe>) -> Option<Option<String>> {
    let p = probe.clone();
    tokio::task::spawn_blocking(move || p.answers().ok().map(|()| p.app_version()))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The blocking client is built in a blocking task and may be dropped
    /// on the runtime without "Cannot drop a runtime in a context where
    /// blocking is not allowed".
    #[tokio::test]
    async fn the_http_probe_lives_on_the_runtime() {
        let probe: Arc<dyn Probe> =
            Arc::new(Http::new(Connection::new("http://127.0.0.1:9", "u", "p")));
        assert_eq!(
            baseline(&probe).await,
            None,
            "a closed port does not answer"
        );
        drop(probe);
        // Built, never used: nothing to drop.
        drop(Http::new(Connection::new("http://127.0.0.1:9", "u", "p")));
    }

    #[test]
    fn values_come_from_ll() {
        let ok = r#"{"LL": { "control": "dev/sps/LoxAPPversion3", "value": "2026-11-06 10:00:00", "Code": "200"}}"#;
        assert_eq!(ll_value(ok).as_deref(), Some("2026-11-06 10:00:00"));
        let lower = r#"{"LL": { "value": "x", "code": "200"}}"#;
        assert_eq!(ll_value(lower).as_deref(), Some("x"));
        let refused = r#"{"LL": { "value": "", "Code": "503"}}"#;
        assert_eq!(ll_value(refused), None);
        assert_eq!(ll_value("<html>503</html>"), None);
    }

    #[test]
    fn a_restart_with_an_outage() {
        let mut w = Watch::new(Some("v1".into()));
        assert_eq!(w.observe(0, &Ok(()), Some("v1")), vec![]);
        let down = Err("503, restarting".to_string());
        assert_eq!(
            w.observe(1, &down, None),
            vec![Event::OutOfService("503, restarting".into())]
        );
        assert_eq!(w.observe(2, &down, None), vec![]);
        // Back, still the old structure for a moment.
        assert_eq!(
            w.observe(6, &Ok(()), Some("v1")),
            vec![Event::BackOnline(6)]
        );
        assert!(!w.done());
        assert_eq!(
            w.observe(7, &Ok(()), Some("v2")),
            vec![Event::Loaded(7, "v2".into())]
        );
        assert!(w.done());
    }

    #[test]
    fn a_restart_too_quick_to_see() {
        let mut w = Watch::new(Some("v1".into()));
        assert_eq!(
            w.observe(2, &Ok(()), Some("v2")),
            vec![Event::Loaded(2, "v2".into())]
        );
        assert!(w.done());
    }

    #[test]
    fn back_and_loaded_in_one_poll() {
        let mut w = Watch::new(Some("v1".into()));
        w.observe(0, &Err("unreachable".into()), None);
        assert_eq!(
            w.observe(9, &Ok(()), Some("v2")),
            vec![Event::BackOnline(9), Event::Loaded(9, "v2".into())]
        );
    }

    #[test]
    fn without_the_old_version_the_first_after_the_outage_counts() {
        let mut w = Watch::new(None);
        // Before the outage the version says nothing.
        assert_eq!(w.observe(0, &Ok(()), Some("v1")), vec![]);
        w.observe(1, &Err("unreachable".into()), None);
        assert_eq!(
            w.observe(5, &Ok(()), Some("v2")),
            vec![Event::BackOnline(5), Event::Loaded(5, "v2".into())]
        );
    }

    /// A Miniserver that plays back one answer per poll; the last repeats.
    pub struct Script(Mutex<Vec<Poll>>);
    /// Whether it answers, and the structure version it reports.
    pub type Poll = (Result<(), String>, Option<String>);

    impl Script {
        pub fn probe(polls: Vec<Poll>) -> Arc<dyn Probe> {
            Arc::new(Script(Mutex::new(polls)))
        }
        fn next(&self) -> Poll {
            let mut s = self.0.lock().unwrap();
            let a = s.first().cloned().unwrap_or((Ok(()), None));
            if s.len() > 1 {
                let _ = s.remove(0);
            }
            a
        }
    }

    impl Probe for Script {
        /// A failed poll asks nothing more: it ends here.
        fn answers(&self) -> Result<(), String> {
            let s = self.0.lock().unwrap();
            let a = s.first().map(|a| a.0.clone()).unwrap_or(Ok(()));
            drop(s);
            if a.is_err() {
                let _ = self.next();
            }
            a
        }
        fn app_version(&self) -> Option<String> {
            self.next().1
        }
    }

    #[tokio::test]
    async fn follow_reports_and_stops_when_done() {
        let probe = Script::probe(vec![
            (Ok(()), Some("v1".into())),
            (Err("503, restarting".into()), None),
            (Err("503, restarting".into()), None),
            (Ok(()), Some("v2".into())),
        ]);
        let (_tx, rx) = watch::channel(false);
        let mut seen = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            follow(probe, Some("v1".into()), rx, |e| seen.push(e)),
        )
        .await
        .expect("follow ends once the new config runs");
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(seen[0], Event::OutOfService("503, restarting".into()));
        assert!(matches!(seen[1], Event::BackOnline(_)));
        assert!(matches!(&seen[2], Event::Loaded(_, v) if v == "v2"));
    }

    #[tokio::test]
    async fn follow_stops_when_told() {
        let probe = Script::probe(vec![(Ok(()), Some("v1".into()))]);
        let (tx, rx) = watch::channel(false);
        let run = tokio::spawn(follow(probe, Some("v1".into()), rx, |_| {}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("follow ends when stopped")
            .unwrap();
    }
}
