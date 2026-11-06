//! The Miniserver's status, and a preflight before Pull, Apply and Sync.
//!
//! Everything here only reads, and nothing logs in:
//! - a TCP connect to the HTTP(S) port lox talks to (443, or the port in
//!   lox's `host`). Apply needs it: lox restarts the program over HTTP(S),
//! - a TCP connect to FTP (port 21) and its greeting. lox does Pull, Apply
//!   and Sync over FTP, and live values only need the HTTP port, so a
//!   firewall can leave live values working while Pull and Apply fail,
//! - `jdev/cfg/apiKey`, which the Miniserver answers without a login: serial,
//!   firmware version, TLS state (`httpsStatus`), whether it sees this host
//!   as local, and (on some versions) whether a live-update slot is free.
//!   Its format is not documented in full, so it only informs: a reply that
//!   cannot be read is a warning and never blocks anything,
//! - when lox's token expires (its `token.json`, shared with lox on purpose:
//!   vislox requests no token of its own),
//! - that it is the right Miniserver: when `apiKey` reports a serial, it must
//!   be the one the config belongs to (the repository's `<serial>` folder,
//!   lox.toml's `serial`, or the lox context's). Another serial blocks Pull,
//!   Apply and Sync: DHCP may have given the address to another device. When
//!   the address does not answer or another serial answers there, a UDP
//!   discovery broadcast (`discover.rs`) looks for the serial on the LAN and
//!   names its new address.
//!
//! Apart from the check, and on a faster clock, the config dates: when the
//! Miniserver's config last changed (`jdev/sps/LoxAPPversion3`, the
//! `lastModified` of its `LoxAPP3.json`), against the same date in the
//! download the base came from (`configdate.rs`). This one request needs
//! lox's user and password (HTTP basic auth, as Apply's restart probe). It
//! only informs: the preflight never waits for it, and a failure only says
//! that it cannot tell.
//!
//! What the token may do (`tokenRights`) and whether the password is weak
//! (`unsecurePass`) come only with a login, which lox does and does not keep,
//! so they are not shown.

use crate::configdate::BaseDate;
use crate::discover;
use crate::restart::Http;
use loxone_client::Connection;
use loxone_client::token::TokenStore;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How long a TCP connect or a reply may take.
const TIMEOUT: Duration = Duration::from_secs(4);
/// "Check again" from the page (no token needed) reuses a check this recent.
const MIN_AGE: Duration = Duration::from_secs(2);
/// Pull, Apply and Sync reuse a check this recent, so that the retries of an
/// Apply (after the user allows something) do not check again each time.
const PREFLIGHT_AGE: Duration = Duration::from_secs(10);
/// A check this old is redone for the status chip.
const MAX_AGE: Duration = Duration::from_secs(60);
/// The page's regular poll (every minute) reuses a check this recent, and
/// asks only for the config dates again.
const POLL_AGE: Duration = Duration::from_secs(5 * 60);
/// The config dates are asked for again when older than this.
const DATE_AGE: Duration = Duration::from_secs(30);
/// How long `LoxAPPversion3` may take before the chip says it cannot tell.
const DATE_TIMEOUT: Duration = Duration::from_secs(5);

/// What needs the Miniserver, and so a preflight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Pull,
    Apply,
    Sync,
    /// A command to a virtual input (`--operate`).
    Operate,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Pull => "pull",
            Action::Apply => "apply",
            Action::Sync => "sync",
            Action::Operate => "operate",
        }
    }
}

pub struct Miniserver {
    target: Result<Target, String>,
    /// The last check: when it started, and what it found.
    last: Mutex<Option<(Instant, Arc<Value>)>>,
    probing: tokio::sync::Mutex<()>,
    /// The Miniserver the config belongs to: the one Pull, Apply and Sync
    /// must talk to.
    expected: Option<Expected>,
    /// The last discovery broadcast (it waits 3 s), reused for a minute.
    discovery: discover::Cache,
    /// Bumped by [`Miniserver::forget`]: a check or a date that started
    /// before keeps its result to itself, since what it read may be old.
    generation: AtomicU64,
    /// The last config dates: when they were asked for, and what came back.
    dates: Mutex<Option<(Instant, Dates)>>,
    dating: tokio::sync::Mutex<()>,
}

/// When the config last changed, on the Miniserver and in the base's download.
#[derive(Debug, Clone, Default, PartialEq)]
struct Dates {
    /// `LoxAPPversion3`, or why it could not be read; `None` when the
    /// Miniserver does not answer.
    miniserver: Option<Result<String, String>>,
    /// The `lastModified` of the download the base came from.
    base: Option<String>,
}

/// The serial the config belongs to, and where vislox read it.
#[derive(Debug, Clone)]
pub struct Expected {
    /// As [`discover::norm_serial`] writes it.
    pub serial: String,
    /// For messages: `the repository folder`, `lox.toml`, `lox context home`.
    pub from: String,
    /// The lox context, for the command that fixes its address.
    pub ctx: Option<String>,
}

struct Target {
    conn: Connection,
    /// The address to connect to: no scheme, port or IPv6 brackets.
    addr: String,
    http_port: u16,
    ftp_port: u16,
    /// The config files whose Loxone Config version is compared with the
    /// firmware: the deployed one, and the pending one.
    current: PathBuf,
    proposed: Option<PathBuf>,
    /// Authenticated `jdev/…` requests (`LoxAPPversion3`).
    http: Arc<Http>,
    /// When the base was taken, in the Miniserver's terms.
    base_date: Arc<BaseDate>,
}

/// What `jdev/cfg/apiKey` tells.
#[derive(Debug, Default, PartialEq)]
struct ApiKey {
    serial: Option<String>,
    version: Option<String>,
    /// 0: no TLS, 1: TLS, 2: the certificate expired.
    https_status: Option<i64>,
    local: Option<bool>,
    event_slots: Option<bool>,
}

