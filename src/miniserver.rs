//! The Miniserver's status, and a preflight before Pull, Apply and Sync.
//!
//! Everything here only reads, and nothing logs in:
//! - a TCP connect to the HTTP(S) port lox talks to (443, or the port in
//!   lox's `host`),
//! - `jdev/cfg/apiKey`, which the Miniserver answers without a login: serial,
//!   firmware version, TLS state (`httpsStatus`), whether it sees this host
//!   as local, and (on some versions) whether a live-update slot is free,
//! - a TCP connect to FTP (port 21) and its greeting. lox does Pull, Apply
//!   and Sync over FTP, and live values only need the HTTP port, so a
//!   firewall can leave live values working while Pull and Apply fail,
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
/// A check this old is redone for the status chip.
const MAX_AGE: Duration = Duration::from_secs(60);

pub struct Miniserver {
    target: Result<Target, String>,
    last: Mutex<Option<(Instant, Arc<Value>)>>,
    probing: tokio::sync::Mutex<()>,
}

struct Target {
    conn: Connection,
    /// The address without scheme and port.
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

    /// The status, checked again when older than `max_age`.
    pub async fn status(&self, max_age: Duration) -> Arc<Value> {
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
                .filter(|(at, _)| ok(*at))
                .map(|(_, v)| v.clone())
        };
        if let Some(v) = reuse(&|at| at.elapsed() < max_age) {
            return v;
        }
        // One check at a time: whoever waited gets the check that ran
        // meanwhile, which started after it asked.
        let _one = self.probing.lock().await;
        if let Some(v) = reuse(&|at| at >= asked) {
            return v;
        }
        let v = Arc::new(probe(t).await);
        *self.last.lock().unwrap() = Some((Instant::now(), v.clone()));
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

    /// Before Pull, Apply and Sync: check again, and say why not when the
    /// Miniserver cannot be reached the way lox needs it. Without a
    /// connection to check (`--no-live`, no lox configuration), lox finds out.
    pub async fn preflight(&self) -> Result<(), String> {
        let v = self.status(Duration::ZERO).await;
        match v["blocked"].as_str() {
            Some(why) => Err(why.to_string()),
            None => Ok(()),
        }
    }
}

/// `https://host:port/` → (`host`, port); the port defaults by scheme.
fn split_host(host: &str) -> (String, u16) {
    let (rest, default) = if let Some(r) = host.strip_prefix("http://") {
        (r, 80)
    } else {
        (host.strip_prefix("https://").unwrap_or(host), 443)
    };
    let rest = rest.split('/').next().unwrap_or(rest);
    match rest.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.ends_with(']') => {
            (h.to_string(), p.parse().unwrap_or(default))
        }
        _ => (rest.to_string(), default),
    }
}

/// The `value` of an `{"LL": …}` reply to `jdev/cfg/apiKey`: an object in
/// single quotes.
fn parse_api_key(body: &str) -> Result<ApiKey, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "not a Miniserver reply".to_string())?;
    let ll = &v["LL"];
    let code = ll["Code"]
        .as_str()
        .map(String::from)
        .or_else(|| ll["code"].as_i64().map(|c| c.to_string()))
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
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .danger_accept_invalid_certs(!conn.verify_ssl)
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
            connect(&t.addr, t.http_port).await?;
            api_key(&t.conn).await
        },
        ftp_greeting(&t.addr, t.ftp_port),
    );
    let token = TokenStore::load_for(&t.conn).map(|ts| ts.valid_until as i64);
    let (current, proposed) = (t.current.clone(), t.proposed.clone());
    let (cfg_current, cfg_pending) = tokio::task::spawn_blocking(move || {
        (
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
    web: Result<ApiKey, String>,
    ftp: Result<String, String>,
    /// When lox's token expires (Unix seconds).
    token: Option<i64>,
    now: i64,
    cfg_current: Option<String>,
    cfg_pending: Option<String>,
}

/// The status as the UI shows it: what was checked, what blocks Pull, Apply
/// and Sync, and what only deserves a warning.
fn report(r: Report) -> Value {
    let mut warnings: Vec<String> = Vec::new();
    let mut blocked: Option<String> = None;
    let scheme = if r.host.starts_with("http://") {
        "HTTP"
    } else {
        "HTTPS"
    };
    let web_name = format!("{scheme} (port {})", r.http_port);
    let ftp_name = format!("FTP (port {})", r.ftp_port);
    let checks = json!([
        { "name": web_name, "ok": r.web.is_ok(),
          "detail": match &r.web { Ok(_) => "answers".to_string(), Err(e) => e.clone() } },
        { "name": ftp_name, "ok": r.ftp.is_ok(),
          "detail": match &r.ftp { Ok(g) => g.clone(), Err(e) => e.clone() } },
    ]);
    let key = r.web.as_ref().ok();
    if let Err(e) = &r.web {
        blocked = Some(format!(
            "the Miniserver does not answer at {} ({}: {e})",
            r.host, web_name
        ));
    }
    if let Err(e) = &r.ftp {
        let why = format!(
            "{ftp_name} is blocked between this host and the Miniserver ({e}). Pull, Apply and \
             Sync copy the config over FTP: allow TCP 21 and 20 to {}",
            r.addr
        );
        // The FTP reason is the useful one when only FTP fails.
        if blocked.is_none() {
            blocked = Some(why);
        }
    }
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
        "reachable": key.is_some(),
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
        assert_eq!(split_host("https://[::1]:444"), ("[::1]".into(), 444));
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
            web: parse_api_key(REAL),
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
        assert_eq!(v["blocked"], Value::Null);
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
        let why = v["blocked"].as_str().unwrap();
        assert!(
            why.starts_with("FTP (port 21) is blocked between this host and the Miniserver"),
            "{why}"
        );
        assert!(why.contains("192.168.1.120"), "{why}");
        // Live values still work: the Miniserver itself answers.
        assert_eq!(v["reachable"], true);
    }

    #[test]
    fn an_unreachable_miniserver_is_named_first() {
        let v = report(Report {
            web: Err("no answer within 4 s".into()),
            ftp: Err("no answer within 4 s".into()),
            ..base()
        });
        assert!(
            v["blocked"]
                .as_str()
                .unwrap()
                .starts_with("the Miniserver does not answer at https://192.168.1.120")
        );
        assert_eq!(v["serial"], Value::Null);
    }

    #[test]
    fn warnings_for_tls_slots_versions_and_the_token() {
        let reply = REAL
            .replace("'httpsStatus':1", "'httpsStatus':2, 'hasEventSlots':false")
            .replace("'local':true", "'local':false");
        let v = report(Report {
            web: parse_api_key(&reply),
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
        assert_eq!(v["blocked"], Value::Null, "warnings never block");
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
        assert!(ms.preflight().await.is_ok());
    }
}
