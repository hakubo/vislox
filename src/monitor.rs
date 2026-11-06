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
use std::time::Duration;
use tokio::sync::broadcast;

/// How many lines vislox keeps for a browser that opens the panel later.
const KEEP: usize = 500;

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

pub struct Monitor {
    on: Result<On, String>,
    tx: broadcast::Sender<Msg>,
}

struct On {
    port: u16,
    advertise: Option<Ipv4Addr>,
    /// The Miniserver's host (from lox's configuration), and the serial the
    /// config belongs to.
    host: String,
    serial: Option<String>,
    minutes: u32,
    http: Option<Arc<Http>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: VecDeque<Line>,
    stats: Option<(i64, Stats)>,
    /// The Miniserver's addresses (resolved from its host).
    from: Vec<IpAddr>,
    /// Bound, or why not.
    listening: Option<Result<String, String>>,
    packets: u64,
    ignored: u64,
    ignored_from: Option<IpAddr>,
    /// Started from vislox (`dev/sps/log/<ip>`): until when (Unix time),
    /// and a number that changes with each start, for the timer.
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
    pub fn new(
        settings: Option<Settings>,
        found: Result<&crate::live::Found, &String>,
        serial: Option<String>,
    ) -> Monitor {
        let (tx, _) = broadcast::channel(256);
        let on = match (settings, found) {
            (None, _) => Err("off (start vislox with --monitor)".to_string()),
            (Some(_), Err(e)) => Err(format!("no lox configuration: {e}")),
            (Some(s), Ok(f)) => Ok(On {
                port: s.port,
                advertise: s.advertise,
                host: host_of(&f.conn.host),
                serial,
                minutes: s.minutes.max(1),
                http: Some(Arc::new(Http::new(f.conn.clone()))),
                state: Mutex::new(State::default()),
            }),
        };
        Monitor { on, tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Msg> {
        self.tx.subscribe()
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

    /// The lines kept, oldest first.
    pub fn lines(&self) -> Value {
        match &self.on {
            Ok(on) => json!(on.state.lock().unwrap().entries),
            Err(_) => json!([]),
        }
    }

    /// Bind the UDP port and keep what the Miniserver sends. Returns at once.
    pub fn spawn(self: &Arc<Self>) {
        let Ok(on) = &self.on else { return };
        let me = self.clone();
        let port = on.port;
        tokio::spawn(async move {
            let from = resolve(&me.on.as_ref().unwrap().host).await;
            let sock = tokio::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port))).await;
            {
                let on = me.on.as_ref().unwrap();
                let mut st = on.state.lock().unwrap();
                st.from = from.clone();
                st.listening = Some(match &sock {
                    Ok(s) => Ok(s.local_addr().map(|a| a.to_string()).unwrap_or_default()),
                    Err(e) => Err(format!("cannot listen on UDP port {port}: {e}")),
                });
            }
            let _ = me.tx.send(Msg::Info);
            let Ok(sock) = sock else { return };
            if from.is_empty() {
                eprintln!(
                    "vislox: Debug-Monitor: cannot resolve the Miniserver's address; every packet is dropped"
                );
            }
            let mut buf = vec![0u8; 65536];
            loop {
                let Ok((n, peer)) = sock.recv_from(&mut buf).await else {
                    continue;
                };
                me.receive(peer.ip(), &buf[..n]);
            }
        });
    }

