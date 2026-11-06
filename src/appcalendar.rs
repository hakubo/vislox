//! Calendar entries changed in the Loxone app.
//!
//! Admins can add, change and delete operating-mode calendar entries in the
//! app. Those edits live on the Miniserver only, until a Pull or Sync takes
//! them over, and an Apply built on the old base overwrites them. vislox
//! asks the Miniserver for the entries it uses
//! (`jdev/sps/calendargetentries`, read-only, as lox's user over HTTP basic
//! auth) and compares them with the deployed revision (`calendar::drift`).
//!
//! When it asks: after each successful structure file read (each live
//! session reads one, and *Read from the Miniserver* does), at most once a
//! minute; after Pull, Apply and Sync; and before Apply. Never on a timer.
//! The command needs admin rights: without them the Miniserver refuses, and
//! vislox only keeps the reason (the banner never shows). It never calls
//! `calendarcreateentry`, `calendarupdateentry` or `calendardeleteentry`.

use crate::calendar::{self, Entry};
use crate::restart::Http;
use loxone_client::Connection;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// A read this recent is reused when a live session starts.
pub const ON_LIVE: Duration = Duration::from_secs(60);
/// A read this recent is reused before Apply.
pub const BEFORE_APPLY: Duration = Duration::from_secs(2);
/// How long the Miniserver may take to answer.
const TIMEOUT: Duration = Duration::from_secs(8);
/// How a refusal starts: the Miniserver answered, but not with entries,
/// most likely since lox's user has no admin rights.
const REFUSED: &str = "refused";

/// The last read: when (Unix ms), and the entries, or why there are none.
#[derive(Clone, Debug)]
pub struct Read {
    pub at: i64,
    pub entries: Result<Arc<Vec<Entry>>, String>,
    /// Counts reads whose result changed, so a browser and the drift
    /// cache know when to look again.
    pub seq: u64,
}

/// Asks the Miniserver for `calendargetentries` (blocking): its `LL.value`.
type Ask = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

pub struct AppCalendar {
    ask: Result<Ask, String>,
    timeout: Duration,
    last: Mutex<Option<(Instant, Read)>>,
    /// One request in flight, also after the caller gave up waiting for it:
    /// the Miniserver serves few requests at once.
    reading: Arc<tokio::sync::Mutex<()>>,
    /// The structure file reads seen ([`AppCalendar::after_structure`]).
    structure_reads: AtomicU64,
    tx: broadcast::Sender<()>,
}

impl AppCalendar {
    /// `conn`: lox's Miniserver, or why vislox does not talk to it.
    pub fn new(conn: Result<Connection, String>) -> AppCalendar {
        AppCalendar::with_ask(
            conn.map(|c| {
                let http = Arc::new(Http::new(c));
                Arc::new(move || http.value("jdev/sps/calendargetentries")) as Ask
            }),
            TIMEOUT,
        )
    }

