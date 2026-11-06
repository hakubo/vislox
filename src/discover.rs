//! Finding Miniservers on the local network: Loxone's UDP discovery.
//!
//! Send one `0x00` byte to UDP port 7070 (broadcast); every Miniserver on
//! the LAN answers with one text line, sent to port **7071** (*Inside the
//! Loxone Miniserver*, "Finding a Miniserver in the local network"):
//!
//! ```text
//! LoxLIVE: Loxone Miniserver 192.168.178.32:80 504F11223344 10.2.3.26 Prog:2019-04-24 21:08:03 Type:0 HwId:A0000 IPv6:,0c112233:10020217/O
//! ```
//!
//! The name has no delimiter: it ends where the `IPv4:port` starts. Then
//! come the serial, the firmware, and `Prog:`, the date of the loaded
//! program. No login is needed, and nothing changes on the Miniserver.
//!
//! The socket listens on 7071 (and sends from it), so a reply to 7071 and a
//! reply to the sender's port both arrive. A real Miniserver (firmware
//! 17.2) answers to 7071 only, so `lox discover`, which listens on a random
//! port, does not see it. The port is bound with `SO_REUSEADDR` (and
//! `SO_REUSEPORT`), so another listener on 7071 does not keep vislox out.
//! When 7071 still cannot be bound, the search says so: replies are then
//! missed, and "no Miniserver answered" would be wrong.
//!
//! Broadcasts stay in the local network: they do not cross VLANs or
//! routers, Remote Connect, or Docker's default bridge network.

use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The port Miniservers listen on for discovery.
const ASK_PORT: u16 = 7070;
/// The port they answer to.
const REPLY_PORT: u16 = 7071;
/// How long to listen for replies, as Loxone's own tools do.
pub const WAIT: Duration = Duration::from_secs(3);
/// A search this recent is reused by [`Cache`].
const CACHE_FOR: Duration = Duration::from_secs(60);

/// What a search found.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Search {
    pub found: Vec<Reply>,
    /// Why replies may be missing (port 7071 taken).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One Miniserver's reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub name: String,
    /// Its address and web port, as it reports them.
    pub ip: String,
    pub port: u16,
    /// Upper case, without separators (`504F94A0E454`).
    pub serial: String,
    pub firmware: String,
    /// When the loaded program was saved (`2019-04-24 21:08:03`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// The hardware revision (`A0000`; `100000` is a Miniserver Go).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hw_id: Option<String>,
}

/// A serial as vislox compares it: upper case, without `:` or `-`.
pub fn norm_serial(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Whether `s` looks like a Miniserver serial (its MAC: 12 hex digits).
pub fn is_serial(s: &str) -> bool {
    let n = norm_serial(s);
    n.len() == 12 && n.chars().all(|c| c.is_ascii_hexdigit()) && n == s.to_ascii_uppercase()
}

/// Read one discovery reply.
pub fn parse_reply(line: &str) -> Option<Reply> {
    let rest = line.trim().strip_prefix("LoxLIVE:")?;
    let words: Vec<&str> = rest.split_whitespace().collect();
    let at = words.iter().position(|w| ip_port(w).is_some())?;
    let (ip, port) = ip_port(words[at])?;
    let serial = norm_serial(words.get(at + 1)?);
    if serial.is_empty() {
        return None;
    }
    let firmware = words.get(at + 2).map(|s| s.to_string()).unwrap_or_default();
    let tail = &words[(at + 3).min(words.len())..];
    let field = |key: &str| {
        tail.iter()
            .position(|w| w.starts_with(key))
            .map(|i| (i, tail[i][key.len()..].to_string()))
    };
    // `Prog:` has a date and a time, two words.
    let program = field("Prog:").map(|(i, date)| match tail.get(i + 1) {
        Some(t) if t.len() == 8 && t.chars().all(|c| c.is_ascii_digit() || c == ':') => {
            format!("{date} {t}")
        }
        _ => date,
    });
    Some(Reply {
        name: words[..at].join(" "),
        ip,
        port,
        serial,
        firmware,
        program: program.filter(|p| !p.is_empty()),
        hw_id: field("HwId:").map(|(_, h)| h).filter(|h| !h.is_empty()),
    })
}

/// `192.168.1.77:80`
fn ip_port(w: &str) -> Option<(String, u16)> {
    let (ip, port) = w.rsplit_once(':')?;
    ip.parse::<Ipv4Addr>().ok()?;
    Some((ip.to_string(), port.parse().ok()?))
}

/// Ask the LAN for Miniservers and collect the replies for `wait`, one per
/// serial. Blocking; errors (no network) give an empty list and a note.
pub fn search(wait: Duration) -> Search {
    search_at(
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, REPLY_PORT)),
        SocketAddr::from((Ipv4Addr::BROADCAST, ASK_PORT)),
        wait,
    )
}

