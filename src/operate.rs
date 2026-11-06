//! Operate virtual inputs from the canvas, as Loxone Config's Liveview lets
//! you click an input to watch the logic react.
//!
//! This is the one part of vislox that sends commands to the Miniserver, so
//! it is off unless vislox is started with `--operate` *and*
//! `--admin-token-file`, and live values come from the Miniserver itself.
//! Every request needs the admin token, and the browser asks the user to
//! confirm each command.
//!
//! Only virtual inputs (`VirtualIn`) of the deployed config can be operated:
//! they exist for testing. A digital one takes `pulse`, `on` or `off`, an
//! analog one a number (within its `MinVal`/`MaxVal` when it checks values).
//! Physical inputs, outputs and other blocks are refused.
//!
//! Commands are `jdev/sps/io/{uuid}/{cmd}` (Communicating with the
//! Miniserver, p. 14). A block with a visualization password takes them only
//! as `jdev/sps/ios/{hash}/{uuid}/{cmd}`: the password is first checked with
//! `jdev/sps/checkuservisupwd/{hash}` (p. 15), which triggers nothing, and
//! each hash uses a fresh single-use key from `jdev/sys/getvisusalt/{user}`.
//! The password is never stored or logged.
//!
//! Every command sent is logged (what, when, the Miniserver's answer) for
//! the History pane; a request that failed before its command went out is
//! recorded as not sent, with the reason. vislox has one shared admin token
//! and no user accounts, so the log names no person, and it shows neither
//! the browser's address nor the Miniserver user.

use axum::http::StatusCode;
use loxone_client::Connection;
use loxone_config::LoxoneDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// How many entries the log keeps.
const LOG_LEN: usize = 200;

/// The Miniserver, for one command: [`Port::open`] makes a session (one HTTP
/// client) that the GETs of that command share. Traits, so tests run against
/// a mock and never reach a Miniserver.
pub trait Port: Send + Sync {
    fn open(&self) -> Result<Box<dyn Session>, String>;
}

/// One GET without retries (a retry could send a command twice): the HTTP
/// status and the body. `mask` is (secret, what to show instead), to keep a
/// hash out of errors and logs.
pub trait Session {
    fn get(&self, path: &str, mask: Option<(&str, &str)>) -> Result<(u16, String), String>;
}

/// The Miniserver over HTTP, with the connection lox uses.
pub struct Http(pub Connection);

struct HttpSession(loxone_client::LoxClient);

impl Port for Http {
    /// Builds a blocking client: call it off the async runtime.
    fn open(&self) -> Result<Box<dyn Session>, String> {
        let c = crate::tls::lox_client(&self.0)?;
        Ok(Box::new(HttpSession(c)))
    }
}

impl Session for HttpSession {
    fn get(&self, path: &str, mask: Option<(&str, &str)>) -> Result<(u16, String), String> {
        self.0
            .get_once(&format!("/{path}"), mask)
            .map_err(|e| e.to_string())
    }
}

/// `s` as one path segment: everything but unreserved characters
/// percent-encoded (UTF-8), so a user name with a space, `/`, `#` or
/// non-ASCII letters stays one segment.
fn path_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A command for a virtual input.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Pulse,
    On,
    Off,
    Value(f64),
}

impl Cmd {
    /// `pulse`, `on`, `off`, or a number.
    pub fn parse(s: &str) -> Option<Cmd> {
        match s.trim() {
            "pulse" => Some(Cmd::Pulse),
            "on" => Some(Cmd::On),
            "off" => Some(Cmd::Off),
            t => t
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(Cmd::Value),
        }
    }

    /// The command as it goes into the path.
    pub fn text(&self) -> String {
        match self {
            Cmd::Pulse => "pulse".into(),
            Cmd::On => "on".into(),
            Cmd::Off => "off".into(),
            Cmd::Value(v) => format!("{v}"),
        }
    }
}

