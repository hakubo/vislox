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
//!   vislox requests no token of its own).
//!
//! What the token may do (`tokenRights`) and whether the password is weak
//! (`unsecurePass`) come only with a login, which lox does and does not keep,
//! so they are not shown.

use loxone_client::Connection;
use loxone_client::token::TokenStore;
use serde_json::{Value, json};
use std::path::PathBuf;
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

/// What needs the Miniserver, and so a preflight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Pull,
    Apply,
    Sync,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Pull => "pull",
            Action::Apply => "apply",
            Action::Sync => "sync",
        }
    }
}

pub struct Miniserver {
    target: Result<Target, String>,
    /// The last check: when it started, and what it found.
    last: Mutex<Option<(Instant, Arc<Value>)>>,
    probing: tokio::sync::Mutex<()>,
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
                current,
                proposed,
            }
        });
        Miniserver {
            target,
            last: Mutex::new(None),
            probing: tokio::sync::Mutex::new(()),
        }
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
        let v = Arc::new(probe(t).await);
        *self.last.lock().unwrap() = Some((started, v.clone()));
        v
    }

    /// For the status chip: a check at most a minute old.
    pub async fn chip(&self) -> Arc<Value> {
        self.status(MAX_AGE).await
    }

    /// "Check again": a new check, unless one ran just now.
    pub async fn recheck(&self) -> Arc<Value> {
        self.status(MIN_AGE).await
    }

    /// Before Pull, Apply and Sync: check (again, unless a check started
    /// in the last few seconds), and say why not when the Miniserver cannot
    /// be reached the way lox needs it for `what`. Without a connection to
    /// check (`--no-live`, no lox configuration), lox finds out.
    pub async fn preflight(&self, what: Action) -> Result<(), String> {
        let v = self.status(PREFLIGHT_AGE).await;
        match v["blocked"][what.key()].as_str() {
            Some(why) => Err(why.to_string()),
            None => Ok(()),
        }
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
        serial: k["snr"].as_str().map(|s| s.replace(':', "").to_uppercase()),
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

async fn probe(t: &Target) -> Value {
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
    })
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
}

/// The status as the UI shows it: what was checked, what blocks Pull, Apply
/// and Sync (each only by what it needs: FTP, and for Apply the web port),
/// and what only deserves a warning.
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
    let blocked = json!({
        "pull": no_ftp,
        "sync": no_ftp,
        "apply": no_web.clone().or(no_ftp.clone()),
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
        "serial": key.and_then(|k| k.serial.clone()),
        "firmware": key.and_then(|k| k.version.clone()),
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

    #[tokio::test]
    async fn without_a_connection_nothing_is_checked() {
        let ms = Miniserver::new(Err("turned off".into()), PathBuf::new(), None);
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
        let ms = Miniserver {
            target: Ok(Target {
                conn: Connection::new(format!("http://127.0.0.1:{web_port}"), "u", "p"),
                addr: "127.0.0.1".into(),
                http_port: web_port,
                ftp_port,
                current: PathBuf::from("/nonexistent"),
                proposed: None,
            }),
            last: Mutex::new(None),
            probing: tokio::sync::Mutex::new(()),
        };
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
}
