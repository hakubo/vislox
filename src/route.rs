//! How vislox reaches the Miniserver: the route.
//!
//! - **CloudDNS** (verified, local): a Gen 2 Miniserver with the CloudDNS
//!   certificate (`*.{serial}.dyndns.loxonecloud.com`, from Let's Encrypt) is
//!   reachable locally under `{ip-with-dashes}.{serial}.dyndns.loxonecloud.com`,
//!   which Loxone's DNS answers with the local address itself. When lox's
//!   host is an `https://` LAN IPv4 address and the serial is known, vislox
//!   connects under that name and checks the certificate (`tls.rs`).
//! - **Local** (lox's host as it is): when the name does not resolve (a
//!   router with DNS rebinding protection), the certificate does not check
//!   out (a custom certificate, an expired one, another serial), or there is
//!   no serial to build the name from. The certificate is then not checked,
//!   and the status chip says so and why.
//! - **Remote Connect** (verified): when lox's context names only a serial,
//!   when `--route remote`, or (`auto`) when lox's address does not answer
//!   and the serial is known, vislox asks Loxone's Connect Service
//!   (`connect.loxonecloud.com/getip?snr=…`, firmware 17.0 and later) for the
//!   relay URL and connects to it. The relay carries HTTPS and WSS only, not
//!   FTP, so Pull, Apply and Sync are refused on this route
//!   (`miniserver.rs`).
//!
//! The route is chosen in the background after start ([`Handle::spawn`]);
//! until then, and whenever choosing fails, vislox uses lox's host. It is
//! checked again every minute while lox's address does not answer (a
//! Miniserver that is still booting, a laptop back home), and when a
//! connection fails ([`Handle::nudge`]). A working local route only gives way
//! to Remote Connect when lox's address stays silent for a minute, so a
//! program restart does not move vislox to the relay; and once the CloudDNS
//! name was verified, vislox never falls back to an unchecked connection to
//! the same address.
//!
//! The route is used by vislox's own connections: live values, the status
//! chip and preflight, the structure file, the app's calendar entries,
//! Apply's restart probe and operating inputs. lox's commands (Pull, Apply,
//! Sync, `lox log`) and the Debug-Monitor's Start and Stop use lox's host.

use crate::discover::norm_serial;
use loxone_client::Connection;
use serde::Serialize;
use serde_json::Value;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, watch};

/// A TCP connect to lox's address.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The DNS lookup of the CloudDNS name.
const DNS_TIMEOUT: Duration = Duration::from_secs(3);
/// The Connect Service's answer.
const HTTP_TIMEOUT: Duration = Duration::from_secs(6);
/// Choosing a route may take this long; then lox's host is used.
const CHOOSE_TIMEOUT: Duration = Duration::from_secs(30);
/// While lox's address does not answer: how often it is tried again.
const RECHECK: Duration = Duration::from_secs(60);
/// A failed connection asks for a new choice at most this often.
const NUDGE_GAP: Duration = Duration::from_secs(30);
/// How long a working local route may be silent before Remote Connect.
const LOCAL_GRACE: Duration = Duration::from_secs(60);
/// Loxone's Connect Service.
pub const CONNECT_SERVICE: &str = "https://connect.loxonecloud.com";

/// Which route `--route` allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// The CloudDNS name, else lox's host; Remote Connect when that does not
    /// answer (or names no host) and the serial is known.
    Auto,
    /// The CloudDNS name, else lox's host; never Remote Connect.
    Local,
    /// Always Remote Connect.
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Local,
    Clouddns,
    Remote,
}

/// The route taken, for the status chip.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub kind: Kind,
    /// What vislox connects to.
    pub url: String,
    /// lox's host (what Pull, Apply and Sync use).
    pub lox_host: String,
    /// Whether the TLS certificate is checked.
    pub verified: bool,
    /// Why this route, and why the certificate is not checked when it is not.
    pub note: Option<String>,
    /// lox's address did not answer when the route was chosen: it is tried
    /// again, and the LAN is searched for the Miniserver.
    pub local_down: bool,
    /// The CloudDNS name was verified before and no longer is: vislox stays
    /// on it rather than fall back to an unchecked connection.
    pub failing: Option<String>,
}

