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
//!
//! Opt-in, a browser can also have the blocks behind the wires of the page it
//! shows queried (`jdev/sps/io/<uuid>/all`, read-only, on the same socket):
//! see `query.rs`. Those values are marked as queried: they are up to one
//! round old.

use crate::query;
use crate::reason::{Backoff, Next, Reason};
use crate::session::{self, Ended};
use loxone_client::Connection;
use loxone_client::stream::StateEvent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// After the last browser turns live values off, the Miniserver connection
/// stays open this long, so a reload does not reconnect.
const LINGER: Duration = Duration::from_secs(10);

/// Block queries: at most 20 a second, all of a page's blocks every 2 s at
/// most (a page with more than 40 takes longer), and an answer within 3 s.
const QUERY_GAP: Duration = Duration::from_millis(50);
const QUERY_ROUND: Duration = Duration::from_secs(2);
const QUERY_WAIT: Duration = Duration::from_secs(3);
/// A block that does not answer this many times in a row is left alone for
/// [`QUERY_PAUSE`].
const QUERY_MISSES: u32 = 3;
const QUERY_PAUSE: Duration = Duration::from_secs(60);
/// How long a token from [`Live::offer`] waits for its stream.
const OFFER_TTL: Duration = Duration::from_secs(30);

/// What vislox learnt about querying one block on this connection.
#[derive(Default, Debug)]
struct BlockState {
    /// It answered with an error code (an alarm's `500`): not asked again.
    failed: Option<String>,
    /// Timeouts in a row.
    misses: u32,
    /// After [`QUERY_MISSES`] timeouts: not asked until then.
    paused: Option<Instant>,
    answered: bool,
}

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
            serial: Some(self.serial.trim().to_string()).filter(|s| !s.is_empty()),
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
    /// The serial lox's configuration names (`lox setup set --serial`).
    pub serial: Option<String>,
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
    /// The value was queried, not streamed (`query.rs`).
    queried: bool,
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
                        queried: false,
                    },
                );
                states.insert(state.clone(), Value::from(su));
            }
        }
        controls.insert(
            uuid.to_string(),
            json!({ "name": name, "type": ctrl["type"], "states": states,
                    // A visualization password guards its commands.
                    "secured": ctrl["isSecured"].as_bool().unwrap_or(false) }),
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
            queried: false,
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

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Status {
    /// `off` (no source), `idle` (nobody watching), `connecting`, `live`,
    /// `offline` (retrying), `stopped` (not retrying until a user turns
    /// live values off and on: see `reason`), `ended` (the NDJSON input
    /// ended).
    pub state: &'static str,
    pub detail: Option<String>,
    /// Why the Miniserver refused or dropped the connection, when it said.
    pub reason: Option<Reason>,
}

/// `{state, detail, reason?, label?}`: `label` is the reason's short label.
impl Serialize for Status {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = ser.serialize_struct("Status", 4)?;
        st.serialize_field("state", self.state)?;
        st.serialize_field("detail", &self.detail)?;
        match self.reason {
            Some(r) => {
                st.serialize_field("reason", &r)?;
                st.serialize_field("label", r.label())?;
            }
            None => {
                st.skip_field("reason")?;
                st.skip_field("label")?;
            }
        }
        st.end()
    }
}

impl Status {
    fn new(state: &'static str, detail: Option<String>) -> Status {
        Status {
            state,
            detail,
            ..Status::default()
        }
    }

    fn because(state: &'static str, reason: Option<Reason>, detail: String) -> Status {
        Status {
            state,
            detail: Some(detail),
            reason,
        }
    }
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
    /// The last session gave up for a reason more attempts would make worse
    /// (blocked, login refused, user disabled). No new session starts, not
    /// for a reload, another tab or a reconnecting stream, until a user
    /// turns live values on again ([`Live::retry`]).
    stopped: bool,
    /// The blocks each browser that asked for it has queried (`query.rs`).
    queries: HashMap<u64, Arc<Vec<query::Block>>>,
    next_query: u64,
    /// Plans waiting for their stream: token → blocks, when offered.
    offers: HashMap<String, (Vec<query::Block>, Instant)>,
    /// Per block, how querying it goes on this connection.
    blocks: HashMap<String, BlockState>,
    /// Connectors with a queried value → their block.
    queried: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Status,
    /// Everything changed (a new structure): clients re-read the snapshot.
    Snapshot,
    /// Changed values: `{uuid: [value, text, label, kind?] | null}` (null:
    /// the value is gone).
    Values(Arc<String>),
}

