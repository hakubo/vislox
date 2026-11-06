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
//! Every command sent is logged (who, what, when, the Miniserver's answer)
//! for the History pane.

use axum::http::StatusCode;
use loxone_client::Connection;
use loxone_config::LoxoneDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use crate::actions::Refusal;

/// How many commands the log keeps.
const LOG_LEN: usize = 200;

/// One GET to the Miniserver, without retries (a retry could send a command
/// twice): the HTTP status and the body. `mask` is (secret, what to show
/// instead), to keep a hash out of errors and logs. A trait, so tests run
/// against a mock and never reach a Miniserver.
pub trait Port: Send + Sync {
    fn get(&self, path: &str, mask: Option<(&str, &str)>) -> Result<(u16, String), String>;
}

/// The Miniserver over HTTP, with the connection lox uses.
pub struct Http(pub Connection);

impl Port for Http {
    fn get(&self, path: &str, mask: Option<(&str, &str)>) -> Result<(u16, String), String> {
        let c = loxone_client::LoxClient::new(self.0.clone()).map_err(|e| e.to_string())?;
        c.get_once(&format!("/{path}"), mask)
            .map_err(|e| e.to_string())
    }
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

/// The virtual input `uuid` in `doc`, or why it cannot be operated.
pub fn find_input(doc: &LoxoneDoc, uuid: &str) -> Result<Input, String> {
    let idx = doc.index();
    let el = idx
        .by_uuid
        .get(uuid)
        .and_then(|p| doc.element_at(p))
        .ok_or("no such object in the deployed config")?;
    let ty = el.attr("Type").unwrap_or_default();
    if ty != "VirtualIn" {
        return Err(format!(
            "only virtual inputs can be operated, not {}",
            if ty.is_empty() { "this object" } else { ty }
        ));
    }
    let num = |k: &str| el.attr(k).and_then(|v| v.trim().parse::<f64>().ok());
    let checks = el.attr("EnVal") == Some("true");
    Ok(Input {
        title: el
            .attr_decoded("Title")
            .map(|t| t.into_owned())
            .unwrap_or_default(),
        analog: el.attr("Analog") == Some("true"),
        min: num("MinVal").filter(|_| checks),
        max: num("MaxVal").filter(|_| checks),
    })
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

/// What a code means (pp. 14, 30).
fn code_text(code: u16, secured: bool) -> &'static str {
    match code {
        200 => "ok",
        401 => "unauthorized",
        403 => "not enough rights",
        404 => "unknown control or command",
        423 => "the user is disabled",
        500 if secured => "wrong visualization password",
        500 => "refused: the block may need its visualization password",
        _ => "error",
    }
}

/// The `Code` and `value` of an `LL` answer, falling back on the HTTP status.
fn answer(status: u16, body: &str) -> (u16, Option<String>) {
    let code = loxone_client::system::ll_code(body).unwrap_or(status);
    (code, loxone_client::system::ll_value(body))
}

/// One command sent, for the log.
#[derive(Clone, Serialize, Debug)]
pub struct Entry {
    pub time: i64,
    /// The browser's address, and the Miniserver user the command ran as.
    pub who: String,
    pub uuid: String,
    pub title: String,
    pub cmd: String,
    pub secured: bool,
    /// The Miniserver's code (`200`), or none when the request failed.
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

pub struct Settings {
    /// `None`: on. Else why not.
    pub off: Option<String>,
    /// The deployed config, to look inputs up in.
    pub current: PathBuf,
    /// The Miniserver user commands run as (for secured commands and the log).
    pub user: String,
}

pub struct Operate {
    s: Settings,
    port: Option<Arc<dyn Port>>,
    log: Mutex<Vec<Entry>>,
    tx: broadcast::Sender<()>,
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
        }
    }

    /// `None` when commands can be sent, else why not.
    pub fn why_not(&self) -> Option<&str> {
        self.s.off.as_deref()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    /// For `/api/history`: whether inputs can be operated, and the log
    /// (newest first).
    pub fn info(&self) -> Value {
        let log: Vec<Entry> = self.log.lock().unwrap().iter().rev().cloned().collect();
        match &self.s.off {
            None => json!({ "available": true, "user": self.s.user, "log": log }),
            Some(why) => json!({ "available": false, "reason": why, "log": log }),
        }
    }

    /// Send one command. The caller has checked the admin token.
    pub async fn send(self: &Arc<Self>, req: Req, who: String) -> Result<Entry, Refusal> {
        if let Some(why) = &self.s.off {
            return Err((
                StatusCode::FORBIDDEN,
                format!("operating inputs is off: {why}"),
            ));
        }
        let bad = |m: String| (StatusCode::BAD_REQUEST, m);
        if !plausible_uuid(&req.uuid) {
            return Err(bad("not a UUID".into()));
        }
        let cmd =
            Cmd::parse(&req.cmd).ok_or_else(|| bad(format!("unknown command `{}`", req.cmd)))?;
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.send_blocking(&req, &cmd, who))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    }