impl Miniserver {
    /// `target`: the connection lox uses, or why there is none.
    pub fn new(
        target: Result<&Connection, String>,
        current: PathBuf,
        proposed: Option<PathBuf>,
        backups: Option<PathBuf>,
    ) -> Miniserver {
        let target = target.map(|conn| {
            let (addr, http_port) = split_host(&conn.host);
            Target {
                conn: conn.clone(),
                addr,
                http_port,
                // lox always uses 21; the override is for tests.
                ftp_port: std::env::var("VISLOX_FTP_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(21),
                base_date: Arc::new(BaseDate::new(current.clone(), backups)),
                http: Arc::new(Http::new(conn.clone())),
                current,
                proposed,
            }
        });
        Miniserver::with_target(target)
    }

    fn with_target(target: Result<Target, String>) -> Miniserver {
        Miniserver {
            target,
            last: Mutex::new(None),
            probing: tokio::sync::Mutex::new(()),
            expected: None,
            discovery: discover::Cache::default(),
            generation: AtomicU64::new(0),
            dates: Mutex::new(None),
            dating: tokio::sync::Mutex::new(()),
        }
    }

    /// Check that the Miniserver at the address is the one with this serial.
    pub fn expecting(mut self, expected: Option<Expected>) -> Miniserver {
        self.expected = expected;
        self
    }

    /// The status, checked again when the last check started more than
    /// `max_age` ago.
    async fn status(&self, max_age: Duration) -> Arc<Value> {
        let t = match &self.target {
            Ok(t) => t,
            Err(why) => return Arc::new(json!({ "available": false, "reason": why })),
        };
        let asked = Instant::now();
        let reuse = |ok: &dyn Fn(Instant) -> bool| {
            self.last
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(started, _)| ok(*started))
                .map(|(_, v)| v.clone())
        };
        if let Some(v) = reuse(&|started| started.elapsed() < max_age) {
            return v;
        }
        // One check at a time. Whoever waited may take a check that ran
        // meanwhile, but only one that started after it asked.
        let _one = self.probing.lock().await;
        if let Some(v) = reuse(&|started| started >= asked) {
            return v;
        }
        let started = Instant::now();
        let generation = self.generation.load(Ordering::SeqCst);
        let v = Arc::new(probe(t, self.expected.as_ref(), &self.discovery).await);
        if self.generation.load(Ordering::SeqCst) == generation {
            *self.last.lock().unwrap() = Some((started, v.clone()));
        }
        v
    }

    /// The config dates, asked for again when older than `max_age`. Apart
    /// from [`Miniserver::status`], so that the preflight never waits for
    /// them.
    async fn dates(&self, t: &Target, max_age: Duration, reachable: bool) -> Dates {
        let asked = Instant::now();
        let reuse = |ok: &dyn Fn(Instant) -> bool| {
            self.dates
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(started, _)| ok(*started))
                .map(|(_, d)| d.clone())
        };
        if let Some(d) = reuse(&|started| started.elapsed() < max_age) {
            return d;
        }
        let _one = self.dating.lock().await;
        if let Some(d) = reuse(&|started| started >= asked) {
            return d;
        }
        let started = Instant::now();
        let generation = self.generation.load(Ordering::SeqCst);
        let base = t.base_date.clone();
        let base = tokio::task::spawn_blocking(move || base.get());
        let miniserver = if reachable {
            let http = t.http.clone();
            let ask = tokio::task::spawn_blocking(move || http.value("jdev/sps/LoxAPPversion3"));
            Some(match tokio::time::timeout(DATE_TIMEOUT, ask).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err("no answer".to_string()),
            })
        } else {
            None
        };
        let d = Dates {
            miniserver,
            base: base.await.ok().flatten(),
        };
        if self.generation.load(Ordering::SeqCst) == generation {
            *self.dates.lock().unwrap() = Some((started, d.clone()));
        }
        d
    }

    /// The check `v`, with the config dates (`configDate`), and a warning
    /// when the Miniserver's config changed after the base.
    async fn with_dates(&self, v: Arc<Value>, max_age: Duration) -> Arc<Value> {
        let Ok(t) = &self.target else {
            return v;
        };
        if v["available"] != true {
            return v;
        }
        let d = self.dates(t, max_age, v["reachable"] == true).await;
        let (date, warning) = date_report(&d, v["identity"] == "wrong");
        let mut v = (*v).clone();
        v["configDate"] = date;
        if let (Some(w), Some(ws)) = (warning, v["warnings"].as_array_mut()) {
            ws.push(w.into());
        }
        Arc::new(v)
    }

    /// For the status chip: a check at most a minute old.
    pub async fn chip(&self) -> Arc<Value> {
        let v = self.status(MAX_AGE).await;
        self.with_dates(v, DATE_AGE).await
    }

    /// The page's regular poll: the config dates at most half a minute old,
    /// the rest of the check at most five minutes.
    pub async fn poll(&self) -> Arc<Value> {
        let v = self.status(POLL_AGE).await;
        self.with_dates(v, DATE_AGE).await
    }

    /// "Check again": a new check, unless one ran just now.
    pub async fn recheck(&self) -> Arc<Value> {
        let v = self.status(MIN_AGE).await;
        self.with_dates(v, MIN_AGE).await
    }

    /// After Pull, Apply or Sync, which may have changed the base and the
    /// Miniserver's config: drop what was checked, and keep a check that is
    /// still under way from storing what it read before.
    pub fn forget(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = None;
        *self.dates.lock().unwrap() = None;
        if let Ok(t) = &self.target {
            t.base_date.forget();
        }
    }

    /// Before Pull, Apply and Sync: check (again, unless a check started
    /// in the last few seconds), and say why not when the Miniserver cannot
    /// be reached the way lox needs it for `what`. Without a connection to
    /// check (`--no-live`, no lox configuration), lox finds out.
    pub async fn preflight(&self, what: Action) -> Result<(), String> {
        let v = self.status(PREFLIGHT_AGE).await;
        if what == Action::Operate {
            return operate_why(&v).map_or(Ok(()), Err);
        }
        match v["blocked"][what.key()].as_str() {
            Some(why) => Err(why.to_string()),
            None => Ok(()),
        }
    }
}

/// Why a command to an input must not be sent, from a check: unless the
/// Miniserver at the address is confirmed to have the config's serial. A
/// command to another device that DHCP gave the address to, or to the wrong
/// context, would switch something in the wrong house. Fails closed:
/// without a check that confirmed the serial, nothing is sent.
fn operate_why(v: &Value) -> Option<String> {
    if v["available"] != true {
        return Some(format!(
            "vislox cannot check the Miniserver ({}), so it cannot confirm that it is the \
             config's",
            v["reason"].as_str().unwrap_or("no status")
        ));
    }
    match v["identity"].as_str() {
        Some("confirmed") => None,
        // The reason that names both serials, as Pull, Apply and Sync give it.
        Some("wrong") => Some(
            v["blocked"]["apply"]
                .as_str()
                .unwrap_or("this is not the Miniserver of this config")
                .to_string(),
        ),
        Some("unchecked") => Some(
            "vislox does not know the serial this config belongs to, so it cannot confirm that \
             the Miniserver at lox's address is the right one"
                .into(),
        ),
        _ => Some(match v["movedTo"].as_str() {
            Some(ip) => format!(
                "the Miniserver at lox's address did not report its serial, and the config's \
                 Miniserver answers at {ip}"
            ),
            None => "the Miniserver at lox's address did not report its serial, so vislox \
                     cannot confirm that it is the config's"
                .into(),
        }),
    }
}