pub struct Live {
    feed: Feed,
    /// Where values come from, for the UI.
    pub source: String,
    tx: broadcast::Sender<Msg>,
    st: Mutex<St>,
    /// Told each structure file a session reads (the In the app section).
    structure: Mutex<Option<Arc<crate::structure::Structure>>>,
}

impl Live {
    pub fn new(feed: Feed, source: String) -> Arc<Live> {
        let status = match &feed {
            Feed::Off(why) => Status::new("off", Some(why.clone())),
            _ => Status::new("idle", None),
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
                stopped: false,
                queries: HashMap::new(),
                next_query: 0,
                offers: HashMap::new(),
                blocks: HashMap::new(),
                queried: HashMap::new(),
            }),
            structure: Mutex::new(None),
        });
        if let Feed::Ndjson(path) = &live.feed {
            tokio::spawn(live.clone().run_ndjson(path.clone()));
        }
        live
    }

    /// Hand each structure file a session reads to `s` as well.
    pub fn share_structure(&self, s: Arc<crate::structure::Structure>) {
        *self.structure.lock().unwrap() = Some(s);
    }

    /// For `/api/history`: whether live values can be had, and from where.
    pub fn info(&self) -> Value {
        match &self.feed {
            Feed::Off(why) => json!({ "available": false, "reason": why }),
            Feed::Miniserver(_) => {
                json!({ "available": true, "source": self.source, "query": true })
            }
            Feed::Ndjson(_) => json!({ "available": true, "source": self.source }),
        }
    }

    pub fn status(&self) -> Status {
        self.st.lock().unwrap().status.clone()
    }

    fn set_status(&self, generation: u64, state: &'static str, detail: Option<String>) {
        self.put_status(generation, Status::new(state, detail));
    }

    fn put_status(&self, generation: u64, s: Status) {
        let mut st = self.st.lock().unwrap();
        if st.generation != generation {
            return;
        }
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
        self.start(&mut st);
        (rx, Watcher(self.clone()))
    }

    /// Start a Miniserver session, unless one runs or the last one stopped.
    fn start(self: &Arc<Self>, st: &mut St) {
        if let Feed::Miniserver(conn) = &self.feed
            && st.run.is_none()
            && !st.stopped
        {
            st.generation += 1;
            let generation = st.generation;
            let (stop_tx, stop_rx) = watch::channel(false);
            st.run = Some((generation, stop_tx));
            st.status = Status::new("connecting", None);
            tokio::spawn(
                self.clone()
                    .run_miniserver(generation, (**conn).clone(), stop_rx),
            );
        }
    }

    /// A user turned live values on: after a session that stopped (blocked,
    /// login refused), allow one again.
    pub fn retry(self: &Arc<Self>) {
        let mut st = self.st.lock().unwrap();
        if !st.stopped {
            return;
        }
        st.stopped = false;
        st.status = Status::new("idle", None);
        if st.clients > 0 {
            self.start(&mut st);
        }
        drop(st);
        let _ = self.tx.send(Msg::Status);
    }

    /// The session `generation` gave up: show why, and start no other until
    /// [`retry`](Live::retry).
    fn stop_for_good(&self, generation: u64, status: Status) {
        let mut st = self.st.lock().unwrap();
        if st.generation != generation {
            return;
        }
        st.status = status;
        st.stopped = true;
        if let Some((_, stop)) = st.run.take() {
            let _ = stop.send(true);
        }
        // Late writes of the session are dropped.
        st.generation += 1;
        drop(st);
        let _ = self.tx.send(Msg::Status);
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
                st.status = Status::new("idle", None);
                st.values.clear();
                st.owners.retain(|_, o| !o.queried);
                st.queried.clear();
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
            let r = Reason::OutOfService;
            self.put_status(
                generation,
                Status::because("offline", Some(r), r.sentence().into()),
            );
        }
    }

    fn load_structure(&self, generation: u64, structure: &Value) {
        let s = read_structure(structure);
        let mut st = self.st.lock().unwrap();
        if st.generation != generation {
            return;
        }
        // The In the app section's copy: compared and cached off the
        // runtime, and only for the running session.
        let shared = self.structure.lock().unwrap().clone();
        if let Some(shared) = shared {
            let raw = structure.clone();
            tokio::task::spawn_blocking(move || shared.set(raw));
        }
        st.owners = s.owners;
        st.controls = Arc::new(s.controls.to_string());
        st.globals = Arc::new(s.globals.to_string());
        st.values.clear();
        st.queried.clear();
        drop(st);
        let _ = self.tx.send(Msg::Snapshot);
    }

    async fn run_miniserver(
        self: Arc<Self>,
        generation: u64,
        conn: Connection,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut backoff = Backoff::default();
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
            let (err, reason, http) = match structure {
                Err(e) => {
                    let r = Reason::from_text(&e);
                    (format!("cannot read the structure file: {e}"), r, true)
                }
                Ok(s) => {
                    self.load_structure(generation, &s);
                    let went_live = AtomicBool::new(false);
                    let going_offline = AtomicBool::new(false);
                    let (queries, asked) = mpsc::channel(1);
                    let ended = tokio::select! {
                        ended = session::run(
                            &conn,
                            stop.clone(),
                            || {
                                went_live.store(true, Ordering::Relaxed);
                                self.set_status(generation, "live", None);
                            },
                            |events| {
                                if events.iter().any(|e| matches!(e, StateEvent::OutOfService)) {
                                    going_offline.store(true, Ordering::Relaxed);
                                }
                                self.apply(generation, events);
                                true
                            },
                            asked,
                        ) => ended,
                        // Never ends: it stops with the session.
                        () = self.query_blocks(generation, &went_live, queries) => Ended::Stopped,
                    };
                    if *stop.borrow() {
                        return;
                    }
                    if went_live.load(Ordering::Relaxed) {
                        backoff.reset();
                    }
                    if going_offline.load(Ordering::Relaxed) {
                        backoff.going_offline();
                    }
                    let (err, reason) = match ended {
                        Ended::Stopped => return,
                        Ended::Closed(None) => {
                            ("the Miniserver closed the connection".into(), None)
                        }
                        Ended::Closed(Some(code)) => (
                            format!("the Miniserver closed the connection (close code {code})"),
                            Reason::from_close(code),
                        ),
                        Ended::Failed(e) => {
                            let r = Reason::from_text(&e);
                            (e, r)
                        }
                    };
                    (err, reason, false)
                }
            };
            let (reason, next) = backoff.next(reason, http);
            let detail = |tail: &str| match reason {
                Some(r) => format!("{}\n({err}) · {tail}", r.sentence()),
                None => format!("{err} · {tail}"),
            };
            let wait = match next {
                Next::Stop => {
                    let tail = "not retrying; turn live values off and on to try again";
                    self.stop_for_good(
                        generation,
                        Status::because("stopped", reason, detail(tail)),
                    );
                    return;
                }
                Next::Wait(w) => w,
            };
            self.put_status(
                generation,
                Status::because("offline", reason, detail(&format!("retrying in {wait} s"))),
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = stop.changed() => return,
            }
        }
    }

    /// A browser asked (with `X-Vislox`) to query `blocks`: a one-use token
    /// for the stream that holds the queries ([`Live::take`]). Unused tokens
    /// expire after [`OFFER_TTL`].
    pub fn offer(&self, blocks: Vec<query::Block>) -> String {
        use sha1::{Digest, Sha1};
        let mut st = self.st.lock().unwrap();
        let now = Instant::now();
        st.offers
            .retain(|_, (_, t)| now.duration_since(*t) < OFFER_TTL);
        st.next_query += 1;
        let mut h = Sha1::new();
        h.update(format!(
            "{:?}|{}|{}|{:p}",
            std::time::SystemTime::now(),
            st.next_query,
            std::process::id(),
            &*st
        ));
        let token = hex::encode(h.finalize());
        st.offers.insert(token.clone(), (blocks, now));
        token
    }

    /// The stream for an offered token: query its blocks while the returned
    /// guard lives. `None` for an unknown, used or expired token.
    pub fn take(self: &Arc<Self>, token: &str) -> Option<Querying> {
        let mut st = self.st.lock().unwrap();
        let (blocks, at) = st.offers.remove(token)?;
        if at.elapsed() >= OFFER_TTL {
            return None;
        }
        st.next_query += 1;
        let id = st.next_query;
        st.queries.insert(id, Arc::new(blocks));
        Some(Querying(self.clone(), id))
    }

    /// A browser left its page: drop the values only it had queried.
    fn unquery(&self, id: u64) {
        let mut st = self.st.lock().unwrap();
        st.queries.remove(&id);
        let blocks: HashSet<String> = st
            .queries
            .values()
            .flat_map(|q| q.iter().map(|b| b.uuid.clone()))
            .collect();
        st.blocks.retain(|b, _| blocks.contains(b));
        let gone: Vec<String> = st
            .queried
            .iter()
            .filter(|(_, b)| !blocks.contains(*b))
            .map(|(u, _)| u.clone())
            .collect();
        let mut out = serde_json::Map::new();
        for u in gone {
            st.queried.remove(&u);
            st.owners.remove(&u);
            if st.values.remove(&u).is_some() {
                out.insert(u, Value::Null);
            }
        }
        drop(st);
        if !out.is_empty() {
            let _ = self
                .tx
                .send(Msg::Values(Arc::new(Value::Object(out).to_string())));
        }
    }

    /// Whether a block needs querying: one that can be mapped, and whose
    /// wanted outputs the Miniserver does not stream.
    fn needs_query(st: &St, b: &query::Block) -> bool {
        !query::unmapped(&b.ty)
            && !b
                .want
                .iter()
                .all(|u| st.owners.get(u).is_some_and(|o| !o.queried))
    }

    /// The blocks to query now: every browser's, once, except those that
    /// need none, failed, or are paused after not answering.
    fn to_query(&self, now: Instant) -> Vec<query::Block> {
        let st = self.st.lock().unwrap();
        let mut out: Vec<query::Block> = Vec::new();
        for q in st.queries.values() {
            for b in q.iter() {
                let s = st.blocks.get(&b.uuid);
                if !Self::needs_query(&st, b)
                    || s.is_some_and(|s| s.failed.is_some() || s.paused.is_some_and(|t| t > now))
                {
                    continue;
                }
                match out.iter_mut().find(|o| o.uuid == b.uuid) {
                    Some(o) => o.want.extend(b.want.iter().cloned()),
                    None => out.push(b.clone()),
                }
            }
        }
        out
    }

    fn still_queried(st: &St, block: &str) -> bool {
        st.queries
            .values()
            .any(|q| q.iter().any(|b| b.uuid == block))
    }

    /// How the queries of one browser's page go: `{blocks, answered,
    /// failed: ["title (type): why"]}`.
    pub fn query_report(&self, q: &Querying) -> Value {
        let st = self.st.lock().unwrap();
        let Some(blocks) = st.queries.get(&q.1) else {
            return json!({ "blocks": 0, "answered": 0, "failed": [] });
        };
        let now = Instant::now();
        let (mut n, mut answered, mut failed) = (0, 0, Vec::new());
        for b in blocks.iter().filter(|b| Self::needs_query(&st, b)) {
            n += 1;
            let s = st.blocks.get(&b.uuid);
            if let Some(why) = s.and_then(|s| s.failed.as_ref()) {
                failed.push(format!("{} ({}): {why}", b.title, b.ty));
            } else if s.is_some_and(|s| s.paused.is_some_and(|t| t > now)) {
                failed.push(format!("{} ({}): no answer", b.title, b.ty));
            } else if s.is_some_and(|s| s.answered) {
                answered += 1;
            }
        }
        failed.sort();
        json!({ "blocks": n, "answered": answered, "failed": failed })
    }

    /// A block's answer (`None`: none within [`QUERY_WAIT`]).
    fn answer(&self, generation: u64, b: &query::Block, ll: Option<Value>, now: Instant) {
        {
            let mut st = self.st.lock().unwrap();
            if st.generation != generation || !Self::still_queried(&st, &b.uuid) {
                return;
            }
            let s = st.blocks.entry(b.uuid.clone()).or_default();
            match &ll {
                None => {
                    s.misses += 1;
                    if s.misses >= QUERY_MISSES {
                        s.misses = 0;
                        s.paused = Some(now + QUERY_PAUSE);
                    }
                    return;
                }
                Some(ll) => {
                    s.misses = 0;
                    s.paused = None;
                    if let Some(why) = query::failure(ll) {
                        s.failed = Some(why);
                        return;
                    }
                    s.answered = true;
                }
            }
        }
        if let Some(ll) = ll {
            self.put_queried(generation, b, &ll);
        }
    }

    /// Store a block's queried outputs; a streamed value always wins, and an
    /// answer for a block no page shows any more is dropped.
    fn put_queried(&self, generation: u64, b: &query::Block, ll: &Value) {
        let mapped = query::map(b, ll);
        let mut out = serde_json::Map::new();
        {
            let mut st = self.st.lock().unwrap();
            if st.generation != generation || !Self::still_queried(&st, &b.uuid) {
                return;
            }
            for (uuid, value, key, name) in mapped {
                if st.owners.get(&uuid).is_some_and(|o| !o.queried) {
                    continue;
                }
                let owner = Owner {
                    label: format!("{} · {name}, queried", b.title),
                    state: key,
                    format: None,
                    names: None,
                    queried: true,
                };
                let val: Val = (value, None);
                if st.values.get(&uuid) != Some(&val) {
                    out.insert(uuid.clone(), entry(&val, Some(&owner)));
                    st.values.insert(uuid.clone(), val);
                }
                st.owners.insert(uuid.clone(), owner);
                st.queried.insert(uuid, b.uuid.clone());
            }
        }
        if !out.is_empty() {
            let _ = self
                .tx
                .send(Msg::Values(Arc::new(Value::Object(out).to_string())));
        }
    }

    /// Query the blocks browsers asked for, in rounds, on the session's
    /// socket, once the session is live. Requests go out every
    /// [`QUERY_GAP`] without waiting for earlier answers, so a block that
    /// does not answer holds up no other. Runs until the session ends.
    async fn query_blocks(
        &self,
        generation: u64,
        live: &AtomicBool,
        asked: mpsc::Sender<session::Query>,
    ) {
        use futures_util::StreamExt;
        use futures_util::stream::FuturesUnordered;
        // What the last connection learnt (failed, paused) is not kept.
        self.st.lock().unwrap().blocks.clear();
        let mut gap = tokio::time::interval(QUERY_GAP);
        gap.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let start = tokio::time::Instant::now();
            let mut todo: std::collections::VecDeque<query::Block> = if live.load(Ordering::Relaxed)
            {
                self.to_query(Instant::now()).into()
            } else {
                Default::default()
            };
            let idle = todo.is_empty();
            let mut waiting = FuturesUnordered::new();
            loop {
                tokio::select! {
                    Some((b, ll)) = waiting.next(), if !waiting.is_empty() => {
                        self.answer(generation, &b, ll, Instant::now());
                    }
                    _ = gap.tick(), if !todo.is_empty() => {
                        let b = todo.pop_front().expect("not empty");
                        if !Self::still_queried(&self.st.lock().unwrap(), &b.uuid) {
                            continue;
                        }
                        let (reply, answer) = oneshot::channel();
                        let q = session::Query { uuid: b.uuid.clone(), reply };
                        if asked.send(q).await.is_err() {
                            return std::future::pending().await;
                        }
                        waiting.push(async move {
                            let ll = tokio::time::timeout(QUERY_WAIT, answer).await.ok().and_then(Result::ok);
                            (b, ll)
                        });
                    }
                    else => break,
                }
            }
            let wait = if idle {
                Duration::from_millis(500)
            } else {
                QUERY_ROUND
            };
            tokio::time::sleep_until(start + wait).await;
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
                let reason = match v["reason"].as_str() {
                    Some("out_of_service") => Some(Reason::OutOfService),
                    _ => why.and_then(Reason::from_text),
                };
                let s = match (reason, why) {
                    (Some(r), Some(w)) => {
                        Status::because("offline", reason, format!("{}\n({w})", r.sentence()))
                    }
                    (Some(r), None) => Status::because("offline", reason, r.sentence().into()),
                    (None, w) => Status::new("offline", w.map(String::from)),
                };
                self.put_status(0, s);
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
            queried: false,
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
    match owner {
        Some(o) if o.queried => json!([v.0, v.1, o.label, "queried"]),
        _ => json!([v.0, v.1, owner.map(|o| o.label.as_str())]),
    }
}

