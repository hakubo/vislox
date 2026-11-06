//! The Miniserver's system log, as `lox log` prints it, for the Log panel.
//!
//! vislox runs `lox log` with the context it picked, so lox reads the log the
//! way it reads everything else (its connection, its token). That only reads:
//! it changes nothing on the Miniserver. Each line is `<date> <time>;<text>`.
//! vislox marks the last program change (an Apply, or a save from Loxone
//! Config), so the entries after it stand out, and picks out the UUIDs a line
//! names (warnings name the block that raised them), so the panel can link
//! them to their blocks.

use crate::live::Found;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `lox log` that takes longer than this has hung.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How many lines to ask for.
pub const LINES: usize = 300;
/// A request within this long of the last fetch gets the same result: a
/// browser that reloads, or two open tabs, do not ask the Miniserver twice.
const REUSE: Duration = Duration::from_secs(2);

pub struct Log {
    cmd: Result<Cmd, String>,
    host: String,
    last: Mutex<Option<(Instant, Value)>>,
    running: tokio::sync::Mutex<()>,
}

struct Cmd {
    lox: PathBuf,
    ctx: Option<String>,
    /// lox looks for a project-local config from here, as vislox did.
    cwd: PathBuf,
}

impl Log {
    pub fn new(lox: &Path, off: Option<&str>, found: Result<&Found, &String>, start: &Path) -> Log {
        let host = found.map(|f| f.conn.host.clone()).unwrap_or_default();
        let cmd = (|| {
            if let Some(why) = off {
                return Err(why.to_string());
            }
            let f = found.map_err(|e| format!("no lox configuration: {e}"))?;
            let lox = crate::pull::which(lox).ok_or_else(|| {
                format!(
                    "`{}` not found (install lox, or pass --lox <path>)",
                    lox.display()
                )
            })?;
            Ok(Cmd {
                lox,
                ctx: f.ctx.clone(),
                cwd: start.to_path_buf(),
            })
        })();
        Log {
            cmd,
            host,
            last: Mutex::new(None),
            running: tokio::sync::Mutex::new(()),
        }
    }

    /// For `/api/history`: whether there is a log to show, and why not.
    pub fn info(&self) -> Value {
        match &self.cmd {
            Ok(_) => json!({ "available": true, "host": self.host }),
            Err(why) => json!({ "available": false, "reason": why }),
        }
    }

    /// The log, fetched now (or within the last 2 s). A failure is part of
    /// the answer (`error`), with the entries of the last fetch that worked.
    pub async fn fetch(&self) -> Value {
        let cmd = match &self.cmd {
            Ok(c) => c,
            Err(why) => return json!({ "available": false, "reason": why }),
        };
        // One fetch at a time; a request that waited gets its result.
        let _one = self.running.lock().await;
        if let Some((at, v)) = self.last.lock().unwrap().as_ref()
            && at.elapsed() < REUSE
        {
            return v.clone();
        }
        let now = chrono::Utc::now().timestamp();
        let v = match run(cmd).await {
            Ok(text) => {
                let entries = parse(&text);
                let since = last_program_change(&entries);
                json!({
                    "available": true,
                    "host": self.host,
                    "fetched": now,
                    "lines": LINES,
                    "since": since,
                    "entries": entries,
                })
            }
            Err(e) => {
                let mut v = self
                    .last
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(
                        || json!({ "available": true, "host": self.host, "entries": [] }),
                    );
                v["error"] = json!(crate::reason::explain(&e));
                v["failed"] = json!(now);
                return v;
            }
        };
        *self.last.lock().unwrap() = Some((Instant::now(), v.clone()));
        v
    }
}

async fn run(cmd: &Cmd) -> Result<String, String> {
    let mut c = tokio::process::Command::new(&cmd.lox);
    if let Some(ctx) = &cmd.ctx {
        c.args(["--ctx", ctx]);
    }
    c.args(["log", "-n", &LINES.to_string(), "--no-color", "--no-header"])
        .current_dir(cmd.cwd.ancestors().find(|d| d.is_dir()).unwrap_or(&cmd.cwd))
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(TIMEOUT, crate::pull::output_retrying(&mut c))
        .await
        .map_err(|_| format!("lox log took longer than {} s", TIMEOUT.as_secs()))?
        .map_err(|e| format!("cannot run {}: {e}", cmd.lox.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = [stderr.trim(), stdout.trim()]
            .into_iter()
            .find(|m| !m.is_empty())
            .unwrap_or("no output");
        return Err(format!("lox log failed: {msg}"));
    }
    Ok(stdout)
}

/// One line of the log.
#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    /// The Miniserver's local time, as it wrote it (`2026-11-06 00:33:33.801`).
    pub at: String,
    pub text: String,
    /// `program` (the program changed, restarted or started), `warning`,
    /// `error` or `info`.
    pub kind: &'static str,
    /// The UUIDs the line names, lower case.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uuids: Vec<String>,
}