/// `https://host:port/` → (address, port); the port defaults by scheme, and
/// an IPv6 address loses its brackets (`[::1]` → `::1`) so it can be
/// connected to.
fn split_host(host: &str) -> (String, u16) {
    let (rest, default) = if let Some(r) = host.strip_prefix("http://") {
        (r, 80)
    } else {
        (host.strip_prefix("https://").unwrap_or(host), 443)
    };
    let rest = rest.split('/').next().unwrap_or(rest);
    if let Some(v6) = rest.strip_prefix('[')
        && let Some((addr, after)) = v6.split_once(']')
    {
        let port = after
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default);
        return (addr.to_string(), port);
    }
    match rest.rsplit_once(':') {
        // A bare IPv6 address has more than one colon: no port.
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(default)),
        _ => (rest.to_string(), default),
    }
}

/// The `value` of an `{"LL": …}` reply to `jdev/cfg/apiKey`: an object in
/// single quotes.
fn parse_api_key(body: &str) -> Result<ApiKey, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "not a Miniserver reply".to_string())?;
    let ll = &v["LL"];
    let code = [&ll["Code"], &ll["code"]]
        .into_iter()
        .find_map(|c| match c {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_default();
    if code != "200" {
        return Err(format!("the Miniserver answered {code}"));
    }
    let value = ll["value"].as_str().ok_or("no value in the reply")?;
    let k: Value = serde_json::from_str(&value.replace('\'', "\""))
        .map_err(|e| format!("cannot read the reply: {e}"))?;
    let flag = |name: &str| match &k[name] {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => Some(s == "true" || s == "1"),
        _ => None,
    };
    Ok(ApiKey {
        // Compared with the config's serial, so normalized the same way.
        serial: k["snr"].as_str().map(discover::norm_serial),
        version: k["version"].as_str().map(String::from),
        https_status: k["httpsStatus"].as_i64(),
        local: flag("local"),
        event_slots: flag("hasEventSlots"),
    })
}

/// The Loxone Config version a config was saved with: the document's
/// `ConfigVersion`, two digits per part (`17021021` = 17.2.10.21).
fn config_version(xml: &[u8]) -> Option<String> {
    let head = &xml[..xml.len().min(64 * 1024)];
    let text = String::from_utf8_lossy(head);
    let doc = text.find("Type=\"Document\"")?;
    let tag_end = text[doc..].find('>')? + doc;
    let tag = &text[doc..tag_end];
    let at = tag.find("ConfigVersion=\"")? + "ConfigVersion=\"".len();
    let digits: String = tag[at..].chars().take_while(char::is_ascii_digit).collect();
    if digits.len() != 8 {
        return None;
    }
    let part = |i: usize| digits[i..i + 2].parse::<u32>().ok();
    Some(format!(
        "{}.{}.{}.{}",
        part(0)?,
        part(2)?,
        part(4)?,
        part(6)?
    ))
}

/// Major, minor and patch: a different build number is the same release.
fn same_release(a: &str, b: &str) -> bool {
    let three = |s: &str| s.split('.').take(3).collect::<Vec<_>>().join(".");
    three(a) == three(b)
}

async fn connect(addr: &str, port: u16) -> Result<TcpStream, String> {
    match tokio::time::timeout(TIMEOUT, TcpStream::connect((addr, port))).await {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("no answer within {} s", TIMEOUT.as_secs())),
    }
}

/// Connect to FTP and read its greeting (`220 …`), then say goodbye.
async fn ftp_greeting(addr: &str, port: u16) -> Result<String, String> {
    let mut s = connect(addr, port).await?;
    let mut buf = vec![0u8; 512];
    let n = tokio::time::timeout(TIMEOUT, s.read(&mut buf))
        .await
        .map_err(|_| format!("connected, but no greeting within {} s", TIMEOUT.as_secs()))?
        .map_err(|e| e.to_string())?;
    let _ = s.write_all(b"QUIT\r\n").await;
    let first = String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if first.starts_with("220") {
        Ok(first)
    } else if first.is_empty() {
        Err("connected, but the server closed the connection".into())
    } else {
        Err(format!("connected, but the server said: {first}"))
    }
}