/// A virtual input of the deployed config.
#[derive(Debug, PartialEq)]
pub struct Input {
    pub title: String,
    pub analog: bool,
    /// The range the input accepts, when it checks values (`EnVal`).
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// The virtual inputs of a config, by UUID: all that can be operated.
pub type Inputs = HashMap<String, Input>;

/// Every virtual input (`VirtualIn`) in `doc`.
pub fn inputs(doc: &LoxoneDoc) -> Inputs {
    let idx = doc.index();
    idx.by_uuid
        .iter()
        .filter_map(|(uuid, path)| {
            let el = doc.element_at(path)?;
            if el.attr("Type") != Some("VirtualIn") {
                return None;
            }
            let num = |k: &str| el.attr(k).and_then(|v| v.trim().parse::<f64>().ok());
            let checks = el.attr("EnVal") == Some("true");
            let input = Input {
                title: el
                    .attr_decoded("Title")
                    .map(|t| t.into_owned())
                    .unwrap_or_default(),
                analog: el.attr("Analog") == Some("true"),
                min: num("MinVal").filter(|_| checks),
                max: num("MaxVal").filter(|_| checks),
            };
            Some((uuid.clone(), input))
        })
        .collect()
}

/// Whether `input` takes `cmd`.
pub fn check_cmd(input: &Input, cmd: &Cmd) -> Result<(), String> {
    match (input.analog, cmd) {
        (false, Cmd::Value(_)) => Err("a digital input takes pulse, on or off".into()),
        (true, Cmd::Value(v)) => {
            if input.min.is_some_and(|m| *v < m) || input.max.is_some_and(|m| *v > m) {
                Err(format!(
                    "{v} is outside the input's range ({} to {})",
                    input.min.map_or("…".into(), |m| m.to_string()),
                    input.max.map_or("…".into(), |m| m.to_string())
                ))
            } else {
                Ok(())
            }
        }
        (true, _) => Err("an analog input takes a number".into()),
        (false, _) => Ok(()),
    }
}

/// A Loxone UUID (`1ca807ba-0062-61d5-ffff450b2297409e`): hex and dashes.
fn plausible_uuid(s: &str) -> bool {
    (8..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// What a code means (pp. 14, 30). A 500 to a secured command comes after
/// the password was accepted, so it is not a wrong password.
fn code_text(code: u16, secured: bool) -> &'static str {
    match code {
        200 => "ok",
        401 => "unauthorized",
        403 => "not enough rights",
        404 => "unknown control or command",
        423 => "the user is disabled",
        500 if secured => {
            "refused by the block, though the password was accepted (the block may not take \
             this command, or the single-use key expired)"
        }
        500 => "refused: the block may need its visualization password",
        _ => "error",
    }
}

/// The `Code` and `value` of an `LL` answer, falling back on the HTTP status.
fn answer(status: u16, body: &str) -> (u16, Option<String>) {
    let code = loxone_client::system::ll_code(body).unwrap_or(status);
    (code, loxone_client::system::ll_value(body))
}

/// One command, for the log.
#[derive(Clone, Serialize, Debug)]
pub struct Entry {
    pub time: i64,
    pub uuid: String,
    pub title: String,
    pub cmd: String,
    pub secured: bool,
    /// The command went out to the Miniserver. When not, `note` says why
    /// (the visualization password's salt could not be had, no connection).
    pub sent: bool,
    /// The Miniserver's code (`200`), or none without an answer.
    pub code: Option<u16>,
    /// The value it answered (the docs call it unreliable: the state update
    /// that follows is what counts).
    pub value: Option<String>,
    /// What went wrong, or what the code means.
    pub note: String,
}

#[derive(Deserialize)]
pub struct Req {
    pub uuid: String,
    pub cmd: String,
    /// The visualization password, for a block that has one.
    #[serde(default)]
    pub visu_password: Option<String>,
}

/// A refused request.
#[derive(Debug)]
pub struct Fail {
    pub status: StatusCode,
    pub error: String,
    /// The Miniserver said the visualization password is wrong
    /// (`checkuservisupwd`): the browser forgets it. No other failure does.
    pub bad_password: bool,
}

impl Fail {
    fn new(status: StatusCode, error: impl Into<String>) -> Fail {
        Fail {
            status,
            error: error.into(),
            bad_password: false,
        }
    }
}

/// A request that passed every check: what to send, to which input.
struct Checked {
    uuid: String,
    title: String,
    cmd: Cmd,
    pw: Option<String>,
}

pub struct Settings {
    /// `None`: on. Else why not.
    pub off: Option<String>,
    /// The Miniserver user commands run as (for `getvisusalt`).
    pub user: String,
}

pub struct Operate {
    s: Settings,
    port: Option<Arc<dyn Port>>,
    log: Mutex<Vec<Entry>>,
    tx: broadcast::Sender<()>,
    /// The inputs of the deployed revision, by its blob id: built once per
    /// revision, not per command.
    deployed: Mutex<Option<(String, Arc<Inputs>)>>,
    /// Each `getvisusalt` key is single-use and the next one replaces it, so
    /// secured commands go one at a time (salt, check, salt, command).
    secured_lock: Mutex<()>,
}

impl Operate {
    pub fn new(s: Settings, port: Option<Arc<dyn Port>>) -> Operate {
        let (tx, _) = broadcast::channel(16);
        let s = Settings {
            off: s
                .off
                .or_else(|| port.is_none().then(|| "no Miniserver connection".into())),
            ..s
        };
        Operate {
            s,
            port,
            log: Mutex::new(Vec::new()),
            tx,
            deployed: Mutex::new(None),
            secured_lock: Mutex::new(()),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    /// For `/api/history` (no token needed): whether inputs can be
    /// operated, and the log (newest first). Nothing about who.
    pub fn info(&self) -> Value {
        let log: Vec<Entry> = self.log.lock().unwrap().iter().rev().cloned().collect();
        match &self.s.off {
            None => json!({ "available": true, "log": log }),
            Some(why) => json!({ "available": false, "reason": why, "log": log }),
        }
    }

    /// The inputs of the deployed revision `blob`; `load` parses it the first
    /// time. Blocking.
    pub fn inputs_of(
        &self,
        blob: &str,
        load: impl FnOnce() -> Result<LoxoneDoc, String>,
    ) -> Result<Arc<Inputs>, String> {
        if let Some((b, i)) = &*self.deployed.lock().unwrap()
            && b == blob
        {
            return Ok(i.clone());
        }
        let i = Arc::new(inputs(&load()?));
        *self.deployed.lock().unwrap() = Some((blob.to_string(), i.clone()));
        Ok(i)
    }

    /// The request, checked against the deployed inputs: a UUID, a known
    /// command, a virtual input, a command that fits it.
    fn check(&self, req: Req, inputs: &Inputs) -> Result<Checked, Fail> {
        if let Some(why) = &self.s.off {
            return Err(Fail::new(
                StatusCode::FORBIDDEN,
                format!("operating inputs is off: {why}"),
            ));
        }
        let bad = |m: String| Fail::new(StatusCode::BAD_REQUEST, m);
        if !plausible_uuid(&req.uuid) {
            return Err(bad("not a UUID".into()));
        }
        let cmd =
            Cmd::parse(&req.cmd).ok_or_else(|| bad(format!("unknown command `{}`", req.cmd)))?;
        let input = inputs.get(&req.uuid).ok_or_else(|| {
            bad("only virtual inputs of the deployed config can be operated".into())
        })?;
        check_cmd(input, &cmd).map_err(bad)?;
        Ok(Checked {
            uuid: req.uuid,
            title: input.title.clone(),
            cmd,
            pw: req.visu_password.filter(|p| !p.is_empty()),
        })
    }

    /// Check the request (400 when it does not fit), then `preflight` (that
    /// the Miniserver at the address is the config's), then send it. The
    /// caller has checked the admin token.
    pub async fn run<F, Fut>(
        self: &Arc<Self>,
        req: Req,
        inputs: &Inputs,
        preflight: F,
    ) -> Result<Entry, Fail>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let c = self.check(req, inputs)?;
        if let Err(why) = preflight().await {
            return Err(Fail::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Cannot operate: {why}"),
            ));
        }
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.send(&c))
            .await
            .map_err(|e| Fail::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    }

    /// Send a checked command and log it. Blocking.
    fn send(&self, c: &Checked) -> Result<Entry, Fail> {
        let mut entry = Entry {
            time: chrono::Utc::now().timestamp(),
            uuid: c.uuid.clone(),
            title: c.title.clone(),
            cmd: c.cmd.text(),
            secured: c.pw.is_some(),
            sent: false,
            code: None,
            value: None,
            note: String::new(),
        };
        let res = match self.port.as_ref().ok_or("no Miniserver connection") {
            Err(e) => Err(Unsent::Other(e.to_string())),
            Ok(port) => match port.open() {
                Err(e) => Err(Unsent::Other(format!("cannot connect: {e}"))),
                Ok(session) => match &c.pw {
                    None => {
                        Ok(session.get(&format!("jdev/sps/io/{}/{}", c.uuid, c.cmd.text()), None))
                    }
                    Some(pw) => {
                        let _one = self.secured_lock.lock().unwrap();
                        self.secured(session.as_ref(), pw, &c.uuid, &c.cmd.text())
                    }
                },
            },
        };
        let out = match res {
            // The command went out.
            Ok(r) => {
                entry.sent = true;
                match r {
                    Ok((status, body)) => {
                        let (code, value) = answer(status, &body);
                        entry.code = Some(code);
                        entry.value = value;
                        entry.note = code_text(code, entry.secured).into();
                    }
                    Err(e) => entry.note = format!("no answer: {e}"),
                }
                Ok(entry.clone())
            }
            Err(u) => {
                let (bad_password, why) = match u {
                    Unsent::BadPassword(w) => (true, w),
                    Unsent::Other(w) => (false, w),
                };
                entry.note = why.clone();
                Err(Fail {
                    status: if bad_password {
                        StatusCode::FORBIDDEN
                    } else {
                        StatusCode::BAD_GATEWAY
                    },
                    error: format!("not sent: {why}"),
                    bad_password,
                })
            }
        };
        self.record(entry);
        out
    }

    fn record(&self, entry: Entry) {
        {
            let mut log = self.log.lock().unwrap();
            log.push(entry);
            let n = log.len();
            if n > LOG_LEN {
                log.drain(..n - LOG_LEN);
            }
        }
        let _ = self.tx.send(());
    }

    /// A fresh hash of the visualization password.
    fn visu_hash(&self, s: &dyn Session, pw: &str) -> Result<String, String> {
        let (status, body) = s.get(
            &format!("jdev/sys/getvisusalt/{}", path_segment(&self.s.user)),
            None,
        )?;
        let v: Value = serde_json::from_str(body.trim())
            .map_err(|_| format!("getvisusalt: unexpected answer (HTTP {status})"))?;
        let code = loxone_client::system::code_of(&v).unwrap_or(status);
        if code != 200 {
            return Err(format!(
                "getvisusalt: code {code} ({})",
                code_text(code, false)
            ));
        }
        let salt = match &v["LL"]["value"] {
            // Some firmware sends the object as a JSON string.
            Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
            other => other.clone(),
        };
        loxone_client::token::visu_hash(pw, &salt).map_err(|e| e.to_string())
    }

    /// Check the password without triggering anything, then send the
    /// secured command. `Err`: the command did not go out; `Ok`: it did,
    /// with its answer or why there was none. Hold `secured_lock`.
    fn secured(
        &self,
        s: &dyn Session,
        pw: &str,
        uuid: &str,
        cmd: &str,
    ) -> Result<Result<(u16, String), String>, Unsent> {
        let h = self.visu_hash(s, pw).map_err(Unsent::Other)?;
        let (status, body) = s
            .get(
                &format!("jdev/sps/checkuservisupwd/{h}"),
                Some((&h, "<hash>")),
            )
            .map_err(|e| Unsent::Other(format!("checking the password: {e}")))?;
        match answer(status, &body).0 {
            200 => {}
            500 => {
                return Err(Unsent::BadPassword(
                    "the Miniserver refused the visualization password".into(),
                ));
            }
            code => {
                return Err(Unsent::Other(format!(
                    "checking the password: code {code} ({})",
                    code_text(code, false)
                )));
            }
        }
        let h = self.visu_hash(s, pw).map_err(Unsent::Other)?;
        Ok(s.get(
            &format!("jdev/sps/ios/{h}/{uuid}/{cmd}"),
            Some((&h, "<hash>")),
        ))
    }
}

/// Why a command did not go out.
enum Unsent {
    BadPassword(String),
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const DOC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ControlList>
<C Type="Document" V="178" U="d" Title="T">
  <C Type="VirtualInCaption" U="cap" Title="VI">
    <C Type="VirtualIn" U="aaaa0001-0000-0010-ffff504f94000001" Title="Test button" Analog="false" MaxVal="1"><Co K="Q" U="q1"/></C>
    <C Type="VirtualIn" U="aaaa0002-0000-0010-ffff504f94000001" Title="Test temp" Analog="true" EnVal="true" MinVal="-50" MaxVal="100"><Co K="Q" U="q2"/></C>
  </C>
  <C Type="Page" U="p" Title="P">
    <C Type="And" U="aaaa0003-0000-0010-ffff504f94000001" Title="AND"/>
  </C>
</C>
</ControlList>"#;
    const BUTTON: &str = "aaaa0001-0000-0010-ffff504f94000001";
    const TEMP: &str = "aaaa0002-0000-0010-ffff504f94000001";
    const GATE: &str = "aaaa0003-0000-0010-ffff504f94000001";
    const PW: &str = "secret";

    fn deployed() -> Inputs {
        inputs(&LoxoneDoc::parse(DOC.as_bytes()).unwrap())
    }

    type Script = Box<dyn Fn(&str) -> (u16, String) + Send + Sync>;

    /// A stand-in Miniserver: records every path, answers from a script.
    /// With `keys`, it acts like the real one for secured commands: each
    /// `getvisusalt` hands out a new key that replaces the last one, and a
    /// hash is accepted once, only when made from the newest key.
    struct Mock {
        sent: Mutex<Vec<String>>,
        answer: Script,
        keys: bool,
        issued: AtomicUsize,
        valid: Mutex<Option<String>>,
        opened: AtomicUsize,
        /// Slows every GET down, so concurrent commands interleave.
        delay: Duration,
    }

    struct MockSession(Arc<Mock>);

    impl Port for Arc<Mock> {
        fn open(&self) -> Result<Box<dyn Session>, String> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(MockSession(self.clone())))
        }
    }

