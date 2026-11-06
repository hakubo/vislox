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
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

/// Keepalive interval: the Miniserver drops sockets that send nothing for ~5 min.
const KEEPALIVE: Duration = Duration::from_secs(30);
/// No traffic at all (keepalive answers included) for this long: dead socket.
const DEAD_SOCKET: Duration = Duration::from_secs(90);
/// Connecting and logging in must finish within this.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A read-only request to send on the session's socket (a block query,
/// `jdev/sps/io/<uuid>/all`, or one of [`COMMANDS`]), and where its answer
/// goes: the `LL` object of the Miniserver's reply.
pub struct Query {
    pub cmd: String,
    /// What the answer's `control` contains.
    key: String,
    pub reply: oneshot::Sender<serde_json::Value>,
}

/// The commands other than block queries that a [`Query`] may send: the
/// heating and cooling periods.
const COMMANDS: &[&str] = &[crate::calendar::HEAT_PERIOD, crate::calendar::COOL_PERIOD];

/// What a command's answer names in its `control`: its path without the
/// leading `j` (`dev/sps/…` for `jdev/sps/…`).
fn control_of(cmd: &str) -> &str {
    cmd.strip_prefix('j').unwrap_or(cmd)
}

/// Whether an answer's `control` (or a whole message) is the answer to a
/// [`Query`], which gets it whatever its `Code`: a block query's, or one of
/// [`COMMANDS`].
fn is_asked(control: &str) -> bool {
    control.contains("sps/io/") || COMMANDS.iter().any(|c| control.contains(control_of(c)))
}

impl Query {
    /// The outputs of a block.
    pub fn io(uuid: &str, reply: oneshot::Sender<serde_json::Value>) -> Query {
        Query {
            cmd: crate::query::command(uuid),
            key: uuid.to_string(),
            reply,
        }
    }

    /// One of [`COMMANDS`]; panics for any other, whose answer would not
    /// come back to it (and whose refusal would end the session).
    pub fn command(cmd: &str, reply: oneshot::Sender<serde_json::Value>) -> Query {
        assert!(COMMANDS.contains(&cmd), "{cmd} is not a query command");
        Query {
            cmd: cmd.to_string(),
            key: control_of(cmd).to_string(),
            reply,
        }
    }
}

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
    /// The answer to a query (any `Code`: a block that cannot be queried
    /// does not end the session): its `LL` object.
    Reply(serde_json::Value),
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
            if let Some(ll) = asked_reply(&t) {
                return Step::Reply(ll);
            }
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

/// The `LL` object of an answer to a query ([`is_asked`]).
fn asked_reply(t: &str) -> Option<serde_json::Value> {
    if !is_asked(t) {
        return None;
    }
    let mut v = serde_json::from_str::<serde_json::Value>(t).ok()?;
    let ll = v.get_mut("LL")?.take();
    is_asked(ll["control"].as_str()?).then_some(ll)
}

/// Run one session. `on_live` is called once the Miniserver has accepted
/// the subscription: when it answers it with `Code` 200 or sends the first
/// states, not when the request is sent (a refused subscription or a 4008
/// close never counts as live). `handler` gets each batch of events and
/// returns `false` to stop. `queries` are sent on the same socket once
/// the session is open, and answered in turn.
pub async fn run<F>(
    conn: &Connection,
    mut stop: watch::Receiver<bool>,
    on_live: impl FnOnce(),
    mut handler: F,
    mut queries: mpsc::Receiver<Query>,
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
    // Sent queries waiting for their answer.
    let mut asked: VecDeque<Query> = VecDeque::new();
    loop {
        let msg = tokio::select! {
            _ = stop.changed() => {
                let _ = ws.close(None).await;
                return Ended::Stopped;
            }
            Some(q) = queries.recv() => {
                // A caller that gave up waiting no longer needs its answer.
                asked.retain(|q| !q.reply.is_closed());
                if let Err(e) = ws.send(Message::Text(q.cmd.clone())).await {
                    return Ended::Failed(chain(&e));
                }
                asked.push_back(q);
                continue;
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
            Step::Reply(ll) => {
                let control = ll["control"].as_str().unwrap_or_default();
                if let Some(i) = asked.iter().position(|q| control.contains(&q.key)) {
                    let q = asked.remove(i).expect("position is in range");
                    let _ = q.reply.send(ll);
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
    fn a_command_query_is_matched_by_its_control() {
        let (reply, _) = oneshot::channel();
        let q = Query::command(crate::calendar::COOL_PERIOD, reply);
        assert_eq!(q.cmd, "jdev/sps/calendargetcoolperiod");
        assert_eq!(q.key, "dev/sps/calendargetcoolperiod");
        assert!(is_asked("dev/sps/calendargetheatperiod"));
        assert!(is_asked("dev/sps/io/0f1e/all"));
        assert!(!is_asked("dev/sps/enablebinstatusupdate"));
        assert!(!is_asked("dev/sps/calendargetentries"));
    }

    #[test]
    #[should_panic(expected = "is not a query command")]
    fn another_command_is_refused() {
        let (reply, _) = oneshot::channel();
        Query::command("jdev/sps/calendargetentries", reply);
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
        // A block query's answer, even a refusal, is the querier's.
        assert!(matches!(
            text(r#"{"LL":{"control":"dev/sps/io/b1/all","value":"","Code":"500"}}"#),
            Step::Reply(ll) if ll["Code"] == "500"
        ));
        // So is the answer to the heating period's request.
        assert!(matches!(
            text(r#"{"LL": { "control": "dev/sps/calendargetheatperiod", "value": "10-01/04-30", "Code": "200"}}"#),
            Step::Reply(ll) if ll["value"] == "10-01/04-30"
        ));
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