async fn api_key(conn: &Connection) -> Result<ApiKey, String> {
    // As lox's own client: no certificate check unless asked for, and no
    // redirect to another host (a Gen 2 Miniserver sends local HTTP to its
    // *.dyndns.loxonecloud.com name).
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .danger_accept_invalid_certs(!conn.verify_ssl)
        .redirect(loxone_client::LoxClient::same_origin_redirect_policy(
            &conn.host,
        ))
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .get(conn.url("jdev/cfg/apiKey"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    parse_api_key(&body)
}

fn read_version(path: &PathBuf) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let xml = crate::source::program_xml(&bytes).ok()?;
    config_version(&xml)
}

async fn probe(t: &Target, expected: Option<&Expected>, discovery: &discover::Cache) -> Value {
    let (web, ftp) = tokio::join!(
        async {
            match connect(&t.addr, t.http_port).await {
                Ok(_) => (Ok(()), Some(api_key(&t.conn).await)),
                Err(e) => (Err(e), None),
            }
        },
        ftp_greeting(&t.addr, t.ftp_port),
    );
    let (web, key) = web;
    let (conn, current, proposed) = (t.conn.clone(), t.current.clone(), t.proposed.clone());
    let (token, cfg_current, cfg_pending) = tokio::task::spawn_blocking(move || {
        (
            TokenStore::load_for(&conn).map(|ts| ts.valid_until as i64),
            read_version(&current),
            proposed.as_ref().and_then(read_version),
        )
    })
    .await
    .unwrap_or_default();
    // When the address does not answer, or another Miniserver answers
    // there, look on the LAN: the right one may have a new address (DHCP),
    // and its discovery reply names it.
    let (search, addr_ip) = if look_on_lan(expected, &web, &key) {
        let ip = tokio::net::lookup_host((t.addr.as_str(), t.http_port))
            .await
            .ok()
            .and_then(|mut a| a.find(|a| a.is_ipv4()))
            .map(|a| a.ip().to_string());
        (Some(discovery.get().await), ip)
    } else {
        (None, None)
    };
    report(Report {
        host: &t.conn.host,
        addr: &t.addr,
        http_port: t.http_port,
        ftp_port: t.ftp_port,
        web,
        key,
        ftp,
        token,
        now: chrono::Utc::now().timestamp(),
        cfg_current,
        cfg_pending,
        expected,
        discovered: search.as_ref().map(|s| s.found.clone()),
        discovery_note: search.as_ref().and_then(|s| s.note.clone()),
        addr_ip,
    })
}

/// Whether to send the discovery broadcast: only with a serial to look for,
/// and only when the address does not answer or another serial answers.
/// An `apiKey` reply that cannot be read says nothing about the serial.
fn look_on_lan(
    expected: Option<&Expected>,
    web: &Result<(), String>,
    key: &Option<Result<ApiKey, String>>,
) -> bool {
    expected.is_some_and(|e| match (web, key) {
        (Err(_), _) => true,
        (Ok(()), Some(Ok(k))) => k.serial.as_ref().is_some_and(|s| *s != e.serial),
        _ => false,
    })
}

/// The lox command that points the context at `ip` (else at the right
/// address), keeping the scheme and port of lox's `host` now.
fn fix_host(e: &Expected, host: &str, ip: Option<&str>) -> String {
    let ctx = e
        .ctx
        .as_deref()
        .map(|c| format!("--ctx {c} "))
        .unwrap_or_default();
    let (scheme, default) = if host.starts_with("http://") {
        ("http", 80)
    } else {
        ("https", 443)
    };
    let (_, port) = split_host(host);
    let port = if port == default {
        String::new()
    } else {
        format!(":{port}")
    };
    format!(
        "`lox {ctx}setup set --host {scheme}://{}{port}`",
        ip.unwrap_or("<address>")
    )
}

struct Report<'a> {
    host: &'a str,
    addr: &'a str,
    http_port: u16,
    ftp_port: u16,
    /// Whether the web port takes a connection.
    web: Result<(), String>,
    /// `jdev/cfg/apiKey`, when the web port took the connection.
    key: Option<Result<ApiKey, String>>,
    ftp: Result<String, String>,
    /// When lox's token expires (Unix seconds).
    token: Option<i64>,
    now: i64,
    cfg_current: Option<String>,
    cfg_pending: Option<String>,
    expected: Option<&'a Expected>,
    /// The LAN's discovery replies, when vislox looked.
    discovered: Option<Vec<discover::Reply>>,
    /// Why the discovery may have missed replies.
    discovery_note: Option<String>,
    /// The address as an IPv4 address, to find its discovery reply.
    addr_ip: Option<String>,
}

/// The status as the UI shows it: what was checked, what blocks Pull, Apply
/// and Sync (each only by what it needs: FTP, and for Apply the web port),
/// and what only deserves a warning.
/// `configDate` for the chip, and the warning when the Miniserver's config
/// changed after the base. The dates are the Miniserver's local time
/// (`YYYY-MM-DD HH:MM:SS`), so they compare as text. Another Miniserver's
/// date (`wrong`) says nothing about this base, and a failed request only
/// says that it cannot tell.
fn date_report(d: &Dates, wrong: bool) -> (Value, Option<String>) {
    let is_date = |s: &str| {
        s.len() == 19
            && s.bytes().enumerate().all(|(i, c)| match i {
                4 | 7 => c == b'-',
                10 => c == b' ',
                13 | 16 => c == b':',
                _ => c.is_ascii_digit(),
            })
    };
    let (miniserver, error) = match (&d.miniserver, wrong) {
        (_, true) | (None, _) => (None, None),
        (Some(Ok(m)), _) => (Some(m.as_str()), None),
        (Some(Err(e)), _) => (None, Some(e.as_str())),
    };
    let order = match (miniserver, d.base.as_deref()) {
        (Some(m), Some(b)) if is_date(m) && is_date(b) => Some(match m.cmp(b) {
            std::cmp::Ordering::Greater => "after",
            std::cmp::Ordering::Equal => "same",
            std::cmp::Ordering::Less => "before",
        }),
        _ => None,
    };
    let changed = order == Some("after");
    let warning = changed.then(|| {
        format!(
            "The Miniserver's config changed at {}, after the base vislox shows (taken at {}). \
             Pull or Sync from Miniserver before you change it.",
            miniserver.unwrap_or_default(),
            d.base.as_deref().unwrap_or_default()
        )
    });
    (
        json!({
            "miniserver": miniserver,
            "base": d.base,
            // `after`, `same` or `before`: the Miniserver's against the base's.
            "order": order,
            "changed": changed,
            "error": error,
        }),
        warning,
    )
}

