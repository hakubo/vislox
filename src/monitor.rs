//! The Debug-Monitor: the Miniserver's live debug output, as UDP frames
//! (see `monframe.rs`), shown in the Log panel next to `lox log`.
//!
//! Off unless vislox is started with `--monitor` (`VISLOX_MONITOR=1`). Then
//! vislox listens on a UDP port (7777 by default) and keeps what the
//! Miniserver it shows sends there; packets from any other address are
//! dropped. The Miniserver sends only once it is told to, in one of two ways:
//!
//! - **Start** in the Log panel calls `dev/sps/log/<this host>`, as Loxone
//!   Config's "Debug Information" does: the Miniserver then sends to that
//!   address, on its Debug-Monitor port (`LoxWATCHPort`, 7777 by default).
//!   **Stop**, vislox stopping, or the time limit (`--monitor-minutes`) call
//!   `dev/sps/log`, which ends it. Nothing in the config changes, and the
//!   program does not restart.
//! - A **Monitor Server** pointed at vislox asks `GET /?<serial>`, and vislox
//!   answers with the XML that names its address and port, the log levels and
//!   a one-day limit (Miniservers that have this setting).

use crate::log;
use crate::monframe::{self, Header, Stats, Strings};
use crate::restart::Http;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, broadcast};

/// How many lines vislox keeps for a browser that opens the panel later.
const KEEP: usize = 500;
/// How often the Miniserver's address is looked up again; a packet from an
/// unknown address asks for it sooner, at most this often.
const RESOLVE_EVERY: Duration = Duration::from_secs(300);
const RESOLVE_SOONEST: Duration = Duration::from_secs(30);
/// Counters reach the browsers at most this often.
const INFO_EVERY: Duration = Duration::from_secs(1);
/// After this many receive errors in a row, the listener gives up.
const RECV_ERRORS: u32 = 50;

pub struct Settings {
    pub port: u16,
    /// The address the Miniserver sends to: this host's, as the Miniserver
    /// reaches it. Default: the local address of a route to the Miniserver.
    pub advertise: Option<Ipv4Addr>,
    /// How long a Start lasts.
    pub minutes: u32,
}

/// What a browser gets: new lines, or new state (stats, status).
#[derive(Clone, Debug)]
pub enum Msg {
    Lines(Arc<String>),
    Info,
}

/// Sends a `dev/…` command to the Miniserver (lox's connection).
pub trait Cmd: Send + Sync {
    fn call(&self, path: &str) -> Result<(), String>;
}

impl Cmd for Http {
    fn call(&self, path: &str) -> Result<(), String> {
        Http::call(self, path)
    }
}

pub struct Monitor {
    on: Result<On, String>,
    tx: broadcast::Sender<Msg>,
    /// Look the Miniserver's address up again.
    resolve: Notify,
}

struct On {
    port: u16,
    advertise: Option<Ipv4Addr>,
    /// The Miniserver's host (from lox's configuration), and the serial the
    /// config belongs to.
    host: String,
    serial: Option<String>,
    minutes: u32,
    /// How long a Start lasts, and the first wait before a failed Stop is
    /// tried again (doubling, up to 5 min).
    limit: Duration,
    retry: Duration,
    cmd: Arc<dyn Cmd>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: VecDeque<Line>,
    stats: Option<(i64, Stats)>,
    /// The Miniserver's addresses (resolved from its host), and when.
    from: Vec<IpAddr>,
    resolved: Option<Instant>,
    /// Bound, or why not.
    listening: Option<Result<String, String>>,
    packets: u64,
    ignored: u64,
    ignored_from: Option<IpAddr>,
    /// Counters changed since the browsers last heard.
    dirty: bool,
    /// Started from vislox (`dev/sps/log/<ip>`): until when (Unix time),
    /// and a number that changes with each start and stop that worked.
    until: Option<i64>,
    generation: u64,
    last_error: Option<String>,
    /// The last log line's event counter, to drop repeats.
    counter: Option<u16>,
    strings: Option<Arc<Strings>>,
    /// The address the Miniserver sends to, once known.
    dest: Option<Ipv4Addr>,
}