impl Route {
    /// lox's host, unchanged.
    fn local(conn: &Connection, note: Option<String>) -> Route {
        Route {
            kind: Kind::Local,
            url: conn.host.clone(),
            lox_host: conn.host.clone(),
            verified: conn.verify_ssl,
            note,
            local_down: false,
            failing: None,
        }
    }

    /// The warning the chip shows: the Miniserver is not verified, or its
    /// verified name stopped checking out.
    pub fn warning(&self) -> Option<String> {
        if let Some(f) = &self.failing {
            return Some(format!(
                "vislox reached the Miniserver under its verified CloudDNS name before, but now {f} \
                 It does not fall back to an unchecked connection: restart vislox to accept one."
            ));
        }
        (self.url.starts_with("https://") && !self.verified).then(|| {
            format!(
                "The connection is encrypted, but vislox does not check the Miniserver's \
                 certificate: {}",
                self.note
                    .as_deref()
                    .unwrap_or("lox's host is not a name the certificate can be checked for.")
            )
        })
    }
}

/// `192-168-1-47.504f94a0e454.dyndns.loxonecloud.com`: the CloudDNS name
/// under which the Miniserver `serial` answers at `ip`.
pub fn clouddns_name(ip: Ipv4Addr, serial: &str) -> String {
    let o = ip.octets();
    format!(
        "{}-{}-{}-{}.{}.dyndns.loxonecloud.com",
        o[0],
        o[1],
        o[2],
        o[3],
        norm_serial(serial).to_ascii_lowercase()
    )
}

/// lox's host without a scheme: (address, port, https).
fn parts(host: &str) -> (String, u16, bool) {
    let https = !host.starts_with("http://");
    let (addr, port) = crate::miniserver::split_host(host);
    (addr, port, https)
}

/// Whether lox's context names no host (a serial only: Remote Connect).
fn no_host(host: &str) -> bool {
    host.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_matches('/')
        .is_empty()
}

/// A LAN address (RFC 1918): what the CloudDNS name can be built for, and
/// what may not answer away from home.
fn lan_ipv4(addr: &str) -> Option<Ipv4Addr> {
    addr.parse::<Ipv4Addr>().ok().filter(Ipv4Addr::is_private)
}

