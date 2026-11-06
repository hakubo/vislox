//! The HTTP and WebSocket clients vislox talks to the Miniserver with, and
//! how they check its TLS certificate.
//!
//! A connection with `verify_ssl` is checked against the public web roots
//! (`webpki-roots`), the way a browser checks it. That works for a Gen 2
//! Miniserver's CloudDNS certificate (`*.{serial}.dyndns.loxonecloud.com`,
//! issued by Let's Encrypt), so for the CloudDNS name of a local address and
//! for a Remote Connect URL (`route.rs`), not for a bare IP address.
//!
//! lox's own clients cannot do this yet: `loxone-client` builds reqwest with
//! `rustls-tls-manual-roots` and adds no roots, so `verify_ssl: true` makes
//! every HTTPS request fail, and its WebSocket never checks the certificate.
//! So, on a verified route:
//! - lox's `LoxClient` gets a client built here ([`lox_client`]): every
//!   request it makes goes through that client. Its connection keeps
//!   `verify_ssl`, so a client lox might build from it fails rather than
//!   skip the check;
//! - the WebSocket is opened here ([`ws_connect`]);
//! - lox's login fetches the Miniserver's RSA key with a client of its own,
//!   sending lox's user and password (`fetch_public_key`). vislox fetches the
//!   key itself, verified and without credentials (it needs none), and hands
//!   lox a connection whose host is a one-off listener on this host's
//!   loopback that serves that key ([`login`]). lox's credentials then go
//!   nowhere but into vislox; the login itself runs on the checked socket.

use loxone_client::{Connection, LoxClient};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::http::Request;

/// lox's HTTP timeout, for the blocking client that replaces its own.
const LOX_TIMEOUT: Duration = Duration::from_secs(10);
/// The key fetch for a login.
const KEY_TIMEOUT: Duration = Duration::from_secs(10);

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// rustls with the public web roots, on the ring provider lox uses.
pub fn verified_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// rustls that takes any certificate: what lox does without `verify_ssl`
/// (Miniservers without the CloudDNS certificate are self-signed).
fn unchecked_config() -> rustls::ClientConfig {
    let mut c = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    c.dangerous()
        .set_certificate_verifier(Arc::new(Unchecked(provider())));
    c
}