/// A line of the Debug-Monitor, in the Log panel's shape.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Line {
    pub at: String,
    pub text: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uuids: Vec<String>,
    /// The string ID, for lines whose format string vislox does not have.
    pub id: u16,
}

impl Monitor {
    /// `settings`: on, or why it is off.
    pub fn new(
        settings: Result<Settings, String>,
        found: Result<&crate::live::Found, &String>,
        serial: Option<String>,
    ) -> Monitor {
        let on = match (settings, found) {
            (Err(why), _) => Err(why),
            (Ok(_), Err(e)) => Err(format!("no lox configuration: {e}")),
            (Ok(s), Ok(f)) => Ok(On::new(
                s,
                &f.conn.host,
                serial,
                Arc::new(Http::new(f.conn.clone())),
            )),
        };
        Monitor::with(on)
    }

    fn with(on: Result<On, String>) -> Monitor {
        let (tx, _) = broadcast::channel(256);
        Monitor {
            on,
            tx,
            resolve: Notify::new(),
        }
    }

    /// The lines kept (oldest first) and a receiver for what comes after
    /// them: taken under one lock, so no line is in both.
    pub fn subscribe(&self) -> (broadcast::Receiver<Msg>, Value) {
        match &self.on {
            Ok(on) => {
                let st = on.state.lock().unwrap();
                (self.tx.subscribe(), json!(st.entries))
            }
            Err(_) => (self.tx.subscribe(), json!([])),
        }
    }

    /// For `/api/history` and the panel: on or off, what it does, and stats.
    pub fn info(&self) -> Value {
        let on = match &self.on {
            Ok(on) => on,
            Err(why) => return json!({ "available": false, "reason": why }),
        };
        let st = on.state.lock().unwrap();
        json!({
            "available": true,
            "port": on.port,
            "host": on.host,
            "dest": st.dest.map(|d| d.to_string()),
            "listening": match &st.listening {
                Some(Ok(a)) => json!({ "ok": true, "on": a }),
                Some(Err(e)) => json!({ "ok": false, "error": e }),
                None => Value::Null,
            },
            "packets": st.packets,
            "ignored": st.ignored,
            "ignoredFrom": st.ignored_from.map(|a| a.to_string()),
            "until": st.until,
            "minutes": on.minutes,
            "error": st.last_error,
            "strings": st.strings.as_ref().map(|s| s.len()),
            "stats": st.stats.as_ref().map(|(at, s)| json!({ "at": at, "cpu": s.cpu, "heapKb": s.heap_kb, "maxKb": s.max_kb, "uptimeMs": s.uptime_ms, "lanTxErrors": s.lan_tx_errors, "linkErrors": s.link_errors })),
        })
    }

