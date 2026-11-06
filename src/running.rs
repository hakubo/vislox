//! Which config the Miniserver runs.
//!
//! `lox config pull` and `lox config push` take the newest
//! `sps_<ver>_<timestamp>.zip` in `/prog` as the running config. The
//! Miniserver loads by a longer order (from sarnau's
//! Inside-The-Loxone-Miniserver, `loadMiniserverConfigurationFile.py`):
//!
//! 1. `Emergency.LoxCC`, only after several reboots within 10 minutes
//! 2. `sps_new.zip`, then `sps_new.LoxCC`: an upload not loaded yet
//! 3. the newest `sps_<ver>_<timestamp>.zip` or `.LoxCC` whose `<ver>` the
//!    firmware supports
//! 4. `sps.zip`, `sps_old.zip`, `sps.LoxPLAN`, `Default.Loxone`
//!
//! So the newest file is not always the running one. The system log says
//! which file was loaded:
//!
//! ```text
//! 2026-11-06 00:23:32.567;PRG got restart command; user: Jakub(192.168.1.123)
//! 2026-11-06 00:23:33.025;Loading sps_new.zip - Remove old custom changes
//! 2026-11-06 00:23:34.971;PRG start program
//! 2026-11-06 00:23:35.279;Program started: /lx/prog/sps_0274_20261105235504.zip
//! 2026-11-06 00:23:35.388;Rename program /lx/prog/sps_0274_20261106002335.zip
//! ```
//!
//! After `Loading sps_new.zip`, `Program started` still names the program
//! before it (firmware 17.2); the upload runs under the name it is renamed
//! to. Without an upload, `Program started` names the running file.
//!
//! vislox reads `/prog` and `def.log` over HTTP (`/dev/fslist`,
//! `/dev/fsget`), the way `lox files` does: it only reads.

use loxone_client::{Connection, LoxClient};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A check that takes longer than this has hung (an unreachable host).
const TIMEOUT: Duration = Duration::from_secs(30);
/// A report is reused for this long, so page loads don't each ask.
const FRESH: Duration = Duration::from_secs(60);

/// A file in `/prog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgFile {
    pub name: String,
    pub size: u64,
}

/// `sps_<ver>_<timestamp>.zip` or `.LoxCC`: version and timestamp.
pub fn dated(name: &str) -> Option<(u32, &str)> {
    let stem = name
        .strip_suffix(".zip")
        .or_else(|| name.strip_suffix(".LoxCC"))?;
    let mut parts = stem.splitn(3, '_');
    if parts.next()? != "sps" {
        return None;
    }
    let ver = parts.next()?.parse().ok()?;
    let ts = parts.next()?;
    (ts.len() == 14 && ts.bytes().all(|b| b.is_ascii_digit())).then_some((ver, ts))
}

/// An upload the Miniserver loads on its next program start.
fn is_upload(name: &str) -> bool {
    name.eq_ignore_ascii_case("sps_new.zip") || name.eq_ignore_ascii_case("sps_new.LoxCC")
}

fn is_emergency(name: &str) -> bool {
    name.eq_ignore_ascii_case("Emergency.LoxCC")
}

/// `/dev/fslist` output: `- 420867 Nov 06 00:23 sps_0274_20261106002335.zip`
/// (kind, size, three date fields, name); directories are left out.
pub fn parse_listing(text: &str) -> Vec<ProgFile> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            if f.next()? != "-" {
                return None;
            }
            let size = f.next()?.parse().ok()?;
            let name = f.skip(3).collect::<Vec<_>>().join(" ");
            (!name.is_empty()).then_some(ProgFile { name, size })
        })
        .collect()
}

/// The newest dated program file (what `lox config pull` downloads, when a
/// `.zip`).
fn newest(files: &[ProgFile]) -> Option<&ProgFile> {
    files
        .iter()
        .filter_map(|f| dated(&f.name).map(|(_, ts)| (ts, f)))
        .max_by(|a, b| a.0.cmp(b.0))
        .map(|(_, f)| f)
}