    fn with_ask(ask: Result<Ask, String>, timeout: Duration) -> AppCalendar {
        let (tx, _) = broadcast::channel(8);
        AppCalendar {
            ask,
            timeout,
            last: Mutex::new(None),
            reading: Arc::new(tokio::sync::Mutex::new(())),
            structure_reads: AtomicU64::new(0),
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
        self.ask.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    /// After a structure file event: read the entries (at most once a
    /// minute), but only when a structure file was read successfully since
    /// the last time (`reads`: the structure's successful reads). A failed
    /// load means the Miniserver did not answer: asking again only waits.
    pub async fn after_structure(&self, reads: u64) -> Option<Read> {
        if self.structure_reads.fetch_max(reads, Ordering::SeqCst) >= reads {
            return None;
        }
        self.read(ON_LIVE).await
    }

    /// Read the entries, unless a read that got them started less than
    /// `max_age` ago. A failed read is never reused that way: before Apply
    /// in particular, it must be tried again.
    pub async fn read(&self, max_age: Duration) -> Option<Read> {
        let ask = self.ask.as_ref().ok()?.clone();
        let asked = Instant::now();
        let reuse = |ok: &dyn Fn(Instant, &Read) -> bool| {
            self.last
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(started, r)| ok(*started, r))
                .map(|(_, r)| r.clone())
        };
        if let Some(r) = reuse(&|started, r| r.entries.is_ok() && started.elapsed() < max_age) {
            return Some(r);
        }
        let one = self.reading.clone().lock_owned().await;
        // A read that started after this one asked is as good as a new one,
        // failed or not.
        if let Some(r) = reuse(&|started, _| started >= asked) {
            return Some(r);
        }
        let started = Instant::now();
        let mut task = tokio::task::spawn_blocking(move || ask());
        let entries = match tokio::time::timeout(self.timeout, &mut task).await {
            Ok(Ok(Ok(v))) => calendar::parse_api(&v).map(Arc::new),
            Ok(Ok(Err(e))) => Err(refusal(&e)),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => {
                // The request still runs: keep the next one waiting for it.
                tokio::spawn(async move {
                    let _ = task.await;
                    drop(one);
                });
                Err("no answer".to_string())
            }
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
            format!("{REFUSED} ({e}): reading calendar entries needs admin rights")
        }
        _ => e.to_string(),
    }
}

/// Whether a read failed because the Miniserver refused it (no admin
/// rights): that is skipped quietly, unlike a Miniserver that did not answer.
pub fn is_refusal(why: &str) -> bool {
    why.starts_with(REFUSED)
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
    use std::sync::atomic::AtomicUsize;

    fn cal() -> AppCalendar {
        AppCalendar::with_ask(Err("off".into()), TIMEOUT)
    }

    /// A stand-in Miniserver: answers with `answer`, after `delay`, and
    /// counts the requests and the most in flight at once.
    fn stand_in(
        answer: Arc<Mutex<Result<String, String>>>,
        delay: Duration,
    ) -> (AppCalendar, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let (asked, busy, most) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );
        let (a, b, m) = (asked.clone(), busy.clone(), most.clone());
        let ask: Ask = Arc::new(move || {
            a.fetch_add(1, Ordering::SeqCst);
            let now = b.fetch_add(1, Ordering::SeqCst) + 1;
            m.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(delay);
            b.fetch_sub(1, Ordering::SeqCst);
            answer.lock().unwrap().clone()
        });
        (
            AppCalendar::with_ask(Ok(ask), Duration::from_millis(100)),
            asked,
            most,
        )
    }

    const ONE: &str =
        r#"[{"uuid":"a","name":"X","operatingMode":0,"calMode":0,"startMonth":1,"startDay":1}]"#;

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

    #[tokio::test]
    async fn a_good_read_is_reused_and_a_failed_one_is_tried_again() {
        let answer = Arc::new(Mutex::new(Err("unreachable".to_string())));
        let (c, asked, _) = stand_in(answer.clone(), Duration::ZERO);
        let r = c.read(BEFORE_APPLY).await.unwrap();
        assert_eq!(r.entries, Err("unreachable".to_string()));
        // Before Apply, a failure from a moment ago is not taken as the answer.
        *answer.lock().unwrap() = Ok(ONE.to_string());
        let r = c.read(BEFORE_APPLY).await.unwrap();
        assert_eq!(r.entries, entries(1));
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        // A good one is.
        c.read(BEFORE_APPLY).await.unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        c.read(Duration::ZERO).await.unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_slow_miniserver_gets_one_request_at_a_time() {
        let answer = Arc::new(Mutex::new(Ok(ONE.to_string())));
        let (c, asked, most) = stand_in(answer, Duration::from_millis(400));
        // The first gives up after 100 ms; the request it sent still runs.
        assert_eq!(
            c.read(Duration::ZERO).await.unwrap().entries,
            Err("no answer".to_string())
        );
        // The next waits for that request to end before it sends its own.
        let r = c.read(Duration::ZERO).await.unwrap();
        assert_eq!(r.entries, Err("no answer".to_string()));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        assert_eq!(most.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn only_a_successful_structure_read_asks() {
        let answer = Arc::new(Mutex::new(Ok(ONE.to_string())));
        let (c, asked, _) = stand_in(answer, Duration::ZERO);
        // A failed structure load: no new successful read, nothing asked.
        assert!(c.after_structure(0).await.is_none());
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        assert!(c.after_structure(1).await.is_some());
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // The same read reported again, or a failure after it: nothing.
        assert!(c.after_structure(1).await.is_none());
        // Another read within the minute reuses the last one.
        assert!(c.after_structure(2).await.is_some());
        assert_eq!(asked.load(Ordering::SeqCst), 1);
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
        let r = refusal("HTTP 403");
        assert_eq!(
            r,
            "refused (HTTP 403): reading calendar entries needs admin rights"
        );
        assert!(is_refusal(&r));
        assert_eq!(refusal("no answer"), "no answer");
        assert!(!is_refusal("no answer"));
    }
}
