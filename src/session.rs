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
use loxone_client::system::ll_code;
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

/// What one message from the Miniserver means for the session.
#[derive(Debug)]
enum Step {
    /// State events (possibly none, for half of a header + payload pair).
    Events(Vec<StateEvent>),
    /// A command was answered with `Code` 200: the subscription is on.
    Answered,
    /// A command was refused: what and its `Code`.
    Refused(String),
    /// Answer a ping.
    Pong(Vec<u8>),
    Closed(Option<u16>),
    Failed(String),
    Ignore,
}

fn step(framer: &mut Framer, msg: Message) -> Step {
    match msg {
        Message::Binary(data) => match framer.binary(&data) {
            Ok(ev) => Step::Events(ev),
            Err(e) => Step::Failed(chain(&e)),
        },
        Message::Text(t) => {
            framer.text();
            match ll_code(&t) {
                None => Step::Ignore,
                Some(200) => Step::Answered,
                Some(code) => {
                    let what = serde_json::from_str::<serde_json::Value>(&t)
                        .ok()
                        .and_then(|v| v.pointer("/LL/control")?.as_str().map(String::from))
                        .unwrap_or_else(|| "command".into());
                    Step::Refused(format!("{what} refused: Code {code}"))
                }
            }
        }
        Message::Ping(data) => Step::Pong(data),
        Message::Close(frame) => Step::Closed(frame.map(|f| u16::from(f.code))),
        _ => Step::Ignore,
    }
}

/// Run one session. `on_live` is called once the Miniserver has accepted
/// the subscription: when it answers it with `Code` 200 or sends the first
/// states, not when the request is sent (a refused subscription or a 4008
/// close never counts as live). `handler` gets each batch of events and
/// returns `false` to stop.
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
    let mut on_live = Some(on_live);
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
        let events = match step(&mut framer, msg) {
            Step::Events(ev) if ev.is_empty() => continue,
            Step::Events(ev) => ev,
            Step::Answered => {
                if let Some(f) = on_live.take() {
                    f();
                }
                continue;
            }
            Step::Refused(e) | Step::Failed(e) => return Ended::Failed(e),
            Step::Pong(data) => {
                if let Err(e) = ws.send(Message::Pong(data)).await {
                    return Ended::Failed(chain(&e));
                }
                continue;
            }
            Step::Closed(code) => return Ended::Closed(code),
            Step::Ignore => continue,
        };
        if let Some(f) = on_live.take() {
            f();
        }
        if !handler(events) {
            return Ended::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    fn text(t: &str) -> Step {
        step(&mut Framer::default(), Message::Text(t.to_string()))
    }

    #[test]
    fn only_an_accepted_subscription_counts_as_live() {
        assert!(matches!(
            text(r#"{"LL":{"control":"dev/sps/enablebinstatusupdate","value":"","Code":"200"}}"#),
            Step::Answered
        ));
        assert!(matches!(
            text(r#"{"LL":{"control":"dev/sps/enablebinstatusupdate","value":"","Code":"4008"}}"#),
            Step::Refused(e) if e == "dev/sps/enablebinstatusupdate refused: Code 4008"
        ));
        assert!(matches!(
            text(r#"{"LL":{"Code":503}}"#),
            Step::Refused(e) if e == "command refused: Code 503"
        ));
        assert!(matches!(text("not json"), Step::Ignore));
        // A close before any answer: the code, not "live".
        let close = Message::Close(Some(CloseFrame {
            code: CloseCode::from(4008),
            reason: "".into(),
        }));
        assert!(matches!(
            step(&mut Framer::default(), close),
            Step::Closed(Some(4008))
        ));
        // A header whose payload has not come yet: no events, so not live.
        let header = vec![0x03, 0x02, 0, 0, 24, 0, 0, 0];
        assert!(matches!(
            step(&mut Framer::default(), Message::Binary(header)),
            Step::Events(ev) if ev.is_empty()
        ));
    }
}