/// The newest dated `.zip`, which `lox config pull` downloads.
fn newest_zip(files: &[ProgFile]) -> Option<&ProgFile> {
    files
        .iter()
        .filter(|f| f.name.ends_with(".zip"))
        .filter_map(|f| dated(&f.name).map(|(_, ts)| (ts, f)))
        .max_by(|a, b| a.0.cmp(b.0))
        .map(|(_, f)| f)
}

/// The last program start in `def.log`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Start {
    /// The Miniserver's clock: `2026-11-06 00:23:34`
    pub at: String,
    /// What a `Loading …` line named before it (`sps_new.zip`,
    /// `Emergency.LoxCC`, …)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded: Option<String>,
    /// The program file it runs, when the log tells
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The firmware the Miniserver booted (`PRG Start 17.2.10.21`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
}

fn base_name(p: &str) -> String {
    p.trim().rsplit('/').next().unwrap_or("").to_string()
}

/// Read `def.log` (UTF-8 with a BOM and the odd bad byte) for the last
/// program start.
pub fn parse_log(text: &str) -> Option<Start> {
    let mut last: Option<Start> = None;
    let mut loading: Option<String> = None;
    let mut firmware: Option<String> = None;
    for line in text.lines() {
        let Some((at, msg)) = line.trim_start_matches('\u{feff}').split_once(';') else {
            continue;
        };
        let at = at.split('.').next().unwrap_or(at).trim();
        let msg = msg.trim();
        if let Some(v) = msg.strip_prefix("PRG Start ") {
            firmware = Some(v.trim().to_string());
            loading = None;
        } else if let Some(rest) = msg.strip_prefix("Loading ") {
            // `Loading sps_new.zip - Remove old custom changes`
            let name = rest.split(" - ").next().unwrap_or(rest);
            loading = Some(base_name(name));
        } else if msg == "PRG start program" {
            let loaded = loading.take();
            let file = loaded.clone().filter(|l| !is_upload(l));
            last = Some(Start {
                at: at.to_string(),
                loaded,
                file,
                firmware: firmware.clone(),
            });
        } else if let Some(p) = msg.strip_prefix("Program started:") {
            // After an upload this names the program before it: the
            // rename that follows names the upload.
            if let Some(s) = last.as_mut().filter(|s| s.file.is_none())
                && !s.loaded.as_deref().is_some_and(is_upload)
            {
                s.file = Some(base_name(p));
            }
        } else if let Some(p) = msg.strip_prefix("Rename program")
            && let Some(s) = last.as_mut()
        {
            s.file = Some(base_name(p));
        }
    }
    last
}

/// A reason to look twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    /// `waiting`, `emergency`, `skipped`, `not-newest`, `no-log`
    pub kind: &'static str,
    pub text: String,
}

/// What the Miniserver runs, and what looks wrong about it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    /// The program file it runs, if known
    pub running: Option<String>,
    /// `def.log` (the log names it), or `load order` (guessed from `/prog`)
    pub from: &'static str,
    /// The last program start, from `def.log`
    pub start: Option<Start>,
    pub firmware: Option<String>,
    /// The newest dated program file in `/prog`
    pub newest: Option<String>,
    /// What `lox config pull` downloads: the newest dated `.zip`
    pub pull: Option<String>,
    /// An upload not loaded yet (`sps_new.zip`)
    pub waiting: Option<String>,
    pub warnings: Vec<Warning>,
    /// The check failed: why (the rest is empty then)
    pub error: Option<String>,
    /// Unix time of the check
    pub at: i64,
}

impl Report {
    pub fn failed(e: String) -> Report {
        Report {
            error: Some(e),
            at: chrono::Utc::now().timestamp(),
            ..Report::default()
        }
    }
}

