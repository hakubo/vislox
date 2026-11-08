//! An Apply's or a Roll back's steps as they happen ([`Progress`]): lox's
//! progress lines ([`push_step`]) and what vislox sees of the program
//! restart.

use super::*;

/// An Apply, step by step.
#[derive(Clone, Serialize)]
pub(super) struct Progress {
    pub(super) steps: Vec<Step>,
    /// lox's last lines, for the details.
    log: Vec<String>,
    at: i64,
}

#[derive(Clone, Serialize)]
pub(super) struct Step {
    pub(super) id: &'static str,
    label: &'static str,
    /// `wait`, `run`, `ok`, `skip` or `fail`
    pub(super) state: &'static str,
    detail: Option<String>,
    /// When it started and ended (Unix seconds).
    at: Option<i64>,
    end: Option<i64>,
}

/// Apply's steps, in order.
const APPLY_STEPS: [(&str, &str); 12] = [
    ("module", "Module compiles"),
    ("removals", "Removals you confirmed"),
    ("tests", "Tests pass"),
    ("check", "Download and drift check"),
    ("upload", "Upload"),
    ("restart", "Program restarting"),
    ("online", "Back online"),
    ("loaded", "New config loaded"),
    ("verify", "Verified"),
    ("commit", "Committed"),
    ("scan", "No personal data in git"),
    ("push", "Pushed to git"),
];
/// A Roll back's steps (#69): Apply's, from the backup and the module
/// before the Apply.
const ROLLBACK_STEPS: [(&str, &str); 13] = [
    ("backup", "Backup checked"),
    ("module", "Module before the Apply compiles to it"),
    ("tests", "Tests pass"),
    ("stage", "Upload is what you reviewed"),
    ("check", "Download and drift check"),
    ("upload", "Upload"),
    ("restart", "Program restarting"),
    ("online", "Back online"),
    ("loaded", "Config loaded"),
    ("verify", "Verified"),
    ("commit", "Committed"),
    ("scan", "No personal data in git"),
    ("push", "Pushed to git"),
];
const LOG_LINES: usize = 12;

impl Progress {
    pub(super) fn apply() -> Progress {
        Progress::of(&APPLY_STEPS)
    }

    pub(super) fn rollback() -> Progress {
        Progress::of(&ROLLBACK_STEPS)
    }

    fn of(steps: &[(&'static str, &'static str)]) -> Progress {
        Progress {
            steps: steps
                .iter()
                .map(|&(id, label)| Step {
                    id,
                    label,
                    state: "wait",
                    detail: None,
                    at: None,
                    end: None,
                })
                .collect(),
            log: Vec::new(),
            at: now(),
        }
    }

    pub(super) fn get(&mut self, id: &str) -> &mut Step {
        self.steps
            .iter_mut()
            .find(|s| s.id == id)
            .expect("a known step")
    }

    /// `id` starts; the steps before it that still run are done.
    pub(super) fn start(&mut self, id: &str) {
        let t = now();
        for s in self.steps.iter_mut().take_while(|s| s.id != id) {
            if s.state == "run" {
                s.state = "ok";
                s.end = Some(t);
            }
        }
        let s = self.get(id);
        if s.state == "wait" {
            s.state = "run";
            s.at = Some(t);
        }
    }

    /// `id` ends as `state` (`ok`, `skip`, `fail`), with what to say.
    pub(super) fn finish(&mut self, id: &str, state: &'static str, detail: Option<String>) {
        let t = now();
        let s = self.get(id);
        s.state = state;
        s.at.get_or_insert(t);
        s.end = Some(t);
        if detail.is_some() {
            s.detail = detail;
        }
    }

    /// End a step that is not done yet; one already done keeps its outcome.
    fn settle(&mut self, id: &str, detail: Option<&str>) {
        if matches!(self.get(id).state, "wait" | "run") {
            self.finish(id, "ok", detail.map(String::from));
        }
    }

    /// A line of lox's output.
    pub(super) fn lox_line(&mut self, line: &str) {
        let line = line.trim_end();
        if line.trim().is_empty() {
            return;
        }
        if self.log.len() == LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(line.to_string());
        match push_step(line) {
            Some("verified") => self.finish("verify", "ok", None),
            Some("verify") => {
                // lox waited until the program ran the upload.
                self.settle("restart", None);
                self.settle("online", None);
                self.settle("loaded", Some("lox saw the new program running"));
                self.start("verify");
            }
            Some(id) => self.start(id),
            None => {}
        }
    }

