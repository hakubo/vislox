//! Calendar entries changed in the Loxone app.
//!
//! Admins can add, change and delete operating-mode calendar entries in the
//! app. Those edits live on the Miniserver only, until a Pull or Sync takes
//! them over, and an Apply built on the old base overwrites them. vislox
//! asks the Miniserver for the entries it uses
//! (`jdev/sps/calendargetentries`, read-only, as lox's user over HTTP basic
//! auth) and compares them with the deployed revision (`calendar::drift`).
//!
//! When it asks: when a structure file is read (each live session reads
//! one, and *Read from the Miniserver* does), at most once a minute; after
//! Pull, Apply and Sync; and before Apply. Never on a timer. The command
//! needs admin rights: without them the Miniserver refuses, and vislox only
//! keeps the reason (the banner never shows). It never calls
//! `calendarcreateentry`, `calendarupdateentry` or `calendardeleteentry`.

use crate::calendar::{self, Entry};
use crate::restart::Http;
use loxone_client::Connection;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// A read this recent is reused when a live session starts.
pub const ON_LIVE: Duration = Duration::from_secs(60);
/// A read this recent is reused before Apply.
pub const BEFORE_APPLY: Duration = Duration::from_secs(10);
/// How long the Miniserver may take to answer.
const TIMEOUT: Duration = Duration::from_secs(8);

/// The last read: when (Unix ms), and the entries, or why there are none.
#[derive(Clone, Debug)]
pub struct Read {
    pub at: i64,
    pub entries: Result<Arc<Vec<Entry>>, String>,
    /// Counts reads whose result changed, so a browser and the drift
    /// cache know when to look again.
    pub seq: u64,
}

pub struct AppCalendar {
    http: Result<Arc<Http>, String>,
    last: Mutex<Option<(Instant, Read)>>,
    /// One read at a time.
    reading: tokio::sync::Mutex<()>,
    tx: broadcast::Sender<()>,
}

impl AppCalendar {
    /// `conn`: lox's Miniserver, or why vislox does not talk to it.
    pub fn new(conn: Result<Connection, String>) -> AppCalendar {
        let (tx, _) = broadcast::channel(8);
        AppCalendar {
            http: conn.map(|c| Arc::new(Http::new(c))),
            last: Mutex::new(None),
            reading: tokio::sync::Mutex::new(()),
            tx,
        }
    }

    /// Told when a read's result changed.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    /// The last read, if any.
    pub fn last(&self) -> Option<Read> {
        self.last.lock().unwrap().as_ref().map(|(_, r)| r.clone())
    }

    /// Whether there is a Miniserver to ask; `Err` says why not.
    pub fn available(&self) -> Result<(), String> {
        self.http.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    /// Read the entries, unless a read started less than `max_age` ago.
    pub async fn read(&self, max_age: Duration) -> Option<Read> {
        let http = self.http.as_ref().ok()?.clone();
        let asked = Instant::now();
        let reuse = |ok: &dyn Fn(Instant) -> bool| {
            self.last
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(started, _)| ok(*started))
                .map(|(_, r)| r.clone())
        };
        if let Some(r) = reuse(&|started| started.elapsed() < max_age) {
            return Some(r);
        }
        let _one = self.reading.lock().await;
        if let Some(r) = reuse(&|started| started >= asked) {
            return Some(r);
        }
        let started = Instant::now();
        let ask = tokio::task::spawn_blocking(move || http.value("jdev/sps/calendargetentries"));
        let entries = match tokio::time::timeout(TIMEOUT, ask).await {
            Ok(Ok(Ok(v))) => calendar::parse_api(&v).map(Arc::new),
            Ok(Ok(Err(e))) => Err(refusal(&e)),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("no answer".to_string()),
        };
        Some(self.store(started, entries))
    }

    fn store(&self, started: Instant, entries: Result<Arc<Vec<Entry>>, String>) -> Read {
        let mut last = self.last.lock().unwrap();
        let prev = last.as_ref().map(|(_, r)| r);
        let same = prev.is_some_and(|p| p.entries == entries);
        let seq = prev.map_or(0, |p| p.seq) + u64::from(!same);
        let r = Read {
            at: now_ms(),
            entries,
            seq,
        };
        *last = Some((started, r.clone()));
        drop(last);
        if !same {
            let _ = self.tx.send(());
        }
        r
    }
}

/// Why the Miniserver gave no entries, in words. A refusal (a code other
/// than 200 in the answer, an HTTP 401 or 403) usually means the user
/// lacks admin rights.
fn refusal(e: &str) -> String {
    match e {
        "an answer without a value" | "HTTP 401" | "HTTP 403" => {
            format!("refused ({e}): reading calendar entries needs admin rights")
        }
        _ => e.to_string(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::EntryDate;

    fn cal() -> AppCalendar {
        AppCalendar::new(Err("off".into()))
    }

    fn entries(day: u32) -> Result<Arc<Vec<Entry>>, String> {
        Ok(Arc::new(vec![Entry {
            uuid: "a".into(),
            name: "X".into(),
            mode: Some(0),
            date: Ok(EntryDate::Fixed { month: 1, day }),
        }]))
    }

    #[tokio::test]
    async fn without_a_miniserver_nothing_is_read() {
        let c = cal();
        assert_eq!(c.available(), Err("off".to_string()));
        assert!(c.read(Duration::ZERO).await.is_none());
        assert!(c.last().is_none());
    }

    #[test]
    fn only_a_changed_result_is_announced() {
        let c = cal();
        let mut rx = c.subscribe();
        assert_eq!(c.store(Instant::now(), entries(1)).seq, 1);
        assert!(rx.try_recv().is_ok());
        assert_eq!(c.store(Instant::now(), entries(1)).seq, 1);
        assert!(rx.try_recv().is_err());
        assert_eq!(c.store(Instant::now(), entries(2)).seq, 2);
        assert_eq!(c.store(Instant::now(), Err("no answer".into())).seq, 3);
        assert_eq!(c.last().unwrap().entries, Err("no answer".to_string()));
    }

    #[test]
    fn a_refusal_names_the_admin_rights() {
        assert_eq!(
            refusal("HTTP 403"),
            "refused (HTTP 403): reading calendar entries needs admin rights"
        );
        assert_eq!(refusal("no answer"), "no answer");
    }
}
