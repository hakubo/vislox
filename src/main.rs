//! vislox — a read-only, Loxone-Config-style view of the configs `lox` and
//! `loxone-config` manage, with live reload, history and change review.

mod actions;
mod appcalendar;
mod calendar;
mod configdate;
mod discover;
mod history;
mod live;
mod log;
mod miniserver;
mod model;
mod operate;
mod pull;
mod query;
mod reason;
mod restart;
mod session;
mod source;
mod structure;

use anyhow::{Context, Result};
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use futures_util::Stream;
use history::History;
use loxone_config::LoxoneDoc;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

/// Visualize a Loxone config the way Loxone Config draws it — read-only, live
/// reloading, with history and changes marked for review.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// A lox.toml project (or its directory), a .Loxone file, a backup .zip,
    /// or a `lox config init` repository. Default: the current directory.
    path: Option<PathBuf>,
    /// List the Miniservers on the local network (UDP discovery), then exit.
    #[arg(long)]
    discover: bool,
    /// In a config repository with several Miniservers: which one.
    #[arg(long)]
    serial: Option<String>,
    /// Address to serve the UI on.
    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: SocketAddr,
    /// Open the UI in the default browser.
    #[arg(long)]
    open: bool,
    /// Live values: the lox context (`lox ctx`) of the Miniserver. Default:
    /// the context for the Miniserver being viewed, else the active one.
    #[arg(long, value_name = "NAME")]
    ctx: Option<String>,
    /// Live values from a file (or `-`: standard input) of
    /// `lox stream -o json --initial` lines, instead of the Miniserver.
    #[arg(long, value_name = "FILE")]
    live_from: Option<PathBuf>,
    /// No live values: never connect to the Miniserver.
    #[arg(long, conflicts_with = "live_from")]
    no_live: bool,
    /// No "Pull from Miniserver" button.
    #[arg(long)]
    no_pull: bool,
    /// The lox binary that pulls the config (`lox config pull`).
    #[arg(long, value_name = "PATH", default_value = "lox")]
    lox: PathBuf,
    /// For a lox.toml project: a file holding the admin token that Apply,
    /// Sync and Discard ask for. Without it they are off.
    #[arg(long, value_name = "FILE")]
    admin_token_file: Option<PathBuf>,
    /// Let the browser operate virtual inputs on the Miniserver (Pulse,
    /// On/Off, a value) to test a change. Needs --admin-token-file and live
    /// values from the Miniserver. Off by default: vislox only listens.
    #[arg(long)]
    operate: bool,
}

struct App {
    label: String,
    history: Mutex<History>,
    views: Mutex<HashMap<(String, String), Arc<String>>>,
    tx: broadcast::Sender<u64>,
    live: Arc<live::Live>,
    /// The Miniserver's structure file: what the app shows.
    structure: Arc<structure::Structure>,
    /// The calendar entries the Miniserver uses, edited in the app or not.
    calendar: appcalendar::AppCalendar,
    /// Their drift from the deployed revision (see [`CalDrift`]).
    cal_drift: Mutex<Option<CalDrift>>,
    pull: pull::Pull,
    log: log::Log,
    ms: miniserver::Miniserver,
    actions: actions::Actions,
    operate: Arc<operate::Operate>,
}

