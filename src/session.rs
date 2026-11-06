//! One live session with the Miniserver: connect, log in, subscribe to state
//! updates, and stream them until stopped or closed.
//!
//! This is `loxone_client::stream::stream_session`, built from the same
//! public parts, except that it keeps what that one drops: the close code
//! the Miniserver sends when it ends the connection (`4003` blocked, `4008`
//! no event slots, …), and the `Code` of a refused subscription. vislox uses
//! them to explain the failure and choose how long to wait (`reason.rs`).

use futures_util::{SinkExt, StreamExt};
use loxone_client::Connection;
use loxone_client::stream::{Framer, StateEvent, ws_authenticate};
use loxone_client::token::{WsStream, token_reuse_rejected};
use loxone_client::ws::LoxWsClient;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

/// Keepalive interval: the Miniserver drops sockets that send nothing for ~5 min.
const KEEPALIVE: Duration = Duration::from_secs(30);
/// No traffic at all (keepalive answers included) for this long: dead socket.
const DEAD_SOCKET: Duration = Duration::from_secs(90);
/// Connecting and logging in must finish within this.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How a session ended.
#[derive(Debug, PartialEq)]
pub enum Ended {
    /// Stopped by the caller.
    Stopped,
    /// The Miniserver closed the socket, with its close code if it sent one.
    Closed(Option<u16>),
    /// Something failed; the message, as for the user.
    Failed(String),
}

/// An error message with its causes, which `loxone_client` keeps out of
/// `Display` (`WS connect` hides `HTTP error: 503 …`).
fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let t = c.to_string();
        if !s.contains(&t) {
            s.push_str(": ");
            s.push_str(&t);
        }
        cur = c.source();
    }
    s
}

/// Connect, log in and subscribe to binary status updates.
async fn open(conn: &Connection) -> Result<WsStream, String> {
    let client = LoxWsClient::new(conn.clone());
    let mut ws = client.connect_raw().await.map_err(|e| chain(&e))?.0;
    let reuse_rejected_before = token_reuse_rejected();
    if let Err(e) = ws_authenticate(&mut ws, conn).await {
        // The stored token was just rejected and the fallback `gettoken`
        // failed on the same socket: retry once on a fresh connection, which
        // skips the token (as `loxone_client` does).
        if reuse_rejected_before || !token_reuse_rejected() {
            return Err(chain(&e));
        }
        ws = client.connect_raw().await.map_err(|e| chain(&e))?.0;
        ws_authenticate(&mut ws, conn)
            .await
            .map_err(|e| chain(&e))?;
    }
    ws.send(Message::Text("jdev/sps/enablebinstatusupdate".to_string()))
        .await
        .map_err(|e| chain(&e))?;
    Ok(ws)
}

/// A refused command's answer (`{"LL": {"Code": "…"}}` other than 200), as an
/// error message.
fn refused(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let ll = v.get("LL")?;
    let code = ll.get("Code").or_else(|| ll.get("code"))?;
    let code = code
        .as_str()
        .map(String::from)
        .or_else(|| code.as_i64().map(|n| n.to_string()))?;
    if code == "200" {
        return None;
    }
    let what = ll
        .get("control")
        .and_then(|c| c.as_str())
        .unwrap_or("command");
    Some(format!("{what} refused: Code {code}"))
}

/// Run one session. `on_live` is called once subscribed; `handler` gets each
/// batch of events and returns `false` to stop.
pub async fn run<F>(
    conn: &Connection,
    mut stop: watch::Receiver<bool>,
    on_live: impl FnOnce(),
    mut handler: F,
) -> Ended
where
    F: FnMut(Vec<StateEvent>) -> bool,
{
    let mut ws = match tokio::time::timeout(CONNECT_TIMEOUT, open(conn)).await {
        Err(_) => return Ended::Failed("connect timeout".into()),
        Ok(Err(e)) => return Ended::Failed(e),
        Ok(Ok(ws)) => ws,
    };
    on_live();
    let mut keepalive = tokio::time::interval(KEEPALIVE);
    keepalive.tick().await;
    let mut framer = Framer::default();
    let mut last_rx = tokio::time::Instant::now();
    loop {
        let msg = tokio::select! {
            _ = stop.changed() => {
                let _ = ws.close(None).await;
                return Ended::Stopped;
            }
            _ = keepalive.tick() => {
                if last_rx.elapsed() > DEAD_SOCKET {
                    return Ended::Failed("connection timed out".into());
                }
                if let Err(e) = ws.send(Message::Text("keepalive".to_string())).await {
                    return Ended::Failed(chain(&e));
                }
                continue;
            }
            m = ws.next() => m,
        };
        let msg = match msg {
            None => return Ended::Closed(None),
            Some(Err(e)) => return Ended::Failed(chain(&e)),
            Some(Ok(m)) => m,
        };
        last_rx = tokio::time::Instant::now();
        let events = match msg {
            Message::Binary(data) => match framer.binary(&data) {
                Ok(ev) => ev,
                Err(e) => return Ended::Failed(chain(&e)),
            },
            Message::Text(t) => {
                framer.text();
                if let Some(e) = refused(&t) {
                    return Ended::Failed(e);
                }
                continue;
            }
            Message::Ping(data) => {
                if let Err(e) = ws.send(Message::Pong(data)).await {
                    return Ended::Failed(chain(&e));
                }
                continue;
            }
            Message::Close(frame) => return Ended::Closed(frame.map(|f| u16::from(f.code))),
            _ => continue,
        };
        if !events.is_empty() && !handler(events) {
            return Ended::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_command_is_an_error() {
        assert_eq!(
            refused(
                r#"{"LL":{"control":"dev/sps/enablebinstatusupdate","value":"","Code":"4008"}}"#
            )
            .as_deref(),
            Some("dev/sps/enablebinstatusupdate refused: Code 4008")
        );
        assert_eq!(
            refused(r#"{"LL":{"control":"keepalive","Code":"200"}}"#),
            None
        );
        assert_eq!(
            refused(r#"{"LL":{"Code":503}}"#).as_deref(),
            Some("command refused: Code 503")
        );
        assert_eq!(refused("not json"), None);
    }
}
