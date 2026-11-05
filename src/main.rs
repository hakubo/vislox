//! vislox — a read-only, Loxone-Config-style view of the configs `lox` and
//! `loxone-config` manage, with live reload, history and change review.

mod history;
mod live;
mod model;
mod source;

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
    /// or a `lox config init` repository.
    #[arg(default_value = ".")]
    path: PathBuf,
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
}

struct App {
    label: String,
    history: Mutex<History>,
    views: Mutex<HashMap<(String, String), Arc<String>>>,
    tx: broadcast::Sender<u64>,
    live: Arc<live::Live>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let src = source::Source::resolve(&args.path, args.serial.as_deref())?;
    let history = History::new(src.current.clone(), src.proposed.clone());
    eprintln!("vislox: watching {}", src.current.display());
    if let Some(p) = &src.proposed {
        eprintln!("vislox: pending changes from {}", p.display());
    }
    if let Some(root) = history.git_root() {
        eprintln!("vislox: history from git repository {}", root.display());
    }
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
        let start = src.current.parent().unwrap_or(&args.path);
        match live::find_miniserver(start, src.miniserver.as_deref(), args.ctx.as_deref()) {
            Ok((conn, from)) => {
                let host = conn.host.clone();
                eprintln!("vislox: live values from {host} (lox config {from})");
                (live::Feed::Miniserver(Box::new(conn)), host)
            }
            Err(why) => {
                if args.ctx.is_some() {
                    anyhow::bail!("{why}");
                }
                eprintln!("vislox: no live values: {why}");
                (live::Feed::Off(why), String::new())
            }
        }
    };
    let (tx, _) = broadcast::channel(16);
    let app = Arc::new(App {
        label: src.label.clone(),
        history: Mutex::new(history),
        views: Mutex::new(HashMap::new()),
        tx,
        live: live::Live::new(feed, live_source),
    });

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
    let stream = BroadcastStream::new(rx)
        .filter_map(|v| v.ok())
        .map(|v| Ok(Event::default().event("history").data(v.to_string())));
    Sse::new(stream).keep_alive(KeepAlive::default())
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