/// A calendar drift, for (the deployed revision's blob, the read's seq):
/// a read with the same result reuses it, with its own `read_at`.
type CalDrift = ((String, u64), Arc<serde_json::Value>);

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.discover {
        let found = tokio::task::spawn_blocking(|| discover::search(discover::WAIT)).await?;
        println!("{}", discover::describe(&found));
        return Ok(());
    }
    let path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let src = match source::Source::resolve(&path, args.serial.as_deref()) {
        Ok(s) => s,
        // Started without a path in a directory with no config: say which
        // Miniservers there are, so the user can pick one.
        Err(e) if offer_discovery(&e, &args) => {
            eprintln!("vislox: {e:#}\nvislox: looking for Miniservers on the local network…");
            let found = tokio::task::spawn_blocking(|| discover::search(discover::WAIT)).await?;
            eprintln!("{}", discover::describe(&found));
            std::process::exit(2);
        }
        Err(e) => return Err(e),
    };
    let history = History::new(src.current.clone(), src.proposed.clone());
    eprintln!("vislox: watching {}", src.current.display());
    if let Some(p) = &src.proposed {
        eprintln!("vislox: pending changes from {}", p.display());
    }
    if let Some(root) = history.git_root() {
        eprintln!("vislox: history from git repository {}", root.display());
    }
    let start = src.current.parent().unwrap_or(&path).to_path_buf();
    let found = live::find_miniserver(&start, src.miniserver.as_deref(), args.ctx.as_deref());
    if let (Some(_), Err(why)) = (&args.ctx, &found) {
        anyhow::bail!("{why}");
    }
    let pull = pull::Pull::new(
        &args.lox,
        args.no_pull.then_some("turned off (--no-pull)"),
        &src,
        found.as_ref(),
        &start,
    );
    match pull.why_not() {
        None => eprintln!("vislox: \"Pull from Miniserver\" runs `lox config pull`"),
        Some(why) if src.miniserver.is_some() => eprintln!("vislox: no pull: {why}"),
        Some(_) => {}
    }
    // The Log panel reads `lox log`; --no-live means vislox never talks to
    // the Miniserver.
    let log = log::Log::new(
        &args.lox,
        args.no_live.then_some("turned off (--no-live)"),
        found.as_ref(),
        &start,
    );
    let (feed, live_source) = if args.no_live {
        (
            live::Feed::Off("turned off (--no-live)".into()),
            String::new(),
        )
    } else if let Some(f) = &args.live_from {
        let name = if f.as_os_str() == "-" {
            "standard input".to_string()
        } else {
            f.display().to_string()
        };
        (live::Feed::Ndjson(f.clone()), name)
    } else {
        match &found {
            Ok(f) => {
                let host = f.conn.host.clone();
                eprintln!("vislox: live values from {host} (lox config {})", f.from);
                (live::Feed::Miniserver(Box::new(f.conn.clone())), host)
            }
            Err(why) => {
                eprintln!("vislox: no live values: {why}");
                (live::Feed::Off(why.clone()), String::new())
            }
        }
    };
    // The status chip and the preflight check the Miniserver lox talks to,
    // unless vislox was told never to connect to it.
    let ms = miniserver::Miniserver::new(
        if args.no_live {
            Err("turned off (--no-live)".to_string())
        } else {
            found.as_ref().map(|f| &f.conn).map_err(Clone::clone)
        },
        src.current.clone(),
        src.proposed.clone(),
        src.project.as_ref().map(|p| p.join("backups")),
    )
    .expecting(expected_serial(&src, found.as_ref().ok()));
    let operate = {
        let conn = match (&found, args.no_live || args.live_from.is_some()) {
            (Ok(f), false) => Some(f.conn.clone()),
            _ => None,
        };
        let off = if !args.operate {
            Some("start vislox with --operate".to_string())
        } else if args.admin_token_file.is_none() {
            Some("start vislox with --admin-token-file".to_string())
        } else if conn.is_none() {
            Some("needs live values from the Miniserver (not --no-live or --live-from)".to_string())
        } else {
            None
        };
        let user = conn.as_ref().map(|c| c.user.clone()).unwrap_or_default();
        let port = conn
            .filter(|_| off.is_none())
            .map(|c| Arc::new(operate::Http(c)) as Arc<dyn operate::Port>);
        match &off {
            None => eprintln!(
                "vislox: operating virtual inputs is ON (--operate): commands go to the Miniserver"
            ),
            Some(why) if args.operate => eprintln!("vislox: no operating inputs: {why}"),
            Some(_) => {}
        }
        Arc::new(operate::Operate::new(operate::Settings { off, user }, port))
    };
    // The structure file comes with live values, or when a user asks for it;
    // like the status chip, never with --no-live.
    let structure = structure::Structure::new(
        if args.no_live {
            Err("turned off (--no-live)".to_string())
        } else {
            found.as_ref().map(|f| f.conn.clone()).map_err(Clone::clone)
        },
        found.as_ref().ok().map(|f| f.ms_dir.as_str()),
    );
    // The calendar entries come with the structure file (live values, or a
    // user's request), and after Pull, Apply and Sync; never with --no-live.
    let calendar = appcalendar::AppCalendar::new(if args.no_live {
        Err("turned off (--no-live)".to_string())
    } else {
        found.as_ref().map(|f| f.conn.clone()).map_err(Clone::clone)
    });
    let live = live::Live::new(feed, live_source);
    live.share_structure(structure.clone());
    let (tx, _) = broadcast::channel(16);
    let app = Arc::new(App {
        label: src.label.clone(),
        history: Mutex::new(history),
        views: Mutex::new(HashMap::new()),
        tx,
        live,
        structure,
        calendar,
        cal_drift: Mutex::new(None),
        pull,
        log,
        ms,
        actions: actions::Actions::new(actions::Settings {
            token: args
                .admin_token_file
                .as_deref()
                .map(actions::read_secret)
                .transpose()?,
            project: src.project.clone(),
            lox: args.lox.clone(),
            // Follows Apply's program restart (read-only); --no-live means
            // vislox never talks to the Miniserver.
            probe: match (&found, &src.project, args.no_live) {
                (Ok(f), Some(_), false) => Some(Arc::new(restart::Http::new(f.conn.clone()))),
                _ => None,
            },
        }),
        operate,
    });
    if src.project.is_some() {
        match app.actions.info()["apply"]["reason"].as_str() {
            None => eprintln!("vislox: Apply, Sync and Discard on"),
            Some(why) => eprintln!("vislox: no Apply: {why}"),
        }
    }

    // Check the Miniserver once at start, and say what is wrong.
    {
        let app = app.clone();
        tokio::spawn(async move {
            let v = app.ms.chip().await;
            if v["available"] != true {
                return;
            }
            match v["blocked"].as_str() {
                Some(why) => eprintln!("vislox: preflight: {why}"),
                None => eprintln!(
                    "vislox: Miniserver {} at {} runs {}",
                    v["serial"].as_str().unwrap_or("?"),
                    v["host"].as_str().unwrap_or("?"),
                    v["firmware"].as_str().unwrap_or("?")
                ),
            }
            for w in v["warnings"].as_array().into_iter().flatten() {
                eprintln!("vislox: warning: {}", w.as_str().unwrap_or_default());
            }
        });
    }

    // Each successful structure file read (a live session starting, or a
    // user asking) also asks for the calendar entries, at most once a minute.
    // A failed load asks for nothing: the Miniserver did not answer.
    {
        let app = app.clone();
        let mut rx = app.structure.subscribe();
        tokio::spawn(async move {
            while let Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) = rx.recv().await {
                app.calendar.after_structure(app.structure.reads()).await;
            }
        });
    }

    spawn_watcher(app.clone())?;
    // Fallback poll, and the only way to notice new git commits.
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3));
            loop {
                tick.tick().await;
                let app = app.clone();
                let _ = tokio::task::spawn_blocking(move || refresh(&app)).await;
            }
        });
    }

    let router = Router::new()
        .route(
            "/",
            get(|| async { asset("text/html", include_str!("../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async { asset("text/javascript", include_str!("../web/app.js")) }),
        )
        .route(
            "/app.css",
            get(|| async { asset("text/css", include_str!("../web/app.css")) }),
        )
        .route(
            "/icon.svg",
            get(|| async { asset("image/svg+xml", include_str!("../assets/icon.svg")) }),
        )
        .route("/api/history", get(api_history))
        .route("/api/view", get(api_view))
        .route("/api/events", get(api_events))
        .route("/api/live", get(api_live))
        .route("/api/live/retry", axum::routing::post(api_live_retry))
        .route(
            "/api/live/query",
            get(api_live_query).post(api_live_query_start),
        )
        .route("/api/miniserver", get(api_miniserver))
        .route("/api/log", get(api_log))
        .route("/api/app", get(api_app))
        .route("/api/app/load", axum::routing::post(api_app_load))
        .route("/api/app/icon", get(api_app_icon))
        .route("/api/calendar", get(api_calendar))
        .route("/api/pull", axum::routing::post(api_pull))
        .route("/api/apply", axum::routing::post(api_apply))
        .route("/api/discard", axum::routing::post(api_discard))
        .route("/api/sync", axum::routing::post(api_sync))
        .route("/api/operate", axum::routing::post(api_operate))
        .with_state(app)
        .layer(tower_http::compression::CompressionLayer::new());

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("cannot listen on {}", args.listen))?;
    let url = format!("http://{}", listener.local_addr()?);
    eprintln!("vislox: open {url}");
    if args.open {
        let _ = open::that(&url);
    }
    // Browsers keep the live-update stream open for good, so waiting for
    // connections to close would never end: stop at once instead.
    tokio::select! {
        res = axum::serve(listener, router) => res?,
        _ = shutdown_signal() => {
            eprintln!("vislox: stopped");
            std::process::exit(0);
        }
    }
    Ok(())
}

/// Whether a failed start lists the Miniservers on the LAN: only when
/// vislox was started without a path or context in a directory that holds
/// no config at all, and may talk to the Miniserver (not `--no-live`).
/// Other errors (several Miniservers, a wrong `--serial`, a broken lox.toml)
/// say what to fix themselves.
fn offer_discovery(e: &anyhow::Error, args: &Args) -> bool {
    args.path.is_none()
        && args.ctx.is_none()
        && !args.no_live
        && e.downcast_ref::<source::NoConfig>().is_some()
}

/// The serial of the Miniserver the config belongs to: the repository's
/// `<serial>` folder, else lox.toml's `serial`, else the lox context's.
fn expected_serial(
    src: &source::Source,
    found: Option<&live::Found>,
) -> Option<miniserver::Expected> {
    let ctx = found.and_then(|f| f.ctx.clone());
    let at = |serial: &str, from: String| miniserver::Expected {
        serial: discover::norm_serial(serial),
        from,
        ctx: ctx.clone(),
    };
    if let Some(ms) = src.miniserver.as_deref().filter(|m| discover::is_serial(m)) {
        return Some(at(ms, "the repository folder".into()));
    }
    if let Some(serial) = &src.serial {
        return Some(at(serial, "lox.toml".into()));
    }
    let f = found?;
    let from = match &f.ctx {
        Some(c) => format!("lox context {c}"),
        None => "lox's configuration".into(),
    };
    f.serial.as_deref().map(|s| at(s, from))
}

/// Ctrl+C, or SIGTERM where there is one (`kill`, systemd, docker stop).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install the SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

fn asset(ty: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, ty),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

/// Re-read files and git; tell the browsers when something changed.
fn refresh(app: &App) {
    let version = {
        let mut h = app.history.lock().unwrap();
        let a = h.refresh_git();
        let b = h.poll_files();
        (a || b).then_some(h.version)
    };
    if let Some(v) = version {
        let _ = app.tx.send(v);
    }
}

fn spawn_watcher(app: Arc<App>) -> Result<()> {
    use notify::{RecursiveMode, Watcher};
    let paths = app.history.lock().unwrap().paths();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    let mut dirs: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| p.parent().map(PathBuf::from))
        .collect();
    dirs.sort();
    dirs.dedup();
    for d in &dirs {
        watcher.watch(d, RecursiveMode::NonRecursive)?;
    }
    let names: Vec<_> = paths
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_owned()))
        .collect();
    std::thread::spawn(move || {
        let _watcher = watcher;
        while let Ok(ev) = rx.recv() {
            let relevant = |ev: &notify::Result<notify::Event>| {
                ev.as_ref().is_ok_and(|e| {
                    e.paths
                        .iter()
                        .any(|p| p.file_name().is_some_and(|n| names.iter().any(|m| m == n)))
                })
            };
            if !relevant(&ev) {
                continue;
            }
            // Let the writer finish, then take one snapshot.
            std::thread::sleep(Duration::from_millis(300));
            while rx.try_recv().is_ok() {}
            refresh(&app);
        }
    });
    Ok(())
}