/// Put `/prog` and the last start in `def.log` together.
pub fn assess(
    files: &[ProgFile],
    log: Result<Option<Start>, String>,
    firmware: Option<String>,
) -> Report {
    let mut r = Report {
        at: chrono::Utc::now().timestamp(),
        newest: newest(files).map(|f| f.name.clone()),
        pull: newest_zip(files).map(|f| f.name.clone()),
        waiting: files
            .iter()
            .find(|f| is_upload(&f.name))
            .map(|f| f.name.clone()),
        ..Report::default()
    };
    let start = match log {
        Ok(s) => s,
        Err(e) => {
            r.warnings.push(Warning {
                kind: "no-log",
                text: format!(
                    "Cannot read def.log ({e}), so the running config is guessed from /prog."
                ),
            });
            None
        }
    };
    r.firmware = firmware.or_else(|| start.as_ref().and_then(|s| s.firmware.clone()));
    match start.as_ref().and_then(|s| s.file.clone()) {
        Some(f) => {
            r.running = Some(f);
            r.from = "def.log";
        }
        None => {
            // The load order, without Emergency.LoxCC (only after reboots)
            // and the firmware's version cap (unknown here).
            r.running = r.newest.clone().or_else(|| {
                ["sps.zip", "sps_old.zip", "sps.LoxPLAN"]
                    .into_iter()
                    .find(|n| files.iter().any(|f| f.name == *n))
                    .map(String::from)
            });
            r.from = "load order";
        }
    }
    r.start = start;
    if let Some(w) = &r.waiting {
        r.warnings.push(Warning {
            kind: "waiting",
            text: format!(
                "{w} is waiting in /prog: an upload the Miniserver has not loaded. It loads when the program restarts."
            ),
        });
    }
    let running = r.running.clone().unwrap_or_default();
    if is_emergency(&running) {
        r.warnings.push(Warning {
            kind: "emergency",
            text: "The Miniserver runs its emergency config (Emergency.LoxCC), after several reboots in a row. \
                   The newest backup is not what it runs."
                .into(),
        });
    } else if r.from == "def.log"
        && let Some(n) = &r.newest
        && *n != running
    {
        match (dated(n), dated(&running)) {
            (Some((nv, _)), Some((rv, _))) if nv > rv => r.warnings.push(Warning {
                kind: "skipped",
                text: format!(
                    "The newest file, {n}, was skipped: it needs a newer firmware (config version {nv}; it runs {running}, version {rv}{}).",
                    r.firmware.as_ref().map(|f| format!(", on firmware {f}")).unwrap_or_default()
                ),
            }),
            _ => r.warnings.push(Warning {
                kind: "not-newest",
                text: format!("The Miniserver runs {running}, not the newest file, {n}."),
            }),
        }
    }
    // The newest file is a .LoxCC it runs, but Pull takes the newest .zip.
    let warned = !r.warnings.iter().all(|w| w.kind == "waiting");
    if r.from == "def.log"
        && !warned
        && let Some(p) = &r.pull
        && *p != running
    {
        r.warnings.push(Warning {
            kind: "not-newest",
            text: format!("Pull downloads {p}, but the Miniserver runs {running}."),
        });
    }
    r
}

/// Ask the Miniserver (blocking: three HTTP requests).
pub fn check(conn: &Connection) -> Report {
    let client = match LoxClient::new(conn.clone()) {
        Ok(c) => c,
        Err(e) => return Report::failed(e.to_string()),
    };
    let listing = match client.get_text("/dev/fslist//prog") {
        Ok(t) => t,
        Err(e) => return Report::failed(format!("cannot list /prog: {e}")),
    };
    let files = parse_listing(&listing);
    if files.is_empty() {
        return Report::failed(format!(
            "/prog lists no files: {}",
            listing.lines().next().unwrap_or("").trim()
        ));
    }
    let log = client
        .log()
        .map(|t| parse_log(&t))
        .map_err(|e| e.to_string());
    let firmware = client
        .value("/dev/cfg/version")
        .ok()
        .filter(|v| !v.is_empty());
    assess(&files, log, firmware)
}