#[derive(Debug)]
struct Unchecked(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for Unchecked {
    fn verify_server_cert(
        &self,
        _: &CertificateDer,
        _: &[CertificateDer],
        _: &ServerName,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The TLS setup for `conn`: checked when it says `verify_ssl`.
fn config(conn: &Connection) -> rustls::ClientConfig {
    if conn.verify_ssl {
        verified_config()
    } else {
        unchecked_config()
    }
}

/// An async client for `conn`: the certificate is checked when
/// `verify_ssl` is set, and no redirect goes to another host (a Gen 2
/// Miniserver sends local HTTP to its CloudDNS name), as lox's own client.
pub fn client(conn: &Connection, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(loxone_client::client::USER_AGENT)
        .timeout(timeout)
        .redirect(LoxClient::same_origin_redirect_policy(&conn.host))
        .use_preconfigured_tls(config(conn))
        .build()
        .map_err(|e| e.to_string())
}

/// lox's HTTP client for `conn`, with a blocking client built here, which
/// checks the certificate when `verify_ssl` is set (lox's own has no roots
/// to check it against). Must not be called on an async thread, as
/// `LoxClient::new`.
pub fn lox_client(conn: &Connection) -> Result<LoxClient, String> {
    let mut c = LoxClient::new(conn.clone()).map_err(|e| e.to_string())?;
    c.client = reqwest::blocking::Client::builder()
        .user_agent(loxone_client::client::USER_AGENT)
        .timeout(LOX_TIMEOUT)
        .redirect(LoxClient::same_origin_redirect_policy(&conn.host))
        .use_preconfigured_tls(config(conn))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(c)
}

/// The WebSocket stream lox's session parts work on.
pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Open the Miniserver's WebSocket (`/ws/rfc6455`), as lox's
/// `LoxWsClient::connect_raw` does it, with the certificate checked when
/// `verify_ssl` is set.
pub async fn ws_connect(conn: &Connection) -> Result<Ws, String> {
    let url = ws_url(&conn.host);
    let authority = url
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    let basic = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("{}:{}", conn.user, conn.pass),
    );
    let req = Request::builder()
        .uri(&url)
        .header("Authorization", format!("Basic {basic}"))
        .header("Host", authority)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .map_err(|e| format!("WS request: {e}"))?;
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(config(conn)));
    tokio_tungstenite::connect_async_tls_with_config(req, None, false, Some(connector))
        .await
        .map(|(ws, _)| ws)
        .map_err(|e| format!("WS connect: {}", crate::session::chain(&e)))
}

fn ws_url(host: &str) -> String {
    host.replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
        .trim_end_matches('/')
        .to_string()
        + "/ws/rfc6455"
}

/// The one-off listener that serves lox's login the Miniserver's key; it
/// stops when dropped.
pub struct KeyServer(tokio::task::JoinHandle<()>);

impl Drop for KeyServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The connection to give lox's login (`ws_authenticate`) for `conn`, and
/// what must live while it runs. Without `verify_ssl`, `conn` as it is.
/// With it, the key is fetched here (checked, no credentials), and lox gets
/// a loopback host that serves it: lox's own fetch, which sends the user and
/// password without checking the certificate, never leaves this host.
pub async fn login(conn: &Connection) -> Result<(Connection, Option<KeyServer>), String> {
    if !conn.verify_ssl {
        return Ok((conn.clone(), None));
    }
    let key_conn = Connection {
        verify_ssl: true,
        ..Connection::new(conn.host.clone(), "", "")
    };
    let r = client(&key_conn, KEY_TIMEOUT)?
        .get(key_conn.url("jdev/sys/getPublicKey"))
        .send()
        .await
        .map_err(|e| {
            format!(
                "cannot fetch the Miniserver's key: {}",
                crate::session::chain(&e)
            )
        })?;
    if !r.status().is_success() {
        return Err(format!(
            "cannot fetch the Miniserver's key: HTTP {}",
            r.status().as_u16()
        ));
    }
    let body = r.text().await.map_err(|e| e.to_string())?;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = l.local_addr().map_err(|e| e.to_string())?.port();
    let task = tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let ok = head
                    .lines()
                    .next()
                    .is_some_and(|l| l.starts_with("GET /jdev/sys/getPublicKey "));
                let reply = if ok {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
                         {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = s.write_all(reply.as_bytes()).await;
            });
        }
    });
    Ok((
        Connection {
            host: format!("http://127.0.0.1:{port}"),
            ..conn.clone()
        },
        Some(KeyServer(task)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_websocket_url_keeps_host_and_port() {
        assert_eq!(
            ws_url("https://192-168-1-47.504f94a0e454.dyndns.loxonecloud.com:443/"),
            "wss://192-168-1-47.504f94a0e454.dyndns.loxonecloud.com:443/ws/rfc6455"
        );
        assert_eq!(
            ws_url("http://127.0.0.1:8080"),
            "ws://127.0.0.1:8080/ws/rfc6455"
        );
    }

    /// A stand-in Miniserver (plain HTTP, so a client that checks
    /// certificates can talk to it) that keeps every request's head.
    async fn stand_in(seen: Arc<Mutex<Vec<String>>>) -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
                let body =
                    r#"{"LL":{"control":"dev/sys/getPublicKey","value":"KEY","Code":"200"}}"#;
                let _ = s
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });
        port
    }

    /// On a verified route, the request that carries lox's user and
    /// password (its key fetch) never reaches the network: the Miniserver
    /// sees one request for the key, without credentials, made by the
    /// checking client; lox's own request lands on the loopback listener.
    #[tokio::test]
    async fn no_credentials_go_out_unchecked_on_a_verified_route() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let port = stand_in(seen.clone()).await;
        let mut conn = Connection::new(format!("http://127.0.0.1:{port}"), "admin", "s3cret");
        conn.verify_ssl = true;
        let (for_lox, server) = tokio::time::timeout(Duration::from_secs(10), login(&conn))
            .await
            .expect("hard timeout")
            .unwrap();
        assert!(server.is_some());
        assert_ne!(
            for_lox.host, conn.host,
            "lox's login gets the loopback host"
        );
        assert!(for_lox.host.starts_with("http://127.0.0.1:"));
        assert!(for_lox.verify_ssl, "nothing is switched to unchecked");
        assert_eq!(
            (for_lox.user.as_str(), for_lox.pass.as_str()),
            ("admin", "s3cret")
        );
        // What lox's `fetch_public_key` does: GET with HTTP basic auth.
        let body = tokio::time::timeout(
            Duration::from_secs(10),
            reqwest::Client::new()
                .get(for_lox.url("/jdev/sys/getPublicKey"))
                .basic_auth(&for_lox.user, Some(&for_lox.pass))
                .send(),
        )
        .await
        .expect("hard timeout")
        .unwrap()
        .text()
        .await
        .unwrap();
        assert!(body.contains("KEY"), "{body}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            seen[0].starts_with("GET /jdev/sys/getPublicKey "),
            "{}",
            seen[0]
        );
        assert!(
            !seen[0].to_ascii_lowercase().contains("authorization"),
            "{}",
            seen[0]
        );
    }

    #[tokio::test]
    async fn the_key_server_serves_nothing_else() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let port = stand_in(seen).await;
        let mut conn = Connection::new(format!("http://127.0.0.1:{port}"), "u", "p");
        conn.verify_ssl = true;
        let (for_lox, _server) = login(&conn).await.unwrap();
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            reqwest::get(for_lox.url("/jdev/sps/io/x/on")),
        )
        .await
        .expect("hard timeout")
        .unwrap();
        assert_eq!(r.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn an_unverified_route_logs_in_as_lox_does() {
        let conn = Connection::new("https://192.168.1.120", "u", "p");
        let (c, server) = login(&conn).await.unwrap();
        assert_eq!(c, conn);
        assert!(server.is_none());
    }

    /// lox's client keeps `verify_ssl`: a request it would build for itself
    /// from the connection fails rather than go out unchecked.
    #[test]
    fn lox_s_client_keeps_the_verify_flag() {
        let mut conn = Connection::new("https://ms.example", "u", "p");
        conn.verify_ssl = true;
        let c = lox_client(&conn).unwrap();
        assert!(c.connection().verify_ssl);
    }

    /// Against a real Miniserver (no login: the key fetch carries no
    /// credentials): `VISLOX_TEST_MINISERVER=https://<clouddns name> cargo
    /// test real_key -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn real_key_fetch() {
        let mut conn = Connection::new(std::env::var("VISLOX_TEST_MINISERVER").unwrap(), "", "");
        conn.verify_ssl = true;
        let (for_lox, _server) = tokio::time::timeout(Duration::from_secs(15), login(&conn))
            .await
            .expect("hard timeout")
            .unwrap();
        let body = reqwest::get(for_lox.url("/jdev/sys/getPublicKey"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains("BEGIN CERTIFICATE"), "{body}");
    }
}