    /// Bind the UDP port and keep what the Miniserver sends; look its address
    /// up now and then; tell the browsers about new counts once a second.
    /// Returns at once.
    pub fn spawn(self: &Arc<Self>) {
        let Ok(on) = &self.on else { return };
        let port = on.port;
        // The Miniserver's address: now, every 5 min, and when asked.
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                me.refresh_from().await;
                tokio::select! {
                    _ = tokio::time::sleep(RESOLVE_EVERY) => {}
                    _ = me.resolve.notified() => {}
                }
            }
        });
        // Counters, throttled.
        let me = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(INFO_EVERY);
            loop {
                tick.tick().await;
                let dirty = me
                    .on
                    .as_ref()
                    .is_ok_and(|on| std::mem::take(&mut on.state.lock().unwrap().dirty));
                if dirty {
                    let _ = me.tx.send(Msg::Info);
                }
            }
        });
        let me = self.clone();
        tokio::spawn(async move {
            let sock = tokio::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port))).await;
            me.set_listening(match &sock {
                Ok(s) => Ok(s.local_addr().map(|a| a.to_string()).unwrap_or_default()),
                Err(e) => Err(format!("cannot listen on UDP port {port}: {e}")),
            });
            let Ok(sock) = sock else { return };
            let mut buf = vec![0u8; 65536];
            let mut errors = 0u32;
            loop {
                match sock.recv_from(&mut buf).await {
                    Ok((n, peer)) => {
                        errors = 0;
                        me.receive(peer.ip(), &buf[..n]);
                    }
                    Err(e) => {
                        errors += 1;
                        if errors >= RECV_ERRORS {
                            me.set_listening(Err(format!(
                                "stopped listening after {errors} receive errors: {e}"
                            )));
                            return;
                        }
                        // 100 ms, doubling, at most 5 s.
                        let wait = 100u64 << errors.min(6);
                        tokio::time::sleep(Duration::from_millis(wait.min(5000))).await;
                    }
                }
            }
        });
    }

    fn set_listening(&self, l: Result<String, String>) {
        if let Ok(on) = &self.on {
            on.state.lock().unwrap().listening = Some(l);
        }
        let _ = self.tx.send(Msg::Info);
    }

    /// Look the Miniserver's address up again.
    async fn refresh_from(&self) {
        let Ok(on) = &self.on else { return };
        let from = resolve(&on.host).await;
        let mut st = on.state.lock().unwrap();
        if from.is_empty() {
            st.last_error = Some(format!(
                "cannot resolve {}; every packet is dropped",
                on.host
            ));
        } else if st
            .last_error
            .as_deref()
            .is_some_and(|e| e.starts_with("cannot resolve"))
        {
            st.last_error = None;
        }
        if !from.is_empty() || st.from.is_empty() {
            st.from = from;
        }
        st.resolved = Some(Instant::now());
        st.dirty = true;
    }

    /// One datagram from `peer`.
    pub fn receive(&self, peer: IpAddr, data: &[u8]) {
        let Ok(on) = &self.on else { return };
        let mut st = on.state.lock().unwrap();
        st.dirty = true;
        if !st.from.iter().any(|a| same_ip(*a, peer)) {
            st.ignored += 1;
            st.ignored_from = Some(peer);
            // The Miniserver's address may have changed: look it up again,
            // at most every 30 s.
            if st.resolved.is_none_or(|t| t.elapsed() >= RESOLVE_SOONEST) {
                st.resolved = Some(Instant::now());
                self.resolve.notify_one();
            }
            return;
        }
        st.packets += 1;
        let mut new = Vec::new();
        for f in monframe::frames(data) {
            let Some(h) = Header::parse(f) else { continue };
            if h.string_id == monframe::STATS {
                if let Some(s) = Stats::parse(f) {
                    st.stats = Some((chrono::Utc::now().timestamp(), s));
                }
                continue;
            }
            if h.string_id >= monframe::TASKS {
                continue;
            }
            // The same line again (sent twice): once is enough.
            if st.counter == Some(h.counter) {
                continue;
            }
            st.counter = Some(h.counter);
            let line = line(&h, f, st.strings.as_deref());
            st.entries.push_back(line.clone());
            while st.entries.len() > KEEP {
                st.entries.pop_front();
            }
            new.push(line);
        }
        // Sent under the lock, so `subscribe` sees each line either in its
        // snapshot or on its receiver, never both.
        if !new.is_empty() {
            let _ = self.tx.send(Msg::Lines(Arc::new(json!(new).to_string())));
        }
    }

    /// The Monitor Server answer for `GET /?<serial>`: `None` when the
    /// monitor is off or the query is not a serial (the page is served).
    pub fn server_xml(&self, query: &str) -> Option<String> {
        let on = self.on.as_ref().ok()?;
        let asked = crate::discover::norm_serial(query);
        if !crate::discover::is_serial(&asked) {
            return None;
        }
        // Another Miniserver asking: tell it to send nothing.
        if on.serial.as_deref().is_some_and(|s| s != asked) {
            return Some(OFF_XML.to_string());
        }
        let dest = self.dest()?;
        Some(log_xml(dest, on.port))
    }

    /// The address the Miniserver sends to: `--monitor-ip`, else this host's
    /// IPv4 address on the route to the Miniserver (`LogDest` and
    /// `dev/sps/log/<ip>` take IPv4).
    fn dest(&self) -> Option<Ipv4Addr> {
        let on = self.on.as_ref().ok()?;
        if let Some(d) = on.advertise {
            on.state.lock().unwrap().dest = Some(d);
            return Some(d);
        }
        let to = {
            let st = on.state.lock().unwrap();
            let v4 = |a: &IpAddr| match a {
                IpAddr::V4(v) => Some(*v),
                IpAddr::V6(v) => v.to_ipv4_mapped(),
            };
            st.from.iter().find_map(v4)?
        };
        let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        // Connecting a UDP socket sends nothing; it picks the route.
        s.connect(SocketAddr::new(IpAddr::V4(to), 7777)).ok()?;
        let d = match s.local_addr().ok()?.ip() {
            IpAddr::V4(a) if !a.is_unspecified() => a,
            _ => return None,
        };
        on.state.lock().unwrap().dest = Some(d);
        Some(d)
    }

    /// Start: `dev/sps/log/<this host>`, until Stop or the time limit.
    pub async fn start(self: &Arc<Self>) -> Result<String, String> {
        let on = self.on.as_ref().map_err(Clone::clone)?;
        // The Miniserver's address may have changed since the last lookup.
        if on.advertise.is_none() {
            self.refresh_from().await;
        }
        let dest = self
            .dest()
            .ok_or("cannot tell this host's IPv4 address for the Miniserver (pass --monitor-ip)")?;
        let cmd = on.cmd.clone();
        let path = format!("dev/sps/log/{dest}");
        tokio::task::spawn_blocking(move || cmd.call(&path))
            .await
            .map_err(|e| e.to_string())?
            .inspect_err(|e| self.note_error(e))?;
        let until = chrono::Utc::now().timestamp() + on.limit.as_secs() as i64;
        let generation = {
            let mut st = on.state.lock().unwrap();
            st.until = Some(until);
            st.last_error = None;
            st.generation += 1;
            st.generation
        };
        let _ = self.tx.send(Msg::Info);
        let me = self.clone();
        let limit = on.limit;
        tokio::spawn(async move {
            tokio::time::sleep(limit).await;
            me.stop_after_limit(generation).await;
        });
        Ok(format!(
            "The Miniserver sends its debug output to {dest} for {} min.",
            on.minutes
        ))
    }

    /// The time limit of the Start `generation` ran out: stop, and try again
    /// (30 s, doubling, up to 5 min) until it works, or until another Start
    /// or Stop takes over.
    async fn stop_after_limit(&self, generation: u64) {
        let Ok(on) = &self.on else { return };
        let mut wait = on.retry;
        loop {
            let current = {
                let st = on.state.lock().unwrap();
                st.generation == generation && st.until.is_some()
            };
            if !current || self.stop().await.is_ok() {
                return;
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(300));
        }
    }

    /// Stop: `dev/sps/log`. A failed Stop leaves the Start in place (and its
    /// time limit tries again).
    pub async fn stop(&self) -> Result<String, String> {
        let on = self.on.as_ref().map_err(Clone::clone)?;
        let cmd = on.cmd.clone();
        let res = tokio::task::spawn_blocking(move || cmd.call("dev/sps/log"))
            .await
            .map_err(|e| e.to_string())?;
        {
            let mut st = on.state.lock().unwrap();
            match &res {
                Ok(()) => {
                    st.generation += 1;
                    st.until = None;
                    st.last_error = None;
                }
                Err(e) => st.last_error = Some(format!("Stop failed: {e}")),
            }
        }
        let _ = self.tx.send(Msg::Info);
        res.map(|_| "The Miniserver no longer sends its debug output.".to_string())
    }

    /// Started from vislox and not stopped yet.
    pub fn started(&self) -> bool {
        self.on
            .as_ref()
            .is_ok_and(|on| on.state.lock().unwrap().until.is_some())
    }

    /// Format strings from the Miniserver's string table, when vislox can
    /// read one (a local `ENG.xml`: `--monitor-strings`).
    pub fn set_strings(&self, s: Strings) {
        if let Ok(on) = &self.on {
            on.state.lock().unwrap().strings = Some(Arc::new(s));
        }
    }

    fn note_error(&self, e: &str) {
        if let Ok(on) = &self.on {
            on.state.lock().unwrap().last_error = Some(e.to_string());
        }
        let _ = self.tx.send(Msg::Info);
    }
}