    fn deployed(&self) -> Result<LoxoneDoc, String> {
        let bytes = std::fs::read(&self.s.current)
            .map_err(|e| format!("cannot read {}: {e}", self.s.current.display()))?;
        let xml = crate::source::program_xml(&bytes).map_err(|e| e.to_string())?;
        LoxoneDoc::parse(&xml).map_err(|e| e.to_string())
    }

    fn send_blocking(&self, req: &Req, cmd: &Cmd, who: String) -> Result<Entry, Refusal> {
        let doc = self
            .deployed()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let input = find_input(&doc, &req.uuid).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        check_cmd(&input, cmd).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        let port = self.port.as_ref().expect("on means connected");
        let pw = req.visu_password.as_deref().filter(|p| !p.is_empty());
        let mut entry = Entry {
            time: chrono::Utc::now().timestamp(),
            who: format!("{who} as {}", self.s.user),
            uuid: req.uuid.clone(),
            title: input.title.clone(),
            cmd: cmd.text(),
            secured: pw.is_some(),
            code: None,
            value: None,
            note: String::new(),
        };
        let res = match pw {
            None => port.get(&format!("jdev/sps/io/{}/{}", req.uuid, cmd.text()), None),
            Some(pw) => self.secured(port.as_ref(), pw, &req.uuid, &cmd.text()),
        };
        match res {
            Ok((status, body)) => {
                let (code, value) = answer(status, &body);
                entry.code = Some(code);
                entry.value = value;
                entry.note = code_text(code, entry.secured).into();
            }
            // The password check failed: nothing was sent, nothing to log.
            Err(e) if e.starts_with(PW_REFUSED) => {
                return Err((StatusCode::FORBIDDEN, e));
            }
            Err(e) => entry.note = e,
        }
        {
            let mut log = self.log.lock().unwrap();
            log.push(entry.clone());
            let n = log.len();
            if n > LOG_LEN {
                log.drain(..n - LOG_LEN);
            }
        }
        let _ = self.tx.send(());
        Ok(entry)
    }

    /// A fresh hash of the visualization password.
    fn visu_hash(&self, port: &dyn Port, pw: &str) -> Result<String, String> {
        let (status, body) = port.get(&format!("jdev/sys/getvisusalt/{}", self.s.user), None)?;
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
    /// secured command.
    fn secured(
        &self,
        port: &dyn Port,
        pw: &str,
        uuid: &str,
        cmd: &str,
    ) -> Result<(u16, String), String> {
        let h = self.visu_hash(port, pw)?;
        let (status, body) = port.get(
            &format!("jdev/sps/checkuservisupwd/{h}"),
            Some((&h, "<hash>")),
        )?;
        let (code, _) = answer(status, &body);
        if code != 200 {
            return Err(format!("{PW_REFUSED} (code {code})"));
        }
        let h = self.visu_hash(port, pw)?;
        port.get(
            &format!("jdev/sps/ios/{h}/{uuid}/{cmd}"),
            Some((&h, "<hash>")),
        )
    }
}

const PW_REFUSED: &str = "the Miniserver refused the visualization password";

#[cfg(test)]
mod tests {
    use super::*;

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

    type Script = Box<dyn Fn(&str) -> (u16, String) + Send + Sync>;

    /// Records every path and answers from a script.
    struct Mock {
        sent: Mutex<Vec<String>>,
        answer: Script,
    }

    impl Port for Mock {
        fn get(&self, path: &str, _: Option<(&str, &str)>) -> Result<(u16, String), String> {
            self.sent.lock().unwrap().push(path.to_string());
            Ok((self.answer)(path))
        }
    }

    fn ll(control: &str, code: u16, value: &str) -> String {
        json!({ "LL": { "control": control, "value": value, "Code": code.to_string() } })
            .to_string()
    }

    fn setup(
        off: Option<&str>,
        answer: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
    ) -> (Arc<Operate>, Arc<Mock>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("config.Loxone");
        std::fs::write(&current, DOC).unwrap();
        let mock = Arc::new(Mock {
            sent: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        });
        let op = Arc::new(Operate::new(
            Settings {
                off: off.map(String::from),
                current,
                user: "tester".into(),
            },
            Some(mock.clone() as Arc<dyn Port>),
        ));
        (op, mock, tmp)
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
        let doc = LoxoneDoc::parse(DOC.as_bytes()).unwrap();
        let b = find_input(&doc, BUTTON).unwrap();
        assert_eq!(b.title, "Test button");
        assert!(!b.analog);
        assert_eq!(b.max, None, "no range check without EnVal");
        let t = find_input(&doc, TEMP).unwrap();
        assert!(t.analog);
        assert_eq!((t.min, t.max), (Some(-50.0), Some(100.0)));
        assert!(find_input(&doc, GATE).unwrap_err().contains("And"));
        assert!(find_input(&doc, "ffff").is_err());

        assert!(check_cmd(&b, &Cmd::Pulse).is_ok());
        assert!(check_cmd(&b, &Cmd::Off).is_ok());
        assert!(check_cmd(&b, &Cmd::Value(1.0)).is_err());
        assert!(check_cmd(&t, &Cmd::Value(21.5)).is_ok());
        assert!(check_cmd(&t, &Cmd::Value(101.0)).is_err());
        assert!(check_cmd(&t, &Cmd::Value(-51.0)).is_err());
        assert!(check_cmd(&t, &Cmd::On).is_err());
    }