async fn api_history(State(app): State<Arc<App>>) -> impl IntoResponse {
    let h = app.history.lock().unwrap();
    let (pending, hist) = h.list();
    Json(serde_json::json!({
        "label": app.label,
        "version": h.version,
        "pending": pending,
        "history": hist,
        "error": h.last_error,
        "live": app.live.info(),
        "pull": app.pull.info(),
        "log": app.log.info(),
        "actions": app.actions.info(),
        "operate": app.operate.info(),
    }))
}

#[derive(Deserialize)]
struct ViewQuery {
    rev: String,
    base: Option<String>,
}

async fn api_view(State(app): State<Arc<App>>, Query(q): Query<ViewQuery>) -> Response {
    let res = tokio::task::spawn_blocking(move || {
        view(&app, &q.rev, q.base.as_deref().filter(|b| !b.is_empty()))
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    match res {
        Ok(body) => (
            [(header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

fn load(app: &App, rev: &str) -> Result<(String, LoxoneDoc), String> {
    let mut h = app.history.lock().unwrap();
    let r = h.find(rev).ok_or_else(|| format!("no revision {rev}"))?;
    let bytes = h
        .content(&r.blob)
        .ok_or_else(|| format!("cannot read the content of {rev}"))?;
    drop(h);
    let xml = source::program_xml(&bytes).map_err(|e| e.to_string())?;
    let doc = LoxoneDoc::parse(&xml).map_err(|e| format!("{rev}: {e}"))?;
    Ok((r.blob, doc))
}

fn view(app: &App, rev: &str, base: Option<&str>) -> Result<Arc<String>, String> {
    let blob_of = |id: &str| app.history.lock().unwrap().find(id).map(|r| r.blob);
    let key = (
        blob_of(rev).ok_or_else(|| format!("no revision {rev}"))?,
        base.and_then(blob_of).unwrap_or_default(),
    );
    if let Some(v) = app.views.lock().unwrap().get(&key) {
        return Ok(v.clone());
    }
    let (_, new) = load(app, rev)?;
    let old = base.map(|b| load(app, b)).transpose()?;
    let title = new
        .document()
        .and_then(|d| d.attr_decoded("Title").map(|t| t.into_owned()))
        .unwrap_or_default();
    let v = model::build(&new, old.as_ref().map(|(_, d)| d), &title);
    let json = Arc::new(serde_json::to_string(&v).map_err(|e| e.to_string())?);
    let mut cache = app.views.lock().unwrap();
    if cache.len() > 24 {
        cache.clear();
    }
    cache.insert(key, json.clone());
    Ok(json)
}

async fn api_events(
    State(app): State<Arc<App>>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = app.tx.subscribe();
    let history = BroadcastStream::new(rx)
        .filter_map(|v| v.ok())
        .map(|v| Ok(Event::default().event("history").data(v.to_string())));
    // Apply, Sync and Discard: the whole state, on every change.
    let a = app.clone();
    let actions = BroadcastStream::new(app.actions.subscribe()).map(move |_| {
        Ok(Event::default()
            .event("actions")
            .data(a.actions.info().to_string()))
    });
    // Commands sent to inputs: the whole log, on every command.
    let o = app.clone();
    let operate = BroadcastStream::new(app.operate.subscribe()).map(move |_| {
        Ok(Event::default()
            .event("operate")
            .data(o.operate.info().to_string()))
    });
    // The structure file: `changed` (the In the app section reads it again),
    // or `read` again unchanged (only a browser waiting after Apply cares).
    let app_changed = BroadcastStream::new(app.structure.subscribe()).map(|e| {
        let what = match e {
            Ok(structure::Event::Read) => "read",
            _ => "changed",
        };
        Ok(Event::default().event("app").data(what))
    });
    // The calendar entries on the Miniserver: read again, with another result.
    let calendar = BroadcastStream::new(app.calendar.subscribe())
        .map(|_| Ok(Event::default().event("calendar").data("changed")));
    Sse::new(
        history
            .merge(actions)
            .merge(operate)
            .merge(app_changed)
            .merge(calendar),
    )
    .keep_alive(KeepAlive::default())
}

/// Requests that make vislox talk to the Miniserver carry an `X-Vislox`
/// header. A page on another site cannot send it without asking first
/// (CORS), so it cannot make vislox log in, pull or read the log.
/// The refusal, when the header is missing.
fn not_from_vislox(headers: &axum::http::HeaderMap) -> Option<Response> {
    (!headers.contains_key("x-vislox")).then(|| {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "missing X-Vislox header" })),
        )
            .into_response()
    })
}

/// What the app shows, from the last structure file read (`LoxAPP3.json`).
async fn api_app(State(app): State<Arc<App>>) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        app.structure.json().to_string(),
    )
        .into_response()
}

/// Read the structure file from the Miniserver now (read-only, with lox's
/// user). Needs the `X-Vislox` header, so another site cannot make vislox
/// log in.
async fn api_app_load(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Response {
    if let Some(r) = not_from_vislox(&headers) {
        return r;
    }
    match app.structure.load().await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": reason::explain(&e) })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct IconQuery {
    name: String,
}

/// An icon the structure file names, from the cache or the Miniserver.
async fn api_app_icon(State(app): State<Arc<App>>, Query(q): Query<IconQuery>) -> Response {
    match app.structure.icon(&q.name).await {
        Ok(bytes) => {
            let (ty, enc) = structure::icon_type(&bytes);
            let mut r = (
                [
                    (header::CONTENT_TYPE, ty),
                    // A name keeps its icon.
                    (header::CACHE_CONTROL, "max-age=86400"),
                    // The Miniserver's SVG, opened on its own, runs nothing.
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "default-src 'none'; style-src 'unsafe-inline'",
                    ),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                bytes,
            )
                .into_response();
            if let Some(enc) = enc {
                r.headers_mut().insert(
                    header::CONTENT_ENCODING,
                    header::HeaderValue::from_static(enc),
                );
            }
            r
        }
        Err(None) => (StatusCode::NOT_FOUND, "not an icon of the structure file").into_response(),
        Err(Some(e)) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

/// A user turned live values on: after a session that stopped (blocked,
/// login refused), allow a new one. Reloads and reconnecting streams do not
/// call this, so they never log in again on their own. Needs the `X-Vislox`
/// header, so another site cannot make vislox log in.
async fn api_live_retry(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Response {
    if let Some(r) = not_from_vislox(&headers) {
        return r;
    }
    app.live.retry();
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Live values for one browser, while it keeps this stream open: `status`,
/// then a `snapshot` of every value, then `values` as they change.
async fn api_live(
    State(app): State<Arc<App>>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let live = app.live.clone();
    let (rx, watcher) = live.subscribe();
    let first = [
        Event::default().event("status").data(live.status_json()),
        Event::default()
            .event("snapshot")
            .data(live.snapshot_json()),
    ];
    let rest = BroadcastStream::new(rx).map(move |m| {
        let _on = &watcher;
        match m {
            Ok(live::Msg::Status) => Event::default().event("status").data(live.status_json()),
            Ok(live::Msg::Values(v)) => Event::default().event("values").data(v.as_str()),
            // A new structure, or this browser fell behind: start over.
            Ok(live::Msg::Snapshot) | Err(_) => Event::default()
                .event("snapshot")
                .data(live.snapshot_json()),
        }
    });
    let stream = tokio_stream::iter(first).chain(rest).map(Ok);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct LiveQuery {
    rev: String,
    page: String,
}

/// Opt-in: start querying the blocks behind the wires of the page a browser
/// shows. Only for the current, deployed revision (what the Miniserver
/// runs), and only with the `X-Vislox` header, since it makes vislox send
/// requests to the Miniserver: `{token, blocks}` for `GET /api/live/query`.
async fn api_live_query_start(
    State(app): State<Arc<App>>,
    headers: axum::http::HeaderMap,
    Json(q): Json<LiveQuery>,
) -> Response {
    let a = app.clone();
    let plan = move || {
        tokio::task::spawn_blocking(move || -> Result<Vec<query::Block>, String> {
            let current = a
                .history
                .lock()
                .unwrap()
                .find(&q.rev)
                .is_some_and(|r| r.current && r.kind != "pending");
            if !current {
                return Err("only the current, deployed revision is queried".into());
            }
            let (_, doc) = load(&a, &q.rev)?;
            Ok(model::query_plan(&doc, &q.page))
        })
    };
    live_query_start(&app.live, &headers, plan).await
}

async fn live_query_start<F>(
    live: &live::Live,
    headers: &axum::http::HeaderMap,
    plan: impl FnOnce() -> F,
) -> Response
where
    F: std::future::Future<
            Output = Result<Result<Vec<query::Block>, String>, tokio::task::JoinError>,
        >,
{
    if let Some(r) = not_from_vislox(headers) {
        return r;
    }
    match plan().await.map_err(|e| e.to_string()).and_then(|r| r) {
        Ok(blocks) => {
            let n = blocks.len();
            Json(serde_json::json!({ "token": live.offer(blocks), "blocks": n })).into_response()
        }
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct QueryToken {
    token: String,
}

/// The stream that holds one page's block queries (from
/// `POST /api/live/query`): they run while it is open. It carries `query`
/// events (`{blocks, answered, failed}`, for this page only) when they
/// change, or `refused` for an unknown, used or expired token. The values
/// come on `/api/live`.
async fn api_live_query(
    State(app): State<Arc<App>>,
    Query(q): Query<QueryToken>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let live = app.live.clone();
    let guard = live.take(&q.token);
    let refused = guard.is_none().then(|| {
        Event::default()
            .event("refused")
            .data(r#"{"error":"the query token is unknown or used; ask again"}"#)
    });
    let reports = futures_util::stream::unfold(
        (live, guard, String::new(), true),
        |(live, guard, last, first)| async move {
            let guard = guard?;
            let mut last = last;
            loop {
                if !first || !last.is_empty() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                let r = live.query_report(&guard).to_string();
                if r != last {
                    last = r.clone();
                    let e = Event::default().event("query").data(r);
                    return Some((e, (live, Some(guard), last, false)));
                }
            }
        },
    );
    let stream = tokio_stream::iter(refused)
        .chain(reports)
        .chain(tokio_stream::pending())
        .map(Ok);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct MsQuery {
    /// Any value but `0` or `false`.
    fresh: Option<String>,
    /// The page's regular poll: any value but `0` or `false`.
    poll: Option<String>,
}

/// The Miniserver's status for the toolbar chip: at most a minute old, or
/// checked again with `?fresh=1`. The page's regular poll (`?poll=1`) asks
/// only for the config dates again, and reuses the rest for five minutes.
async fn api_miniserver(State(app): State<Arc<App>>, Query(q): Query<MsQuery>) -> Response {
    let on = |v: &Option<String>| v.as_deref().is_some_and(|f| f != "0" && f != "false");
    let v = if on(&q.fresh) {
        app.ms.recheck().await
    } else if on(&q.poll) {
        app.ms.poll().await
    } else {
        app.ms.chip().await
    };
    Json(v.as_ref().clone()).into_response()
}

/// The Miniserver's system log (`lox log`), fetched now. Needs the
/// `X-Vislox` header: lox logs in to the Miniserver for it, and another site
/// must not be able to make it do that.
async fn api_log(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Response {
    if let Some(r) = not_from_vislox(&headers) {
        return r;
    }
    Json(app.log.fetch().await).into_response()
}

/// Refuse Pull, Apply and Sync when the Miniserver cannot be reached the
/// way lox needs it (FTP; for Apply also its web port), with the reason.
async fn preflight(app: &App, what: miniserver::Action) -> Option<Response> {
    let why = app.ms.preflight(what).await.err()?;
    let what = format!("{what:?}").to_lowercase();
    Some(
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                serde_json::json!({ "error": format!("Cannot {what}: {why}"), "preflight": true }),
            ),
        )
            .into_response(),
    )
}

/// Fetch the Miniserver's current config with `lox config pull`, then show
/// the new revision at once. Needs the `X-Vislox` header, which a page on
/// another site cannot send without asking first (CORS), so no other site
/// can start a pull in this browser.
async fn api_pull(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Response {
    if let Some(r) = not_from_vislox(&headers) {
        return r;
    }
    // A pull already running refuses at once, without a check.
    if app.pull.why_not().is_none()
        && !app.pull.busy()
        && let Some(r) = preflight(&app, miniserver::Action::Pull).await
    {
        return r;
    }
    match app.pull.run().await {
        Ok(p) => {
            let a = app.clone();
            let _ = tokio::task::spawn_blocking(move || refresh(&a)).await;
            // The base may have changed: the next check reads its date again.
            app.ms.forget();
            reread_calendar(&app);
            Json(serde_json::json!({ "committed": p.committed })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": reason::explain(&e) })),
        )
            .into_response(),
    }
}

fn refused((status, msg): actions::Refusal) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// Deploy the pending changes with `lox config push --force`, commit, push.
async fn api_apply(
    State(app): State<Arc<App>>,
    headers: axum::http::HeaderMap,
    body: Option<Json<actions::ApplyOpts>>,
) -> Response {
    if let Err(r) = app.actions.check(&headers) {
        return refused(r);
    }
    if app.actions.wants_miniserver()
        && let Some(r) = preflight(&app, miniserver::Action::Apply).await
    {
        return r;
    }
    let opts = body.map(|Json(o)| o).unwrap_or_default();
    let res = app.actions.apply(&opts).await;
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || refresh(&a)).await;
    // The base may have changed: the next check reads its date again.
    app.ms.forget();
    reread_calendar(&app);
    match res {
        Ok(m) => Json(serde_json::json!({ "message": m })).into_response(),
        // What the user could allow to retry (a newer Loxone Config, custom changes).
        Err((status, msg)) => (
            status,
            Json(serde_json::json!({
                "error": reason::explain(&msg),
                "hints": actions::apply_hints(&msg),
            })),
        )
            .into_response(),
    }
}

/// After Pull, Apply and Sync: read the Miniserver's calendar entries
/// again, in the background, now that the base or the Miniserver changed.
fn reread_calendar(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        app.calendar.read(Duration::ZERO).await;
    });
}

#[derive(Deserialize)]
struct CalQuery {
    /// Any value but `0` or `false`: read the entries again (before Apply),
    /// unless a read started in the last few seconds. Needs `X-Vislox`.
    fresh: Option<String>,
}

/// The calendar entries edited in the Loxone app: how the Miniserver's
/// differ from the deployed revision's. Reads nothing from the Miniserver
/// unless `fresh`.
async fn api_calendar(
    State(app): State<Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<CalQuery>,
) -> Response {
    if let Err(why) = app.calendar.available() {
        return Json(serde_json::json!({ "available": false, "reason": why })).into_response();
    }
    if q.fresh.as_deref().is_some_and(|f| f != "0" && f != "false") {
        if let Some(r) = not_from_vislox(&headers) {
            return r;
        }
        app.calendar.read(appcalendar::BEFORE_APPLY).await;
    }
    let a = app.clone();
    match tokio::task::spawn_blocking(move || calendar_drift(&a)).await {
        Ok(Ok(v)) => Json(v.as_ref().clone()).into_response(),
        Ok(Err(e)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// The last read of the Miniserver's calendar entries against the deployed
/// revision (the current one that is not pending). Blocking: it may parse
/// that revision.
fn calendar_drift(app: &App) -> Result<Arc<serde_json::Value>, String> {
    let Some(read) = app.calendar.last() else {
        return Ok(Arc::new(serde_json::json!({ "available": true })));
    };
    let entries = match &read.entries {
        Ok(e) => e.clone(),
        Err(why) => {
            return Ok(Arc::new(serde_json::json!({
                "available": true,
                "read_at": read.at,
                "error": why,
                // No admin rights: skipped quietly, also before Apply.
                "refused": appcalendar::is_refusal(why),
            })));
        }
    };
    let rev = {
        let h = app.history.lock().unwrap();
        let (_, hist) = h.list();
        hist.into_iter().find(|r| r.current)
    };
    let Some(rev) = rev else {
        return Ok(Arc::new(serde_json::json!({
            "available": true,
            "read_at": read.at,
            "error": "no deployed revision to compare them with",
        })));
    };
    let key = (rev.blob.clone(), read.seq);
    if let Some((k, v)) = &*app.cal_drift.lock().unwrap()
        && *k == key
    {
        let mut v = (**v).clone();
        v["read_at"] = read.at.into();
        return Ok(Arc::new(v));
    }
    let (_, doc) = load(app, &rev.id)?;
    let (file, modes) = calendar::of_doc(&doc);
    let drift = calendar::drift(&file, &entries, &modes);
    let v = Arc::new(serde_json::json!({
        "available": true,
        "read_at": read.at,
        "rev": rev.id,
        "entries": entries.len(),
        "drift": drift,
    }));
    *app.cal_drift.lock().unwrap() = Some((key, v.clone()));
    Ok(v)
}

/// Put the module, lock and compiled config back to the last commit.
async fn api_discard(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Response {
    if let Err(r) = app.actions.check(&headers) {
        return refused(r);
    }
    let res = app.actions.discard().await;
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || refresh(&a)).await;
    match res {
        Ok(m) => Json(serde_json::json!({ "message": m })).into_response(),
        Err(r) => refused(r),
    }
}

/// Make the Miniserver's running config the project's base, commit, push.
async fn api_sync(State(app): State<Arc<App>>, headers: axum::http::HeaderMap) -> Response {
    if let Err(r) = app.actions.check(&headers) {
        return refused(r);
    }
    if app.actions.wants_miniserver()
        && let Some(r) = preflight(&app, miniserver::Action::Sync).await
    {
        return r;
    }
    let res = app.actions.sync().await;
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || refresh(&a)).await;
    // The base may have changed: the next check reads its date again.
    app.ms.forget();
    reread_calendar(&app);
    match res {
        Ok(m) => Json(serde_json::json!({ "message": m })).into_response(),
        Err((status, msg)) => refused((status, reason::explain(&msg))),
    }
}

/// The virtual inputs of the deployed revision (the current one that is not
/// pending), parsed once per revision. Blocking.
fn deployed_inputs(app: &App) -> Result<Arc<operate::Inputs>, String> {
    let rev = {
        let h = app.history.lock().unwrap();
        let (_, hist) = h.list();
        hist.into_iter()
            .find(|r| r.current)
            .ok_or("no deployed revision to look the input up in")?
    };
    app.operate
        .inputs_of(&rev.blob, || load(app, &rev.id).map(|(_, d)| d))
}

/// Send one command to a virtual input (opt-in: `--operate`). Needs the
/// `X-Vislox` header and the admin token; the browser has asked the user to
/// confirm it. The request is checked first (400), then the preflight makes
/// sure the Miniserver at the address is the config's.
async fn api_operate(
    State(app): State<Arc<App>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<operate::Req>,
) -> Response {
    if let Some(r) = not_from_vislox(&headers) {
        return r;
    }
    if let Err(r) = app.actions.check(&headers) {
        return refused(r);
    }
    let a = app.clone();
    let inputs = match tokio::task::spawn_blocking(move || deployed_inputs(&a)).await {
        Ok(Ok(i)) => i,
        Ok(Err(e)) => return refused((StatusCode::INTERNAL_SERVER_ERROR, e)),
        Err(e) => return refused((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };
    let res = app
        .operate
        .run(req, &inputs, || {
            app.ms.preflight(miniserver::Action::Operate)
        })
        .await;
    match res {
        Ok(e) => Json(e).into_response(),
        Err(f) => (
            f.status,
            Json(serde_json::json!({ "error": f.error, "badPassword": f.bad_password })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(cli: &[&str]) -> Args {
        Args::parse_from(std::iter::once("vislox").chain(cli.iter().copied()))
    }

    #[test]
    fn only_a_directory_without_any_config_lists_the_lan() {
        let empty = tempfile::tempdir().unwrap();
        let none = source::Source::resolve(empty.path(), None).unwrap_err();
        assert!(offer_discovery(&none, &args(&[])));
        // A path, a context or --no-live: no broadcast.
        assert!(!offer_discovery(&none, &args(&["."])));
        assert!(!offer_discovery(&none, &args(&["--ctx", "home"])));
        assert!(!offer_discovery(&none, &args(&["--no-live"])));

        // Several Miniservers, a wrong --serial, a broken lox.toml: the error says it.
        let repo = tempfile::tempdir().unwrap();
        for serial in ["504F94000001", "504F94000002"] {
            std::fs::create_dir(repo.path().join(serial)).unwrap();
            std::fs::write(repo.path().join(serial).join("config.Loxone"), "x").unwrap();
        }
        let several = source::Source::resolve(repo.path(), None).unwrap_err();
        assert!(!offer_discovery(&several, &args(&[])));
        let wrong = source::Source::resolve(repo.path(), Some("FFFF")).unwrap_err();
        assert!(!offer_discovery(&wrong, &args(&["--serial", "FFFF"])));
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join("lox.toml"), "base = [").unwrap();
        let broken = source::Source::resolve(broken.path(), None).unwrap_err();
        assert!(!offer_discovery(&broken, &args(&[])));
    }

    #[tokio::test]
    async fn only_vislox_starts_block_queries() {
        let live = live::Live::new(live::Feed::Off("test".into()), "test".into());
        let planned = std::sync::atomic::AtomicBool::new(false);
        let plan = || {
            planned.store(true, std::sync::atomic::Ordering::Relaxed);
            tokio::task::spawn_blocking(|| Ok(Vec::new()))
        };
        // Another site's request (no X-Vislox): refused, nothing planned or offered.
        let r = live_query_start(&live, &axum::http::HeaderMap::new(), plan).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(!planned.load(std::sync::atomic::Ordering::Relaxed));
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-vislox", "query".parse().unwrap());
        let r = live_query_start(&live, &h, plan).await;
        assert_eq!(r.status(), StatusCode::OK);
        let body = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let token = j["token"].as_str().unwrap();
        assert!(live.take(token).is_some());
        assert!(live.take(token).is_none());
    }

    #[test]
    fn a_project_s_serial_comes_from_its_lox_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::write(d.join("base.Loxone"), "x").unwrap();
        std::fs::create_dir(d.join("pages")).unwrap();
        std::fs::write(
            d.join("lox.toml"),
            "base = \"base.Loxone\"\nmodule = \"pages\"\nout = \"out.Loxone\"\nserial = \"504f94000001\"\n",
        )
        .unwrap();
        let src = source::Source::resolve(d, None).unwrap();
        let e = expected_serial(&src, None).unwrap();
        assert_eq!(
            (e.serial.as_str(), e.from.as_str()),
            ("504F94000001", "lox.toml")
        );
    }
}