    /// One datagram from `peer`.
    pub fn receive(&self, peer: IpAddr, data: &[u8]) {
        let Ok(on) = &self.on else { return };
        let mut new = Vec::new();
        let mut info = false;
        {
            let mut st = on.state.lock().unwrap();
            if !st.from.iter().any(|a| same_ip(*a, peer)) {
                st.ignored += 1;
                if st.ignored_from.is_none() {
                    info = true;
                }
                st.ignored_from = Some(peer);
                drop(st);
                if info {
                    let _ = self.tx.send(Msg::Info);
                }
                return;
            }
            st.packets += 1;
            for f in monframe::frames(data) {
                let Some(h) = Header::parse(f) else { continue };
                if h.string_id == monframe::STATS {
                    if let Some(s) = Stats::parse(f) {
                        st.stats = Some((chrono::Utc::now().timestamp(), s));
                        info = true;
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
            if st.packets == 1 {
                info = true;
            }
        }
        if !new.is_empty() {
            let _ = self.tx.send(Msg::Lines(Arc::new(json!(new).to_string())));
        }
        if info {
            let _ = self.tx.send(Msg::Info);
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
    /// address on the route to the Miniserver.
    fn dest(&self) -> Option<Ipv4Addr> {
        let on = self.on.as_ref().ok()?;
        if let Some(d) = on.advertise {
            on.state.lock().unwrap().dest = Some(d);
            return Some(d);
        }
        let to = on.state.lock().unwrap().from.first().copied()?;
        let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        // Connecting a UDP socket sends nothing; it picks the route.
        s.connect(SocketAddr::new(to, 7777)).ok()?;
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
        let dest = self
            .dest()
            .ok_or("cannot tell this host's address for the Miniserver (pass --monitor-ip)")?;
        let http = on.http.clone().ok_or("no connection")?;
        let path = format!("dev/sps/log/{dest}");
        tokio::task::spawn_blocking(move || http.call(&path))
            .await
            .map_err(|e| e.to_string())?
            .inspect_err(|e| self.note_error(e))?;
        let until = chrono::Utc::now().timestamp() + on.minutes as i64 * 60;
        let generation = {
            let mut st = on.state.lock().unwrap();
            st.until = Some(until);
            st.last_error = None;
            st.generation += 1;
            st.generation
        };
        let _ = self.tx.send(Msg::Info);
        let me = self.clone();
        let minutes = on.minutes;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(minutes as u64 * 60)).await;
            let current = me
                .on
                .as_ref()
                .is_ok_and(|on| on.state.lock().unwrap().generation == generation);
            if current {
                let _ = me.stop().await;
            }
        });
        Ok(format!(
            "The Miniserver sends its debug output to {dest} for {minutes} min."
        ))
    }

    /// Stop: `dev/sps/log`.
    pub async fn stop(&self) -> Result<String, String> {
        let on = self.on.as_ref().map_err(Clone::clone)?;
        let http = on.http.clone().ok_or("no connection")?;
        let res = tokio::task::spawn_blocking(move || http.call("dev/sps/log"))
            .await
            .map_err(|e| e.to_string())?;
        {
            let mut st = on.state.lock().unwrap();
            st.generation += 1;
            match &res {
                Ok(()) => {
                    st.until = None;
                    st.last_error = None;
                }
                Err(e) => st.last_error = Some(e.clone()),
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

    #[cfg(test)]
    fn accept_from(&self, a: IpAddr) {
        if let Ok(on) = &self.on {
            on.state.lock().unwrap().from = vec![a];
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

/// `https://192.168.1.120:443/x` → `192.168.1.120`; `[fe80::1]:80` → `fe80::1`.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let hostport = rest.split('/').next().unwrap_or(rest);
    let hostport = hostport.rsplit_once('@').map_or(hostport, |(_, h)| h);
    if let Some(v6) = hostport.strip_prefix('[') {
        return v6.split(']').next().unwrap_or(v6).to_string();
    }
    hostport
        .rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(hostport, |(h, _)| h)
        .to_string()
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

    fn monitor(serial: Option<&str>) -> Arc<Monitor> {
        let f = found("http://127.0.0.1:1");
        Arc::new(Monitor::new(
            Some(Settings {
                port: 0,
                advertise: Some(Ipv4Addr::new(192, 168, 1, 82)),
                minutes: 60,
            }),
            Ok(&f),
            serial.map(String::from),
        ))
    }

    #[test]
    fn off_unless_asked_for() {
        let f = found("http://127.0.0.1");
        let m = Monitor::new(None, Ok(&f), None);
        assert_eq!(m.info()["available"], false);
        assert!(m.server_xml("504F94A0E454").is_none());
        let e = "no config".to_string();
        let m = Monitor::new(
            Some(Settings {
                port: 7777,
                advertise: None,
                minutes: 60,
            }),
            Err(&e),
            None,
        );
        assert!(m.info()["reason"].as_str().unwrap().contains("no config"));
    }

    #[test]
    fn only_the_miniserver_s_packets_count() {
        let m = monitor(None);
        m.accept_from("192.168.1.120".parse().unwrap());
        let mut rx = m.subscribe();
        let f = frame(1, 1, 0, 0, &string("PRG start program"), 1);
        m.receive("192.168.1.99".parse().unwrap(), &f);
        assert_eq!(m.lines(), json!([]));
        assert_eq!(m.info()["ignored"], 1);
        assert_eq!(m.info()["ignoredFrom"], "192.168.1.99");
        // The Miniserver, also as an IPv4-mapped IPv6 address.
        m.receive("::ffff:192.168.1.120".parse().unwrap(), &f);
        let lines = m.lines();
        assert_eq!(lines[0]["text"], "PRG start program");
        assert_eq!(lines[0]["kind"], "program");
        assert_eq!(m.info()["packets"], 1);
        let mut got_lines = false;
        while let Ok(msg) = rx.try_recv() {
            got_lines |= matches!(msg, Msg::Lines(_));
        }
        assert!(got_lines);
    }

    #[test]
    fn repeats_are_dropped_and_stats_kept() {
        let m = monitor(None);
        m.accept_from("192.168.1.120".parse().unwrap());
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
        m.receive("192.168.1.120".parse().unwrap(), &data);
        let lines = m.lines();
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
        m.accept_from("192.168.1.120".parse().unwrap());
        m.set_strings(Strings::parse(
            r#"<String IDV="300" Text="Link %s: %d errors"/>"#,
        ));
        let mut body = string("Tree 1");
        body.push(0x03);
        body.extend(4u32.to_le_bytes());
        m.receive(
            "192.168.1.120".parse().unwrap(),
            &frame(300, 1, 0, 0, &body, 2),
        );
        // Unknown ID without parameters: say which message it was.
        m.receive(
            "192.168.1.120".parse().unwrap(),
            &frame(301, 2, 0, 0, &[], 0),
        );
        let l = m.lines();
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
    fn the_host_comes_from_lox_s_address() {
        assert_eq!(host_of("https://192.168.1.120"), "192.168.1.120");
        assert_eq!(host_of("http://ms.local:8080/x"), "ms.local");
        assert_eq!(host_of("192.168.1.120:443"), "192.168.1.120");
        assert_eq!(host_of("https://[fe80::1]:443"), "fe80::1");
        assert_eq!(host_of("ms"), "ms");
    }

    #[tokio::test]
    async fn the_listener_takes_frames_from_the_miniserver_only() {
        // Bind on a free port; the Miniserver is 127.0.0.1.
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let f = found("http://127.0.0.1:1");
        let m = Arc::new(Monitor::new(
            Some(Settings {
                port,
                advertise: None,
                minutes: 60,
            }),
            Ok(&f),
            None,
        ));
        let mut rx = m.subscribe();
        m.spawn();
        // Wait until it listens.
        while !matches!(rx.recv().await, Ok(Msg::Info)) {}
        assert_eq!(m.info()["listening"]["ok"], true, "{}", m.info());
        let s = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        s.send_to(&frame(5, 1, 0, 0, &string("hello"), 1), ("127.0.0.1", port))
            .await
            .unwrap();
        loop {
            if let Ok(Msg::Lines(l)) = rx.recv().await {
                assert!(l.contains("hello"));
                break;
            }
        }
        // The address the Miniserver would send to: this host's, on its route.
        assert!(
            m.server_xml("504F94A0E454")
                .unwrap()
                .contains("/dev/udp/127.0.0.1/")
        );
    }
}