/// The running config, checked on demand and kept for a minute.
pub struct Running {
    conn: Result<Connection, String>,
    last: Mutex<Option<(Instant, Report)>>,
    one: tokio::sync::Mutex<()>,
}

impl Running {
    pub fn new(conn: Result<Connection, String>) -> Running {
        Running {
            conn,
            last: Mutex::new(None),
            one: tokio::sync::Mutex::new(()),
        }
    }

    pub fn available(&self) -> bool {
        self.conn.is_ok()
    }

    /// The last report, or a new one when it is older than a minute (or
    /// `fresh`).
    pub async fn get(&self, fresh: bool) -> Report {
        let conn = match &self.conn {
            Ok(c) => c.clone(),
            Err(e) => return Report::failed(e.clone()),
        };
        let _one = self.one.lock().await;
        if !fresh
            && let Some((t, r)) = &*self.last.lock().unwrap()
            && t.elapsed() < FRESH
        {
            return r.clone();
        }
        let r =
            match tokio::time::timeout(TIMEOUT, tokio::task::spawn_blocking(move || check(&conn)))
                .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => Report::failed(e.to_string()),
                Err(_) => Report::failed(format!(
                    "the Miniserver did not answer within {} s",
                    TIMEOUT.as_secs()
                )),
            };
        *self.last.lock().unwrap() = Some((Instant::now(), r.clone()));
        r
    }
}

/// Did an Apply load? `before` is the report from before the upload, `after`
/// the one after the program restart. The reasons it did not, if any.
pub fn verify_load(before: &Report, after: &Report) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(e) = &after.error {
        why.push(format!("cannot check the Miniserver: {e}"));
        return why;
    }
    if let Some(w) = &after.waiting {
        why.push(format!(
            "{w} is still in /prog: the Miniserver did not load the upload"
        ));
    }
    match &after.start {
        None => why.push("def.log shows no program start".into()),
        Some(s) => {
            let new_start = before.start.as_ref().is_none_or(|b| s.at > b.at);
            if !new_start {
                why.push(format!(
                    "def.log shows no program start after the upload (last: {})",
                    s.at
                ));
            } else if !s.loaded.as_deref().is_some_and(is_upload) {
                why.push(format!(
                    "the program restarted at {} without loading the upload{}",
                    s.at,
                    s.loaded
                        .as_ref()
                        .map(|l| format!(" (it loaded {l})"))
                        .unwrap_or_default()
                ));
            }
        }
    }
    if why.is_empty() {
        match (&after.running, &after.pull) {
            (Some(r), Some(p)) if r == p => {
                if before.running.as_ref() == Some(r) {
                    why.push(format!(
                        "the Miniserver still runs {r}, as before the upload"
                    ));
                }
            }
            (Some(r), Some(p)) => why.push(format!(
                "the Miniserver runs {r}, but lox verified {p} (the newest backup)"
            )),
            (None, _) => why.push("def.log does not name the program it runs".into()),
            (_, None) => why.push("/prog holds no backup".into()),
        }
    }
    for w in &after.warnings {
        if w.kind == "emergency" {
            why.push(w.text.clone());
        }
    }
    why
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "d      0 Nov 06 00:33 .
d      0 Jan 01 01:00 ..
- 336932 Oct 24 13:25 sps_0248_20241024132548.zip
- 100378 Nov 05 23:52 Default.Loxone
- 420867 Nov 06 00:23 sps_0274_20261106002335.zip
- 420533 Oct 31 20:00 sps_0274_20261031200009.zip
- 518068 Nov 06 00:33 sps_0274_20261106003351.zip
-  88841 Nov 06 00:33 Emergency.LoxCC
";

    const LOG: &str = "\u{feff}2026-11-05 23:53:54.216;PRG Start 17.2.10.21
