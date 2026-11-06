//! Live values: the Miniserver's state stream, for the browser to put on the
//! wires of the config.
//!
//! A wire's source connector in the program file has the same UUID as the
//! state the Miniserver streams for it (`LoxAPP3.json`), so the browser can
//! match a value to a wire by UUID. Only states the app sees are streamed:
//! the outputs of blocks that are visualized, and the inputs and outputs of
//! the periphery. Logic in between has no value.
//!
//! Values come from one of:
//! - the Miniserver, with the connection `lox` uses (its `config.yaml`). The
//!   WebSocket is opened while a browser has live values on, and closed
//!   shortly after the last one turns them off. vislox only listens; it sends
//!   no commands.
//! - the NDJSON output of `lox stream -o json --initial` (`--live-from`), from
//!   a file (followed as it grows) or standard input.

use loxone_client::Connection;
use loxone_client::stream::{self, StateEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// After the last browser turns live values off, the Miniserver connection
/// stays open this long, so a reload does not reconnect.
const LINGER: Duration = Duration::from_secs(10);
/// Longest wait between reconnect attempts.
const MAX_BACKOFF_SECS: u64 = 30;

/// Where live values come from.
pub enum Feed {
    /// None: why not.
    Off(String),
    Miniserver(Box<Connection>),
    /// NDJSON from `lox stream -o json`; `-` is standard input.
    Ndjson(PathBuf),
}

// ── The lox configuration ───────────────────────────────────────────────

/// One Miniserver entry of lox's `config.yaml`.
#[derive(Deserialize, Default, Clone)]
struct LoxConf {
    #[serde(default)]
    host: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    pass: String,
    #[serde(default)]
    serial: String,
    #[serde(default)]
    verify_ssl: Option<bool>,
    /// The config repository `lox config pull` commits to.
    #[serde(default)]
    config_repo: Option<String>,
}

#[derive(Deserialize, Default)]
struct GlobalConf {
    #[serde(default)]
    active_context: Option<String>,
    #[serde(default)]
    contexts: HashMap<String, LoxConf>,
}

impl LoxConf {
    /// The Miniserver a config repository names `name`: its serial, or its
    /// address as `lox config init` writes it (`192_168_1_77`).
    fn is(&self, name: &str) -> bool {
        let norm = |s: &str| {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        };
        let host = self
            .host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        let name = norm(name);
        (!self.serial.is_empty() && norm(&self.serial) == name) || norm(host) == name
    }

    /// The folder `lox config pull` writes this Miniserver's config to:
    /// its serial, else its address (`ms_dir` in lox's `gitops.rs`).
    fn ms_dir(&self) -> String {
        if !self.serial.is_empty() {
            return self.serial.clone();
        }
        self.host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split(':')
            .next()
            .unwrap_or("default")
            .replace(['/', '.'], "_")
    }

    fn found(&self, data_dir: &Path, from: String, ctx: Option<String>) -> Found {
        Found {
            conn: self.connection(data_dir),
            from,
            ctx,
            config_repo: self.config_repo.as_ref().map(PathBuf::from),
            ms_dir: self.ms_dir(),
        }
    }

    fn connection(&self, data_dir: &Path) -> Connection {
        let host = if self.host.starts_with("http://") || self.host.starts_with("https://") {
            self.host.clone()
        } else {
            format!("https://{}", self.host)
        };
        Connection {
            host,
            user: self.user.clone(),
            pass: self.pass.clone(),
            verify_ssl: self.verify_ssl.unwrap_or(false),
            // Shared with lox: one token for both.
            token_path: Some(data_dir.join("token.json")),
            // Read fresh on every connect: a push changes it.
            structure_cache: None,
        }
    }
}

/// The Miniserver lox would talk to, and how lox was told about it.
pub struct Found {
    pub conn: Connection,
    /// The config file, for messages.
    pub from: String,
    /// The context, when it came from a config with several.
    pub ctx: Option<String>,
    /// The repository `lox config pull` commits to.
    pub config_repo: Option<PathBuf>,
    /// The folder in it `lox config pull` writes to.
    pub ms_dir: String,
}

/// The Miniserver connection lox would use, found the way lox finds it:
/// `LOX_CONFIG`, then a project-local `.lox/config.yaml` above `start` (or
/// the working directory), then `~/.lox/config.yaml`. In a config with
/// several contexts: `ctx`, else the one for the Miniserver being viewed
/// (`miniserver`), else the active one.
pub fn find_miniserver(
    start: &Path,
    miniserver: Option<&str>,
    ctx: Option<&str>,
) -> Result<Found, String> {
    let home = std::env::home_dir().unwrap_or_default();
    find_in(
        std::env::var_os("LOX_CONFIG").map(PathBuf::from),
        &home,
        start,
        miniserver,
        ctx,
    )
}

fn find_in(
    env: Option<PathBuf>,
    home: &Path,
    start: &Path,
    miniserver: Option<&str>,
    ctx: Option<&str>,
) -> Result<Found, String> {
    let flat = |path: &Path, data_dir: &Path| -> Result<Found, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let c: LoxConf = serde_yaml::from_str(&text)
            .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
        if c.host.is_empty() {
            return Err(format!("{} names no host", path.display()));
        }
        Ok(c.found(data_dir, path.display().to_string(), None))
    };
    if let Some(path) = env {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        return flat(&path, &dir);
    }
    let global_dir = home.join(".lox");
    if ctx.is_none() {
        let cwd = std::env::current_dir().ok();
        for top in [Some(start), cwd.as_deref()].into_iter().flatten() {
            for dir in top.ancestors() {
                let d = dir.join(".lox");
                if d != global_dir && d.join("config.yaml").is_file() {
                    return flat(&d.join("config.yaml"), &d);
                }
            }
        }
    }
    let path = global_dir.join("config.yaml");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        "no lox configuration (run `lox setup set --host … --user … --pass …`)".to_string()
    })?;
    let any: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    if any.get("contexts").is_none() {
        if ctx.is_some() {
            return Err(format!("{} has no contexts", path.display()));
        }
        return flat(&path, &global_dir);
    }
    let g: GlobalConf =
        serde_yaml::from_value(any).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let mut names: Vec<&String> = g.contexts.keys().collect();
    names.sort();
    let name = match ctx {
        Some(c) => g
            .contexts
            .contains_key(c)
            .then(|| c.to_string())
            .ok_or_else(|| format!("no lox context `{c}`"))?,
        None => names
            .iter()
            .find(|n| miniserver.is_some_and(|m| g.contexts[n.as_str()].is(m)))
            .map(|n| n.to_string())
            .or(g.active_context.clone())
            .ok_or("no active lox context (run `lox ctx use <name>`, or pass --ctx)")?,
    };
    let c = g
        .contexts
        .get(&name)
        .ok_or_else(|| format!("no lox context `{name}`"))?;
    Ok(c.found(
        &global_dir.join("contexts").join(&name),
        format!("{} (context {name})", path.display()),
        Some(name),
    ))
}