/// A UDP socket at `addr` that shares its port with other listeners.
fn bind_shared(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.bind(&addr.into())?;
    Ok(s.into())
}

/// Listen at `bind` (and send from it), ask `to`.
fn search_at(bind: SocketAddr, to: SocketAddr, wait: Duration) -> Search {
    let mut note = None;
    let socket = bind_shared(bind).or_else(|e| {
        if bind.port() == 0 {
            return Err(e);
        }
        note = Some(format!(
            "UDP port {} is taken by another program ({e}). Miniservers answer there, so \
             their replies were missed.",
            bind.port()
        ));
        bind_shared(SocketAddr::new(bind.ip(), 0))
    });
    let Ok(socket) = socket else {
        return Search {
            found: Vec::new(),
            note: Some("cannot open a UDP socket".into()),
        };
    };
    if socket.send_to(&[0x00], to).is_err() {
        return Search {
            found: Vec::new(),
            note: Some(format!("cannot send the discovery broadcast to {to}")),
        };
    }
    let deadline = Instant::now() + wait;
    let mut found: Vec<Reply> = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || socket.set_read_timeout(Some(left)).is_err() {
            break;
        }
        let Ok((n, _)) = socket.recv_from(&mut buf) else {
            break;
        };
        if let Some(r) = parse_reply(&String::from_utf8_lossy(&buf[..n]))
            && !found.iter().any(|f| f.serial == r.serial)
        {
            found.push(r);
        }
    }
    Search { found, note }
}

/// The last search, reused for a minute: an unreachable address is probed
/// at every Pull, Apply and Sync, and each search waits [`WAIT`].
#[derive(Default)]
pub struct Cache {
    last: Mutex<Option<(Instant, Arc<Search>)>>,
}

impl Cache {
    /// A cache that already holds `s` (tests: no broadcast).
    #[cfg(test)]
    pub fn primed(s: Search) -> Cache {
        Cache {
            last: Mutex::new(Some((Instant::now(), Arc::new(s)))),
        }
    }

    pub async fn get(&self) -> Arc<Search> {
        if let Some((at, s)) = self.last.lock().unwrap().as_ref()
            && at.elapsed() < CACHE_FOR
        {
            return s.clone();
        }
        let s = Arc::new(
            tokio::task::spawn_blocking(|| search(WAIT))
                .await
                .unwrap_or_default(),
        );
        *self.last.lock().unwrap() = Some((Instant::now(), s.clone()));
        s
    }
}

/// The web address for a discovery reply: HTTPS on the standard port, as
/// lox connects by default, unless the reply names another web port.
pub fn web_url(r: &Reply) -> String {
    if r.port == 80 {
        format!("https://{}", r.ip)
    } else {
        format!("http://{}:{}", r.ip, r.port)
    }
}