/// A host the tests stand in on: never routed through the Loxone cloud.
fn loopback(addr: &str) -> bool {
    addr == "localhost"
        || addr
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

async fn answers(addr: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio::net::TcpStream::connect((addr, port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// Whether lox's address takes a TCP connection.
async fn lox_answers(lox: &Connection) -> bool {
    let (addr, port, _) = parts(&lox.host);
    !no_host(&lox.host) && answers(&addr, port).await
}

/// The CloudDNS name for lox's `https://` LAN address, checked: it
/// resolves to that address, the certificate is valid for it, and the
/// Miniserver there reports `serial`.
async fn clouddns(
    conn: &Connection,
    ip: Ipv4Addr,
    port: u16,
    serial: &str,
) -> Result<Connection, String> {
    let name = clouddns_name(ip, serial);
    let rebinding = "A router with DNS rebinding protection blocks names that point to a local \
                     address: allow dyndns.loxonecloud.com";
    let resolved =
        tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((name.as_str(), port)))
            .await
            .map_err(|_| {
                format!(
                    "the CloudDNS name {name} did not resolve within {} s",
                    DNS_TIMEOUT.as_secs()
                )
            })?
            .map_err(|e| format!("the CloudDNS name {name} does not resolve ({e}). {rebinding}"))?;
    if !resolved
        .into_iter()
        .any(|a| a.ip() == std::net::IpAddr::V4(ip))
    {
        return Err(format!(
            "the CloudDNS name {name} does not resolve to {ip}. {rebinding}"
        ));
    }
    let port_part = if port == 443 {
        String::new()
    } else {
        format!(":{port}")
    };
    let checked = Connection {
        host: format!("https://{name}{port_part}"),
        verify_ssl: true,
        ..conn.clone()
    };
    let key = crate::miniserver::api_key(&checked).await.map_err(|e| {
        let hint = if e.contains("not valid for name") {
            format!(
                "The certificate is for another name: the Miniserver at {ip} may not be {}",
                norm_serial(serial)
            )
        } else {
            "The Miniserver needs the CloudDNS certificate (Gen 2, CloudDNS or Remote Connect \
             on); a custom certificate or an expired one cannot be checked this way"
                .to_string()
        };
        format!("its certificate could not be verified for {name} ({e}). {hint}")
    })?;
    match key.serial {
        Some(s) if s != norm_serial(serial) => Err(format!(
            "{name} reports serial {s}, not {}",
            norm_serial(serial)
        )),
        _ => Ok(checked),
    }
}

/// What the Connect Service's status code means.
pub fn connect_service_says(status: u16, serial: &str) -> String {
    let what = match status {
        400 => "the serial is not valid",
        404 => {
            "it knows no relay for this Miniserver: Remote Connect is off, or the Miniserver runs \
             firmware older than 17.0"
        }
        500 => "the service has an internal error; try again later",
        504 => {
            "the Miniserver is set up for Remote Connect but not connected to a relay. It is \
             offline, or its network blocks the tunnel (it needs outbound TCP 22 to \
             *.remcov3.loxonecloud.com, 443 to registry.loxonecloud.com and 8443 to \
             eu.ccbroker.loxonecloud.com)"
        }
        429 => "too many requests; try again in a minute",
        // The older services' codes, which the same address may still send.
        403 => "the Miniserver is not registered with Loxone, or CloudDNS is off",
        405 => "the Miniserver does not report to CloudDNS",
        409 => "the Miniserver has an unsecure password, so external access is blocked",
        412 => "the Miniserver's port is not open",
        418 => "access was denied",
        481 => "the Miniserver could not connect to the Remote Connect service",
        482 => "the Miniserver's Remote Connect connection was aborted after a timeout",
        483 => "the Miniserver is making a scheduled restart",
        _ => {
            return format!(
                "Remote Connect for {serial}: the Connect Service answered HTTP {status}"
            );
        }
    };
    format!("Remote Connect for {serial}: {what} (Connect Service, HTTP {status})")
}

/// The relay URL in the Connect Service's `{"url": "…"}`.
fn relay_url(body: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|_| "the Connect Service's answer is not JSON".to_string())?;
    let url = v["url"]
        .as_str()
        .ok_or("the Connect Service's answer names no URL")?
        .trim_end_matches('/');
    if !url.starts_with("https://") {
        return Err(format!("the Connect Service's URL is not HTTPS: {url}"));
    }
    Ok(url.to_string())
}

/// Ask the Connect Service (`service`) where the Miniserver `serial` is
/// reachable. The service's certificate is checked (an `http://` stand-in
/// in tests has none).
async fn get_ip(service: &str, serial: &str) -> Result<String, String> {
    let mut probe = Connection::new(service.trim_end_matches('/'), "", "");
    probe.verify_ssl = true;
    let client = crate::tls::client(&probe, HTTP_TIMEOUT)?;
    let r = client
        .get(probe.url(&format!("getip?snr={serial}")))
        .send()
        .await
        .map_err(|e| {
            format!(
                "Remote Connect for {serial}: {} does not answer ({}). This host needs outbound \
                 TCP 443 to connect.loxonecloud.com",
                probe.host,
                crate::session::chain(&e)
            )
        })?;
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    if status != 200 {
        return Err(connect_service_says(status, serial));
    }
    relay_url(&body).map_err(|e| format!("Remote Connect for {serial}: {e}"))
}

/// The Remote Connect route: the relay URL, checked.
async fn remote(
    conn: &Connection,
    serial: &str,
    service: &str,
) -> Result<(Connection, Route), String> {
    let url = get_ip(service, serial).await?;
    let relay = Connection {
        host: url.clone(),
        verify_ssl: true,
        ..conn.clone()
    };
    if let Err(e) = crate::miniserver::api_key(&relay).await {
        let (addr, port, _) = parts(&url);
        return Err(if answers(&addr, port).await {
            format!("Remote Connect for {serial}: the relay {url} could not be verified ({e})")
        } else {
            format!(
                "Remote Connect for {serial}: the relay {url} does not answer ({e}). Outbound TCP \
                 20000–49000 to *.remcov3.loxonecloud.com is probably blocked on this network"
            )
        });
    }
    Ok((
        relay,
        Route {
            kind: Kind::Remote,
            url,
            lox_host: conn.host.clone(),
            verified: true,
            note: None,
            local_down: false,
            failing: None,
        },
    ))
}

/// The route to take, and the connection vislox's own clients use for it.
/// `serial`: the Miniserver the config belongs to, else the one lox names.
/// `service`: the Connect Service. Fails only when there is no route at all
/// (Remote Connect failed, and there is no host to fall back to).
pub async fn resolve(
    conn: &Connection,
    serial: Option<&str>,
    mode: Mode,
    service: &str,
) -> Result<(Connection, Route), String> {
    let serial = serial.map(norm_serial).filter(|s| !s.is_empty());
    let needs_serial = || {
        "Remote Connect needs the Miniserver's serial: set it with `lox setup set --serial …`"
            .to_string()
    };
    if mode == Mode::Remote || no_host(&conn.host) {
        if mode == Mode::Local {
            return Err("lox's configuration names no host (and --route local)".into());
        }
        let s = serial.as_deref().ok_or_else(needs_serial)?;
        return remote(conn, s, service).await;
    }
    let (addr, port, https) = parts(&conn.host);
    if loopback(&addr) {
        return Ok((conn.clone(), Route::local(conn, None)));
    }
    let Some(serial) = serial else {
        let note = https.then(|| {
            "the serial is not known, so vislox cannot build the Miniserver's CloudDNS name: set \
             it with `lox setup set --serial …`."
                .to_string()
        });
        return Ok((conn.clone(), Route::local(conn, note)));
    };
    if !answers(&addr, port).await {
        let down = |note: String| {
            let mut r = Route::local(conn, Some(note));
            r.local_down = true;
            (conn.clone(), r)
        };
        if mode == Mode::Local {
            return Ok(down(format!(
                "lox's address {} does not answer.",
                conn.host
            )));
        }
        return Ok(match remote(conn, &serial, service).await {
            Ok((c, mut r)) => {
                r.note = Some(format!(
                    "lox's address {} does not answer, so vislox connects through Remote Connect",
                    conn.host
                ));
                r.local_down = true;
                (c, r)
            }
            Err(e) => down(format!("lox's address does not answer, and {e}.")),
        });
    }
    if conn.verify_ssl || !https {
        // As lox was told: verified already, or no TLS at all.
        return Ok((conn.clone(), Route::local(conn, None)));
    }
    let Some(ip) = lan_ipv4(&addr) else {
        let note = format!(
            "vislox checks the certificate through the Miniserver's CloudDNS name, which it builds \
             only for a local IPv4 address, not for {addr}."
        );
        return Ok((conn.clone(), Route::local(conn, Some(note))));
    };
    match clouddns(conn, ip, port, &serial).await {
        Ok(c) => {
            let route = Route {
                kind: Kind::Clouddns,
                url: c.host.clone(),
                lox_host: conn.host.clone(),
                verified: true,
                note: None,
                local_down: false,
                failing: None,
            };
            Ok((c, route))
        }
        Err(e) => Ok((conn.clone(), Route::local(conn, Some(format!("{e}."))))),
    }
}

/// What a new choice does to the current route.
#[derive(Debug, PartialEq)]
enum Take {
    /// Use the new route.
    New,
    /// Keep the current one (the new choice is no better, or the same).
    Keep,
    /// Keep the current, verified name, and say why it fails.
    Failing(String),
}

/// Whether to take `new` over `cur`. `first`: nothing was chosen yet.
/// `silent_for`: how long lox's address has not answered.
fn take(cur: &Route, new: &Route, first: bool, silent_for: Option<Duration>) -> Take {
    if cur == new {
        return Take::Keep;
    }
    if first {
        return Take::New;
    }
    // A verified name that stopped checking out (another device at the
    // address, a man in the middle) is not traded for an unchecked
    // connection to the same address.
    if cur.kind == Kind::Clouddns && new.kind == Kind::Local && !new.verified {
        let why = new
            .note
            .clone()
            .unwrap_or_else(|| "it does not check out.".into());
        return Take::Failing(why);
    }
    // A local route that worked moves to the relay only when lox's address
    // stays silent: a program restart is not a move.
    if cur.kind != Kind::Remote
        && !cur.local_down
        && new.local_down
        && silent_for.is_none_or(|d| d < LOCAL_GRACE)
    {
        return Take::Keep;
    }
    Take::New
}

/// The current route, shared by everything that talks to the Miniserver,
/// and chosen again when it should be.
pub struct Handle {
    lox: Connection,
    serial: Option<String>,
    mode: Mode,
    service: String,
    /// `false`: lox's host, never another (`--live-from`, tests).
    chooses: bool,
    /// The connection, its route, and whether a route was chosen yet.
    cur: RwLock<(Connection, Route, bool)>,
    changed: watch::Sender<u64>,
    nudged: Notify,
    last_choice: Mutex<Option<Instant>>,
    silent_since: Mutex<Option<Instant>>,
}

impl Handle {
    /// lox's host, as it is.
    pub fn fixed(lox: Connection) -> Arc<Handle> {
        Handle::with(lox, None, Mode::Local, String::new(), false)
    }

    /// lox's host until [`Handle::spawn`] has chosen a route.
    pub fn new(lox: Connection, serial: Option<String>, mode: Mode, service: &str) -> Arc<Handle> {
        Handle::with(lox, serial, mode, service.to_string(), true)
    }

    fn with(
        lox: Connection,
        serial: Option<String>,
        mode: Mode,
        service: String,
        chooses: bool,
    ) -> Arc<Handle> {
        let note = chooses.then(|| "vislox is still choosing its route.".to_string());
        let route = Route::local(&lox, note);
        Arc::new(Handle {
            cur: RwLock::new((lox.clone(), route, !chooses)),
            lox,
            serial,
            mode,
            service,
            chooses,
            changed: watch::channel(0).0,
            nudged: Notify::new(),
            last_choice: Mutex::new(None),
            silent_since: Mutex::new(None),
        })
    }

    /// A handle already on `route` (tests).
    #[cfg(test)]
    pub fn on(lox: Connection, conn: Connection, route: Route) -> Arc<Handle> {
        let h = Handle::with(lox, None, Mode::Auto, String::new(), true);
        h.put(conn, route);
        h
    }

    /// The connection vislox's own clients use now.
    pub fn conn(&self) -> Connection {
        self.cur.read().unwrap().0.clone()
    }

    pub fn route(&self) -> Route {
        self.cur.read().unwrap().1.clone()
    }

    /// lox's own connection (Pull, Apply, Sync use its host).
    pub fn lox(&self) -> &Connection {
        &self.lox
    }

    /// Bumped whenever the route changes.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// A connection on the route failed: choose again soon.
    pub fn nudge(&self) {
        self.nudged.notify_one();
    }

    /// Choose the route now. A failure or a timeout leaves lox's host.
    async fn choose(&self) {
        *self.last_choice.lock().unwrap() = Some(Instant::now());
        let chosen = tokio::time::timeout(
            CHOOSE_TIMEOUT,
            resolve(&self.lox, self.serial.as_deref(), self.mode, &self.service),
        )
        .await;
        let (conn, route) = match chosen {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => (
                self.lox.clone(),
                Route::local(&self.lox, Some(format!("{e}."))),
            ),
            Err(_) => (
                self.lox.clone(),
                Route::local(
                    &self.lox,
                    Some(format!(
                        "choosing the route took more than {} s, so vislox uses lox's host.",
                        CHOOSE_TIMEOUT.as_secs()
                    )),
                ),
            ),
        };
        self.put(conn, route);
    }

    fn put(&self, conn: Connection, route: Route) {
        let silent_for = {
            let mut s = self.silent_since.lock().unwrap();
            if route.local_down {
                Some(s.get_or_insert_with(Instant::now).elapsed())
            } else {
                *s = None;
                None
            }
        };
        let mut cur = self.cur.write().unwrap();
        let first = !cur.2;
        let mut old = cur.1.clone();
        old.failing = None;
        match take(&old, &route, first, silent_for) {
            Take::Keep => {
                cur.2 = true;
                if cur.1.failing.take().is_some() {
                    self.changed.send_modify(|n| *n += 1);
                }
                return;
            }
            Take::Failing(why) => {
                if cur.1.failing.as_deref() != Some(why.as_str()) {
                    eprintln!("vislox: the CloudDNS name no longer checks out: {why}");
                    cur.1.failing = Some(why);
                    self.changed.send_modify(|n| *n += 1);
                }
                return;
            }
            Take::New => {}
        }
        match route.kind {
            Kind::Clouddns => eprintln!(
                "vislox: connecting under the CloudDNS name {} (certificate checked)",
                route.url
            ),
            Kind::Remote => eprintln!("vislox: connecting through Remote Connect: {}", route.url),
            Kind::Local => eprintln!("vislox: connecting to lox's host {}", route.url),
        }
        if let Some(w) = route.warning() {
            eprintln!("vislox: {w}");
        }
        *cur = (conn, route, true);
        drop(cur);
        self.changed.send_modify(|n| *n += 1);
    }

    /// Choose the route in the background, then again every minute while
    /// lox's address does not answer, and after a failed connection.
    pub fn spawn(self: &Arc<Self>) {
        if !self.chooses {
            return;
        }
        let h = self.clone();
        tokio::spawn(async move {
            h.choose().await;
            loop {
                let nudged = tokio::select! {
                    _ = tokio::time::sleep(RECHECK) => false,
                    _ = h.nudged.notified() => true,
                };
                let recent = h
                    .last_choice
                    .lock()
                    .unwrap()
                    .is_some_and(|t| t.elapsed() < NUDGE_GAP);
                if nudged && recent {
                    continue;
                }
                let r = h.route();
                let worth = nudged || r.local_down || r.failing.is_some();
                // Away from home the relay works: only lox's address coming
                // back changes the choice, so the cloud is not asked.
                if !worth || (r.kind == Kind::Remote && !nudged && !lox_answers(&h.lox).await) {
                    continue;
                }
                h.choose().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clouddns_name_is_built_from_address_and_serial() {
        assert_eq!(
            clouddns_name("192.168.1.47".parse().unwrap(), "50:4F:94:A0:E4:54"),
            "192-168-1-47.504f94a0e454.dyndns.loxonecloud.com"
        );
    }

    #[test]
    fn only_lan_addresses_get_a_clouddns_name() {
        assert!(lan_ipv4("192.168.1.120").is_some());
        assert!(lan_ipv4("10.0.0.5").is_some());
        assert!(lan_ipv4("172.20.1.1").is_some());
        assert!(lan_ipv4("8.8.8.8").is_none());
        assert!(lan_ipv4("ms.local").is_none());
        assert!(lan_ipv4("127.0.0.1").is_none());
    }

    #[test]
    fn a_context_with_only_a_serial_has_no_host() {
        assert!(no_host(""));
        assert!(no_host("https://"));
        assert!(!no_host("https://192.168.1.120"));
    }

    #[test]
    fn the_connect_service_s_codes_are_explained() {
        let s = "504F94A0E454";
        assert!(connect_service_says(504, s).contains("not connected to a relay"));
        assert!(connect_service_says(404, s).contains("Remote Connect is off"));
        assert!(connect_service_says(409, s).contains("unsecure password"));
        assert!(connect_service_says(483, s).contains("scheduled restart"));
        assert!(connect_service_says(502, s).contains("HTTP 502"));
    }

    #[test]
    fn the_relay_url_must_be_https() {
        assert_eq!(
            relay_url(
                r#"{"url":"https://167-233-234-75.504F94A0E454.dyndns.loxonecloud.com:29235"}"#
            ),
            Ok("https://167-233-234-75.504F94A0E454.dyndns.loxonecloud.com:29235".into())
        );
        assert!(relay_url(r#"{"url":"http://x"}"#).is_err());
        assert!(relay_url("{}").is_err());
    }

    #[test]
    fn an_unverified_https_route_warns_and_says_why() {
        let c = Connection::new("https://192.168.1.120", "u", "p");
        let r = Route::local(&c, Some("the CloudDNS name does not resolve.".into()));
        assert!(r.warning().unwrap().contains("does not resolve"));
        let plain = Connection::new("http://192.168.1.120", "u", "p");
        assert!(Route::local(&plain, None).warning().is_none());
    }

    fn lan() -> Connection {
        Connection::new("https://192.168.1.120", "u", "p")
    }

    fn clouddns_route() -> Route {
        Route {
            kind: Kind::Clouddns,
            url: "https://192-168-1-120.504f94a0e454.dyndns.loxonecloud.com".into(),
            lox_host: lan().host,
            verified: true,
            note: None,
            local_down: false,
            failing: None,
        }
    }

    fn remote_route(local_down: bool) -> Route {
        Route {
            kind: Kind::Remote,
            url: "https://relay.504f94a0e454.dyndns.loxonecloud.com:29235".into(),
            lox_host: lan().host,
            verified: true,
            note: None,
            local_down,
            failing: None,
        }
    }

    fn down() -> Route {
        let mut r = Route::local(&lan(), Some("lox's address does not answer.".into()));
        r.local_down = true;
        r
    }

    #[test]
    fn a_verified_name_never_falls_back_to_an_unchecked_connection() {
        let bad = Route::local(
            &lan(),
            Some("its certificate could not be verified.".into()),
        );
        assert!(matches!(
            take(&clouddns_route(), &bad, false, None),
            Take::Failing(w) if w.contains("could not be verified")
        ));
        // At start, the fallback is taken (with the chip's warning).
        assert_eq!(
            take(&Route::local(&lan(), None), &bad, true, None),
            Take::New
        );
    }

    #[test]
    fn a_working_local_route_waits_before_it_moves_to_the_relay() {
        let away = remote_route(true);
        assert_eq!(take(&clouddns_route(), &away, false, None), Take::Keep);
        assert_eq!(
            take(
                &clouddns_route(),
                &away,
                false,
                Some(Duration::from_secs(5))
            ),
            Take::Keep
        );
        assert_eq!(
            take(&clouddns_route(), &away, false, Some(LOCAL_GRACE)),
            Take::New
        );
        // At start (a Miniserver that does not answer yet), the relay at once.
        assert_eq!(
            take(&Route::local(&lan(), None), &away, true, None),
            Take::New
        );
    }

    #[test]
    fn back_home_the_relay_gives_way_to_the_local_route() {
        assert_eq!(
            take(&remote_route(true), &clouddns_route(), false, None),
            Take::New
        );
        assert_eq!(take(&down(), &clouddns_route(), false, None), Take::New);
    }

    /// The handle switches and tells its users; a verified name that fails
    /// later stays, with the warning.
    #[test]
    fn the_handle_switches_back_and_tells_its_users() {
        let h = Handle::new(lan(), Some("504F94A0E454".into()), Mode::Auto, "");
        let mut changes = h.changes();
        let relay = Connection {
            host: remote_route(true).url,
            verify_ssl: true,
            ..lan()
        };
        h.put(relay, remote_route(true));
        assert_eq!(h.route().kind, Kind::Remote);
        assert!(changes.has_changed().unwrap());
        changes.mark_unchanged();
        let named = Connection {
            host: clouddns_route().url,
            verify_ssl: true,
            ..lan()
        };
        h.put(named.clone(), clouddns_route());
        assert_eq!(h.route().kind, Kind::Clouddns);
        assert_eq!(h.conn().host, named.host);
        assert!(changes.has_changed().unwrap());
        h.put(
            lan(),
            Route::local(
                &lan(),
                Some("its certificate could not be verified.".into()),
            ),
        );
        assert_eq!(h.conn().host, named.host, "no unchecked fallback");
        assert!(h.route().warning().unwrap().contains("does not fall back"));
    }

    #[tokio::test]
    async fn a_failed_choice_keeps_lox_s_host() {
        // No host and no serial: no route at all. Live values, the chip and
        // the rest still get lox's connection, with the reason.
        let h = Handle::new(
            Connection::new("", "u", "p"),
            None,
            Mode::Auto,
            "http://127.0.0.1:9",
        );
        tokio::time::timeout(Duration::from_secs(10), h.choose())
            .await
            .expect("hard timeout");
        let r = h.route();
        assert_eq!(r.kind, Kind::Local);
        assert_eq!(h.conn().host, "");
        assert!(r.note.unwrap().contains("lox setup set --serial"));
    }

    #[tokio::test]
    async fn a_test_host_is_taken_as_it_is() {
        let c = Connection::new("http://127.0.0.1:1", "u", "p");
        let (conn, r) = tokio::time::timeout(
            Duration::from_secs(10),
            resolve(&c, Some("504F94000001"), Mode::Auto, "http://127.0.0.1:9"),
        )
        .await
        .expect("hard timeout")
        .unwrap();
        assert_eq!(
            (conn.host.as_str(), r.kind),
            ("http://127.0.0.1:1", Kind::Local)
        );
    }

    /// A stand-in Connect Service that answers `status`.
    async fn service(status: &'static str) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\n\r\n{{}}").as_bytes(),
                    )
                    .await;
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn a_relay_that_is_not_connected_is_explained() {
        let svc = service("504 Gateway Timeout").await;
        let c = Connection::new("", "u", "p");
        let e = tokio::time::timeout(
            Duration::from_secs(10),
            resolve(&c, Some("504F94A0E454"), Mode::Auto, &svc),
        )
        .await
        .expect("hard timeout")
        .unwrap_err();
        assert!(e.contains("not connected to a relay"), "{e}");
    }

    /// A LAN address that does not answer: the route says so, and is
    /// chosen again later.
    #[tokio::test]
    async fn an_address_that_does_not_answer_is_tried_again() {
        let svc = service("404 Not Found").await;
        let c = Connection::new("https://10.255.255.1", "u", "p");
        let (_, r) = tokio::time::timeout(
            Duration::from_secs(15),
            resolve(&c, Some("504F94A0E454"), Mode::Auto, &svc),
        )
        .await
        .expect("hard timeout")
        .unwrap();
        assert_eq!(r.kind, Kind::Local);
        assert!(r.local_down);
        assert!(r.note.unwrap().contains("Remote Connect is off"));
    }

    /// Against a real Miniserver (no login: only `jdev/cfg/apiKey` and the
    /// Connect Service): `VISLOX_TEST_MINISERVER=https://192.168.1.120
    /// VISLOX_TEST_SERIAL=504F94… cargo test real_ -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn real_miniserver_routes() {
        let host = std::env::var("VISLOX_TEST_MINISERVER").unwrap();
        let serial = std::env::var("VISLOX_TEST_SERIAL").unwrap();
        for (host, mode) in [
            (host.as_str(), Mode::Local),
            (host.as_str(), Mode::Remote),
            ("", Mode::Auto),
            ("https://192.168.254.254", Mode::Auto),
        ] {
            let c = Connection::new(host, "", "");
            let r = tokio::time::timeout(
                Duration::from_secs(40),
                resolve(&c, Some(&serial), mode, CONNECT_SERVICE),
            )
            .await
            .expect("hard timeout");
            println!("{host:?} {mode:?}: {:?}", r.map(|(_, r)| r));
        }
    }
}