// ── Number formats (as lox and the Loxone app show them) ───────────────

/// A value in a control's printf-like format (`%.1f°`, `%i%%`, `Lux`).
/// Follows `lox_format` in lox's `util.rs`.
pub fn lox_format(fmt: &str, v: f64) -> String {
    let Some(pos) = fmt.find('%') else {
        return format!("{} {}", trim_num(v, 1), fmt).trim().to_string();
    };
    let (pre, rest) = fmt.split_at(pos);
    let rest = &rest[1..];
    let mut prec: Option<usize> = None;
    let mut end = 0;
    let mut spec = 'f';
    let mut digits = String::new();
    let mut seen_dot = false;
    for (i, c) in rest.char_indices() {
        if c == '.' {
            seen_dot = true;
        } else if c.is_ascii_digit() {
            if seen_dot {
                digits.push(c);
            }
        } else if matches!(c, '-' | '+' | ' ' | 'l') {
            // flags and length modifiers
        } else {
            spec = c;
            end = i + c.len_utf8();
            break;
        }
    }
    if seen_dot {
        prec = digits.parse().ok();
    }
    let suffix = rest.get(end..).unwrap_or("").replace("%%", "%");
    let num = match spec {
        'i' | 'd' | 'u' => format!("{}", v.round() as i64),
        '%' => return format!("{pre}%{suffix}"),
        's' => trim_num(v, 2),
        _ => match prec {
            Some(p) => format!("{v:.p$}"),
            None => format!("{v:.6}"),
        },
    };
    format!("{pre}{num}{suffix}")
}