/// `<date> <time>;<text>` lines; a line without a time continues the one
/// before it.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        match line.split_once(';').filter(|(t, _)| is_time(t)) {
            Some((at, rest)) => out.push(Entry {
                at: at.to_string(),
                text: rest.trim().to_string(),
                kind: "info",
                uuids: Vec::new(),
            }),
            None => match out.last_mut() {
                Some(e) => {
                    e.text.push('\n');
                    e.text.push_str(line.trim());
                }
                None => out.push(Entry {
                    at: String::new(),
                    text: line.trim().to_string(),
                    kind: "info",
                    uuids: Vec::new(),
                }),
            },
        }
    }
    for e in &mut out {
        e.kind = kind(&e.text);
        e.uuids = uuids(&e.text);
    }
    out
}

/// `2026-11-06 00:33:33.801` (the fraction is optional).
fn is_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 19
        && b.iter().enumerate().take(19).all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        })
        && (b.len() == 19 || (b[19] == b'.' && b[20..].iter().all(u8::is_ascii_digit)))
}

fn kind(text: &str) -> &'static str {
    if is_program_change(text)
        || text.starts_with("PRG Start ")
        || text.starts_with("PRG start program")
        || text.starts_with("Program started")
        || text.starts_with("Restart SPS")
    {
        return "program";
    }
    let lower = text.to_lowercase();
    if text.starts_with("Warning") || lower.contains("warning") {
        "warning"
    } else if lower.contains("error") || lower.contains("fail") || lower.contains("exception") {
        "error"
    } else {
        "info"
    }
}

/// What a deployed config leaves in the log: `lox config push` (Apply) and
/// a save from Loxone Config both write these when the new program loads.
fn is_program_change(text: &str) -> bool {
    text.starts_with("Program changed by user") || text.starts_with("PRG got restart command")
}

/// The index of the first entry of the last program change: what came after
/// it is what the deployed config did. `None` when the lines hold none.
pub fn last_program_change(entries: &[Entry]) -> Option<usize> {
    let last = entries.iter().rposition(|e| is_program_change(&e.text))?;
    // "Program changed by user" and "PRG got restart command" come together:
    // start at the first of the pair.
    let mut i = last;
    while i > 0 && is_program_change(&entries[i - 1].text) && entries[i - 1].at == entries[last].at
    {
        i -= 1;
    }
    Some(i)
}