    impl Session for MockSession {
        fn get(&self, path: &str, _: Option<(&str, &str)>) -> Result<(u16, String), String> {
            let m = &self.0;
            m.sent.lock().unwrap().push(path.to_string());
            std::thread::sleep(m.delay);
            if m.keys {
                if path.contains("getvisusalt") {
                    let n = m.issued.fetch_add(1, Ordering::SeqCst);
                    let key = hex::encode(format!("key{n}"));
                    let salt = json!({ "key": key, "salt": "abcd", "hashAlg": "SHA256" });
                    *m.valid.lock().unwrap() =
                        Some(loxone_client::token::visu_hash(PW, &salt).unwrap());
                    let v = json!({ "LL": { "control": path, "Code": "200", "value": salt } });
                    return Ok((200, v.to_string()));
                }
                if path.contains("checkuservisupwd") || path.contains("/sps/ios/") {
                    let hash = path.split('/').nth(3).unwrap_or_default().to_string();
                    let ok = m.valid.lock().unwrap().take().as_deref() == Some(hash.as_str());
                    return Ok((200, ll(path, if ok { 200 } else { 500 }, "1")));
                }
            }
            Ok((m.answer)(path))
        }
    }

    fn ll(control: &str, code: u16, value: &str) -> String {
        json!({ "LL": { "control": control, "value": value, "Code": code.to_string() } })
            .to_string()
    }