/// The list `vislox --discover` prints.
pub fn describe(search: &Search) -> String {
    let found = &search.found;
    let note = search
        .note
        .as_deref()
        .map(|n| format!("\nNote: {n}"))
        .unwrap_or_default();
    if found.is_empty() {
        if let Some(n) = &search.note {
            return format!("Could not search the local network properly: {n}");
        }
        return "No Miniserver answered on the local network (UDP 7070/7071). Broadcasts do not \
                cross VLANs, routers or Docker's bridge network."
            .into();
    }
    let mut out = String::from("Miniservers on the local network:\n");
    for r in found {
        out.push_str(&format!(
            "  {}  {}:{}  firmware {}  {}{}\n",
            r.serial,
            r.ip,
            r.port,
            r.firmware,
            r.name,
            r.program
                .as_deref()
                .map(|p| format!("  (program of {p})"))
                .unwrap_or_default(),
        ));
    }
    let r = &found[0];
    out.push_str(&format!(
        "\nTo use one with vislox, tell lox about it, then fetch its config:\n  \
         lox ctx add <name> --host {} --user <user> --pass <pass> --serial {}\n  \
         lox config init ~/loxone-config && lox config pull\n  \
         vislox ~/loxone-config{note}",
        web_url(r),
        r.serial
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "LoxLIVE: Loxone Miniserver 192.168.178.32:80 504F11223344 10.2.3.26 Prog:2019-04-24 21:08:03 Type:0 HwId:A0000 IPv6:,0c112233:10020217/O";

    #[test]
    fn a_reply_is_read() {
        let r = parse_reply(DOC).unwrap();
        assert_eq!(r.name, "Loxone Miniserver");
        assert_eq!((r.ip.as_str(), r.port), ("192.168.178.32", 80));
        assert_eq!(r.serial, "504F11223344");
        assert_eq!(r.firmware, "10.2.3.26");
        assert_eq!(r.program.as_deref(), Some("2019-04-24 21:08:03"));
        assert_eq!(r.hw_id.as_deref(), Some("A0000"));
    }

    #[test]
    fn a_real_miniserver_s_reply_is_read() {
        // Received on port 7071 from a Miniserver Gen 2 (firmware 17.2.10.21).
        let r = parse_reply("LoxLIVE: MSE450/Olek - Kowalskie 192.168.1.120:80 504F94A0E454 17.2.10.21 Prog:2026-10-31 19:59:59 Type:0 HwId:20000000 IPv6:fe80::524f:94ff:fea0:e454 DHCP:1,13000003:0/X,13000001:15020918/O").unwrap();
        assert_eq!(r.name, "MSE450/Olek - Kowalskie");
        assert_eq!(r.ip, "192.168.1.120");
        assert_eq!(r.serial, "504F94A0E454");
        assert_eq!(r.firmware, "17.2.10.21");
        assert_eq!(r.program.as_deref(), Some("2026-10-31 19:59:59"));
        assert_eq!(r.hw_id.as_deref(), Some("20000000"));
        // Music and Audio servers answer the same broadcast; they are not Miniservers.
        assert!(
            parse_reply(
                "LoxMedia: as80BA IP:192.168.1.121 MAC:50:4f:94:f0:80:ba Product:MusicserverII"
            )
            .is_none()
        );
        assert!(parse_reply(r#"{"t":"Audioserver","s":"E6:59:CF:1E:B8:DB"}"#).is_none());
    }

    #[test]
    fn a_name_with_spaces_and_digits_ends_at_the_address() {
        let r = parse_reply(
            "LoxLIVE: Haus 2 OG 10.0.0.5:8080 50:4f:94:a0:e4:54 17.2.10.21 Prog:2026-11-01 10:00:00\n",
        )
        .unwrap();
        assert_eq!(r.name, "Haus 2 OG");
        assert_eq!(r.port, 8080);
        assert_eq!(r.serial, "504F94A0E454");
        assert_eq!(r.hw_id, None);
        assert!(parse_reply("hello").is_none());
        assert!(parse_reply("LoxLIVE: no address here").is_none());
    }

    #[test]
    fn serials_are_compared_without_separators() {
        assert_eq!(norm_serial("50:4f:94:a0:e4:54"), "504F94A0E454");
        assert!(is_serial("504F94A0E454"));
        assert!(!is_serial("192_168_1_77"));
        assert!(!is_serial("50:4F:94:A0:E4:54"));
    }

    #[test]
    fn replies_are_collected_from_a_local_stand_in() {
        // A stand-in Miniserver on loopback answers the sender's port.
        let ms = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = ms.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut b = [0u8; 4];
            let (n, from) = ms.recv_from(&mut b).unwrap();
            assert_eq!(&b[..n], &[0]);
            ms.send_to(DOC.as_bytes(), from).unwrap();
            ms.send_to(DOC.as_bytes(), from).unwrap();
        });
        // Not the real 0.0.0.0:7071: replies from the LAN would get in.
        let s = search_at(
            "127.0.0.1:0".parse().unwrap(),
            at,
            Duration::from_millis(500),
        );
        t.join().unwrap();
        assert_eq!(s.found.len(), 1, "one entry per serial");
        assert_eq!(s.found[0].serial, "504F11223344");
        assert_eq!(s.note, None);
        let text = describe(&s);
        assert!(text.contains("192.168.178.32:80"), "{text}");
        assert!(text.contains("--host https://192.168.178.32 "), "{text}");
        assert!(describe(&Search::default()).contains("No Miniserver"));
    }

    #[test]
    fn a_taken_reply_port_is_said() {
        let s = Search {
            found: Vec::new(),
            note: Some("UDP port 7071 is taken".into()),
        };
        assert!(describe(&s).contains("7071 is taken"), "{}", describe(&s));
        assert!(!describe(&s).contains("No Miniserver answered"));
    }

    #[test]
    fn the_reply_s_web_port_is_kept() {
        let mut r = parse_reply(DOC).unwrap();
        assert_eq!(web_url(&r), "https://192.168.178.32");
        r.port = 8080;
        assert_eq!(web_url(&r), "http://192.168.178.32:8080");
    }
}