impl On {
    fn new(s: Settings, host: &str, serial: Option<String>, cmd: Arc<dyn Cmd>) -> On {
        let minutes = s.minutes.max(1);
        On {
            port: s.port,
            advertise: s.advertise,
            host: crate::miniserver::split_host(host).0,
            serial,
            minutes,
            limit: Duration::from_secs(minutes as u64 * 60),
            retry: Duration::from_secs(30),
            cmd,
            state: Mutex::new(State::default()),
        }
    }
}

fn line(h: &Header, f: &[u8], strings: Option<&Strings>) -> Line {
    let params = monframe::params(f, h.params);
    let fmt = strings.and_then(|s| s.get(h.string_id));
    let mut text = monframe::format(fmt, &params);
    if text.is_empty() {
        text = format!("(message {})", h.string_id);
    }
    Line {
        at: h.at(),
        kind: log::kind(&text),
        uuids: log::uuids(&text),
        text,
        id: h.string_id,
    }
}

const OFF_XML: &str = "<?xml version=\"1.0\"?>\n<Log Version=\"10\" LogKeep=\"true\" NoIPchange=\"false\">\n  <LogMode>off</LogMode>\n</Log>\n";

/// What a Monitor Server answers: send to `dest:port`, for a day at most.
/// The levels: general and logic (SPS) information, Link/Tree bus warnings
/// and restarts, no protocol, file system or network detail.
pub fn log_xml(dest: Ipv4Addr, port: u16) -> String {
    format!(
        "<?xml version=\"1.0\"?>
<Log Version=\"10\" LogKeep=\"true\" NoIPchange=\"false\">
  <LogMode>on</LogMode>
  <LogDest>/dev/udp/{dest}/{port}</LogDest>
  <LogLevelCommon>info</LogLevelCommon>
  <LogLevelSPS>info</LogLevelSPS>
  <LogLevelProtocol>off</LogLevelProtocol>
  <LogLevelBus>info</LogLevelBus>
  <LogLevelFilesystem>off</LogLevelFilesystem>
  <LogLevelNet>off</LogLevelNet>
  <LogDay>1</LogDay>
</Log>
"
    )
}