/// One browser's page with block queries on; dropping it ends them.
pub struct Querying(Arc<Live>, u64);

impl Drop for Querying {
    fn drop(&mut self) {
        self.0.unquery(self.1);
    }
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
        assert_eq!(controls["c1"]["secured"], false);
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
        assert_eq!(live.status().reason, None);
    }

    #[test]
    fn a_known_disconnect_reason_is_explained() {
        let live = Live::new(Feed::Off("test".into()), "test".into());
        let mut batch = serde_json::Map::new();
        live.ndjson_line(
            &json!({"event": "disconnected", "reason": "out_of_service"}),
            &mut batch,
        );
        assert_eq!(live.status().reason, Some(Reason::OutOfService));
        let j: Value = serde_json::from_str(&live.status_json()).unwrap();
        assert_eq!(j["label"], "going offline");
        live.ndjson_line(
            &json!({"event": "disconnected", "reason": "error", "message": "HTTP 401: /jdev/sps/LoxAPP3.json"}),
            &mut batch,
        );
        let st = live.status();
        assert_eq!(st.reason, Some(Reason::Unauthorized));
        let d = st.detail.unwrap();
        assert!(
            d.starts_with("The Miniserver refused")
                && d.ends_with("(HTTP 401: /jdev/sps/LoxAPP3.json)")
        );
        let j: Value = serde_json::from_str(&live.status_json()).unwrap();
        assert_eq!(j["reason"], "unauthorized");
    }

    fn block(uuid: &str, ty: &str, ports: &[(&str, &str)], want: &[&str]) -> query::Block {
        query::Block {
            uuid: uuid.into(),
            ty: ty.into(),
            title: format!("{ty} 1"),
            ports: ports
                .iter()
                .map(|(k, u)| (k.to_string(), u.to_string(), k.to_string()))
                .collect(),
            want: want.iter().map(|w| w.to_string()).collect(),
        }
    }

    #[test]
    fn queried_values_are_marked_and_go_when_no_page_needs_them() {
        let live = Live::new(Feed::Off("test".into()), "test".into());
        let mut rx = live.tx.subscribe();
        // `s1` is streamed: the query neither asks for it nor overwrites it.
        live.load_structure(
            0,
            &json!({ "controls": { "c": { "name": "Lux", "type": "InfoOnlyAnalog", "states": { "value": "s1" } } } }),
        );
        live.apply(
            0,
            vec![StateEvent::ValueState {
                uuid: "s1".into(),
                value: 5.0,
            }],
        );
        let f = block(
            "f",
            "Formula",
            &[("Input1", "f-i"), ("AQ", "f-aq"), ("TQ", "f-tq")],
            &["f-aq"],
        );
        let streamed = block("v", "VirtualIn", &[("Q", "s1")], &["s1"]);
        let tab1 = tab(&live, vec![f.clone(), streamed.clone()]);
        let tab2 = tab(&live, vec![f.clone()]);
        let todo: Vec<String> = live
            .to_query(Instant::now())
            .into_iter()
            .map(|b| b.uuid)
            .collect();
        assert_eq!(todo, ["f"]);

        let ll = json!({"Code": "200", "output0": {"name": "R", "nr": 1, "value": 21.5},
                        "output1": {"name": "E", "nr": 2, "value": "ok"}});
        live.put_queried(0, &f, &ll);
        let snap: Value = serde_json::from_str(&live.snapshot_json()).unwrap();
        assert_eq!(
            snap["values"]["f-aq"],
            json!([21.5, null, "Formula 1 · AQ, queried", "queried"])
        );
        assert_eq!(snap["values"]["f-tq"][0], "ok");
        assert_eq!(snap["values"]["s1"], json!([5, null, "Lux · value"]));
        // A block that answers for a streamed connector does not overwrite it.
        live.put_queried(0, &streamed, &json!({"Code": "200", "value": "7"}));
        let snap: Value = serde_json::from_str(&live.snapshot_json()).unwrap();
        assert_eq!(snap["values"]["s1"][0], 5);
        // A session that ended writes nothing.
        live.put_queried(
            3,
            &f,
            &json!({"Code": "200", "output0": {"name": "R", "nr": 1, "value": 1}}),
        );
        assert_eq!(live.st.lock().unwrap().values["f-aq"].0, json!(21.5));

        // One tab leaves: the other still shows the block.
        drop(tab1);
        assert!(live.st.lock().unwrap().values.contains_key("f-aq"));
        while rx.try_recv().is_ok() {}
        drop(tab2);
        let st = live.st.lock().unwrap();
        assert!(!st.values.contains_key("f-aq") && !st.owners.contains_key("f-aq"));
        assert!(st.values.contains_key("s1"));
        drop(st);
        match rx.try_recv() {
            Ok(Msg::Values(v)) => {
                let v: Value = serde_json::from_str(&v).unwrap();
                assert_eq!(v["f-aq"], Value::Null);
                assert_eq!(v["f-tq"], Value::Null);
            }
            _ => panic!("the browsers learn that the values are gone"),
        }
        // An answer that comes after the last tab left is dropped.
        live.answer(0, &f, Some(ll), Instant::now());
        assert!(!live.st.lock().unwrap().values.contains_key("f-aq"));
        assert!(rx.try_recv().is_err());
    }

    fn tab(live: &Arc<Live>, blocks: Vec<query::Block>) -> Querying {
        let token = live.offer(blocks);
        live.take(&token).expect("a fresh token")
    }

    #[test]
    fn a_query_token_is_good_once() {
        let live = Live::new(Feed::Off("test".into()), "test".into());
        let t1 = live.offer(vec![block("a", "Formula", &[("AQ", "a-aq")], &["a-aq"])]);
        let t2 = live.offer(Vec::new());
        assert_ne!(t1, t2);
        assert!(live.take("guess").is_none());
        let q = live.take(&t1).expect("offered");
        assert!(live.take(&t1).is_none(), "used");
        assert_eq!(live.query_report(&q)["blocks"], 1);
        // An offer nobody takes in time expires.
        live.st.lock().unwrap().offers.get_mut(&t2).unwrap().1 -= OFFER_TTL;
        assert!(live.take(&t2).is_none());
    }

    #[test]
    fn errors_and_silence_are_reported_per_page() {
        let live = Live::new(Feed::Off("test".into()), "test".into());
        let a = block("a", "Formula", &[("AQ", "a-aq")], &["a-aq"]);
        let b = block("b", "Formula", &[("AQ", "b-aq")], &["b-aq"]);
        let c = block("c", "Alarm", &[("Q", "c-q")], &["c-q"]);
        let p1 = tab(&live, vec![a.clone(), c.clone()]);
        let p2 = tab(&live, vec![b.clone()]);
        let now = Instant::now();
        live.answer(0, &c, Some(json!({"Code": "500", "value": ""})), now);
        for _ in 0..QUERY_MISSES - 1 {
            live.answer(0, &a, None, now);
        }
        // Two misses: still asked.
        let todo = |at| -> Vec<String> { live.to_query(at).into_iter().map(|b| b.uuid).collect() };
        assert_eq!(todo(now).len(), 2);
        live.answer(0, &a, None, now);
        // The third: paused, and it holds up nothing else.
        assert_eq!(todo(now), ["b"]);
        assert_eq!(todo(now + QUERY_PAUSE + Duration::from_secs(1)).len(), 2);
        let r1 = live.query_report(&p1);
        assert_eq!(r1["blocks"], 2);
        assert_eq!(
            r1["failed"],
            json!([
                "Alarm 1 (Alarm): Code 500",
                "Formula 1 (Formula): no answer"
            ])
        );
        // The other page sees only its own block.
        live.answer(0, &b, Some(json!({"Code": "200", "value": "1"})), now);
        assert_eq!(
            live.query_report(&p2),
            json!({"blocks": 1, "answered": 1, "failed": []})
        );
    }

    #[tokio::test]
    async fn a_silent_block_holds_up_no_other() {
        let live = Live::new(Feed::Off("test".into()), "test".into());
        let silent = block("a", "Formula", &[("AQ", "a-aq")], &["a-aq"]);
        let quick = block("b", "Formula", &[("AQ", "b-aq")], &["b-aq"]);
        let _page = tab(&live, vec![silent, quick]);
        let (asked, mut queries) = mpsc::channel::<session::Query>(1);
        // The Miniserver: answers b at once, never a.
        let miniserver = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Some(q) = queries.recv().await {
                if q.uuid == "b" {
                    let _ = q.reply.send(
                        json!({"Code": "200", "output0": {"name": "R", "nr": 1, "value": 4}}),
                    );
                } else {
                    held.push(q);
                }
            }
        });
        let on = AtomicBool::new(true);
        let l = live.clone();
        let started = Instant::now();
        let run = async move { l.query_blocks(0, &on, asked).await };
        tokio::select! {
            () = run => unreachable!(),
            () = async {
                while !live.st.lock().unwrap().values.contains_key("b-aq") {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => {}
        }
        assert!(started.elapsed() < QUERY_WAIT, "b waited for a");
        miniserver.abort();
    }

    fn running(live: &Live) -> bool {
        live.st.lock().unwrap().run.is_some()
    }

    #[tokio::test]
    async fn a_stopped_session_stays_stopped_until_a_user_turns_live_on() {
        // Nothing listens there: attempts fail at once and are retried.
        let conn = Connection::new("http://127.0.0.1:9", "u", "p");
        let live = Live::new(Feed::Miniserver(Box::new(conn)), "test".into());
        let (_rx, tab) = live.subscribe();
        assert!(running(&live));
        let generation = live.st.lock().unwrap().generation;
        let r = Reason::Blocked;
        live.stop_for_good(generation, Status::because("stopped", Some(r), "x".into()));
        assert!(!running(&live));
        // A reload, a second tab, a reconnecting stream: no new login.
        drop(tab);
        let (_rx2, _tab2) = live.subscribe();
        let (_rx3, _tab3) = live.subscribe();
        assert!(!running(&live));
        let st = live.status();
        assert_eq!((st.state, st.reason), ("stopped", Some(Reason::Blocked)));
        let j: Value = serde_json::from_str(&live.status_json()).unwrap();
        assert_eq!(j["label"], "blocked");
        // The user turns live values off and on.
        live.retry();
        assert!(running(&live));
        assert_eq!(live.status().state, "connecting");
        live.retry();
        assert!(running(&live));
    }
}