/// Loxone UUIDs (`1d525782-02d4-82be-ffff60c50ef75c80`) in a line, lower
/// case, each once.
pub fn uuids(text: &str) -> Vec<String> {
    const SHAPE: [usize; 4] = [8, 4, 4, 16];
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // Only at the start of a word.
        if i > 0 && b[i - 1].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let mut j = i;
        let ok = SHAPE.iter().enumerate().all(|(n, len)| {
            if n > 0 {
                if b.get(j) != Some(&b'-') {
                    return false;
                }
                j += 1;
            }
            let run = b[j.min(b.len())..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            j += len;
            run == *len
        });
        if ok && !b.get(j).is_some_and(u8::is_ascii_alphanumeric) {
            let u = text[i..j].to_ascii_lowercase();
            if !out.contains(&u) {
                out.push(u);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // From a real Miniserver (17.2), shortened.
    const SAMPLE: &str = "\
2026-11-05 23:55:09.970;PRG start Modbus Extension (Modbus Extension) serial 09d8bb5c version 15.0.3.20
2026-11-06 00:22:28.953;Program send: /lx/prog/sps_0274_20261105235504.zip
2026-11-06 00:23:32.567;Program changed by user 'Jakub'
2026-11-06 00:23:32.567;PRG got restart command; user: Jakub(192.168.1.123)
2026-11-06 00:23:34.971;PRG start program
2026-11-06 00:23:36.622;weather error authentication exception: user is not active
2026-11-06 04:25:22.890;Warning 1167, Sieć - Odczyt licznika zmniejszył się (Kotłownia), all, MeterAbsBi (Kotłownia,1d525782-02d4-82be-ffff60c50ef75c80)
2026-11-06 10:57:23.700;Remote Connect Legacy Registration done
";

    #[test]
    fn lines_split_into_time_and_text() {
        let e = parse(SAMPLE);
        assert_eq!(e.len(), 8);
        assert_eq!(e[1].at, "2026-11-06 00:22:28.953");
        assert_eq!(
            e[1].text,
            "Program send: /lx/prog/sps_0274_20261105235504.zip"
        );
        // A `;` in the text stays in it.
        assert_eq!(
            e[3].text,
            "PRG got restart command; user: Jakub(192.168.1.123)"
        );
        assert_eq!(e[7].text, "Remote Connect Legacy Registration done");
    }

    #[test]
    fn lines_are_classified() {
        let kinds: Vec<_> = parse(SAMPLE).iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                "info", "info", "program", "program", "program", "error", "warning", "info"
            ]
        );
    }

    #[test]
    fn a_warning_names_its_block() {
        let e = parse(SAMPLE);
        assert_eq!(e[6].uuids, ["1d525782-02d4-82be-ffff60c50ef75c80"]);
        assert!(e[0].uuids.is_empty());
        assert_eq!(
            uuids("x 10000003-0000-0032-01FF504F94000001,10000003-0000-0032-01ff504f94000001."),
            ["10000003-0000-0032-01ff504f94000001"]
        );
        // Not a UUID: too short, too long, or inside a word.
        assert!(uuids("1d525782-02d4-82be-ffff60c50ef75c8").is_empty());
        assert!(uuids("1d525782-02d4-82be-ffff60c50ef75c800").is_empty());
        assert!(uuids("a1d525782-02d4-82be-ffff60c50ef75c80").is_empty());
        assert!(uuids("serial 09d8bb5c version").is_empty());
    }

    #[test]
    fn the_last_program_change_starts_at_its_first_line() {
        let e = parse(SAMPLE);
        assert_eq!(last_program_change(&e), Some(2));
        assert_eq!(last_program_change(&e[..2]), None);
        // A restart without a new program is not a program change.
        let r = parse("2026-11-06 10:57:06.255;PRG Start 17.2.10.21\n");
        assert_eq!(last_program_change(&r), None);
        assert_eq!(r[0].kind, "program");
    }

    #[test]
    fn a_line_without_a_time_continues_the_one_before() {
        let e = parse("2026-11-06 10:00:00.000;first\n  more of it\n\nstray\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].text, "first\nmore of it\nstray");
        let e = parse("no time here\n2026-11-06 10:00:00;second\n");
        assert_eq!(e[0].at, "");
        assert_eq!(e[1].at, "2026-11-06 10:00:00");
        assert!(!is_time("2026-11-06 10:00:00.0x"));
        assert!(!is_time("2026/11/06 10:00:00"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_log_comes_from_lox_with_vislox_s_context() {
        use loxone_client::Connection;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let lox = tmp.path().join("lox");
        let f = Found {
            conn: Connection::new("https://192.168.1.120", "u", "p"),
            from: "test".into(),
            ctx: Some("home".into()),
            config_repo: None,
            ms_dir: "192_168_1_120".into(),
            serial: None,
        };
        let script = |body: &str| {
            std::fs::write(&lox, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&lox, std::fs::Permissions::from_mode(0o755)).unwrap();
            Log::new(&lox, None, Ok(&f), tmp.path())
        };
        let l = script(&format!(
            r#"[ "$*" = "--ctx home log -n {LINES} --no-color --no-header" ] || {{ echo "unexpected: $*" >&2; exit 2; }}
printf '%s' "{SAMPLE}""#
        ));
        assert_eq!(l.info()["available"], true);
        let v = l.fetch().await;
        assert_eq!(v["entries"].as_array().unwrap().len(), 8, "{v}");
        assert_eq!(v["since"], 2);
        assert!(v.get("error").is_none());

        // A failure keeps nothing it did not have, and says why.
        let l = script("echo 'login refused: Code 4003' >&2; exit 1");
        let v = l.fetch().await;
        assert!(v["error"].as_str().unwrap().contains("Code 4003"), "{v}");
        assert_eq!(v["entries"].as_array().unwrap().len(), 0);

        // Off, or no lox configuration: not available, and why.
        let off = Log::new(&lox, Some("turned off (--no-live)"), Ok(&f), tmp.path());
        assert_eq!(off.info()["available"], false);
        assert_eq!(off.fetch().await["reason"], "turned off (--no-live)");
        let err = "no config".to_string();
        let none = Log::new(&lox, None, Err(&err), tmp.path());
        assert!(
            none.info()["reason"]
                .as_str()
                .unwrap()
                .contains("no config")
        );
    }
}
