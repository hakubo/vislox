//! Why the Miniserver refused or dropped a connection, in plain words, and
//! how long to wait before trying again.
//!
//! The Miniserver says why with a WebSocket close code (`4003` … `4008`) or a
//! response code (`401`, `403`, `420`, `423`, `503`, `901`), as listed in
//! Loxone's *Communicating with the Miniserver* (V17.0, p. 8, 30, 31). Some
//! need different handling than "back off and retry": retrying after
//! "blocked for too many failed logins" makes the block longer, and "no event
//! slots" means closing another client, not waiting.

use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// 4003: blocked after too many failed logins.
    Blocked,
    /// 4004 / 4005: a user, or the connected user, was changed.
    UserChanged,
    /// 4006 / 423: the user is disabled.
    UserDisabled,
    /// 4007: a firmware update is running.
    Updating,
    /// 4008: no free event slot (at most 31 clients get state updates).
    NoSlots,
    /// 401: user or password refused.
    Unauthorized,
    /// 403: the user lacks the rights.
    Forbidden,
    /// 420: did not authenticate in time.
    AuthTimeout,
    /// 503: restarting, not ready yet.
    Restarting,
    /// 901: too many connections.
    TooManyConnections,
    /// The out-of-service message: going offline for a reboot or update.
    OutOfService,
}

/// What to do after a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Retry {
    /// Do not try again on its own: another attempt would not help or would
    /// make things worse.
    Stop,
    /// Wait `first` seconds, doubling on each repeat up to `max`.
    Backoff { first: u64, max: u64 },
}

/// Backoff for failures without a known reason.
pub const DEFAULT_RETRY: Retry = Retry::Backoff { first: 1, max: 30 };

impl Reason {
    /// From a WebSocket close code.
    pub fn from_close(code: u16) -> Option<Reason> {
        Some(match code {
            4003 => Reason::Blocked,
            4004 | 4005 => Reason::UserChanged,
            4006 => Reason::UserDisabled,
            4007 => Reason::Updating,
            4008 => Reason::NoSlots,
            _ => return None,
        })
    }

    /// From a response code (an `LL` answer's `Code`, or an HTTP status).
    pub fn from_code(code: u16) -> Option<Reason> {
        Some(match code {
            401 => Reason::Unauthorized,
            403 => Reason::Forbidden,
            420 => Reason::AuthTimeout,
            423 => Reason::UserDisabled,
            503 => Reason::Restarting,
            901 => Reason::TooManyConnections,
            c => return Reason::from_close(c),
        })
    }

    /// From an error message (of `loxone-client`, or of `lox` on the command
    /// line): a known code written as `Code 401`, `(401)`, `HTTP 503`,
    /// `"Code":"423"`, `close code 4003` and the like.
    pub fn from_text(text: &str) -> Option<Reason> {
        let lower = text.to_ascii_lowercase();
        const BEFORE: [&str; 8] = [
            "code ",
            "code: ",
            "code\":\"",
            "code\": \"",
            "code\":",
            "code\": ",
            "(",
            "http ",
        ];
        let b = lower.as_bytes();
        for (i, _) in lower.match_indices(|c: char| c.is_ascii_digit()) {
            // Only where a number starts.
            if i > 0 && b[i - 1].is_ascii_alphanumeric() {
                continue;
            }
            let end = lower[i..]
                .find(|c: char| !c.is_ascii_digit())
                .map_or(lower.len(), |n| i + n);
            if end < lower.len() && b[end].is_ascii_alphabetic() {
                continue;
            }
            let head = &lower[..i];
            if !BEFORE.iter().any(|p| head.ends_with(p)) && !head.ends_with("http error: ") {
                continue;
            }
            if let Some(r) = lower[i..end].parse().ok().and_then(Reason::from_code) {
                return Some(r);
            }
        }
        None
    }