async fn resolve(host: &str) -> Vec<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return vec![ip];
    }
    match tokio::net::lookup_host((host, 0)).await {
        Ok(a) => a.map(|s| s.ip()).collect(),
        Err(_) => Vec::new(),
    }
}

/// The same address, also when one is an IPv4-mapped IPv6 address.
fn same_ip(a: IpAddr, b: IpAddr) -> bool {
    let v4 = |x: IpAddr| match x {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(x),
        x => x,
    };
    v4(a) == v4(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monframe::tests::{frame, stats, string};
    use loxone_client::Connection;

    fn found(host: &str) -> crate::live::Found {
        crate::live::Found {
            conn: Connection::new(host, "u", "p"),
            from: "test".into(),
            ctx: None,
            config_repo: None,
            ms_dir: "x".into(),
            serial: None,
        }
    }

    /// A stand-in for the Miniserver's `dev/…` commands: records them, and
    /// fails the first `fail` ones.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
        fail: Mutex<u32>,
    }

    impl Cmd for Fake {
        fn call(&self, path: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(path.to_string());
            let mut f = self.fail.lock().unwrap();
            if *f > 0 {
                *f -= 1;
                return Err("no answer".into());
            }
            Ok(())
        }
    }

    fn with_fake(fake: Arc<Fake>, limit: Duration) -> Arc<Monitor> {
        let mut on = On::new(
            Settings {
                port: 0,
                advertise: Some(Ipv4Addr::new(192, 168, 1, 82)),
                minutes: 60,
            },
            "https://192.168.1.120",
            None,
            fake,
        );
        on.limit = limit;
        on.retry = Duration::from_millis(20);
        Arc::new(Monitor::with(Ok(on)))
    }

    fn monitor(serial: Option<&str>) -> Arc<Monitor> {
        Arc::new(Monitor::with(Ok(On::new(
            Settings {
                port: 0,
                advertise: Some(Ipv4Addr::new(192, 168, 1, 82)),
                minutes: 60,
            },
            "http://127.0.0.1:1",
            serial.map(String::from),
            Arc::new(Fake::default()),
        ))))
    }

    impl Monitor {
        fn accept_from(&self, a: &[&str]) {
            if let Ok(on) = &self.on {
                on.state.lock().unwrap().from = a.iter().map(|x| x.parse().unwrap()).collect();
            }
        }
    }

    const MS: &str = "192.168.1.120";

    /// A test that hangs fails instead.
    async fn within<F: std::future::Future>(f: F) -> F::Output {
        tokio::time::timeout(std::time::Duration::from_secs(30), f)
            .await
            .expect("the test took longer than 30 s")
    }

    #[test]
    fn off_unless_asked_for() {
        let f = found("http://127.0.0.1");
        let m = Monitor::new(Err("turned off (--no-live)".into()), Ok(&f), None);
        assert_eq!(m.info()["available"], false);
        assert_eq!(m.info()["reason"], "turned off (--no-live)");
        assert!(m.server_xml("504F94A0E454").is_none());
        let e = "no config".to_string();
        let s = Settings {
            port: 7777,
            advertise: None,
            minutes: 60,
        };
        let m = Monitor::new(Ok(s), Err(&e), None);
        assert!(m.info()["reason"].as_str().unwrap().contains("no config"));
    }

    #[test]
    fn only_the_miniserver_s_packets_count() {
        let m = monitor(None);
        m.accept_from(&[MS]);
        let (mut rx, _) = m.subscribe();
        let f = frame(1, 1, 0, 0, &string("PRG start program"), 1);
        m.receive("192.168.1.99".parse().unwrap(), &f);
        assert_eq!(m.subscribe().1, json!([]));
        assert_eq!(m.info()["ignored"], 1);
        assert_eq!(m.info()["ignoredFrom"], "192.168.1.99");
        // The Miniserver, also as an IPv4-mapped IPv6 address.
        m.receive("::ffff:192.168.1.120".parse().unwrap(), &f);
        let lines = m.subscribe().1;
        assert_eq!(lines[0]["text"], "PRG start program");
        assert_eq!(lines[0]["kind"], "program");
        assert_eq!(m.info()["packets"], 1);
        assert!(matches!(rx.try_recv(), Ok(Msg::Lines(_))));
    }

    #[test]
    fn repeats_are_dropped_and_stats_kept() {
        let m = monitor(None);
        m.accept_from(&[MS]);
        let warn = frame(
            2,
            9,
            0,
            0,
            &string("Warning 1167, all, MeterAbsBi (Sieć,1d525782-02d4-82be-ffff60c50ef75c80)"),
            1,
        );
        let mut data = warn.clone();
        data.extend(&warn);
        data.extend(frame(
            monframe::STATS,
            1,
            0,
            0,
            &stats(900, 1000, 4096, 8192),
            0,
        ));
        m.receive(MS.parse().unwrap(), &data);
        let lines = m.subscribe().1;
        assert_eq!(lines.as_array().unwrap().len(), 1);
        assert_eq!(lines[0]["kind"], "warning");
        assert_eq!(lines[0]["uuids"][0], "1d525782-02d4-82be-ffff60c50ef75c80");
        let s = &m.info()["stats"];
        assert!((s["cpu"].as_f64().unwrap() - 10.0).abs() < 0.01, "{s}");
        assert_eq!(s["heapKb"], 4);
    }

    #[test]
    fn format_strings_come_from_the_string_table() {
        let m = monitor(None);
        m.accept_from(&[MS]);
        m.set_strings(Strings::parse(
            r#"<String IDV="300" Text="Link %s: %d errors"/>"#,
        ));
        let mut body = string("Tree 1");
        body.push(0x03);
        body.extend(4u32.to_le_bytes());
        m.receive(MS.parse().unwrap(), &frame(300, 1, 0, 0, &body, 2));
        // Unknown ID without parameters: say which message it was.
        m.receive(MS.parse().unwrap(), &frame(301, 2, 0, 0, &[], 0));
        let l = m.subscribe().1;
        assert_eq!(l[0]["text"], "Link Tree 1: 4 errors");
        assert_eq!(l[1]["text"], "(message 301)");
    }

    #[test]
    fn the_monitor_server_answer_names_vislox() {
        let m = monitor(Some("504F94A0E454"));
        let xml = m.server_xml("504f94a0e454").unwrap();
        assert!(
            xml.contains("<LogDest>/dev/udp/192.168.1.82/0</LogDest>"),
            "{xml}"
        );
        assert!(xml.contains("<LogMode>on</LogMode>"));
        assert!(xml.contains("<LogDay>1</LogDay>"));
        // Another Miniserver: logging off. Not a serial: the page.
        assert!(
            m.server_xml("504F94000001")
                .unwrap()
                .contains("<LogMode>off</LogMode>")
        );
        assert!(m.server_xml("rev=abc").is_none());
        assert!(m.server_xml("").is_none());
    }

    #[test]
    fn the_host_comes_from_lox_s_address_also_bare_ipv6() {
        let host = |h: &str| {
            let f = found(h);
            let s = Settings {
                port: 0,
                advertise: None,
                minutes: 1,
            };
            Monitor::new(Ok(s), Ok(&f), None).info()["host"].clone()
        };
        assert_eq!(host("https://192.168.1.120"), "192.168.1.120");
        assert_eq!(host("http://ms.local:8080/x"), "ms.local");
        assert_eq!(host("https://[fe80::1]:443"), "fe80::1");
        assert_eq!(host("fe80::1"), "fe80::1");
        assert_eq!(host("https://fe80::1"), "fe80::1");
    }

    #[test]
    fn the_destination_is_ipv4() {
        let m = Arc::new(Monitor::with(Ok(On::new(
            Settings {
                port: 7777,
                advertise: None,
                minutes: 1,
            },
            "ms.local",
            None,
            Arc::new(Fake::default()),
        ))));
        // Not looked up yet, or IPv6 only: no address to send to.
        assert!(m.server_xml("504F94A0E454").is_none());
        m.accept_from(&["fe80::1"]);
        assert!(m.server_xml("504F94A0E454").is_none());
        // IPv4 too: that one, whatever the order.
        m.accept_from(&["::1", "127.0.0.1"]);
        assert!(
            m.server_xml("504F94A0E454")
                .unwrap()
                .contains("<LogDest>/dev/udp/127.0.0.1/7777</LogDest>")
        );
    }

    #[tokio::test]
    async fn a_failed_stop_keeps_the_start_and_the_time_limit_tries_again() {
        within(async {
            let fake = Arc::new(Fake::default());
            let m = with_fake(fake.clone(), Duration::from_millis(50));
            m.start().await.unwrap();
            assert!(m.started());
            // Stop by hand fails: still started, with the reason.
            *fake.fail.lock().unwrap() = 1;
            assert!(m.stop().await.is_err());
            assert!(m.started());
            assert!(
                m.info()["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("Stop failed")
            );
            // The time limit stops it; its first two tries fail, the third works.
            *fake.fail.lock().unwrap() = 2;
            for _ in 0..200 {
                if !m.started() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(!m.started(), "{}", m.info());
            assert_eq!(
                *fake.calls.lock().unwrap(),
                [
                    "dev/sps/log/192.168.1.82",
                    "dev/sps/log",
                    "dev/sps/log",
                    "dev/sps/log",
                    "dev/sps/log"
                ]
            );
            assert!(m.info()["error"].is_null());
        })
        .await;
    }

    #[tokio::test]
    async fn a_stop_or_new_start_cancels_the_old_time_limit() {
        within(async {
            let fake = Arc::new(Fake::default());
            let m = with_fake(fake.clone(), Duration::from_millis(40));
            m.start().await.unwrap();
            m.stop().await.unwrap();
            m.start().await.unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
            // The first Start's limit has run out, the second's not yet.
            assert!(m.started());
            tokio::time::sleep(Duration::from_millis(60)).await;
            assert!(!m.started());
            assert_eq!(fake.calls.lock().unwrap().len(), 4);
        })
        .await;
    }

    #[test]
    fn a_line_is_in_the_snapshot_or_on_the_receiver_never_both() {
        let m = monitor(None);
        m.accept_from(&[MS]);
        let sender = {
            let m = m.clone();
            std::thread::spawn(move || {
                for i in 0..600u16 {
                    let f = frame(1, i, 0, 0, &string(&format!("line {i}")), 1);
                    m.receive(MS.parse().unwrap(), &f);
                }
            })
        };
        // At most 100 snapshots, spread over the sending: each holds up to
        // 500 lines, so taking them as fast as possible grows without bound
        // while the sender is starved (as on a busy CI runner).
        let mut taken = Vec::new();
        while !sender.is_finished() && taken.len() < 100 {
            taken.push(m.subscribe());
            std::thread::sleep(Duration::from_micros(200));
        }
        sender.join().unwrap();
        assert!(!taken.is_empty());
        for (mut rx, snap) in taken {
            let mut seen: std::collections::HashSet<String> = snap
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l["text"].as_str().unwrap().to_string())
                .collect();
            while let Ok(msg) = rx.try_recv() {
                if let Msg::Lines(l) = msg {
                    for l in serde_json::from_str::<Vec<Value>>(&l).unwrap() {
                        let t = l["text"].as_str().unwrap().to_string();
                        assert!(seen.insert(t.clone()), "{t} twice");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn the_listener_takes_frames_and_reports_counts() {
        within(async {
            // Bind on a free port; the Miniserver is 127.0.0.1.
            let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            let f = found("http://127.0.0.1:1");
            let s = Settings {
                port,
                advertise: None,
                minutes: 60,
            };
            let m = Arc::new(Monitor::new(Ok(s), Ok(&f), None));
            let (mut rx, _) = m.subscribe();
            m.spawn();
            // Listening, and the Miniserver's address looked up.
            while m.info()["listening"]["ok"] != true || m.server_xml("504F94A0E454").is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let s = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            s.send_to(&frame(5, 1, 0, 0, &string("hello"), 1), ("127.0.0.1", port))
                .await
                .unwrap();
            // The line at once, then the new count within a second.
            let mut line = false;
            loop {
                match rx.recv().await {
                    Ok(Msg::Lines(l)) => line = l.contains("hello"),
                    Ok(Msg::Info) if line && m.info()["packets"] == 1 => break,
                    _ => {}
                }
            }
            assert!(
                m.server_xml("504F94A0E454")
                    .unwrap()
                    .contains("/dev/udp/127.0.0.1/")
            );
        })
        .await;
    }
}