2026-11-05 23:54:10.505;PRG start program
2026-11-05 23:54:11.139;Program started: /lx/prog/sps_0274_20261031200009.zip
2026-11-05 23:54:56.447;Program send: /lx/prog/sps_0274_20261031200009.zip
2026-11-06 00:23:32.567;PRG got restart command; user: Jakub(192.168.1.123)
2026-11-06 00:23:33.025;Loading sps_new.zip - Remove old custom changes
2026-11-06 00:23:34.971;PRG start program
2026-11-06 00:23:35.279;Program started: /lx/prog/sps_0274_20261031200009.zip
2026-11-06 00:23:35.388;Rename program /lx/prog/sps_0274_20261106002335.zip
2026-11-06 00:23:37.626;PRG start Relay Extension (Relay 1) serial 0bdbbf08 version 12.4.5.25
";

    fn files(names: &[&str]) -> Vec<ProgFile> {
        names
            .iter()
            .map(|n| ProgFile {
                name: n.to_string(),
                size: 1,
            })
            .collect()
    }

    #[test]
    fn the_listing_has_files_not_directories() {
        let f = parse_listing(LISTING);
        assert_eq!(f.len(), 6);
        assert_eq!(
            f[0],
            ProgFile {
                name: "sps_0248_20241024132548.zip".into(),
                size: 336932
            }
        );
        assert_eq!(newest(&f).unwrap().name, "sps_0274_20261106003351.zip");
    }

    #[test]
    fn dated_names() {
        assert_eq!(
            dated("sps_0274_20261106003351.zip"),
            Some((274, "20261106003351"))
        );
        assert_eq!(
            dated("sps_162_20210101000000.LoxCC"),
            Some((162, "20210101000000"))
        );
        assert_eq!(dated("sps_new.zip"), None);
        assert_eq!(dated("sps_0274_2026.zip"), None);
        assert_eq!(dated("Emergency.LoxCC"), None);
    }

    #[test]
    fn an_upload_runs_under_its_new_name() {
        let s = parse_log(LOG).unwrap();
        assert_eq!(s.at, "2026-11-06 00:23:34");
        assert_eq!(s.loaded.as_deref(), Some("sps_new.zip"));
        assert_eq!(s.file.as_deref(), Some("sps_0274_20261106002335.zip"));
        assert_eq!(s.firmware.as_deref(), Some("17.2.10.21"));
    }

    #[test]
    fn a_plain_start_runs_what_it_names() {
        let log = "2026-11-05 23:53:54.216;PRG Start 17.2.10.21
2026-11-05 23:54:10.505;PRG start program
2026-11-05 23:54:11.139;Program started: /lx/prog/sps_0274_20261031200009.zip
";
        let s = parse_log(log).unwrap();
        assert_eq!(s.loaded, None);
        assert_eq!(s.file.as_deref(), Some("sps_0274_20261031200009.zip"));
    }

    #[test]
    fn an_upload_without_a_rename_is_not_the_old_file() {
        let log = "2026-11-06 00:23:33.025;Loading sps_new.zip - Remove old custom changes
2026-11-06 00:23:34.971;PRG start program
2026-11-06 00:23:35.279;Program started: /lx/prog/sps_0274_20261031200009.zip
";
        assert_eq!(parse_log(log).unwrap().file, None);
    }

    #[test]
    fn the_emergency_config_is_named() {
        let log = "2026-11-06 00:23:33.025;Loading Emergency.LoxCC
2026-11-06 00:23:34.971;PRG start program
2026-11-06 00:23:35.279;Program started: /lx/prog/sps_0274_20261031200009.zip
";
        let s = parse_log(log).unwrap();
        assert_eq!(s.file.as_deref(), Some("Emergency.LoxCC"));
        let r = assess(&parse_listing(LISTING), Ok(Some(s)), None);
        assert_eq!(r.warnings[0].kind, "emergency");
    }

    #[test]
    fn the_running_file_is_the_newest() {
        let mut log = LOG.to_string();
        log.push_str(
            "2026-11-06 00:33:48.829;Loading sps_new.zip - Remove old custom changes
2026-11-06 00:33:50.752;PRG start program
2026-11-06 00:33:51.100;Rename program /lx/prog/sps_0274_20261106003351.zip
",
        );
        let r = assess(&parse_listing(LISTING), Ok(parse_log(&log)), None);
        assert_eq!(r.running.as_deref(), Some("sps_0274_20261106003351.zip"));
        assert_eq!(r.from, "def.log");
        assert_eq!(r.firmware.as_deref(), Some("17.2.10.21"));
        // Emergency.LoxCC is always there; it only matters when loaded.
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn warnings_for_a_waiting_upload_and_an_older_running_file() {
        let mut f = parse_listing(LISTING);
        f.push(ProgFile {
            name: "sps_new.zip".into(),
            size: 5,
        });
        let r = assess(&f, Ok(parse_log(LOG)), None);
        assert_eq!(r.running.as_deref(), Some("sps_0274_20261106002335.zip"));
        assert_eq!(r.waiting.as_deref(), Some("sps_new.zip"));
        let kinds: Vec<_> = r.warnings.iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["waiting", "not-newest"]);
    }

    #[test]
    fn a_file_for_newer_firmware_is_skipped() {
        let f = files(&["sps_0162_20210101000000.zip", "sps_0170_20210201000000.zip"]);
        let log = "2021-02-01 10:00:00.000;PRG Start 10.2.3.26
2021-02-01 10:00:01.000;PRG start program
2021-02-01 10:00:02.000;Program started: /lx/prog/sps_0162_20210101000000.zip
";
        let r = assess(&f, Ok(parse_log(log)), None);
        assert_eq!(r.warnings[0].kind, "skipped");
        assert!(r.warnings[0].text.contains("firmware 10.2.3.26"));
    }

    #[test]
    fn without_the_log_the_load_order_guesses() {
        let r = assess(&parse_listing(LISTING), Err("access denied".into()), None);
        assert_eq!(r.from, "load order");
        assert_eq!(r.running.as_deref(), Some("sps_0274_20261106003351.zip"));
        assert_eq!(r.warnings[0].kind, "no-log");
        let r = assess(&files(&["sps.zip", "Default.Loxone"]), Ok(None), None);
        assert_eq!(r.running.as_deref(), Some("sps.zip"));
    }

    #[test]
    fn an_apply_is_verified_by_the_load() {
        let before = assess(&parse_listing(LISTING), Ok(parse_log(LOG)), None);
        // lox uploaded and restarted: a new start loaded sps_new.zip.
        let mut log = LOG.to_string();
        log.push_str(
            "2026-11-06 00:33:48.829;Loading sps_new.zip - Remove old custom changes
2026-11-06 00:33:50.752;PRG start program
2026-11-06 00:33:51.100;Rename program /lx/prog/sps_0274_20261106003351.zip
",
        );
        let after = assess(&parse_listing(LISTING), Ok(parse_log(&log)), None);
        assert_eq!(verify_load(&before, &after), Vec::<String>::new());

        // No restart since: the upload still waits.
        let mut f = parse_listing(LISTING);
        f.push(ProgFile {
            name: "sps_new.zip".into(),
            size: 5,
        });
        let stuck = assess(&f, Ok(parse_log(LOG)), None);
        let why = verify_load(&before, &stuck);
        assert!(why[0].contains("sps_new.zip is still in /prog"), "{why:?}");
        assert!(
            why[1].contains("no program start after the upload"),
            "{why:?}"
        );

        // A restart that did not load the upload.
        let mut log = LOG.to_string();
        log.push_str(
            "2026-11-06 00:40:00.000;PRG start program
2026-11-06 00:40:01.000;Program started: /lx/prog/sps_0274_20261106002335.zip
",
        );
        let plain = assess(&parse_listing(LISTING), Ok(parse_log(&log)), None);
        assert!(verify_load(&before, &plain)[0].contains("without loading the upload"));

        let failed = Report::failed("timeout".into());
        assert!(verify_load(&before, &failed)[0].contains("timeout"));
    }
}