fn trim_num(v: f64, prec: usize) -> String {
    let s = format!("{v:.prec$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A state value as JSON: whole numbers as integers (`1`, not `1.0`).
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

// ── The structure file ──────────────────────────────────────────────────

/// Who streams a state.
#[derive(Clone, Debug, PartialEq)]
struct Owner {
    /// `Control · state`, for tooltips.
    label: String,
    state: String,
    /// The control's number format, for its `value` state.
    format: Option<String>,
    /// Names for the values (the operating modes, for `operatingMode`).
    names: Option<Arc<HashMap<String, String>>>,
}

impl Owner {
    fn text(&self, v: f64) -> Option<String> {
        if let Some(names) = &self.names {
            return names.get(&format!("{}", v.round() as i64)).cloned();
        }
        (self.state == "value")
            .then_some(self.format.as_deref())
            .flatten()
            .map(|f| lox_format(f, v))
    }
}

/// What the structure file tells: state UUID → owner, the controls (block
/// UUID → name, type and states) and the global states (name → state UUID)
/// as the browser receives them.
struct Structure {
    owners: HashMap<String, Owner>,
    controls: Value,
    globals: Value,
}

fn read_structure(structure: &Value) -> Structure {
    fn add(
        owners: &mut HashMap<String, Owner>,
        controls: &mut serde_json::Map<String, Value>,
        uuid: &str,
        ctrl: &Value,
        parent: Option<&str>,
    ) {
        let own = ctrl["name"].as_str().unwrap_or_default();
        let name = match parent {
            Some(p) => format!("{p} / {own}"),
            None => own.to_string(),
        };
        let d = &ctrl["details"];
        let format = d["format"]
            .as_str()
            .or_else(|| d["actualFormat"].as_str())
            .map(String::from);
        let mut states = serde_json::Map::new();
        for (state, su) in ctrl["states"].as_object().into_iter().flatten() {
            // A state may be a list of UUIDs; only single ones carry a value.
            if let Some(su) = su.as_str() {
                owners.insert(
                    su.to_string(),
                    Owner {
                        label: format!("{name} · {state}"),
                        state: state.clone(),
                        format: format.clone(),
                        names: None,
                    },
                );
                states.insert(state.clone(), Value::from(su));
            }
        }
        controls.insert(
            uuid.to_string(),
            json!({ "name": name, "type": ctrl["type"], "states": states }),
        );
        for (su, sub) in ctrl["subControls"].as_object().into_iter().flatten() {
            add(owners, controls, su, sub, Some(own));
        }
    }
    let mut owners = HashMap::new();
    let mut controls = serde_json::Map::new();
    for (uuid, ctrl) in structure["controls"].as_object().into_iter().flatten() {
        add(&mut owners, &mut controls, uuid, ctrl, None);
    }
    // The Miniserver's own states: its clock, sunrise and sunset, the
    // operating mode. Some have the UUID of a system variable in the config
    // (`Sunrise`, `Calendar`), so they also become controls with a `value`.
    let modes: HashMap<String, String> = structure["operatingModes"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect();
    let modes = Arc::new(modes);
    let mut globals = serde_json::Map::new();
    for (name, su) in structure["globalStates"].as_object().into_iter().flatten() {
        let Some(su) = su.as_str() else {
            continue;
        };
        globals.insert(name.clone(), Value::from(su));
        owners.entry(su.to_string()).or_insert_with(|| Owner {
            label: format!("Miniserver · {name}"),
            state: name.clone(),
            format: None,
            names: (name == "operatingMode" && !modes.is_empty()).then(|| modes.clone()),
        });
        controls.entry(su.to_string()).or_insert_with(|| {
            json!({ "name": format!("Miniserver · {name}"), "type": "GlobalState", "states": { "value": su } })
        });
    }
    Structure {
        owners,
        controls: Value::Object(controls),
        globals: Value::Object(globals),
    }
}

// ── The hub ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, PartialEq, Debug)]
pub struct Status {
    /// `off` (no source), `idle` (nobody watching), `connecting`, `live`,
    /// `offline` (retrying), `ended` (the NDJSON input ended).
    pub state: &'static str,
    pub detail: Option<String>,
}

/// A stored value: the value and its text in the control's format.
type Val = (Value, Option<String>);

struct St {
    status: Status,
    values: HashMap<String, Val>,
    owners: HashMap<String, Owner>,
    /// `controls` of the snapshot, serialized.
    controls: Arc<String>,
    /// `globals` of the snapshot, serialized.
    globals: Arc<String>,
    clients: usize,
    /// The running Miniserver session: its generation and stop switch.
    run: Option<(u64, watch::Sender<bool>)>,
    /// Bumped for each Miniserver session; an ended session's late writes
    /// carry an old one and are dropped.
    generation: u64,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Status,
    /// Everything changed (a new structure): clients re-read the snapshot.
    Snapshot,
    /// Changed values: `{uuid: [value, text, label]}`.
    Values(Arc<String>),
}

pub struct Live {
    feed: Feed,
    /// Where values come from, for the UI.
    pub source: String,
    tx: broadcast::Sender<Msg>,
    st: Mutex<St>,
}

impl Live {
    pub fn new(feed: Feed, source: String) -> Arc<Live> {
        let status = match &feed {
            Feed::Off(why) => Status {
                state: "off",
                detail: Some(why.clone()),
            },
            _ => Status {
                state: "idle",
                detail: None,
            },
        };
        let (tx, _) = broadcast::channel(256);
        let live = Arc::new(Live {
            feed,
            source,
            tx,
            st: Mutex::new(St {
                status,
                values: HashMap::new(),
                owners: HashMap::new(),
                controls: Arc::new("{}".into()),
                globals: Arc::new("{}".into()),
                clients: 0,
                run: None,
                generation: 0,
            }),
        });
        if let Feed::Ndjson(path) = &live.feed {
            tokio::spawn(live.clone().run_ndjson(path.clone()));
        }
        live
    }

    /// For `/api/history`: whether live values can be had, and from where.
    pub fn info(&self) -> Value {
        match &self.feed {
            Feed::Off(why) => json!({ "available": false, "reason": why }),
            _ => json!({ "available": true, "source": self.source }),
        }
    }

    pub fn status(&self) -> Status {
        self.st.lock().unwrap().status.clone()
    }

    fn set_status(&self, generation: u64, state: &'static str, detail: Option<String>) {
        let mut st = self.st.lock().unwrap();
        if st.generation != generation {
            return;
        }
        let s = Status { state, detail };
        if st.status != s {
            st.status = s;
            drop(st);
            let _ = self.tx.send(Msg::Status);
        }
    }

    /// The current status as JSON.
    pub fn status_json(&self) -> String {
        serde_json::to_string(&self.status()).unwrap_or_default()
    }

    /// Every value, the controls and the global states:
    /// `{values: {uuid: [v, text, label]}, controls, globals}`.
    pub fn snapshot_json(&self) -> String {
        let st = self.st.lock().unwrap();
        let values: serde_json::Map<String, Value> = st
            .values
            .iter()
            .map(|(u, v)| (u.clone(), entry(v, st.owners.get(u))))
            .collect();
        format!(
            "{{\"values\":{},\"controls\":{},\"globals\":{}}}",
            Value::Object(values),
            st.controls,
            st.globals
        )
    }

    /// A browser turned live values on: start the Miniserver session if
    /// none runs. The returned guard turns them off again when dropped.
    pub fn subscribe(self: &Arc<Self>) -> (broadcast::Receiver<Msg>, Watcher) {
        let rx = self.tx.subscribe();
        let mut st = self.st.lock().unwrap();
        st.clients += 1;
        if let Feed::Miniserver(conn) = &self.feed
            && st.run.is_none()
        {
            st.generation += 1;
            let generation = st.generation;
            let (stop_tx, stop_rx) = watch::channel(false);
            st.run = Some((generation, stop_tx));
            st.status = Status {
                state: "connecting",
                detail: None,
            };
            tokio::spawn(
                self.clone()
                    .run_miniserver(generation, (**conn).clone(), stop_rx),
            );
        }
        (rx, Watcher(self.clone()))
    }

    fn unsubscribe(self: &Arc<Self>) {
        let mut st = self.st.lock().unwrap();
        st.clients = st.clients.saturating_sub(1);
        if st.clients > 0 || st.run.is_none() {
            return;
        }
        let me = self.clone();
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        rt.spawn(async move {
            tokio::time::sleep(LINGER).await;
            let mut st = me.st.lock().unwrap();
            if st.clients == 0
                && let Some((_, stop)) = st.run.take()
            {
                let _ = stop.send(true);
                st.generation += 1;
                st.status = Status {
                    state: "idle",
                    detail: None,
                };
                st.values.clear();
                drop(st);
                let _ = me.tx.send(Msg::Status);
            }
        });
    }

    /// Store new values and tell the browsers which ones changed.
    fn apply(&self, generation: u64, events: Vec<StateEvent>) {
        let mut out = serde_json::Map::new();
        let mut gone = false;
        {
            let mut st = self.st.lock().unwrap();
            if st.generation != generation {
                return;
            }
            for e in events {
                let (uuid, val) = match e {
                    StateEvent::ValueState { uuid, value } => {
                        let text = st.owners.get(&uuid).and_then(|o| o.text(value));
                        (uuid, (number(value), text))
                    }
                    StateEvent::TextState { uuid, text, .. } => (uuid, (Value::from(text), None)),
                    StateEvent::OutOfService => {
                        gone = true;
                        continue;
                    }
                    _ => continue,
                };
                if st.values.get(&uuid) != Some(&val) {
                    out.insert(uuid.clone(), entry(&val, st.owners.get(&uuid)));
                    st.values.insert(uuid, val);
                }
            }
        }
        if !out.is_empty() {
            let _ = self
                .tx
                .send(Msg::Values(Arc::new(Value::Object(out).to_string())));
        }
        if gone {
            self.set_status(
                generation,
                "offline",
                Some("the Miniserver is going offline (reboot or update)".into()),
            );
        }
    }

    fn load_structure(&self, generation: u64, structure: &Value) {
        let s = read_structure(structure);
        let mut st = self.st.lock().unwrap();
        if st.generation != generation {
            return;
        }
        st.owners = s.owners;
        st.controls = Arc::new(s.controls.to_string());
        st.globals = Arc::new(s.globals.to_string());
        st.values.clear();
        drop(st);
        let _ = self.tx.send(Msg::Snapshot);
    }

    async fn run_miniserver(
        self: Arc<Self>,
        generation: u64,
        conn: Connection,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut wait = 1;
        loop {
            if *stop.borrow() {
                return;
            }
            self.set_status(generation, "connecting", None);
            let c = conn.clone();
            let structure = tokio::task::spawn_blocking(move || -> Result<Value, String> {
                let mut client = loxone_client::LoxClient::new(c).map_err(|e| e.to_string())?;
                client.get_structure().cloned().map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            let err = match structure {
                Err(e) => format!("cannot read the structure file: {e}"),
                Ok(s) => {
                    self.load_structure(generation, &s);
                    let went_live = AtomicBool::new(false);
                    let res = stream::stream_session(
                        &conn,
                        stop.clone(),
                        || {
                            went_live.store(true, Ordering::Relaxed);
                            self.set_status(generation, "live", None);
                        },
                        |events| {
                            self.apply(generation, events);
                            true
                        },
                    )
                    .await;
                    if *stop.borrow() {
                        return;
                    }
                    if went_live.load(Ordering::Relaxed) {
                        wait = 1;
                    }
                    match res {
                        Ok(()) => "the Miniserver closed the connection".to_string(),
                        Err(e) => e.to_string(),
                    }
                }
            };
            self.set_status(
                generation,
                "offline",
                Some(format!("{err} · retrying in {wait} s")),
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = stop.changed() => return,
            }
            wait = (wait * 2).min(MAX_BACKOFF_SECS);
        }
    }

    /// Read `lox stream -o json` lines: from standard input until it ends,
    /// or from a file, following it as it grows.
    async fn run_ndjson(self: Arc<Self>, path: PathBuf) {
        use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
        let stdin = path.as_os_str() == "-";
        let input: Box<dyn AsyncRead + Unpin + Send> = if stdin {
            Box::new(tokio::io::stdin())
        } else {
            match tokio::fs::File::open(&path).await {
                Ok(f) => Box::new(f),
                Err(e) => {
                    self.set_status(
                        0,
                        "offline",
                        Some(format!("cannot read {}: {e}", path.display())),
                    );
                    return;
                }
            }
        };
        let mut reader = BufReader::new(input);
        self.set_status(0, "live", None);
        let mut line = String::new();
        let mut batch = serde_json::Map::new();
        loop {
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    self.flush(&mut batch);
                    if stdin {
                        self.set_status(0, "ended", Some("the input ended".into()));
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    continue;
                }
                Ok(_) if !line.ends_with('\n') => continue,
                Ok(_) => {}
                Err(e) => {
                    self.set_status(0, "offline", Some(format!("cannot read: {e}")));
                    return;
                }
            }
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                self.ndjson_line(&v, &mut batch);
            }
            line.clear();
            // Send what is there once the input has nothing more ready.
            if batch.len() >= 500 || reader.buffer().is_empty() {
                self.flush(&mut batch);
            }
        }
    }

    fn ndjson_line(&self, v: &Value, batch: &mut serde_json::Map<String, Value>) {
        match v["event"].as_str() {
            Some("disconnected") => {
                let why = v["message"].as_str().or(v["reason"].as_str());
                self.set_status(0, "offline", why.map(String::from));
                return;
            }
            Some("reconnected") => {
                self.set_status(0, "live", None);
                return;
            }
            _ => {}
        }
        let Some(uuid) = v["uuid"].as_str() else {
            return;
        };
        let value = match &v["value"] {
            Value::String(s) => s
                .parse::<f64>()
                .map(number)
                .unwrap_or_else(|_| Value::from(s.as_str())),
            Value::Null => return,
            other => other.clone(),
        };
        let owner = Owner {
            label: format!(
                "{} · {}",
                v["control"].as_str().unwrap_or_default(),
                v["state"].as_str().unwrap_or_default()
            ),
            state: v["state"].as_str().unwrap_or_default().to_string(),
            format: None,
            names: None,
        };
        let val: Val = (value, None);
        let mut st = self.st.lock().unwrap();
        if st.values.get(uuid) != Some(&val) {
            batch.insert(uuid.to_string(), entry(&val, Some(&owner)));
            st.values.insert(uuid.to_string(), val);
        }
        st.owners.insert(uuid.to_string(), owner);
    }

    fn flush(&self, batch: &mut serde_json::Map<String, Value>) {
        if !batch.is_empty() {
            let out = std::mem::take(batch);
            let _ = self
                .tx
                .send(Msg::Values(Arc::new(Value::Object(out).to_string())));
        }
    }
}

fn entry(v: &Val, owner: Option<&Owner>) -> Value {
    json!([v.0, v.1, owner.map(|o| o.label.as_str())])
}

/// One browser with live values on; dropping it turns them off.
pub struct Watcher(Arc<Live>);

impl Drop for Watcher {
    fn drop(&mut self) {
        self.0.unsubscribe();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loxone_formats() {
        assert_eq!(lox_format("%.1f°C", 21.456), "21.5°C");
        assert_eq!(lox_format("%i%%", 42.4), "42%");
        assert_eq!(lox_format("%.0flx", 22000.0), "22000lx");
        assert_eq!(lox_format("Lux", 300.0), "300 Lux");
        assert_eq!(lox_format("%.2f ppm", 400.0), "400.00 ppm");
    }

    #[test]
    fn whole_numbers_stay_integers() {
        assert_eq!(number(1.0), json!(1));
        assert_eq!(number(21.5), json!(21.5));
    }

    #[test]
    fn the_structure_names_every_state_and_its_format() {
        let s = json!({ "controls": {
            "c1": { "name": "Lux", "type": "InfoOnlyAnalog", "details": { "format": "%.0flx" },
                    "states": { "value": "s1", "error": "s2" } },
            "c2": { "name": "Licht", "type": "LightControllerV2", "states": { "activeMoods": "s3" },
                    "subControls": { "c3": { "name": "Spots", "type": "Switch", "states": { "active": "s4" } } } }
        }});
        let Structure {
            owners, controls, ..
        } = read_structure(&s);
        assert_eq!(owners["s1"].label, "Lux · value");
        assert_eq!(owners["s1"].text(22000.0).as_deref(), Some("22000lx"));
        assert_eq!(owners["s2"].text(1.0), None);
        assert_eq!(owners["s4"].label, "Licht / Spots · active");
        assert_eq!(controls["c3"]["states"]["active"], "s4");
        assert_eq!(controls["c1"]["type"], "InfoOnlyAnalog");
    }

    #[test]
    fn global_states_are_named_and_the_operating_mode_has_its_name() {
        let s = json!({
            "controls": { "c1": { "name": "Lux", "type": "InfoOnlyAnalog", "states": { "value": "s1" } } },
            "globalStates": { "sunrise": "g1", "operatingMode": "g2", "miniserverTime": "g3" },
            "operatingModes": { "7": "Friday", "-1": "Alarm" }
        });
        let st = read_structure(&s);
        assert_eq!(st.globals["miniserverTime"], "g3");
        assert_eq!(st.owners["g1"].label, "Miniserver · sunrise");
        assert_eq!(st.owners["g1"].text(416.0), None);
        assert_eq!(st.owners["g2"].text(7.0).as_deref(), Some("Friday"));
        assert_eq!(st.owners["g2"].text(3.0), None);
        // A system variable with a global state's UUID gets it as its value.
        assert_eq!(st.controls["g1"]["states"]["value"], "g1");
        assert_eq!(st.controls["c1"]["name"], "Lux");
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_context_for_the_viewed_miniserver_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        write(
            &home.join(".lox/config.yaml"),
            "active_context: office\ncontexts:\n  office:\n    host: 10.0.0.2\n    user: u\n    pass: p\n  \
             home:\n    host: https://192.168.1.120\n    user: u\n    pass: p\n    config_repo: /srv/cfg\n",
        );
        let start = tmp.path().join("repo");
        std::fs::create_dir_all(&start).unwrap();
        let f = find_in(None, &home, &start, Some("192_168_1_120"), None).unwrap();
        assert_eq!(f.conn.host, "https://192.168.1.120");
        assert!(f.from.contains("context home"), "{}", f.from);
        assert_eq!(f.ctx.as_deref(), Some("home"));
        assert_eq!(f.ms_dir, "192_168_1_120");
        assert_eq!(f.config_repo, Some(PathBuf::from("/srv/cfg")));
        assert!(
            f.conn
                .token_path
                .unwrap()
                .ends_with("contexts/home/token.json")
        );
        let f = find_in(None, &home, &start, Some("504F94000001"), None).unwrap();
        assert_eq!(f.conn.host, "https://10.0.0.2");
        assert_eq!(f.ms_dir, "10_0_0_2");
        let f = find_in(None, &home, &start, None, Some("home")).unwrap();
        assert_eq!(f.conn.host, "https://192.168.1.120");
        assert!(find_in(None, &home, &start, None, Some("nope")).is_err());
    }

    #[test]
    fn a_project_local_config_comes_before_the_global_one() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        write(
            &home.join(".lox/config.yaml"),
            "host: global\nuser: u\npass: p\n",
        );
        let project = tmp.path().join("proj");
        write(
            &project.join(".lox/config.yaml"),
            "host: http://local\nuser: u\npass: p\n",
        );
        let sub = project.join("pages");
        std::fs::create_dir_all(&sub).unwrap();
        let f = find_in(None, &home, &sub, None, None).unwrap();
        assert_eq!(f.conn.host, "http://local");
        assert_eq!(f.ctx, None);
        let f = find_in(None, &home, &tmp.path().join("home"), None, None).unwrap();
        assert_eq!(f.conn.host, "https://global");
        let env = tmp.path().join("env.yaml");
        write(&env, "host: envhost\nuser: u\npass: p\n");
        let f = find_in(Some(env), &home, &sub, None, None).unwrap();
        assert_eq!(f.conn.host, "https://envhost");
        assert!(
            find_in(
                None,
                &tmp.path().join("nobody"),
                &tmp.path().join("nobody"),
                None,
                None
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn ndjson_lines_become_values() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("s.ndjson");
        std::fs::write(
            &f,
            concat!(
                r#"{"control":"Lux","state":"value","value":"22000","uuid":"s1"}"#,
                "\n",
                r#"{"control":"Text","state":"text","value":"hello","uuid":"s2"}"#,
                "\n",
                r#"{"event":"disconnected","reason":"closed","message":"gone"}"#,
                "\n",
            ),
        )
        .unwrap();
        let live = Live::new(Feed::Ndjson(f), "test".into());
        for _ in 0..50 {
            if live.status().state == "offline" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snap: Value = serde_json::from_str(&live.snapshot_json()).unwrap();
        assert_eq!(snap["values"]["s1"], json!([22000, null, "Lux · value"]));
        assert_eq!(snap["values"]["s2"][0], "hello");
        assert_eq!(live.status().detail.as_deref(), Some("gone"));
    }
}