    /// What vislox saw of the restart.
    pub(super) fn restart_event(&mut self, e: restart::Event) {
        match e {
            restart::Event::OutOfService(why) => {
                self.get("restart").detail = Some(format!("out of service ({why})"));
                self.start("online");
            }
            restart::Event::BackOnline(secs) => {
                self.finish("online", "ok", Some(format!("after {secs} s")));
            }
            restart::Event::Loaded(secs, version) => {
                if self.get("online").state == "wait" {
                    self.finish("restart", "ok", Some("no outage seen".into()));
                    self.finish("online", "ok", Some("answered throughout".into()));
                }
                self.finish(
                    "loaded",
                    "ok",
                    Some(format!("after {secs} s · structure {version}")),
                );
            }
        }
    }

    /// The running step failed (else the first one not done).
    pub(super) fn fail(&mut self, why: &str) {
        let id = self
            .steps
            .iter()
            .find(|s| s.state == "run")
            .or_else(|| self.steps.iter().find(|s| s.state == "wait"))
            .map(|s| s.id);
        if let Some(id) = id {
            self.finish(id, "fail", Some(last_lines(why, 1)));
        }
    }
}

/// The step a line of `lox config push` starts (its `say` lines in
/// `config_push.rs`); `verified` ends the verify step.
pub(super) fn push_step(line: &str) -> Option<&'static str> {
    let l = line.trim_start();
    if l.starts_with("Downloading the live config") {
        Some("check")
    } else if l.starts_with("Uploading as") {
        Some("upload")
    } else if l.starts_with("Restarting the program") || l.starts_with("Rebooting the Miniserver") {
        Some("restart")
    } else if l.starts_with("Downloading the config again") {
        Some("verify")
    } else if l.starts_with("Pushed and verified") {
        Some("verified")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lox_lines_move_the_steps_along() {
        let mut p = Progress::apply();
        let state = |p: &mut Progress, id| p.get(id).state;
        p.lox_line("Downloading the live config...");
        assert_eq!(state(&mut p, "check"), "run");
        p.lox_line("Changes (live → compiled):");
        p.lox_line("Uploading as sps_new.zip...");
        assert_eq!(state(&mut p, "check"), "ok");
        assert_eq!(state(&mut p, "upload"), "run");
        p.lox_line("Restarting the program...");
        assert_eq!(state(&mut p, "upload"), "ok");
        assert_eq!(state(&mut p, "restart"), "run");
        p.restart_event(restart::Event::OutOfService("503, restarting".into()));
        assert_eq!(state(&mut p, "online"), "run");
        p.restart_event(restart::Event::BackOnline(7));
        assert_eq!(p.get("online").detail.as_deref(), Some("after 7 s"));
        p.restart_event(restart::Event::Loaded(8, "2026-11-06 10:00:00".into()));
        p.lox_line("Downloading the config again to verify...");
        // What vislox saw stays; the restart is done.
        assert_eq!(state(&mut p, "restart"), "ok");
        assert_eq!(
            p.get("restart").detail.as_deref(),
            Some("out of service (503, restarting)")
        );
        assert_eq!(state(&mut p, "loaded"), "ok");
        assert!(
            p.get("loaded")
                .detail
                .as_deref()
                .unwrap()
                .contains("2026-11-06")
        );
        assert_eq!(state(&mut p, "verify"), "run");
        p.lox_line("Pushed and verified. lox.toml now points at backups/x.Loxone; commit …");
        assert_eq!(state(&mut p, "verify"), "ok");
        assert_eq!(p.log.len(), 6);
    }

    #[test]
    fn a_restart_without_an_outage_and_a_failure() {
        let mut p = Progress::apply();
        p.lox_line("Restarting the program...");
        p.restart_event(restart::Event::Loaded(2, "v2".into()));
        assert_eq!(p.get("restart").state, "ok");
        assert_eq!(
            p.get("online").detail.as_deref(),
            Some("answered throughout")
        );
        p.lox_line("Downloading the config again to verify...");
        p.fail("lox config push failed:\nThe Miniserver does not run what was pushed");
        assert_eq!(p.get("verify").state, "fail");
        assert_eq!(
            p.get("verify").detail.as_deref(),
            Some("The Miniserver does not run what was pushed")
        );
        assert_eq!(p.get("commit").state, "wait");
    }
}
