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
//! vislox therefore builds the clients here, and hands lox's code only a
//! [`Connection`] without `verify_ssl` where lox makes a request of its own
//! (the RSA key fetch of the WebSocket login: [`for_lox`]).

use loxone_client::{Connection, LoxClient};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::http::Request;

/// lox's HTTP timeout, for the blocking client that replaces its own.
const LOX_TIMEOUT: Duration = Duration::from_secs(10);

/// rustls with the public web roots, on the ring provider lox uses.
pub fn verified_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// An async client for `conn`: the certificate is checked when
/// `verify_ssl` is set, and no redirect goes to another host (a Gen 2
/// Miniserver sends local HTTP to its CloudDNS name), as lox's own client.
pub fn client(conn: &Connection, timeout: Duration) -> Result<reqwest::Client, String> {
    let b = reqwest::Client::builder()
        .user_agent(loxone_client::client::USER_AGENT)
        .timeout(timeout)
        .redirect(LoxClient::same_origin_redirect_policy(&conn.host));
    let b = if conn.verify_ssl {
        b.use_preconfigured_tls(verified_config())
    } else {
        b.danger_accept_invalid_certs(true)
    };
    b.build().map_err(|e| e.to_string())
}

/// lox's HTTP client for `conn`, with a blocking client that checks the
/// certificate when `verify_ssl` is set (lox's own has no roots to check
/// it against). Must not be called on an async thread, as `LoxClient::new`.
pub fn lox_client(conn: &Connection) -> Result<LoxClient, String> {
    let mut c = LoxClient::new(for_lox(conn)).map_err(|e| e.to_string())?;
    if conn.verify_ssl {
        c.client = reqwest::blocking::Client::builder()
            .user_agent(loxone_client::client::USER_AGENT)
            .timeout(LOX_TIMEOUT)
            .redirect(LoxClient::same_origin_redirect_policy(&conn.host))
            .use_preconfigured_tls(verified_config())
            .build()
            .map_err(|e| e.to_string())?;
    }
    Ok(c)
}

/// `conn` for a request lox makes with a client of its own, which would
/// fail with `verify_ssl` (it has no roots).
pub fn for_lox(conn: &Connection) -> Connection {
    Connection {
        verify_ssl: false,
        ..conn.clone()
    }
}

/// The WebSocket stream lox's session parts work on.
pub type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Open the Miniserver's WebSocket (`/ws/rfc6455`) as lox's `LoxWsClient`
/// does, but check the certificate when `verify_ssl` is set.
pub async fn ws_connect(conn: &Connection) -> Result<Ws, String> {
    if !conn.verify_ssl {
        return loxone_client::ws::LoxWsClient::new(conn.clone())
            .connect_raw()
            .await
            .map(|(ws, _)| ws)
            .map_err(|e| crate::session::chain(&e));
    }
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
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(verified_config()));
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn lox_gets_no_verify_flag_it_cannot_honour() {
        let mut c = Connection::new("https://ms", "u", "p");
        c.verify_ssl = true;
        assert!(!for_lox(&c).verify_ssl);
        assert_eq!(for_lox(&c).host, "https://ms");
    }
}