fn report(r: Report) -> Value {
    let mut warnings: Vec<String> = Vec::new();
    let scheme = if r.host.starts_with("http://") {
        "HTTP"
    } else {
        "HTTPS"
    };
    let web_name = format!("{scheme} (port {})", r.http_port);
    let ftp_name = format!("FTP (port {})", r.ftp_port);
    let checks = json!([
        { "name": web_name, "ok": r.web.is_ok(),
          "detail": match &r.web { Ok(()) => "answers".to_string(), Err(e) => e.clone() } },
        { "name": ftp_name, "ok": r.ftp.is_ok(),
          "detail": match &r.ftp { Ok(g) => g.clone(), Err(e) => e.clone() } },
    ]);
    let no_ftp = r.ftp.as_ref().err().map(|e| {
        format!(
            "{ftp_name} is blocked between this host and the Miniserver ({e}). Pull, Apply and \
             Sync copy the config over FTP: allow TCP 21 and 20 to {}",
            r.addr
        )
    });
    let no_web = r.web.as_ref().err().map(|e| {
        format!(
            "the Miniserver does not answer at {} ({web_name}: {e}). Apply restarts its program \
             over {scheme}",
            r.host
        )
    });
    let key = match &r.key {
        Some(Ok(k)) => Some(k),
        Some(Err(e)) => {
            warnings.push(format!(
                "The Miniserver answers, but its status (jdev/cfg/apiKey) could not be read: {e}."
            ));
            None
        }
        None => None,
    };
    // Who answers at the address: `apiKey`, else its discovery reply.
    let at_addr = r
        .discovered
        .iter()
        .flatten()
        .find(|d| r.addr_ip.as_deref() == Some(d.ip.as_str()) || d.ip == r.addr);
    let serial = key
        .and_then(|k| k.serial.clone())
        .or_else(|| at_addr.map(|d| d.serial.clone()));
    let firmware = key
        .and_then(|k| k.version.clone())
        .or_else(|| at_addr.map(|d| d.firmware.clone()));
    // The right one elsewhere on the LAN.
    let elsewhere = r.expected.and_then(|e| {
        r.discovered
            .iter()
            .flatten()
            .find(|d| d.serial == e.serial && at_addr.is_none_or(|a| a.ip != d.ip))
    });
    let (identity, wrong, moved) = match (r.expected, &serial) {
        (None, _) => ("unchecked", None, None),
        (Some(e), Some(s)) if *s == e.serial => ("confirmed", None, None),
        (Some(e), Some(s)) => {
            let fix = match elsewhere {
                Some(d) => format!(
                    "{} answers at {} on the local network: point lox at it with {}",
                    e.serial,
                    d.ip,
                    fix_host(e, r.host, Some(&d.ip))
                ),
                None => format!(
                    "Check the address in lox's configuration ({})",
                    fix_host(e, r.host, None)
                ),
            };
            let why = format!(
                "this is not the Miniserver of this config: {} reports serial {s}, but the config \
                 belongs to {} ({}). {fix}",
                r.addr, e.serial, e.from
            );
            ("wrong", Some(why), None)
        }
        (Some(e), None) => {
            let moved = elsewhere.map(|d| {
                format!(
                    "the Miniserver does not answer at {}, but {} answers at {} on the local \
                     network. If its address changed, point lox at it: {}",
                    r.host,
                    e.serial,
                    d.ip,
                    fix_host(e, r.host, Some(&d.ip))
                )
            });
            ("unknown", None, moved)
        }
    };
    // Another Miniserver at the address blocks everything: lox would pull
    // from it, or deploy to it. A Miniserver that moved explains what
    // already fails.
    let reason = |own: Option<String>| match (&wrong, own) {
        (Some(w), _) => Some(w.clone()),
        (None, own) => own.map(|o| moved.clone().unwrap_or(o)),
    };
    let blocked = json!({
        "pull": reason(no_ftp.clone()),
        "sync": reason(no_ftp.clone()),
        "apply": reason(no_web.clone().or(no_ftp.clone())),
    });
    if let Some(k) = key {
        match k.https_status {
            Some(2) => warnings.push("The Miniserver's TLS certificate has expired.".into()),
            Some(1) if r.host.starts_with("http://") => warnings
                .push("The Miniserver offers TLS, but lox connects without it (http://).".into()),
            _ => {}
        }
        if k.local == Some(false) {
            warnings.push(
                "The Miniserver sees this host as remote, not local: it may refuse some requests."
                    .into(),
            );
        }
        if k.event_slots == Some(false) {
            warnings.push(
                "No live-update slot is free on the Miniserver (31 clients at most): live values may \
                 not connect."
                    .into(),
            );
        }
        if let Some(fw) = &k.version {
            for (what, v) in [
                ("The deployed config", &r.cfg_current),
                ("The pending config", &r.cfg_pending),
            ] {
                if let Some(v) = v
                    && !same_release(v, fw)
                {
                    warnings.push(format!(
                        "{what} was saved with Loxone Config {v}, but the Miniserver runs {fw}."
                    ));
                }
            }
        }
    }
    if let Some(until) = r.token
        && until <= r.now
    {
        warnings.push(
            "lox's token has expired: lox asks for a new one with the password at its next login."
                .into(),
        );
    }
    let tls = key.and_then(|k| k.https_status).map(|s| match s {
        0 => "none",
        1 => "on",
        2 => "expired",
        _ => "unknown",
    });
    json!({
        "available": true,
        "checked": r.now,
        "host": r.host,
        "reachable": r.web.is_ok(),
        "serial": serial,
        "firmware": firmware,
        "expected": r.expected.map(|e| json!({ "serial": e.serial, "from": e.from })),
        // `confirmed`, `wrong`, `unknown` (no serial seen), or `unchecked`
        // (no serial to compare with).
        "identity": identity,
        "movedTo": elsewhere.map(|d| d.ip.as_str()),
        "discovered": r.discovered,
        "discoveryNote": r.discovery_note,
        "tls": tls,
        "local": key.and_then(|k| k.local),
        "eventSlots": key.and_then(|k| k.event_slots),
        "tokenValidUntil": r.token,
        "config": { "deployed": r.cfg_current, "pending": r.cfg_pending },
        "checks": checks,
        "blocked": blocked,
        "warnings": warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"{"LL": { "control": "dev/cfg/apiKey", "value": "{'snr': '50:4F:94:A0:E4:54', 'version':'17.2.10.21', 'key':'3236', 'isInTrust':false, 'local':true,'address':'192.168.1.120','certTLD':'com', 'httpsStatus':1}", "Code": "200"}}"#;

    #[test]
    fn the_api_key_reply_is_read() {
        let k = parse_api_key(REAL).unwrap();
        assert_eq!(k.serial.as_deref(), Some("504F94A0E454"));
        assert_eq!(k.version.as_deref(), Some("17.2.10.21"));
        assert_eq!(k.https_status, Some(1));
        assert_eq!(k.local, Some(true));
        assert_eq!(k.event_slots, None);
        let slots = REAL.replace("'httpsStatus':1", "'httpsStatus':2, 'hasEventSlots':false");
        let k = parse_api_key(&slots).unwrap();
        assert_eq!((k.https_status, k.event_slots), (Some(2), Some(false)));
        assert!(parse_api_key("<html>").is_err());
        assert!(
            parse_api_key(r#"{"LL":{"value":"","Code":"401"}}"#)
                .unwrap_err()
                .contains("401")
        );
        // A numeric code is read too.
        let numeric = REAL.replace(r#""Code": "200""#, r#""Code": 200"#);
        assert_eq!(
            parse_api_key(&numeric).unwrap().serial.as_deref(),
            Some("504F94A0E454")
        );
    }

    #[test]
    fn hosts_split_into_address_and_port() {
        assert_eq!(
            split_host("https://192.168.1.120"),
            ("192.168.1.120".into(), 443)
        );
        assert_eq!(split_host("http://ms.local/"), ("ms.local".into(), 80));
        assert_eq!(split_host("https://ms:8443"), ("ms".into(), 8443));
        assert_eq!(split_host("127.0.0.1:9"), ("127.0.0.1".into(), 9));
        // IPv6: without brackets, so that it can be connected to.
        assert_eq!(split_host("https://[::1]:444"), ("::1".into(), 444));
        assert_eq!(split_host("http://[fe80::1]/"), ("fe80::1".into(), 80));
        assert_eq!(split_host("fe80::1"), ("fe80::1".into(), 443));
    }

    fn expect(serial: &str) -> Expected {
        Expected {
            serial: serial.into(),
            from: "the repository folder".into(),
            ctx: Some("home".into()),
        }
    }

    fn reply(ip: &str, serial: &str) -> discover::Reply {
        discover::parse_reply(&format!(
            "LoxLIVE: Loxone Miniserver {ip}:80 {serial} 17.2.10.21 Prog:2026-11-01 10:00:00 Type:0 HwId:A0000"
        ))
        .unwrap()
    }

    fn blocks(v: &Value) -> Vec<String> {
        ["pull", "apply", "sync"]
            .iter()
            .map(|w| v["blocked"][w].as_str().unwrap_or("-").to_string())
            .collect()
    }

    #[test]
    fn the_right_serial_is_confirmed() {
        let e = expect("504F94A0E454");
        let v = report(Report {
            expected: Some(&e),
            ..base()
        });
        assert_eq!(v["identity"], "confirmed");
        assert_eq!(blocks(&v), ["-", "-", "-"]);
        assert_eq!(v["expected"]["serial"], "504F94A0E454");
        assert_eq!(report(base())["identity"], "unchecked");
    }

    #[test]
    fn another_serial_at_the_address_blocks_every_action() {
        let e = expect("504F94000001");
        let v = report(Report {
            expected: Some(&e),
            // Even when FTP fails too: the wrong device is the reason to give.
            ftp: Err("refused".into()),
            ..base()
        });
        assert_eq!(v["identity"], "wrong");
        for why in blocks(&v) {
            assert!(
                why.starts_with("this is not the Miniserver of this config: 192.168.1.120 reports serial 504F94A0E454, but the config belongs to 504F94000001 (the repository folder)"),
                "{why}"
            );
            assert!(
                why.contains("`lox --ctx home setup set --host https://<address>`"),
                "{why}"
            );
        }
    }

    #[test]
    fn when_another_miniserver_has_the_address_the_right_one_is_looked_for() {
        // DHCP gave 192.168.1.120 to another Miniserver; ours moved to .130.
        let e = expect("504F94000001");
        let v = report(Report {
            host: "https://192.168.1.120:8443",
            expected: Some(&e),
            discovered: Some(vec![
                reply("192.168.1.120", "504F94A0E454"),
                reply("192.168.1.130", "504F94000001"),
            ]),
            addr_ip: Some("192.168.1.120".into()),
            ..base()
        });
        assert_eq!(v["identity"], "wrong");
        assert_eq!(v["movedTo"], "192.168.1.130");
        let why = v["blocked"]["sync"].as_str().unwrap();
        assert!(
            why.contains("504F94000001 answers at 192.168.1.130"),
            "{why}"
        );
        // lox's scheme and port stay.
        assert!(
            why.contains("`lox --ctx home setup set --host https://192.168.1.130:8443`"),
            "{why}"
        );
    }

    #[test]
    fn a_miniserver_with_a_new_address_is_found_on_the_lan() {
        let e = expect("504F94A0E454");
        let v = report(Report {
            web: Err("no answer within 4 s".into()),
            key: None,
            ftp: Err("no answer within 4 s".into()),
            expected: Some(&e),
            discovered: Some(vec![
                reply("192.168.1.9", "504F94111111"),
                reply("192.168.1.130", "504F94A0E454"),
            ]),
            addr_ip: Some("192.168.1.120".into()),
            ..base()
        });
        assert_eq!(v["identity"], "unknown");
        assert_eq!(v["movedTo"], "192.168.1.130");
        for why in blocks(&v) {
            assert!(
                why.contains("504F94A0E454 answers at 192.168.1.130"),
                "{why}"
            );
            assert!(
                why.contains("`lox --ctx home setup set --host https://192.168.1.130`"),
                "{why}"
            );
        }
        assert_eq!(v["discovered"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_moved_miniserver_explains_only_what_already_fails() {
        // The web port is down, FTP answers: only Apply is blocked.
        let e = expect("504F94A0E454");
        let v = report(Report {
            web: Err("refused".into()),
            key: None,
            expected: Some(&e),
            discovered: Some(vec![reply("192.168.1.130", "504F94A0E454")]),
            ..base()
        });
        let b = blocks(&v);
        assert_eq!((b[0].as_str(), b[2].as_str()), ("-", "-"));
        assert!(b[1].contains("answers at 192.168.1.130"), "{}", b[1]);
    }

    #[test]
    fn the_discovery_reply_at_the_address_still_names_it() {
        let e = expect("504F94A0E454");
        // HTTP is blocked, but the device at the address answers discovery.
        let v = report(Report {
            web: Err("refused".into()),
            key: None,
            expected: Some(&e),
            discovered: Some(vec![reply("192.168.1.120", "504F94111111")]),
            addr_ip: Some("192.168.1.120".into()),
            ..base()
        });
        assert_eq!(v["serial"], "504F94111111");
        assert_eq!(v["firmware"], "17.2.10.21");
        assert_eq!(v["identity"], "wrong");
        assert_eq!(v["movedTo"], Value::Null);
        // Nobody answers anywhere: unknown, and the plain reasons stay.
        let v = report(Report {
            web: Err("refused".into()),
            key: None,
            expected: Some(&e),
            discovered: Some(vec![]),
            ..base()
        });
        assert_eq!(v["identity"], "unknown");
        assert_eq!(blocks(&v)[0], "-");
        assert!(blocks(&v)[1].starts_with("the Miniserver does not answer"));
    }

    #[test]
    fn an_unreadable_api_key_is_not_a_wrong_miniserver() {
        let e = expect("504F94000001");
        let v = report(Report {
            key: Some(Err("cannot read the reply".into())),
            expected: Some(&e),
            ..base()
        });
        assert_eq!(v["identity"], "unknown");
        assert_eq!(blocks(&v), ["-", "-", "-"]);
    }

    #[test]
    fn the_fix_keeps_lox_s_scheme_and_port() {
        let e = expect("504F94000001");
        assert_eq!(
            fix_host(&e, "http://10.0.0.5", Some("10.0.0.9")),
            "`lox --ctx home setup set --host http://10.0.0.9`"
        );
        assert_eq!(
            fix_host(&e, "http://10.0.0.5:8080", None),
            "`lox --ctx home setup set --host http://<address>:8080`"
        );
        assert_eq!(
            fix_host(&e, "https://10.0.0.5", Some("10.0.0.9")),
            "`lox --ctx home setup set --host https://10.0.0.9`"
        );
    }

    #[test]
    fn any_serial_separator_matches() {
        // `apiKey` and the config may write the serial differently.
        for snr in ["50:4F:94:A0:E4:54", "50-4f-94-a0-e4-54", "504f94a0e454"] {
            let reply = REAL.replace("50:4F:94:A0:E4:54", snr);
            let e = expect(&discover::norm_serial("50-4F-94-A0-E4-54"));
            let v = report(Report {
                key: Some(parse_api_key(&reply)),
                expected: Some(&e),
                ..base()
            });
            assert_eq!(v["identity"], "confirmed", "{snr}");
            assert_eq!(blocks(&v), ["-", "-", "-"], "{snr}");
        }
    }

    #[test]
    fn the_lan_is_searched_only_for_a_serial_that_is_not_at_the_address() {
        let (right, other) = (expect("504F94A0E454"), expect("504F94000001"));
        let down: Result<(), String> = Err("no answer".into());
        let real = Some(parse_api_key(REAL));
        assert!(!look_on_lan(None, &down, &None), "nothing to look for");
        assert!(look_on_lan(Some(&right), &down, &None));
        assert!(!look_on_lan(Some(&right), &Ok(()), &real));
        assert!(look_on_lan(Some(&other), &Ok(()), &real));
        assert!(!look_on_lan(Some(&other), &Ok(()), &Some(Err("x".into()))));
    }

    #[tokio::test]
    async fn an_ipv6_host_can_be_connected_to() {
        // Where the machine has no IPv6 loopback, there is nothing to test.
        let Ok(l) = tokio::net::TcpListener::bind("[::1]:0").await else {
            return;
        };
        let port = l.local_addr().unwrap().port();
        let (addr, p) = split_host(&format!("http://[::1]:{port}"));
        assert_eq!(p, port);
        assert!(connect(&addr, p).await.is_ok());
    }

    #[test]
    fn the_config_version_is_read_from_the_document() {
        let xml = br#"<?xml version="1.0"?><ControlList><C Type="Document" V="17021021" Title="x" ConfigVersion="17021021" PaperFormat="2">"#;
        assert_eq!(config_version(xml).as_deref(), Some("17.2.10.21"));
        assert_eq!(
            config_version(&std::fs::read("tests/fixtures/house.Loxone").unwrap()).as_deref(),
            Some("17.1.7.27")
        );
        assert_eq!(config_version(b"<C Type=\"Document\">"), None);
        assert!(same_release("17.2.10.21", "17.2.10.3"));
        assert!(!same_release("17.1.7.27", "17.2.10.21"));
    }

    fn base() -> Report<'static> {
        Report {
            host: "https://192.168.1.120",
            addr: "192.168.1.120",
            http_port: 443,
            ftp_port: 21,
            web: Ok(()),
            key: Some(parse_api_key(REAL)),
            ftp: Ok("220 Welcome".into()),
            token: Some(2_000),
            now: 1_000,
            cfg_current: Some("17.2.10.21".into()),
            cfg_pending: None,
            expected: None,
            discovered: None,
            discovery_note: None,
            addr_ip: None,
        }
    }

    #[test]
    fn a_healthy_miniserver_blocks_nothing() {
        let v = report(base());
        assert_eq!(
            v["blocked"],
            json!({ "pull": null, "apply": null, "sync": null })
        );
        assert_eq!(v["warnings"], json!([]));
        assert_eq!(v["serial"], "504F94A0E454");
        assert_eq!(v["tls"], "on");
        assert_eq!(v["checks"][1]["ok"], true);
    }

    #[test]
    fn blocked_ftp_is_named_as_such() {
        let v = report(Report {
            ftp: Err("Connection refused (os error 111)".into()),
            ..base()
        });
        for what in ["pull", "apply", "sync"] {
            let why = v["blocked"][what].as_str().unwrap();
            assert!(
                why.starts_with("FTP (port 21) is blocked between this host and the Miniserver"),
                "{why}"
            );
            assert!(why.contains("192.168.1.120"), "{why}");
        }
        // Live values still work: the Miniserver itself answers.
        assert_eq!(v["reachable"], true);
    }

    #[test]
    fn a_closed_web_port_blocks_only_apply() {
        let v = report(Report {
            web: Err("Connection refused".into()),
            key: None,
            ..base()
        });
        assert!(
            v["blocked"]["apply"]
                .as_str()
                .unwrap()
                .starts_with("the Miniserver does not answer at https://192.168.1.120")
        );
        // Pull and Sync only need FTP.
        assert_eq!(v["blocked"]["pull"], Value::Null);
        assert_eq!(v["blocked"]["sync"], Value::Null);
        assert_eq!(v["reachable"], false);
        assert_eq!(v["serial"], Value::Null);
    }

    #[test]
    fn an_unreadable_status_reply_only_warns() {
        // An apostrophe in a value breaks the single-quote reading; a reply
        // that cannot be read must not stop anything.
        let odd = REAL.replace("'certTLD':'com'", "'name':'Olek's house'");
        for key in [parse_api_key(&odd), Err("HTTP 404 Not Found".to_string())] {
            assert!(key.is_err());
            let v = report(Report {
                key: Some(key),
                ..base()
            });
            assert_eq!(
                v["blocked"],
                json!({ "pull": null, "apply": null, "sync": null })
            );
            assert_eq!(v["reachable"], true);
            assert!(
                v["warnings"][0]
                    .as_str()
                    .unwrap()
                    .contains("jdev/cfg/apiKey) could not be read"),
                "{}",
                v["warnings"]
            );
        }
    }

    #[test]
    fn warnings_for_tls_slots_versions_and_the_token() {
        let reply = REAL
            .replace("'httpsStatus':1", "'httpsStatus':2, 'hasEventSlots':false")
            .replace("'local':true", "'local':false");
        let v = report(Report {
            key: Some(parse_api_key(&reply)),
            token: Some(500),
            cfg_current: Some("17.2.10.3".into()),
            cfg_pending: Some("17.1.7.27".into()),
            ..base()
        });
        let w: Vec<&str> = v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap())
            .collect();
        assert_eq!(
            v["blocked"],
            json!({ "pull": null, "apply": null, "sync": null }),
            "warnings never block"
        );
        assert!(w.iter().any(|w| w.contains("certificate has expired")));
        assert!(w.iter().any(|w| w.contains("remote")));
        assert!(w.iter().any(|w| w.contains("live-update slot")));
        assert!(w.iter().any(|w| w.contains("token has expired")));
        assert!(
            w.iter()
                .any(|w| w.starts_with("The pending config was saved with Loxone Config 17.1.7.27"))
        );
        assert!(!w.iter().any(|w| w.starts_with("The deployed config")));
    }

    #[test]
    fn inputs_are_operated_only_on_the_confirmed_miniserver() {
        assert_eq!(
            operate_why(&json!({ "available": true, "identity": "confirmed" })),
            None
        );
        let wrong = json!({ "available": true, "identity": "wrong",
            "blocked": { "apply": "this is not the Miniserver of this config: 1.2.3.4 reports serial B" } });
        assert!(operate_why(&wrong).unwrap().contains("reports serial B"));
        for id in ["unknown", "unchecked"] {
            let why = operate_why(&json!({ "available": true, "identity": id })).unwrap();
            assert!(why.contains("confirm"), "{why}");
        }
        let moved = json!({ "available": true, "identity": "unknown", "movedTo": "10.0.0.9" });
        assert!(operate_why(&moved).unwrap().contains("10.0.0.9"));
        // Fails closed: no check, no confirmed serial, no command.
        let why = operate_why(&json!({ "available": false, "reason": "turned off" })).unwrap();
        assert!(
            why.contains("turned off") && why.contains("confirm"),
            "{why}"
        );
        assert!(operate_why(&json!({})).is_some());
        assert!(
            operate_why(&json!({ "available": true })).is_some(),
            "no identity"
        );
    }

    #[tokio::test]
    async fn without_a_connection_no_input_is_operated() {
        let ms = Miniserver::new(Err("turned off".into()), PathBuf::new(), None, None);
        assert!(ms.preflight(Action::Operate).await.is_err());
    }

    #[tokio::test]
    async fn without_a_connection_nothing_is_checked() {
        let ms = Miniserver::new(Err("turned off".into()), PathBuf::new(), None, None);
        assert_eq!(ms.chip().await["available"], false);
        assert!(ms.preflight(Action::Apply).await.is_ok());
    }

    /// A stand-in Miniserver on the loopback: a web port that closes every
    /// connection, and an FTP server that greets after `delay` and counts.
    async fn stand_in(delay: Duration) -> (Miniserver, Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let web = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ftp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (web_port, ftp_port) = (
            web.local_addr().unwrap().port(),
            ftp.local_addr().unwrap().port(),
        );
        tokio::spawn(async move {
            while let Ok((s, _)) = web.accept().await {
                drop(s);
            }
        });
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = ftp.accept().await {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = s.write_all(b"220 hi\r\n").await;
                });
            }
        });
        let conn = Connection::new(format!("http://127.0.0.1:{web_port}"), "u", "p");
        let ms = Miniserver::with_target(Ok(Target {
            http: Arc::new(Http::new(conn.clone())),
            conn,
            addr: "127.0.0.1".into(),
            http_port: web_port,
            ftp_port,
            current: PathBuf::from("/nonexistent"),
            proposed: None,
            base_date: Arc::new(BaseDate::new(PathBuf::from("/nonexistent"), None)),
        }));
        (ms, count)
    }

    #[tokio::test]
    async fn a_new_check_does_not_take_one_that_started_before_it_was_asked_for() {
        use std::sync::atomic::Ordering;
        let (ms, count) = stand_in(Duration::from_millis(300)).await;
        let ms = Arc::new(ms);
        let first = tokio::spawn({
            let ms = ms.clone();
            async move { ms.status(Duration::ZERO).await }
        });
        // The first check is under way (FTP has not greeted yet)…
        while count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // …so a check asked for now must run on its own.
        let v = ms.status(Duration::ZERO).await;
        first.await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        // The web port took the connection; its status reply is unreadable,
        // which only warns.
        assert_eq!(v["reachable"], true);
        assert_eq!(v["blocked"]["apply"], Value::Null);
        // A recent check is reused by the chip and by the preflight.
        ms.chip().await;
        assert!(ms.preflight(Action::Pull).await.is_ok());
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_check_that_started_before_forget_is_not_kept() {
        use std::sync::atomic::Ordering;
        let (ms, count) = stand_in(Duration::from_millis(300)).await;
        let ms = Arc::new(ms);
        let first = tokio::spawn({
            let ms = ms.clone();
            async move { ms.status(Duration::ZERO).await }
        });
        while count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // A Pull finishes while the check is under way: what it read may be
        // from before the Pull, so it must not be reused afterwards.
        ms.forget();
        first.await.unwrap();
        assert!(ms.last.lock().unwrap().is_none());
        // A check that starts afterwards is kept.
        ms.status(Duration::ZERO).await;
        assert!(ms.last.lock().unwrap().is_some());
    }

    fn dates(ms: Option<Result<&str, &str>>, base: Option<&str>) -> Dates {
        Dates {
            miniserver: ms.map(|r| r.map(String::from).map_err(String::from)),
            base: base.map(String::from),
        }
    }

    #[test]
    fn only_a_config_changed_after_the_base_is_a_warning() {
        let base = Some("2026-11-06 00:23:31");
        // Same date: nothing to say.
        let (v, w) = date_report(&dates(Some(Ok("2026-11-06 00:23:31")), base), false);
        assert_eq!(
            (v["order"].as_str(), v["changed"].as_bool(), w),
            (Some("same"), Some(false), None)
        );
        // Saved on the Miniserver since: a warning.
        let (v, w) = date_report(&dates(Some(Ok("2026-11-06 14:32:10")), base), false);
        assert_eq!(
            (v["order"].as_str(), v["changed"].as_bool()),
            (Some("after"), Some(true))
        );
        let w = w.unwrap();
        assert!(w.starts_with("The Miniserver's config changed at 2026-11-06 14:32:10, after the base vislox shows (taken at 2026-11-06 00:23:31)"), "{w}");
        // The Miniserver runs an older config than the base (restored): not
        // "changed after the base".
        let (v, w) = date_report(&dates(Some(Ok("2026-10-31 20:00:00")), base), false);
        assert_eq!(
            (v["order"].as_str(), v["changed"].as_bool(), w),
            (Some("before"), Some(false), None)
        );
        // No date for the base, or one that is not a date: nothing compared.
        let (v, w) = date_report(&dates(Some(Ok("2026-11-06 14:32:10")), None), false);
        assert_eq!((v["order"].clone(), w), (Value::Null, None));
        let (v, _) = date_report(&dates(Some(Ok("soon")), base), false);
        assert_eq!(v["order"], Value::Null);
        // Another Miniserver's date says nothing about this base.
        let (v, w) = date_report(&dates(Some(Ok("2026-11-06 14:32:10")), base), true);
        assert_eq!(
            (v["miniserver"].clone(), v["changed"].as_bool(), w),
            (Value::Null, Some(false), None)
        );
        // A failed request only says that it cannot tell: no warning.
        let (v, w) = date_report(&dates(Some(Err("HTTP 401")), base), false);
        assert_eq!(
            (v["error"].as_str(), v["changed"].as_bool(), w),
            (Some("HTTP 401"), Some(false), None)
        );
    }
}