    fn mock(answer: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> Arc<Mock> {
        Arc::new(Mock {
            sent: Mutex::new(Vec::new()),
            answer: Box::new(answer),
            keys: false,
            issued: AtomicUsize::new(0),
            valid: Mutex::new(None),
            opened: AtomicUsize::new(0),
            delay: Duration::ZERO,
        })
    }

    fn keyed(delay: Duration) -> Arc<Mock> {
        let mut m = mock(ok);
        Arc::get_mut(&mut m).unwrap().keys = true;
        Arc::get_mut(&mut m).unwrap().delay = delay;
        m
    }

    fn operate(off: Option<&str>, m: &Arc<Mock>, user: &str) -> Arc<Operate> {
        Arc::new(Operate::new(
            Settings {
                off: off.map(String::from),
                user: user.into(),
            },
            Some(Arc::new(m.clone()) as Arc<dyn Port>),
        ))
    }

    fn req(uuid: &str, cmd: &str, pw: Option<&str>) -> Req {
        Req {
            uuid: uuid.into(),
            cmd: cmd.into(),
            visu_password: pw.map(String::from),
        }
    }

    fn ok(path: &str) -> (u16, String) {
        (200, ll(path, 200, "1"))
    }

    async fn pass() -> Result<(), String> {
        Ok(())
    }

    fn log(op: &Operate) -> Vec<Value> {
        op.info()["log"].as_array().unwrap().clone()
    }

    #[test]
    fn commands_parse() {
        assert_eq!(Cmd::parse("pulse"), Some(Cmd::Pulse));
        assert_eq!(Cmd::parse(" on "), Some(Cmd::On));
        assert_eq!(Cmd::parse("21.5"), Some(Cmd::Value(21.5)));
        assert_eq!(Cmd::Value(1.0).text(), "1");
        assert_eq!(Cmd::Value(-3.25).text(), "-3.25");
        assert_eq!(Cmd::parse("NaN"), None);
        assert_eq!(Cmd::parse("inf"), None);
        assert_eq!(Cmd::parse("on/../../reboot"), None);
        assert_eq!(Cmd::parse(""), None);
    }

    #[test]
    fn only_virtual_inputs_with_fitting_commands() {
        let i = deployed();
        assert_eq!(i.len(), 2, "the gate is not an input");
        let b = &i[BUTTON];
        assert_eq!(b.title, "Test button");
        assert!(!b.analog);
        assert_eq!(b.max, None, "no range check without EnVal");
        let t = &i[TEMP];
        assert!(t.analog);
        assert_eq!((t.min, t.max), (Some(-50.0), Some(100.0)));

        assert!(check_cmd(b, &Cmd::Pulse).is_ok());
        assert!(check_cmd(b, &Cmd::Off).is_ok());
        assert!(check_cmd(b, &Cmd::Value(1.0)).is_err());
        assert!(check_cmd(t, &Cmd::Value(21.5)).is_ok());
        assert!(check_cmd(t, &Cmd::Value(101.0)).is_err());
        assert!(check_cmd(t, &Cmd::Value(-51.0)).is_err());
        assert!(check_cmd(t, &Cmd::On).is_err());
    }

    #[test]
    fn the_deployed_inputs_are_parsed_once_per_revision() {
        let op = operate(None, &mock(ok), "u");
        let loads = AtomicUsize::new(0);
        let load = || {
            loads.fetch_add(1, Ordering::SeqCst);
            LoxoneDoc::parse(DOC.as_bytes()).map_err(|e| e.to_string())
        };
        assert_eq!(op.inputs_of("blob1", load).unwrap().len(), 2);
        assert_eq!(op.inputs_of("blob1", load).unwrap().len(), 2);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        op.inputs_of("blob2", load).unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn user_names_are_one_path_segment() {
        assert_eq!(path_segment("admin"), "admin");
        assert_eq!(path_segment("a b/c#d"), "a%20b%2Fc%23d");
        assert_eq!(path_segment("Łukasz"), "%C5%81ukasz");
    }

    #[tokio::test]
    async fn off_sends_nothing() {
        let m = mock(ok);
        let op = operate(Some("start vislox with --operate"), &m, "u");
        let e = op
            .run(req(BUTTON, "pulse", None), &deployed(), pass)
            .await
            .unwrap_err();
        assert_eq!(e.status, StatusCode::FORBIDDEN);
        assert!(m.sent.lock().unwrap().is_empty());
        assert_eq!(op.info()["available"], false);

        let op = Operate::new(
            Settings {
                off: None,
                user: "u".into(),
            },
            None,
        );
        assert_eq!(op.info()["reason"], "no Miniserver connection");
    }

    /// Item 8 of the review: a request that does not fit is refused (400)
    /// before the preflight runs, and nothing is sent.
    #[tokio::test]
    async fn requests_are_checked_before_the_preflight() {
        let m = mock(ok);
        let op = operate(None, &m, "u");
        let checked = AtomicBool::new(false);
        for (uuid, cmd) in [
            (GATE, "on"),
            (BUTTON, "42"),
            (TEMP, "pulse"),
            (TEMP, "1000"),
            ("../../dev/sys/reboot", "on"),
            (BUTTON, "reboot"),
        ] {
            let e = op
                .run(req(uuid, cmd, None), &deployed(), || async {
                    checked.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .await
                .unwrap_err();
            assert_eq!(e.status, StatusCode::BAD_REQUEST, "{uuid} {cmd}");
        }
        assert!(
            !checked.load(Ordering::SeqCst),
            "no preflight for a bad request"
        );
        assert!(m.sent.lock().unwrap().is_empty());
        assert!(log(&op).is_empty());
    }

    #[tokio::test]
    async fn a_failed_preflight_sends_nothing() {
        let m = mock(ok);
        let op = operate(None, &m, "u");
        let e = op
            .run(req(BUTTON, "on", None), &deployed(), || async {
                Err("this is not the Miniserver of this config".to_string())
            })
            .await
            .unwrap_err();
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(e.error.contains("not the Miniserver"));
        assert!(m.sent.lock().unwrap().is_empty());
        assert!(log(&op).is_empty());
    }

    #[tokio::test]
    async fn a_command_is_sent_once_over_one_session_and_logged() {
        let m = mock(ok);
        let op = operate(None, &m, "tester");
        let mut rx = op.subscribe();
        let e = op
            .run(req(TEMP, "21.5", None), &deployed(), pass)
            .await
            .unwrap();
        assert_eq!(
            *m.sent.lock().unwrap(),
            vec![format!("jdev/sps/io/{TEMP}/21.5")]
        );
        assert_eq!(m.opened.load(Ordering::SeqCst), 1);
        assert!(e.sent);
        assert_eq!(e.code, Some(200));
        assert_eq!(e.value.as_deref(), Some("1"));
        assert_eq!(e.note, "ok");
        assert!(rx.try_recv().is_ok());
        let info = op.info();
        assert_eq!(info["log"][0]["title"], "Test temp");
        assert_eq!(info["log"][0]["cmd"], "21.5");
        // Read without a token: neither the user nor an address.
        assert!(!info.to_string().contains("tester"));
        assert!(info["log"][0].get("who").is_none());
    }

    #[tokio::test]
    async fn codes_are_reported() {
        for (code, note) in [(403, "not enough rights"), (423, "the user is disabled")] {
            let op = operate(None, &mock(move |p| (200, ll(p, code, "0"))), "u");
            let e = op
                .run(req(BUTTON, "on", None), &deployed(), pass)
                .await
                .unwrap();
            assert!(e.sent);
            assert_eq!((e.code, e.note.as_str()), (Some(code), note));
        }
    }

    #[tokio::test]
    async fn a_secured_command_checks_the_password_first() {
        let m = keyed(Duration::ZERO);
        let op = operate(None, &m, "a b/c");
        let e = op
            .run(req(BUTTON, "pulse", Some(PW)), &deployed(), pass)
            .await
            .unwrap();
        assert!(e.secured && e.sent);
        assert_eq!(e.code, Some(200));
        let sent = m.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 4, "{sent:?}");
        assert_eq!(sent[0], "jdev/sys/getvisusalt/a%20b%2Fc");
        assert!(sent[1].starts_with("jdev/sps/checkuservisupwd/"));
        assert_eq!(sent[2], sent[0]);
        assert!(
            sent[3].starts_with("jdev/sps/ios/") && sent[3].ends_with(&format!("/{BUTTON}/pulse"))
        );
        assert_eq!(m.opened.load(Ordering::SeqCst), 1, "one client per command");
        assert!(!sent.iter().any(|s| s.contains(PW)));
        assert!(!op.info().to_string().contains(PW));
    }

    /// Item 1 of the review: two secured commands at once would replace
    /// each other's single-use key; they go one at a time and both pass.
    #[tokio::test]
    async fn concurrent_secured_commands_do_not_spoil_each_others_key() {
        let m = keyed(Duration::from_millis(20));
        let op = operate(None, &m, "u");
        let i = deployed();
        let (a, b) = tokio::join!(
            op.run(req(BUTTON, "on", Some(PW)), &i, pass),
            op.run(req(BUTTON, "off", Some(PW)), &i, pass),
        );
        for r in [a, b] {
            let e = r.unwrap();
            assert_eq!((e.sent, e.code), (true, Some(200)), "{}", e.note);
        }
        // Salt, check, salt, command; then the same for the other.
        let sent = m.sent.lock().unwrap().clone();
        let kinds: Vec<&str> = sent
            .iter()
            .map(|p| p.split('/').nth(2).unwrap_or_default())
            .collect();
        assert_eq!(
            kinds,
            [
                "getvisusalt",
                "checkuservisupwd",
                "getvisusalt",
                "ios",
                "getvisusalt",
                "checkuservisupwd",
                "getvisusalt",
                "ios"
            ]
        );
    }

    #[tokio::test]
    async fn a_wrong_password_sends_no_command() {
        let m = keyed(Duration::ZERO);
        let op = operate(None, &m, "u");
        let e = op
            .run(req(BUTTON, "pulse", Some("wrong")), &deployed(), pass)
            .await
            .unwrap_err();
        assert_eq!(e.status, StatusCode::FORBIDDEN);
        assert!(e.bad_password);
        assert!(e.error.contains("visualization password"));
        let sent = m.sent.lock().unwrap().clone();
        assert!(
            !sent
                .iter()
                .any(|s| s.contains("/sps/ios/") || s.contains("/sps/io/"))
        );
        let l = log(&op);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0]["sent"], false);
    }

    /// Item 5 of the review: a 500 to the command itself, after the password
    /// was accepted, is not a wrong password.
    #[tokio::test]
    async fn a_refused_secured_command_is_not_a_wrong_password() {
        // Spend the key between the check and the command: the command's
        // hash no longer matches, and the Miniserver answers 500.
        let m = keyed(Duration::ZERO);
        let e = {
            struct Spoil(Arc<Mock>);
            impl Port for Spoil {
                fn open(&self) -> Result<Box<dyn Session>, String> {
                    struct S(Arc<Mock>);
                    impl Session for S {
                        fn get(
                            &self,
                            path: &str,
                            mask: Option<(&str, &str)>,
                        ) -> Result<(u16, String), String> {
                            if path.contains("/sps/ios/") {
                                self.0.valid.lock().unwrap().take();
                            }
                            MockSession(self.0.clone()).get(path, mask)
                        }
                    }
                    Ok(Box::new(S(self.0.clone())))
                }
            }
            let op = Arc::new(Operate::new(
                Settings {
                    off: None,
                    user: "u".into(),
                },
                Some(Arc::new(Spoil(m.clone())) as Arc<dyn Port>),
            ));
            op.run(req(BUTTON, "pulse", Some(PW)), &deployed(), pass)
                .await
                .unwrap()
        };
        assert!(e.sent);
        assert_eq!(e.code, Some(500));
        assert!(e.note.contains("password was accepted"), "{}", e.note);
    }

    /// Item 2 of the review: a failure before the command went out is
    /// recorded as not sent, with the reason.
    #[tokio::test]
    async fn failures_before_the_command_are_not_sent() {
        for (answer, why) in [
            (
                Box::new(|p: &str| (401u16, ll(p, 401, ""))) as Script,
                "getvisusalt: code 401",
            ),
            (
                Box::new(|_: &str| (200u16, "<html>".to_string())) as Script,
                "unexpected answer",
            ),
        ] {
            let m = Arc::new(Mock {
                sent: Mutex::new(Vec::new()),
                answer,
                keys: false,
                issued: AtomicUsize::new(0),
                valid: Mutex::new(None),
                opened: AtomicUsize::new(0),
                delay: Duration::ZERO,
            });
            let op = operate(None, &m, "u");
            let e = op
                .run(req(BUTTON, "on", Some(PW)), &deployed(), pass)
                .await
                .unwrap_err();
            assert!(!e.bad_password, "{why}: the password may be right");
            assert!(e.error.starts_with("not sent: "), "{}", e.error);
            assert!(e.error.contains(why), "{}", e.error);
            let l = log(&op);
            assert_eq!(l.len(), 1);
            assert_eq!(l[0]["sent"], false);
            assert_eq!(l[0]["code"], Value::Null);
            assert!(!m.sent.lock().unwrap().iter().any(|s| s.contains("/sps/io")));
        }

        struct Down;
        impl Port for Down {
            fn open(&self) -> Result<Box<dyn Session>, String> {
                Err("connection refused".into())
            }
        }
        let op = Arc::new(Operate::new(
            Settings {
                off: None,
                user: "u".into(),
            },
            Some(Arc::new(Down) as Arc<dyn Port>),
        ));
        let e = op
            .run(req(BUTTON, "on", None), &deployed(), pass)
            .await
            .unwrap_err();
        assert!(e.error.contains("connection refused"));
        assert_eq!(log(&op)[0]["sent"], false);
    }

    #[test]
    fn answers_and_uuids() {
        assert_eq!(answer(200, &ll("x", 200, "1")), (200, Some("1".into())));
        assert_eq!(answer(401, "Unauthorized").0, 401);
        assert!(plausible_uuid(BUTTON));
        assert!(!plausible_uuid("a/b"));
        assert!(!plausible_uuid("x"));
    }
}