    #[tokio::test]
    async fn off_sends_nothing() {
        let (op, mock, _t) = setup(Some("start vislox with --operate"), ok);
        let e = op
            .send(req(BUTTON, "pulse", None), "me".into())
            .await
            .unwrap_err();
        assert_eq!(e.0, StatusCode::FORBIDDEN);
        assert!(mock.sent.lock().unwrap().is_empty());
        assert_eq!(op.info()["available"], false);

        let op = Operate::new(
            Settings {
                off: None,
                current: PathBuf::from("x"),
                user: "u".into(),
            },
            None,
        );
        assert_eq!(op.info()["reason"], "no Miniserver connection");
    }

    #[tokio::test]
    async fn refused_requests_send_nothing() {
        let (op, mock, _t) = setup(None, ok);
        for (uuid, cmd) in [
            (GATE, "on"),
            (BUTTON, "42"),
            (TEMP, "pulse"),
            (TEMP, "1000"),
            ("../../dev/sys/reboot", "on"),
            (BUTTON, "reboot"),
        ] {
            let e = op
                .send(req(uuid, cmd, None), "me".into())
                .await
                .unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{uuid} {cmd}");
        }
        assert!(mock.sent.lock().unwrap().is_empty());
        assert!(op.info()["log"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_command_is_sent_once_and_logged() {
        let (op, mock, _t) = setup(None, ok);
        let mut rx = op.subscribe();
        let e = op
            .send(req(TEMP, "21.5", None), "127.0.0.1".into())
            .await
            .unwrap();
        assert_eq!(
            *mock.sent.lock().unwrap(),
            vec![format!("jdev/sps/io/{TEMP}/21.5")]
        );
        assert_eq!(e.code, Some(200));
        assert_eq!(e.value.as_deref(), Some("1"));
        assert_eq!(e.note, "ok");
        assert!(rx.try_recv().is_ok());
        let info = op.info();
        assert_eq!(info["log"][0]["title"], "Test temp");
        assert_eq!(info["log"][0]["who"], "127.0.0.1 as tester");
        assert_eq!(info["log"][0]["cmd"], "21.5");
    }

    #[tokio::test]
    async fn codes_are_reported() {
        let (op, _m, _t) = setup(None, |p| (200, ll(p, 403, "0")));
        let e = op.send(req(BUTTON, "on", None), "me".into()).await.unwrap();
        assert_eq!(e.code, Some(403));
        assert_eq!(e.note, "not enough rights");
        let (op, _m, _t) = setup(None, |p| (200, ll(p, 423, "0")));
        let e = op.send(req(BUTTON, "on", None), "me".into()).await.unwrap();
        assert_eq!(e.note, "the user is disabled");
    }

    fn salt_answer(path: &str) -> (u16, String) {
        let v = json!({ "LL": { "control": path, "Code": "200",
            "value": { "key": "31323334", "salt": "abcd", "hashAlg": "SHA256" } } });
        (200, v.to_string())
    }

    #[tokio::test]
    async fn a_secured_command_checks_the_password_first() {
        let (op, mock, _t) = setup(None, |p| {
            if p.contains("getvisusalt") {
                salt_answer(p)
            } else {
                ok(p)
            }
        });
        let e = op
            .send(req(BUTTON, "pulse", Some("secret")), "me".into())
            .await
            .unwrap();
        assert!(e.secured);
        assert_eq!(e.code, Some(200));
        let sent = mock.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 4, "{sent:?}");
        assert_eq!(sent[0], "jdev/sys/getvisusalt/tester");
        assert!(sent[1].starts_with("jdev/sps/checkuservisupwd/"));
        assert_eq!(sent[2], "jdev/sys/getvisusalt/tester");
        assert!(
            sent[3].starts_with("jdev/sps/ios/") && sent[3].ends_with(&format!("/{BUTTON}/pulse"))
        );
        assert!(!sent.iter().any(|s| s.contains("secret")));
        assert!(!op.info().to_string().contains("secret"));
    }

    #[tokio::test]
    async fn a_wrong_password_sends_no_command() {
        let (op, mock, _t) = setup(None, |p| {
            if p.contains("getvisusalt") {
                salt_answer(p)
            } else {
                (200, ll(p, 500, "0"))
            }
        });
        let e = op
            .send(req(BUTTON, "pulse", Some("wrong")), "me".into())
            .await
            .unwrap_err();
        assert_eq!(e.0, StatusCode::FORBIDDEN);
        assert!(e.1.contains("visualization password"));
        let sent = mock.sent.lock().unwrap().clone();
        assert!(
            !sent
                .iter()
                .any(|s| s.contains("/sps/ios/") || s.contains("/sps/io/"))
        );
        assert!(op.info()["log"].as_array().unwrap().is_empty());
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
