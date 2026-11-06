//! How vislox reaches the Miniserver: the route, chosen once at start.
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
//! The route is used by vislox's own connections: live values, the status
//! chip and preflight, the structure file, Apply's restart probe and
//! operating inputs. lox's commands (Pull, Apply, Sync, `lox log`) still use
//! lox's own host and certificate setting.

use crate::discover::norm_serial;
use loxone_client::Connection;
use serde::Serialize;
use serde_json::Value;
use std::net::Ipv4Addr;
use std::time::Duration;

/// A TCP connect to lox's address.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The DNS lookup of the CloudDNS name.
const DNS_TIMEOUT: Duration = Duration::from_secs(3);
/// The verified `jdev/cfg/apiKey` request, and the Connect Service's answer.
const HTTP_TIMEOUT: Duration = Duration::from_secs(6);
/// Loxone's Connect Service (`VISLOX_REMOTE_CONNECT` replaces it in tests).
const CONNECT_SERVICE: &str = "https://connect.loxonecloud.com";

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
        }
    }

    /// The warning the chip shows when the connection is encrypted but the
    /// Miniserver is not verified.
    pub fn warning(&self) -> Option<String> {
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

/// A request with the certificate checked: the Miniserver's status, which
/// needs no login.
async fn verified_api_key(conn: &Connection) -> Result<(), String> {
    let client = crate::tls::client(conn, HTTP_TIMEOUT)?;
    let r = client
        .get(conn.url("jdev/cfg/apiKey"))
        .send()
        .await
        .map_err(|e| crate::session::chain(&e))?;
    if r.status().is_success() {
        Ok(())
    } else {
        Err(format!("it answered HTTP {}", r.status().as_u16()))
    }
}

/// The CloudDNS name for lox's `https://` LAN address, checked: it
/// resolves to that address and the certificate is valid for it.
async fn clouddns(
    conn: &Connection,
    ip: Ipv4Addr,
    port: u16,
    serial: &str,
) -> Result<Connection, String> {
    let name = clouddns_name(ip, serial);
    let resolved =
        tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((name.as_str(), port)))
            .await
            .map_err(|_| {
                format!(
                    "the CloudDNS name {name} did not resolve within {} s",
                    DNS_TIMEOUT.as_secs()
                )
            })?
            .map_err(|e| {
                format!(
                    "the CloudDNS name {name} does not resolve ({e}). A router with DNS rebinding \
                 protection blocks names that point to a local address: allow \
                 dyndns.loxonecloud.com"
                )
            })?;
    if !resolved
        .into_iter()
        .any(|a| a.ip() == std::net::IpAddr::V4(ip))
    {
        return Err(format!(
            "the CloudDNS name {name} does not resolve to {ip}. A router with DNS rebinding \
             protection blocks names that point to a local address: allow dyndns.loxonecloud.com"
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
    verified_api_key(&checked).await.map_err(|e| {
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
    Ok(checked)
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
        429 => "too many requests (it allows about 10 at a time); try again in a minute",
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

/// Ask the Connect Service where the Miniserver `serial` is reachable.
async fn get_ip(serial: &str) -> Result<String, String> {
    let base = std::env::var("VISLOX_REMOTE_CONNECT").unwrap_or_else(|_| CONNECT_SERVICE.into());
    let mut probe = Connection::new(base.trim_end_matches('/'), "", "");
    probe.verify_ssl = probe.host.starts_with("https://");
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
async fn remote(conn: &Connection, serial: &str) -> Result<(Connection, Route), String> {
    let url = get_ip(serial).await?;
    let relay = Connection {
        host: url.clone(),
        verify_ssl: true,
        ..conn.clone()
    };
    if let Err(e) = verified_api_key(&relay).await {
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
        },
    ))
}

/// The route to take, and the connection vislox's own clients use for it.
/// `serial`: the Miniserver the config belongs to, else the one lox names.
/// Fails only when there is no route at all (Remote Connect failed, and
/// there is no host to fall back to).
pub async fn resolve(
    conn: &Connection,
    serial: Option<&str>,
    mode: Mode,
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
        return remote(conn, s).await;
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
    let here = answers(&addr, port).await;
    if !here && mode == Mode::Auto {
        return match remote(conn, &serial).await {
            Ok((c, mut r)) => {
                r.note = Some(format!(
                    "lox's address {} does not answer, so vislox connects through Remote Connect",
                    conn.host
                ));
                Ok((c, r))
            }
            Err(e) => Ok((
                conn.clone(),
                Route::local(
                    conn,
                    Some(format!("lox's address does not answer, and {e}.")),
                ),
            )),
        };
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
    if !here {
        let note = format!("lox's address {} does not answer.", conn.host);
        return Ok((conn.clone(), Route::local(conn, Some(note))));
    }
    match clouddns(conn, ip, port, &serial).await {
        Ok(c) => {
            let route = Route {
                kind: Kind::Clouddns,
                url: c.host.clone(),
                lox_host: conn.host.clone(),
                verified: true,
                note: None,
            };
            Ok((c, route))
        }
        Err(e) => Ok((conn.clone(), Route::local(conn, Some(format!("{e}."))))),
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

    #[tokio::test]
    async fn a_test_host_is_taken_as_it_is() {
        let c = Connection::new("http://127.0.0.1:1", "u", "p");
        let (conn, r) = tokio::time::timeout(
            Duration::from_secs(10),
            resolve(&c, Some("504F94000001"), Mode::Auto),
        )
        .await
        .expect("hard timeout")
        .unwrap();
        assert_eq!(
            (conn.host.as_str(), r.kind),
            ("http://127.0.0.1:1", Kind::Local)
        );
    }

    #[tokio::test]
    async fn without_a_serial_there_is_no_remote_connect() {
        let c = Connection::new("", "u", "p");
        let e = tokio::time::timeout(Duration::from_secs(10), resolve(&c, None, Mode::Auto))
            .await
            .expect("hard timeout")
            .unwrap_err();
        assert!(e.contains("lox setup set --serial"), "{e}");
    }

    /// A stand-in Connect Service: the code it answers is explained.
    #[tokio::test]
    async fn a_relay_that_is_not_connected_is_explained() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(b"HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 2\r\n\r\n{}")
                    .await;
            }
        });
        // SAFETY: only this test reads the variable.
        unsafe { std::env::set_var("VISLOX_REMOTE_CONNECT", format!("http://127.0.0.1:{port}")) };
        let c = Connection::new("", "u", "p");
        let e = tokio::time::timeout(
            Duration::from_secs(10),
            resolve(&c, Some("504F94A0E454"), Mode::Auto),
        )
        .await
        .expect("hard timeout")
        .unwrap_err();
        assert!(e.contains("not connected to a relay"), "{e}");
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
            let r = tokio::time::timeout(Duration::from_secs(40), resolve(&c, Some(&serial), mode))
                .await
                .expect("hard timeout");
            println!("{host:?} {mode:?}: {:?}", r.map(|(_, r)| r));
        }
    }
}