    /// A short label for the live status.
    pub fn label(self) -> &'static str {
        match self {
            Reason::Blocked => "blocked",
            Reason::UserChanged => "user changed",
            Reason::UserDisabled => "user disabled",
            Reason::Updating => "firmware update",
            Reason::NoSlots => "no free slot",
            Reason::Unauthorized => "login refused",
            Reason::Forbidden => "no rights",
            Reason::AuthTimeout => "login timed out",
            Reason::Restarting => "restarting",
            Reason::TooManyConnections => "too many connections",
            Reason::OutOfService => "going offline",
        }
    }

    /// What happened and what to do, in a plain sentence.
    pub fn sentence(self) -> &'static str {
        match self {
            Reason::Blocked => {
                "The Miniserver blocked this login after too many failed attempts. Check the user and \
                 password in lox's configuration (lox setup). Every new attempt makes the block longer."
            }
            Reason::UserChanged => {
                "The Miniserver's users were changed, so the login may no longer be valid. Logging in again."
            }
            Reason::UserDisabled => {
                "The Miniserver user is disabled. Enable it in Loxone Config, or set another user in lox's \
                 configuration (lox setup)."
            }
            Reason::Updating => {
                "The Miniserver is installing a firmware update. Trying again once it is back, which can \
                 take several minutes."
            }
            Reason::NoSlots => {
                "The Miniserver has no free slot for another live connection: at most 31 apps and clients \
                 get state updates at once. Close an app or another client; trying again every few minutes."
            }
            Reason::Unauthorized => {
                "The Miniserver refused the user or password. Check them in lox's configuration (lox \
                 setup). Repeated failed logins get the user blocked."
            }
            Reason::Forbidden => {
                "The Miniserver user lacks the rights for this. Give it the rights in Loxone Config's user \
                 management."
            }
            Reason::AuthTimeout => {
                "The login did not finish in time (the Miniserver allows a few seconds). Trying again."
            }
            Reason::Restarting => {
                "The Miniserver is restarting and not ready yet. Trying again shortly."
            }
            Reason::TooManyConnections => {
                "The Miniserver has too many open connections (48 on a Gen 1, 256 on a Gen 2). Close some \
                 apps or clients; trying again every few minutes."
            }
            Reason::OutOfService => {
                "The Miniserver is going offline for a reboot or an update. Trying again once it is back."
            }
        }
    }

    /// How to retry a live connection.
    pub fn retry(self) -> Retry {
        match self {
            // More attempts would lengthen the block or get the user blocked.
            Reason::Blocked | Reason::Unauthorized => Retry::Stop,
            // Nothing changes until someone changes the user.
            Reason::UserDisabled | Reason::Forbidden => Retry::Stop,
            Reason::UserChanged | Reason::AuthTimeout => Retry::Backoff { first: 1, max: 30 },
            Reason::Restarting => Retry::Backoff { first: 5, max: 30 },
            Reason::OutOfService => Retry::Backoff { first: 10, max: 60 },
            Reason::Updating => Retry::Backoff {
                first: 30,
                max: 120,
            },
            Reason::NoSlots | Reason::TooManyConnections => Retry::Backoff {
                first: 60,
                max: 300,
            },
        }
    }
}

/// An error message from `lox` (Pull, Apply, Sync), with the plain sentence
/// for a known code put first.
pub fn explain(msg: &str) -> String {
    match Reason::from_text(msg) {
        Some(r) => format!("{}\n\n{msg}", r.sentence()),
        None => msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_and_response_codes() {
        assert_eq!(Reason::from_close(4003), Some(Reason::Blocked));
        assert_eq!(Reason::from_close(4005), Some(Reason::UserChanged));
        assert_eq!(Reason::from_close(1000), None);
        assert_eq!(Reason::from_code(423), Some(Reason::UserDisabled));
        assert_eq!(Reason::from_code(4008), Some(Reason::NoSlots));
        assert_eq!(Reason::from_code(200), None);
    }

    #[test]
    fn codes_in_error_messages() {
        let t = |s: &str| Reason::from_text(s);
        assert_eq!(
            t(r#"authwithtoken failed (401): {"LL":{"Code":"401"}}"#),
            Some(Reason::Unauthorized)
        );
        assert_eq!(
            t("io/abc/on: Code 403 (forbidden)"),
            Some(Reason::Forbidden)
        );
        assert_eq!(t("HTTP 503: /jdev/cfg/apiKey"), Some(Reason::Restarting));
        assert_eq!(
            t("WS connect: HTTP error: 901"),
            Some(Reason::TooManyConnections)
        );
        assert_eq!(
            t("the Miniserver closed the connection (close code 4008)"),
            Some(Reason::NoSlots)
        );
        assert_eq!(t(r#"{"LL": {"code": 423}}"#), Some(Reason::UserDisabled));
        // Numbers that are not codes.
        assert_eq!(t("retrying in 401 s"), None);
        assert_eq!(t("uuid 1a2b4003-0000 (4003a)"), None);
        assert_eq!(t("lox config push failed: network down"), None);
    }

    #[test]
    fn stop_on_logins_that_would_get_blocked() {
        assert_eq!(Reason::Blocked.retry(), Retry::Stop);
        assert_eq!(Reason::Unauthorized.retry(), Retry::Stop);
        assert!(matches!(
            Reason::NoSlots.retry(),
            Retry::Backoff { first: 60, .. }
        ));
    }

    #[test]
    fn explain_puts_the_sentence_first() {
        let e = explain("lox config pull failed: Error: gettoken failed (4003)");
        assert!(e.starts_with("The Miniserver blocked"));
        assert!(e.ends_with("gettoken failed (4003)"));
        assert_eq!(explain("no output"), "no output");
    }
}
